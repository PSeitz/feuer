use super::*;
use crate::test_metrics::{registry, value};
use std::sync::Barrier;

#[test]
fn recovery_claims_survive_release_but_inspection_alone_does_not_claim_a_chunk() {
    let allocator = DiskChunkAllocator::for_disk_range(3 * CHUNK_BYTES..7 * CHUNK_BYTES).unwrap();
    allocator.start_recovery(3 * CHUNK_BYTES, 7 * CHUNK_BYTES);
    let inspected = allocator.reserve_for_recovery(5).unwrap();
    assert_eq!(inspected.range(), 5 * CHUNK_BYTES..6 * CHUNK_BYTES);
    assert!(allocator.reserve_for_recovery(5).is_none());
    let write_region = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    assert_eq!(write_region.range().start, 3 * CHUNK_BYTES);
    drop(write_region);
    assert!(allocator.reserve_for_recovery(3).is_none());
    drop(inspected);
    let recovered = allocator.reserve_for_recovery(5).unwrap();
    recovered.mark_recovered();
    let guard = recovered.read_guard();
    drop(recovered);
    assert_eq!(allocator.available_bytes(), 3 * CHUNK_BYTES);
    drop(guard);
    assert_eq!(allocator.available_bytes(), 4 * CHUNK_BYTES);
    assert!(allocator.reserve_for_recovery(5).is_none());
    // All split free runs coalesce again; claims only exclude recovery, never new allocation.
    let all = allocator.reserve_chunks(4).unwrap();
    assert_eq!(
        all.iter().map(|region| region.range().start).collect::<Vec<_>>(),
        (3..7).map(|chunk| chunk * CHUNK_BYTES).collect::<Vec<_>>()
    );
    drop(all);
    allocator.finish_recovery();
    assert!(allocator.reserve_for_recovery(4).is_none());
}

#[test]
fn writes_and_recovery_cannot_reserve_the_same_chunk_concurrently() {
    for _ in 0..100 {
        let allocator = DiskChunkAllocator::new(CHUNK_BYTES).unwrap();
        allocator.start_recovery(0, CHUNK_BYTES);
        let barrier = Arc::new(Barrier::new(2));
        let writer = allocator.clone();
        let writer_barrier = barrier.clone();
        let task = std::thread::spawn(move || {
            writer_barrier.wait();
            writer.reserve_chunks(1)
        });
        barrier.wait();
        let recovered = allocator.reserve_for_recovery(0);
        let write_regions = task.join().unwrap();
        assert_ne!(recovered.is_some(), write_regions.is_some());
    }
}

#[test]
fn recovered_chunks_exclude_inspection_and_follow_shared_ownership_until_reuse() {
    let (registry, backend) = registry();
    let allocator = DiskChunkAllocator::with_metrics(0..CHUNK_BYTES, DiskMetrics::new(&backend)).unwrap();
    let recovered_chunks = || value(&registry, "feuer_disk_recovered_chunks", &[]);
    allocator.start_recovery(0, CHUNK_BYTES);
    let inspected = allocator.reserve_for_recovery(0).unwrap();
    assert_eq!(recovered_chunks(), 0.0);
    drop(inspected);
    assert_eq!(recovered_chunks(), 0.0);

    let recovered = allocator.reserve_for_recovery(0).unwrap();
    let shared = recovered.slice(0..4096);
    recovered.mark_recovered();
    shared.mark_recovered();
    assert_eq!(recovered_chunks(), 1.0);
    let guard = shared.read_guard();
    let other_guard = guard.clone();
    allocator.finish_recovery();
    drop(recovered);
    drop(shared);
    drop(guard);
    assert_eq!(recovered_chunks(), 1.0);
    drop(other_guard);
    assert_eq!(recovered_chunks(), 0.0);

    let written = allocator.reserve_chunks(1).unwrap();
    assert_eq!(recovered_chunks(), 0.0);
    drop(written);
    drop(allocator);
    assert_eq!(recovered_chunks(), 0.0);
}

#[test]
fn capacity_metrics_follow_whole_chunk_ownership_and_allocator_drop() {
    let (registry, backend) = registry();
    let metrics = DiskMetrics::new(&backend);
    let allocator = DiskChunkAllocator::with_metrics(0..2 * CHUNK_BYTES, metrics.clone()).unwrap();
    let other = DiskChunkAllocator::with_metrics(2 * CHUNK_BYTES..3 * CHUNK_BYTES, metrics).unwrap();
    let chunks = |state| value(&registry, "feuer_disk_chunks", &[("state", state)]);
    assert_eq!(chunks("free"), 3.0);
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    assert_eq!(chunks("allocated"), 1.0);
    let read_guard = chunk.read_guard();
    drop(chunk);
    assert_eq!(chunks("free"), 2.0);
    drop(read_guard);
    assert_eq!(chunks("free"), 3.0);
    assert_eq!(chunks("allocated"), 0.0);
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let same_chunk = chunk.slice(chunk.range());
    drop(chunk);
    assert_eq!(chunks("allocated"), 1.0);
    drop(same_chunk);
    assert_eq!(chunks("allocated"), 0.0);
    drop(allocator);
    assert_eq!(chunks("free"), 1.0);
    drop(other);
    assert_eq!(chunks("free"), 0.0);
}

fn assert_all_chunks_free(allocator: &DiskChunkAllocator, capacity: u64) {
    let free = allocator.free.lock().unwrap();
    assert_eq!(
        free.free_chunk_count_by_start,
        BTreeMap::from([(0, capacity / CHUNK_BYTES)])
    );
    assert_eq!(free.available_chunks * CHUNK_BYTES, capacity);
}

#[test]
fn reserves_whole_chunks_through_100_mib() {
    let capacity = 102 * CHUNK_BYTES;
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    for count in [1, 2, 40, 100, 102] {
        let regions = allocator.reserve_chunks(count).unwrap();
        assert_eq!(regions.len() as u64, count);
        for (index, region) in regions.iter().enumerate() {
            assert_eq!(
                region.range(),
                index as u64 * CHUNK_BYTES..(index as u64 + 1) * CHUNK_BYTES
            );
        }
        drop(regions);
        assert_all_chunks_free(&allocator, capacity);
    }
}

#[test]
fn free_chunk_counts_are_independent_of_start() {
    let allocator = DiskChunkAllocator::for_disk_range(7 * CHUNK_BYTES..12 * CHUNK_BYTES).unwrap();
    assert_eq!(
        allocator.free.lock().unwrap().free_chunk_count_by_start,
        BTreeMap::from([(7, 5)])
    );
    let chunks = allocator.reserve_chunks(2).unwrap();
    assert_eq!(chunks[0].range(), 7 * CHUNK_BYTES..8 * CHUNK_BYTES);
    assert_eq!(chunks[1].range(), 8 * CHUNK_BYTES..9 * CHUNK_BYTES);
    assert_eq!(
        allocator.free.lock().unwrap().free_chunk_count_by_start,
        BTreeMap::from([(9, 3)])
    );
    drop(chunks);
    assert_eq!(
        allocator.free.lock().unwrap().free_chunk_count_by_start,
        BTreeMap::from([(7, 5)])
    );
}

#[test]
fn rejects_invalid_sizes_and_reserves_all_or_nothing() {
    for capacity in [0, 1, CHUNK_BYTES - 1, CHUNK_BYTES + 1, 1 << 63, u64::MAX] {
        assert!(DiskChunkAllocator::new(capacity).is_none());
    }
    let allocator = DiskChunkAllocator::new(2 * CHUNK_BYTES).unwrap();
    for count in [0, 3, u64::MAX] {
        assert!(allocator.reserve_chunks(count).is_none());
        assert_all_chunks_free(&allocator, 2 * CHUNK_BYTES);
    }
    let first = allocator.reserve_chunks(1).unwrap();
    assert!(allocator.reserve_chunks(2).is_none());
    assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    let second = allocator.reserve_chunks(1).unwrap();
    assert!(allocator.reserve_chunks(1).is_none());
    drop((first, second));
    assert_all_chunks_free(&allocator, 2 * CHUNK_BYTES);
}

#[test]
fn entry_regions_and_readers_prevent_whole_chunk_reuse() {
    let allocator = DiskChunkAllocator::new(CHUNK_BYTES).unwrap();
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let first = chunk.slice(1..2);
    let second = chunk.slice(2..4);
    let read_guard = first.read_guard();
    drop((chunk, first));
    assert!(allocator.reserve_chunks(1).is_none());
    drop(second);
    assert!(allocator.reserve_chunks(1).is_none());
    assert_eq!(read_guard.range(), 1..2);
    drop(read_guard);
    assert_all_chunks_free(&allocator, CHUNK_BYTES);
}

#[test]
fn reuse_waits_for_all_concurrent_readers() {
    let allocator = DiskChunkAllocator::new(CHUNK_BYTES).unwrap();
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let final_read_guard = chunk.read_guard();
    let barrier = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let read_guard = chunk.read_guard();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(read_guard.range(), 0..CHUNK_BYTES);
            });
        }
        drop(chunk);
        assert!(allocator.reserve_chunks(1).is_none());
        barrier.wait();
    });
    assert!(allocator.reserve_chunks(1).is_none());
    drop(final_read_guard);
    assert_all_chunks_free(&allocator, CHUNK_BYTES);
}

#[test]
fn forty_tib_initialization_is_sparse() {
    let capacity = 40 * (1u64 << 40);
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    assert_all_chunks_free(&allocator, capacity);
    let large = allocator.reserve_chunks(100).unwrap();
    assert_eq!(allocator.free.lock().unwrap().free_chunk_count_by_start.len(), 1);
    drop(large);
    assert_all_chunks_free(&allocator, capacity);
}

#[test]
fn fragmented_free_chunks_coalesce_after_reuse() {
    let allocator = DiskChunkAllocator::new(5 * CHUNK_BYTES).unwrap();
    let mut regions = allocator.reserve_chunks(5).unwrap().into_iter();
    drop(regions.next());
    let second = regions.next().unwrap();
    drop(regions.next());
    let fourth = regions.next().unwrap();
    drop(regions.next());
    let scattered = allocator.reserve_chunks(3).unwrap();
    assert_eq!(
        scattered.iter().map(|region| region.range().start).collect::<Vec<_>>(),
        [0, 2 * CHUNK_BYTES, 4 * CHUNK_BYTES]
    );
    drop((scattered, second, fourth));
    assert_all_chunks_free(&allocator, 5 * CHUNK_BYTES);
}

#[test]
fn randomized_reuse_matches_whole_chunk_ownership() {
    let capacity = 16 * CHUNK_BYTES;
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    let mut occupied = [false; 16];
    let mut entries = Vec::<Vec<DiskRegion>>::new();
    let mut random = 0x4de3_8129_f307_4a65u64;
    for _ in 0..10_000 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        if !entries.is_empty() && random.is_multiple_of(3) {
            let regions = entries.swap_remove(random as usize % entries.len());
            for region in &regions {
                let chunk_index = (region.range().start / CHUNK_BYTES) as usize;
                assert!(occupied[chunk_index]);
                occupied[chunk_index] = false;
            }
        } else {
            let count = random % 5 + 1;
            match allocator.reserve_chunks(count) {
                Some(regions) => {
                    for region in &regions {
                        let chunk_index = (region.range().start / CHUNK_BYTES) as usize;
                        assert!(!occupied[chunk_index]);
                        occupied[chunk_index] = true;
                    }
                    entries.push(regions);
                }
                None => assert!(count > occupied.iter().filter(|&&used| !used).count() as u64),
            }
        }
        assert_eq!(
            allocator.available_bytes(),
            occupied.iter().filter(|&&used| !used).count() as u64 * CHUNK_BYTES
        );
    }
    drop(entries);
    assert_all_chunks_free(&allocator, capacity);
}

#[test]
fn concurrent_reservation_and_release_restores_all_free_chunks() {
    let capacity = 16 * CHUNK_BYTES;
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let allocator = &allocator;
            scope.spawn(move || {
                for _ in 0..1000 {
                    let regions = allocator.reserve_chunks(2).unwrap();
                    let read_guards: Vec<_> = regions.iter().map(DiskRegion::read_guard).collect();
                    drop(regions);
                    drop(read_guards);
                }
            });
        }
    });
    assert_all_chunks_free(&allocator, capacity);
}
