use super::*;
use crate::test_metrics::{registry, value};
use std::sync::Barrier;

#[test]
fn recovery_claims_survive_release_but_inspection_does_not_claim_chunks() {
    let allocator = DiskChunkAllocator::for_disk_range(3 * CHUNK_BYTES..7 * CHUNK_BYTES).unwrap();
    allocator.start_recovery(3 * CHUNK_BYTES, 7 * CHUNK_BYTES);
    let inspected = allocator.reserve_for_recovery(5, 2).unwrap();
    assert_eq!(inspected.range(), 5 * CHUNK_BYTES..7 * CHUNK_BYTES);
    assert!(allocator.reserve_for_recovery(5, 1).is_none());
    let written = allocator.reserve_chunks(1).unwrap();
    assert_eq!(written.range().start, 3 * CHUNK_BYTES);
    drop(written);
    assert!(allocator.reserve_for_recovery(3, 1).is_none());
    // Even a partly claimed range must fail atomically.
    assert!(allocator.reserve_for_recovery(3, 2).is_none());
    drop(inspected);
    let recovered = allocator.reserve_for_recovery(5, 2).unwrap();
    recovered.mark_recovered();
    let guard = recovered.read_guard();
    drop(recovered);
    assert_eq!(allocator.available_bytes(), 2 * CHUNK_BYTES);
    drop(guard);
    assert_eq!(allocator.available_bytes(), 4 * CHUNK_BYTES);
    assert!(allocator.reserve_for_recovery(5, 2).is_none());
    let all = allocator.reserve_chunks(4).unwrap();
    assert_eq!(all.range(), 3 * CHUNK_BYTES..7 * CHUNK_BYTES);
    drop(all);
    allocator.finish_recovery();
    assert!(allocator.reserve_for_recovery(4, 1).is_none());
}

#[test]
fn writes_and_recovery_cannot_reserve_the_same_chunks_concurrently() {
    for _ in 0..100 {
        let allocator = DiskChunkAllocator::new(2 * CHUNK_BYTES).unwrap();
        allocator.start_recovery(0, 2 * CHUNK_BYTES);
        let barrier = Arc::new(Barrier::new(2));
        let writer = allocator.clone();
        let writer_barrier = barrier.clone();
        let task = std::thread::spawn(move || {
            writer_barrier.wait();
            writer.reserve_chunks(2)
        });
        barrier.wait();
        let recovered = allocator.reserve_for_recovery(0, 2);
        let written = task.join().unwrap();
        assert_ne!(recovered.is_some(), written.is_some());
    }
}

#[test]
fn recovered_chunks_follow_shared_ownership_until_reuse() {
    let (registry, backend) = registry();
    let allocator = DiskChunkAllocator::with_metrics(0..3 * CHUNK_BYTES, DiskMetrics::new(&backend)).unwrap();
    let recovered_chunks = || value(&registry, "feuer_disk_recovered_chunks", &[]);
    allocator.start_recovery(0, 3 * CHUNK_BYTES);
    let inspected = allocator.reserve_for_recovery(0, 3).unwrap();
    assert_eq!(recovered_chunks(), 0.0);
    drop(inspected);
    let recovered = allocator.reserve_for_recovery(0, 3).unwrap();
    let shared = recovered.slice(4096..2 * CHUNK_BYTES);
    recovered.mark_recovered();
    shared.mark_recovered();
    assert_eq!(recovered_chunks(), 3.0);
    let guard = shared.read_guard();
    let other_guard = guard.clone();
    allocator.finish_recovery();
    drop((recovered, shared, guard));
    assert_eq!(recovered_chunks(), 3.0);
    drop(other_guard);
    assert_eq!(recovered_chunks(), 0.0);
    let written = allocator.reserve_chunks(3).unwrap();
    assert_eq!(recovered_chunks(), 0.0);
    drop((written, allocator));
    assert_eq!(recovered_chunks(), 0.0);
}

#[test]
fn capacity_metrics_follow_whole_allocation_ownership_and_allocator_drop() {
    let (registry, backend) = registry();
    let metrics = DiskMetrics::new(&backend);
    let allocator = DiskChunkAllocator::with_metrics(0..2 * CHUNK_BYTES, metrics.clone()).unwrap();
    let other = DiskChunkAllocator::with_metrics(2 * CHUNK_BYTES..3 * CHUNK_BYTES, metrics).unwrap();
    let chunks = |state| value(&registry, "feuer_disk_chunks", &[("state", state)]);
    assert_eq!(chunks("free"), 3.0);
    let region = allocator.reserve_chunks(2).unwrap();
    assert_eq!(chunks("allocated"), 2.0);
    let guard = region.slice(4096..8192).read_guard();
    drop(region);
    assert_eq!(chunks("free"), 1.0);
    drop(guard);
    assert_eq!(chunks("free"), 3.0);
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
fn reserves_contiguous_chunks_through_100_mib() {
    let capacity = 102 * CHUNK_BYTES;
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    for count in [1, 2, 40, 100, 102] {
        let region = allocator.reserve_chunks(count).unwrap();
        assert_eq!(region.range(), 0..count * CHUNK_BYTES);
        assert_eq!(region.chunk_count(), count);
        drop(region);
        assert_all_chunks_free(&allocator, capacity);
    }
}

#[test]
fn free_chunk_counts_are_independent_of_start() {
    let allocator = DiskChunkAllocator::for_disk_range(7 * CHUNK_BYTES..12 * CHUNK_BYTES).unwrap();
    let region = allocator.reserve_chunks(2).unwrap();
    assert_eq!(region.range(), 7 * CHUNK_BYTES..9 * CHUNK_BYTES);
    assert_eq!(
        allocator.free.lock().unwrap().free_chunk_count_by_start,
        BTreeMap::from([(9, 3)])
    );
    drop(region);
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
    let second = allocator.reserve_chunks(1).unwrap();
    assert!(allocator.reserve_chunks(1).is_none());
    drop((first, second));
    assert_all_chunks_free(&allocator, 2 * CHUNK_BYTES);
}

#[test]
fn slices_and_concurrent_readers_prevent_reuse_of_the_whole_run() {
    let allocator = DiskChunkAllocator::new(3 * CHUNK_BYTES).unwrap();
    let region = allocator.reserve_chunks(3).unwrap();
    let first = region.slice(4096..8192);
    let second = region.slice(CHUNK_BYTES..2 * CHUNK_BYTES);
    let final_guard = first.read_guard();
    let barrier = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let guard = second.read_guard();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(guard.range(), CHUNK_BYTES..2 * CHUNK_BYTES);
            });
        }
        drop((region, first, second));
        assert!(allocator.reserve_chunks(1).is_none());
        barrier.wait();
    });
    assert!(allocator.reserve_chunks(1).is_none());
    drop(final_guard);
    assert_all_chunks_free(&allocator, 3 * CHUNK_BYTES);
}

#[test]
fn forty_tib_initialization_is_sparse() {
    let capacity = 40 * (1u64 << 40);
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    let large = allocator.reserve_chunks(100).unwrap();
    assert_eq!(allocator.free.lock().unwrap().free_chunk_count_by_start.len(), 1);
    drop(large);
    assert_all_chunks_free(&allocator, capacity);
}

#[test]
fn fragmented_space_is_not_combined_and_coalesces_after_release() {
    let allocator = DiskChunkAllocator::new(5 * CHUNK_BYTES).unwrap();
    let mut regions: Vec<_> = (0..5).map(|_| allocator.reserve_chunks(1).unwrap()).collect();
    drop(regions.remove(4));
    drop(regions.remove(2));
    drop(regions.remove(0));
    assert_eq!(allocator.available_bytes(), 3 * CHUNK_BYTES);
    assert!(allocator.reserve_chunks(2).is_none());
    assert!(allocator.reserve_chunks(3).is_none());
    assert_eq!(allocator.available_bytes(), 3 * CHUNK_BYTES);
    drop(regions);
    assert_all_chunks_free(&allocator, 5 * CHUNK_BYTES);
    assert_eq!(allocator.reserve_chunks(5).unwrap().range(), 0..5 * CHUNK_BYTES);
}

#[test]
fn randomized_reuse_matches_contiguous_chunk_ownership() {
    let capacity = 16 * CHUNK_BYTES;
    let allocator = DiskChunkAllocator::new(capacity).unwrap();
    let mut occupied = [false; 16];
    let mut entries = Vec::<DiskRegion>::new();
    let mut random = 0x4de3_8129_f307_4a65u64;
    for _ in 0..10_000 {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        if !entries.is_empty() && random.is_multiple_of(3) {
            let region = entries.swap_remove(random as usize % entries.len());
            for chunk in region.range().start / CHUNK_BYTES..region.range().end / CHUNK_BYTES {
                assert!(occupied[chunk as usize]);
                occupied[chunk as usize] = false;
            }
        } else {
            let count = random % 5 + 1;
            match allocator.reserve_chunks(count) {
                Some(region) => {
                    for chunk in region.range().start / CHUNK_BYTES..region.range().end / CHUNK_BYTES {
                        assert!(!occupied[chunk as usize]);
                        occupied[chunk as usize] = true;
                    }
                    entries.push(region);
                }
                None => assert!(!occupied.windows(count as usize).any(|run| run.iter().all(|used| !used))),
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
                    let region = allocator.reserve_chunks(2).unwrap();
                    let guard = region.read_guard();
                    drop(region);
                    drop(guard);
                }
            });
        }
    });
    assert_all_chunks_free(&allocator, capacity);
}
