use super::*;
use crate::{DataFile, IoMetrics};
use bytes::Bytes;
use std::sync::Barrier;
use tokio::sync::oneshot;

const CHUNK_PAYLOAD_BYTES: u64 = CHUNK_BYTES - BLOCK_BYTES;

fn allocated_bytes(regions: &[DiskRegion]) -> u64 {
    regions
        .iter()
        .map(|region| region.range().end - region.range().start)
        .sum()
}

fn assert_empty(allocator: &DiskAllocator, capacity: u64) {
    let free = allocator.free.lock().unwrap();
    assert_eq!(free.chunks, BTreeMap::from([(0, capacity / CHUNK_BYTES)]));
    assert!(free.blocks.is_empty());
    assert_eq!(
        free.available_blocks * BLOCK_BYTES,
        capacity / CHUNK_BYTES * CHUNK_PAYLOAD_BYTES
    );
}

#[test]
fn reserves_tiny_through_100_mib_without_using_index_pages() {
    let capacity = 102 * CHUNK_BYTES;
    let allocator = DiskAllocator::new(capacity).unwrap();
    for bytes in [
        1,
        4,
        4095,
        4096,
        4097,
        CHUNK_PAYLOAD_BYTES,
        CHUNK_BYTES,
        40 * CHUNK_BYTES,
        100 * CHUNK_BYTES,
    ] {
        let regions = allocator.reserve(bytes).unwrap();
        assert_eq!(allocated_bytes(&regions), bytes.next_multiple_of(BLOCK_BYTES));
        let mut previous_end = 0;
        for region in &regions {
            let range = region.range();
            assert!(range.start >= previous_end);
            assert!(range.end <= capacity);
            assert!(range.start.is_multiple_of(BLOCK_BYTES));
            assert!(range.end.is_multiple_of(BLOCK_BYTES));
            assert_ne!(range.start % CHUNK_BYTES, 0);
            assert_eq!(range.start / CHUNK_BYTES, (range.end - 1) / CHUNK_BYTES);
            previous_end = range.end;
        }
        drop(regions);
        assert_empty(&allocator, capacity);
    }
}

#[test]
fn rejects_invalid_sizes_and_failed_reservation_does_not_consume_space() {
    for capacity in [0, 1, BLOCK_BYTES, CHUNK_BYTES + BLOCK_BYTES, 1 << 63, u64::MAX] {
        assert!(DiskAllocator::new(capacity).is_none());
    }
    let allocator = DiskAllocator::new(CHUNK_BYTES).unwrap();
    for bytes in [0, CHUNK_PAYLOAD_BYTES + 1, u64::MAX] {
        assert!(allocator.reserve(bytes).is_none());
        assert_empty(&allocator, CHUNK_BYTES);
    }
    let region = allocator.reserve(1).unwrap();
    let before = allocator.free.lock().unwrap().available_blocks;
    assert!(allocator.reserve(CHUNK_PAYLOAD_BYTES).is_none());
    assert_eq!(allocator.free.lock().unwrap().available_blocks, before);
    let rest = allocator.reserve(CHUNK_PAYLOAD_BYTES - BLOCK_BYTES).unwrap();
    assert!(allocator.reserve(1).is_none());
    drop((region, rest));
    assert_empty(&allocator, CHUNK_BYTES);
}

#[test]
fn large_entry_tail_shares_a_subdivided_chunk() {
    let allocator = DiskAllocator::new(2 * CHUNK_BYTES).unwrap();
    let large = allocator.reserve(CHUNK_PAYLOAD_BYTES + 17).unwrap();
    assert_eq!(large.len(), 2);
    assert_eq!(large[0].range(), BLOCK_BYTES..CHUNK_BYTES);
    assert_eq!(
        large[1].range(),
        CHUNK_BYTES + BLOCK_BYTES..CHUNK_BYTES + 2 * BLOCK_BYTES
    );
    let small = allocator.reserve(7).unwrap();
    assert_eq!(
        small[0].range(),
        CHUNK_BYTES + 2 * BLOCK_BYTES..CHUNK_BYTES + 3 * BLOCK_BYTES
    );
    drop(large);
    let whole = allocator.reserve(CHUNK_PAYLOAD_BYTES).unwrap();
    assert_eq!(whole[0].range(), BLOCK_BYTES..CHUNK_BYTES);
    drop((small, whole));
    assert_empty(&allocator, 2 * CHUNK_BYTES);
}

#[test]
fn fragmented_blocks_can_satisfy_large_reservations_without_relocation() {
    let allocator = DiskAllocator::new(CHUNK_BYTES).unwrap();
    let mut entries: Vec<_> = (0..PAYLOAD_BLOCKS_PER_CHUNK)
        .map(|_| Some(allocator.reserve(BLOCK_BYTES).unwrap()))
        .collect();
    let mut freed = 0;
    for index in (0..entries.len()).step_by(2) {
        entries[index].take();
        freed += 1;
    }
    let fragmented = allocator.reserve(freed * BLOCK_BYTES).unwrap();
    assert_eq!(fragmented.len() as u64, freed);
    assert!(allocator.reserve(1).is_none());
    for region in &fragmented {
        assert_eq!(region.range().end - region.range().start, BLOCK_BYTES);
    }
    drop((entries, fragmented));
    assert_empty(&allocator, CHUNK_BYTES);
}

#[test]
fn packed_entries_and_readers_share_one_physical_allocation() {
    let allocator = DiskAllocator::new(CHUNK_BYTES).unwrap();
    let region = Arc::new(allocator.reserve(4 + 17 + 2000).unwrap().pop().unwrap());
    let entries = [region.clone(), region.clone(), region.clone()];
    let reader = entries[1].read_guard();
    let rest = allocator.reserve(CHUNK_PAYLOAD_BYTES - BLOCK_BYTES).unwrap();
    drop(region);
    drop(entries);
    assert!(allocator.reserve(1).is_none());
    drop(reader);
    let reused = allocator.reserve(BLOCK_BYTES).unwrap();
    assert_eq!(reused[0].range(), BLOCK_BYTES..2 * BLOCK_BYTES);
    drop((rest, reused));
    assert_empty(&allocator, CHUNK_BYTES);
}

#[test]
fn eviction_waits_for_all_concurrent_readers() {
    let allocator = DiskAllocator::new(CHUNK_BYTES).unwrap();
    let region = allocator.reserve(CHUNK_PAYLOAD_BYTES).unwrap().pop().unwrap();
    let final_reader = region.read_guard();
    let barrier = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let reader = region.read_guard();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(reader.range(), BLOCK_BYTES..CHUNK_BYTES);
                drop(reader);
            });
        }
        drop(region);
        assert!(allocator.reserve(1).is_none());
        barrier.wait();
    });
    assert!(allocator.reserve(1).is_none());
    drop(final_reader);
    assert_empty(&allocator, CHUNK_BYTES);
}

#[test]
fn unknown_completion_never_releases_reserved_storage() {
    let allocator = DiskAllocator::new(CHUNK_BYTES).unwrap();
    let region = allocator.reserve(CHUNK_PAYLOAD_BYTES).unwrap().pop().unwrap();
    region.quarantine();
    assert!(allocator.reserve(1).is_none());
    let free = allocator.free.lock().unwrap();
    assert_eq!(free.available_blocks, 0);
    assert!(free.chunks.is_empty());
    assert!(free.blocks.is_empty());
}

#[test]
fn forty_tib_initialization_is_sparse() {
    let capacity = 40 * (1u64 << 40);
    let allocator = DiskAllocator::new(capacity).unwrap();
    assert_empty(&allocator, capacity);
    let large = allocator.reserve(100 * CHUNK_BYTES).unwrap();
    let small = allocator.reserve(1).unwrap();
    {
        let free = allocator.free.lock().unwrap();
        assert_eq!(free.chunks.len(), 1);
        assert_eq!(free.blocks.len(), 1);
        assert_eq!(large.len(), 101);
    }
    drop((large, small));
    assert_empty(&allocator, capacity);
}

#[test]
fn randomized_reuse_matches_a_block_ownership_model() {
    let capacity = 5 * CHUNK_BYTES;
    let allocator = DiskAllocator::new(capacity).unwrap();
    let mut occupied = vec![false; (capacity / BLOCK_BYTES) as usize];
    for index in (0..occupied.len()).step_by(BLOCKS_PER_CHUNK as usize) {
        occupied[index] = true; // metadata pages
    }
    let mut entries = Vec::<Vec<DiskRegion>>::new();
    let mut random = 0x4de3_8129_f307_4a65u64;
    for _ in 0..10_000 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        if !entries.is_empty() && random.is_multiple_of(3) {
            let regions = entries.swap_remove(random as usize % entries.len());
            for region in &regions {
                let range = region.range();
                for block in range.start / BLOCK_BYTES..range.end / BLOCK_BYTES {
                    assert!(occupied[block as usize]);
                    occupied[block as usize] = false;
                }
            }
            drop(regions);
        } else {
            let bytes = random % (2 * CHUNK_BYTES) + 1;
            let free_blocks = occupied.iter().filter(|&&used| !used).count() as u64;
            match allocator.reserve(bytes) {
                Some(regions) => {
                    assert_eq!(allocated_bytes(&regions), bytes.next_multiple_of(BLOCK_BYTES));
                    for region in &regions {
                        let range = region.range();
                        for block in range.start / BLOCK_BYTES..range.end / BLOCK_BYTES {
                            assert!(!occupied[block as usize]);
                            occupied[block as usize] = true;
                        }
                    }
                    entries.push(regions);
                }
                None => assert!(bytes.div_ceil(BLOCK_BYTES) > free_blocks),
            }
        }
        assert_eq!(
            allocator.free.lock().unwrap().available_blocks,
            occupied.iter().filter(|&&used| !used).count() as u64
        );
    }
    drop(entries);
    assert_empty(&allocator, capacity);
}

#[test]
fn concurrent_reservation_and_release_restores_the_arena() {
    let capacity = 16 * CHUNK_BYTES;
    let allocator = DiskAllocator::new(capacity).unwrap();
    std::thread::scope(|scope| {
        for worker in 0..8 {
            let allocator = &allocator;
            scope.spawn(move || {
                for iteration in 0..1000 {
                    let regions = allocator
                        .reserve(1 + (iteration * 7919 + worker) % CHUNK_BYTES)
                        .unwrap();
                    let readers: Vec<_> = regions.iter().map(DiskRegion::read_guard).collect();
                    drop(regions);
                    drop(readers);
                }
            });
        }
    });
    assert_empty(&allocator, capacity);
}

#[tokio::test]
async fn packed_unaligned_reads_remain_valid_after_eviction_and_reuse() {
    let temporary = tempfile::tempdir().unwrap();
    let file = DataFile::open(temporary.path(), CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    let allocator = DiskAllocator::new(file.capacity()).unwrap();
    let region = Arc::new(allocator.reserve(3 + 11).unwrap().pop().unwrap());
    let rest = allocator.reserve(CHUNK_PAYLOAD_BYTES - BLOCK_BYTES).unwrap();
    let mut block = vec![0; BLOCK_BYTES as usize];
    block[..3].copy_from_slice(b"one");
    block[3..14].copy_from_slice(b"second item");
    file.write_at(region.range().start, &Bytes::from(block)).await.unwrap();
    let first_reader = region.read_guard();
    let second_reader = region.read_guard();
    drop(region); // whole-block eviction
    assert!(allocator.reserve(1).is_none());
    let (first, second) = tokio::join!(
        file.read_at(first_reader.range().start, 3),
        file.read_at(second_reader.range().start + 3, 11),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first, &b"one"[..]);
    assert_eq!(second, &b"second item"[..]);
    drop(first_reader);
    assert!(allocator.reserve(1).is_none());
    drop(second_reader);
    let reused = allocator.reserve(BLOCK_BYTES).unwrap();
    assert_eq!(reused[0].range(), BLOCK_BYTES..2 * BLOCK_BYTES);
    file.write_at(reused[0].range().start, &Bytes::from(vec![0xab; BLOCK_BYTES as usize]))
        .await
        .unwrap();
    assert_eq!(first, &b"one"[..]);
    assert_eq!(second, &b"second item"[..]);
    drop((rest, reused));
    assert_empty(&allocator, CHUNK_BYTES);
}

#[tokio::test]
async fn abandoned_writer_result_keeps_reservations_until_owner_finishes() {
    let temporary = tempfile::tempdir().unwrap();
    let file = DataFile::open(temporary.path(), CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    let allocator = DiskAllocator::new(file.capacity()).unwrap();
    let regions = allocator.reserve(CHUNK_PAYLOAD_BYTES).unwrap();
    let (result, receiver) = oneshot::channel();
    let (proceed, wait) = oneshot::channel();
    let writer = tokio::spawn(async move {
        wait.await.unwrap();
        for region in &regions {
            let range = region.range();
            file.write_at(
                range.start,
                &Bytes::from(vec![0x55; (range.end - range.start) as usize]),
            )
            .await
            .unwrap();
        }
        // An abandoned result drops the regions only after all writes completed.
        let _ = result.send(regions);
    });
    drop(receiver);
    assert!(allocator.reserve(1).is_none());
    proceed.send(()).unwrap();
    writer.await.unwrap();
    assert_empty(&allocator, CHUNK_BYTES);
}

#[tokio::test]
async fn writes_100_mib_in_multiple_chunks_and_reads_only_requested_slices() {
    let temporary = tempfile::tempdir().unwrap();
    let capacity = 102 * CHUNK_BYTES;
    let file = DataFile::open(temporary.path(), capacity, IoMetrics::noop())
        .await
        .unwrap();
    let allocator = DiskAllocator::new(capacity).unwrap();
    let length = 100 * CHUNK_BYTES;
    let regions = allocator.reserve(length).unwrap();
    let original = Bytes::from((0..length).map(|offset| (offset % 251) as u8).collect::<Vec<_>>());
    let mut offset = 0;
    for region in &regions {
        let range = region.range();
        let end = offset + (range.end - range.start) as usize;
        file.write_at(range.start, &original.slice(offset..end)).await.unwrap();
        offset = end;
    }
    for request in [
        13..41,
        CHUNK_PAYLOAD_BYTES - 3..CHUNK_PAYLOAD_BYTES + 19,
        length - 17..length,
    ] {
        let mut result = Vec::new();
        let mut logical_start = 0;
        let mut requested_io_bytes = 0;
        for region in &regions {
            let range = region.range();
            let logical_end = logical_start + range.end - range.start;
            let start = request.start.max(logical_start);
            let end = request.end.min(logical_end);
            if start < end {
                let guard = region.read_guard();
                let read = file
                    .read_at(guard.range().start + start - logical_start, (end - start) as usize)
                    .await
                    .unwrap();
                requested_io_bytes += end - start;
                result.extend_from_slice(&read);
            }
            logical_start = logical_end;
        }
        assert_eq!(requested_io_bytes, request.end - request.start);
        assert_eq!(result, original.slice(request.start as usize..request.end as usize));
    }
    drop(regions);
    assert_empty(&allocator, capacity);
}
