//! Ownership and reuse of consecutive whole chunks.

use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::DiskMetrics;

pub(super) const CHUNK_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct DiskChunkAllocator {
    free: Arc<Mutex<FreeChunks>>,
    pub(super) chunk_capacity: u64,
    /// First chunk address -> reserved chunks and number of payloads using them.
    payload_chunks: Arc<Mutex<BTreeMap<u64, (ReservedChunks, usize)>>>,
}

/// Consecutive free chunks and their metrics.
#[derive(Debug)]
struct FreeChunks {
    /// Consecutive free chunks: first chunk number -> count. Adjacent runs are merged.
    free_chunk_count_by_start: BTreeMap<u64, u64>,
    metrics: Arc<DiskMetrics>,
}

impl FreeChunks {
    /// Removes the specified chunks from the free-chunk map and updates metrics.
    /// If any are unavailable, leaves both unchanged.
    fn remove_free_chunks(&mut self, chunks: Range<u64>) -> Option<()> {
        let (&start, &count) = self.free_chunk_count_by_start.range(..=chunks.start).next_back()?;
        if chunks.start >= chunks.end || chunks.end > start + count {
            return None;
        }
        self.free_chunk_count_by_start.remove(&start);
        if start < chunks.start {
            self.free_chunk_count_by_start.insert(start, chunks.start - start);
        }
        if chunks.end < start + count {
            self.free_chunk_count_by_start
                .insert(chunks.end, start + count - chunks.end);
        }
        let count = chunks.end - chunks.start;
        self.metrics.free_chunks.decrease(count);
        self.metrics.allocated_chunks.increase(count);
        Some(())
    }

    /// Returns this allocator's reserved chunks to the free map, merging adjacent runs.
    fn release(&mut self, chunks: Range<u64>) {
        let count = chunks.end - chunks.start;
        let mut start = chunks.start;
        let mut end = chunks.end;
        if let Some((&previous, &length)) = self.free_chunk_count_by_start.range(..start).next_back()
            && previous + length == start
        {
            start = previous;
            self.free_chunk_count_by_start.remove(&previous);
        }
        if let Some(length) = self.free_chunk_count_by_start.remove(&end) {
            end += length;
        }
        self.free_chunk_count_by_start.insert(start, end - start);
        self.metrics.free_chunks.increase(count);
        self.metrics.allocated_chunks.decrease(count);
    }
}

impl Drop for FreeChunks {
    fn drop(&mut self) {
        let free_chunk_count = self.free_chunk_count_by_start.values().sum();
        self.metrics.free_chunks.decrease(free_chunk_count);
    }
}

impl DiskChunkAllocator {
    #[cfg(test)]
    fn new(capacity: u64) -> Option<Self> {
        Self::for_disk_range(0..capacity)
    }

    #[cfg(test)]
    pub(super) fn available_bytes(&self) -> u64 {
        let free = self.free.lock().unwrap();
        free.free_chunk_count_by_start.values().sum::<u64>() * CHUNK_BYTES
    }

    #[cfg(test)]
    pub(super) fn for_disk_range(disk_range: Range<u64>) -> Option<Self> {
        Self::with_metrics(disk_range, DiskMetrics::noop())
    }

    pub(super) fn with_metrics(disk_range: Range<u64>, metrics: Arc<DiskMetrics>) -> Option<Self> {
        if disk_range.start >= disk_range.end
            || disk_range.end > i64::MAX as u64
            || !disk_range.start.is_multiple_of(CHUNK_BYTES)
            || !disk_range.end.is_multiple_of(CHUNK_BYTES)
        {
            return None;
        }
        let chunk_capacity = (disk_range.end - disk_range.start) / CHUNK_BYTES;
        metrics.free_chunks.increase(chunk_capacity);
        Some(Self {
            chunk_capacity,
            payload_chunks: Arc::new(Mutex::new(BTreeMap::new())),
            free: Arc::new(Mutex::new(FreeChunks {
                free_chunk_count_by_start: BTreeMap::from([(disk_range.start / CHUNK_BYTES, chunk_capacity)]),
                metrics,
            })),
        })
    }

    /// Reserves consecutive chunks starting at the specified chunk number.
    pub(super) fn reserve_chunks_at(&self, first_chunk_number: u64, count: u64) -> Option<ReservedChunks> {
        let chunk_numbers = first_chunk_number..first_chunk_number.checked_add(count)?;
        let mut free = self.free.lock().unwrap();
        free.remove_free_chunks(chunk_numbers.clone())?;
        Some(self.own_reserved_chunks(chunk_numbers))
    }

    /// Keeps a recovered payload's chunks reserved until that payload is released.
    /// Shares an existing reservation when it contains the payload.
    /// Rejects conflicts and payloads outside this allocator's shard.
    pub(super) fn hold_chunks_for_recovered_payload(&self, payload_range: &Range<u64>) -> Option<()> {
        let mut payload_chunks = self.payload_chunks.lock().unwrap();
        if let Some((_, (reserved_chunks, payload_count))) =
            payload_chunks.range_mut(..=payload_range.start).next_back()
            && reserved_chunks.disk_byte_range().contains(&payload_range.start)
        {
            if payload_range.end > reserved_chunks.disk_byte_range().end {
                return None;
            }
            *payload_count += 1;
        } else {
            let first_chunk_number = payload_range.start / CHUNK_BYTES;
            let mut reserved_chunks = self.reserve_chunks_at(
                first_chunk_number,
                payload_range.end.div_ceil(CHUNK_BYTES) - first_chunk_number,
            )?;
            reserved_chunks.mark_recovered();
            payload_chunks.insert(reserved_chunks.disk_byte_range().start, (reserved_chunks, 1));
        }
        Some(())
    }

    /// Reserves one contiguous run of whole chunks. Failure consumes no space.
    pub(super) fn reserve_chunks(&self, count: u64) -> Option<ReservedChunks> {
        let mut free = self.free.lock().unwrap();
        let (&start, _) = free
            .free_chunk_count_by_start
            .iter()
            .find(|(_, length)| **length >= count)?;
        let chunk_numbers = start..start + count;
        free.remove_free_chunks(chunk_numbers.clone())?;
        Some(self.own_reserved_chunks(chunk_numbers))
    }

    /// Keeps these chunks reserved until all their payloads have been released.
    pub(super) fn hold_chunks_until_payloads_released(&self, reserved_chunks: ReservedChunks, payload_count: usize) {
        self.payload_chunks.lock().unwrap().insert(
            reserved_chunks.disk_byte_range().start,
            (reserved_chunks, payload_count),
        );
    }

    /// Releases one held payload's use of its chunks, freeing them after their last payload.
    /// The caller must supply an address in held payload chunks, once per held payload.
    pub(super) fn release_payload(&self, payload_address: u64) {
        let mut payload_chunks = self.payload_chunks.lock().unwrap();
        let (&start, (_, payload_count)) = payload_chunks.range_mut(..=payload_address).next_back().unwrap();
        *payload_count -= 1;
        if *payload_count == 0 {
            payload_chunks.remove(&start);
        }
    }

    /// Creates an owner that returns already-reserved chunks to the free map when dropped.
    fn own_reserved_chunks(&self, chunk_numbers: Range<u64>) -> ReservedChunks {
        ReservedChunks {
            free: self.free.clone(),
            chunk_numbers,
            recovered: false,
        }
    }
}

/// Reserved consecutive chunks, returned to the allocator when dropped.
#[derive(Debug)]
pub(super) struct ReservedChunks {
    free: Arc<Mutex<FreeChunks>>,
    /// Numbers of the reserved chunks, not disk byte offsets.
    chunk_numbers: Range<u64>,
    recovered: bool,
}

impl ReservedChunks {
    /// Disk byte range occupied by the reserved chunks.
    pub(super) fn disk_byte_range(&self) -> Range<u64> {
        self.chunk_numbers.start * CHUNK_BYTES..self.chunk_numbers.end * CHUNK_BYTES
    }

    pub(super) fn chunk_count(&self) -> u64 {
        self.chunk_numbers.end - self.chunk_numbers.start
    }

    /// Counts recovered chunks until their reservation is released.
    pub(super) fn mark_recovered(&mut self) {
        if !self.recovered {
            self.free
                .lock()
                .unwrap()
                .metrics
                .recovered_chunks
                .increase(self.chunk_count());
            self.recovered = true;
        }
    }
}

impl Drop for ReservedChunks {
    fn drop(&mut self) {
        let mut free = self.free.lock().unwrap();
        if self.recovered {
            free.metrics
                .recovered_chunks
                .decrease(self.chunk_numbers.end - self.chunk_numbers.start);
        }
        free.release(self.chunk_numbers.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_metrics::{registry, value};
    use std::sync::Barrier;

    #[test]
    fn allocator_releases_shared_chunks_after_the_last_payload() {
        let allocator = DiskChunkAllocator::new(CHUNK_BYTES).unwrap();
        let reserved_chunks = allocator.reserve_chunks(1).unwrap();
        allocator.hold_chunks_until_payloads_released(reserved_chunks, 2);
        allocator.release_payload(0);
        assert_eq!(allocator.available_bytes(), 0);
        allocator.release_payload(4096);
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    }

    #[test]
    fn recovery_reservations_respect_shard_bounds_and_chunk_ownership() {
        let allocator = DiskChunkAllocator::for_disk_range(CHUNK_BYTES..3 * CHUNK_BYTES).unwrap();
        let _metadata = allocator.reserve_chunks_at(1, 1).unwrap();
        for (payload, accepted) in [
            (0..4096, false),
            (CHUNK_BYTES..CHUNK_BYTES + 4096, false),
            (2 * CHUNK_BYTES..4 * CHUNK_BYTES, false),
            (3 * CHUNK_BYTES..3 * CHUNK_BYTES + 4096, false),
            (2 * CHUNK_BYTES..2 * CHUNK_BYTES + 4096, true),
            (2 * CHUNK_BYTES..2 * CHUNK_BYTES + 8192, true),
            (2 * CHUNK_BYTES..4 * CHUNK_BYTES, false),
        ] {
            assert_eq!(
                allocator.hold_chunks_for_recovered_payload(&payload).is_some(),
                accepted,
                "{payload:?}"
            );
        }
        allocator.release_payload(2 * CHUNK_BYTES);
        assert_eq!(allocator.available_bytes(), 0);
        allocator.release_payload(2 * CHUNK_BYTES);
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    }

    #[test]
    fn writes_and_recovery_cannot_reserve_the_same_chunks_concurrently() {
        for _ in 0..100 {
            let allocator = DiskChunkAllocator::new(2 * CHUNK_BYTES).unwrap();
            let barrier = Arc::new(Barrier::new(2));
            let writer = allocator.clone();
            let writer_barrier = barrier.clone();
            let task = std::thread::spawn(move || {
                writer_barrier.wait();
                writer.reserve_chunks(2)
            });
            barrier.wait();
            let recovered = allocator.reserve_chunks_at(0, 2);
            let written = task.join().unwrap();
            assert_ne!(recovered.is_some(), written.is_some());
        }
    }

    #[test]
    fn recovered_chunk_metrics_follow_allocator_ownership_until_reuse() {
        let (registry, backend) = registry();
        let allocator = DiskChunkAllocator::with_metrics(0..3 * CHUNK_BYTES, DiskMetrics::new(&backend)).unwrap();
        let recovered_chunks = || value(&registry, "feuer_disk_recovered_chunks", &[]);
        let inspected = allocator.reserve_chunks_at(0, 3).unwrap();
        assert_eq!(recovered_chunks(), 0.0);
        drop(inspected);
        let mut recovered = allocator.reserve_chunks_at(0, 3).unwrap();
        recovered.mark_recovered();
        recovered.mark_recovered();
        assert_eq!(recovered_chunks(), 3.0);
        allocator.hold_chunks_until_payloads_released(recovered, 1);
        assert_eq!(recovered_chunks(), 3.0);
        allocator.release_payload(4096);
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
        let reserved_chunks = allocator.reserve_chunks(2).unwrap();
        assert_eq!(chunks("allocated"), 2.0);
        allocator.hold_chunks_until_payloads_released(reserved_chunks, 1);
        assert_eq!(chunks("free"), 1.0);
        allocator.release_payload(4096);
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
    }

    #[test]
    fn reserves_contiguous_chunks_through_100_mib() {
        let capacity = 102 * CHUNK_BYTES;
        let allocator = DiskChunkAllocator::new(capacity).unwrap();
        for count in [1, 2, 40, 100, 102] {
            let reserved_chunks = allocator.reserve_chunks(count).unwrap();
            assert_eq!(reserved_chunks.disk_byte_range(), 0..count * CHUNK_BYTES);
            assert_eq!(reserved_chunks.chunk_count(), count);
            drop(reserved_chunks);
            assert_all_chunks_free(&allocator, capacity);
        }
    }

    #[test]
    fn free_chunk_counts_are_independent_of_start() {
        let allocator = DiskChunkAllocator::for_disk_range(7 * CHUNK_BYTES..12 * CHUNK_BYTES).unwrap();
        let reserved_chunks = allocator.reserve_chunks(2).unwrap();
        assert_eq!(reserved_chunks.disk_byte_range(), 7 * CHUNK_BYTES..9 * CHUNK_BYTES);
        assert_eq!(
            allocator.free.lock().unwrap().free_chunk_count_by_start,
            BTreeMap::from([(9, 3)])
        );
        drop(reserved_chunks);
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
        let mut reservations: Vec<_> = (0..5).map(|_| allocator.reserve_chunks(1).unwrap()).collect();
        drop(reservations.remove(4));
        drop(reservations.remove(2));
        drop(reservations.remove(0));
        assert_eq!(allocator.available_bytes(), 3 * CHUNK_BYTES);
        assert!(allocator.reserve_chunks(2).is_none());
        assert!(allocator.reserve_chunks(3).is_none());
        assert_eq!(allocator.available_bytes(), 3 * CHUNK_BYTES);
        drop(reservations);
        assert_all_chunks_free(&allocator, 5 * CHUNK_BYTES);
        assert_eq!(
            allocator.reserve_chunks(5).unwrap().disk_byte_range(),
            0..5 * CHUNK_BYTES
        );
    }

    #[test]
    fn randomized_reuse_matches_contiguous_chunk_ownership() {
        let capacity = 16 * CHUNK_BYTES;
        let allocator = DiskChunkAllocator::new(capacity).unwrap();
        let mut occupied = [false; 16];
        let mut reservations = Vec::<ReservedChunks>::new();
        let mut random = 0x4de3_8129_f307_4a65u64;
        for _ in 0..10_000 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            if !reservations.is_empty() && random.is_multiple_of(3) {
                let reserved_chunks = reservations.swap_remove(random as usize % reservations.len());
                for chunk in reserved_chunks.chunk_numbers.clone() {
                    assert!(occupied[chunk as usize]);
                    occupied[chunk as usize] = false;
                }
            } else {
                let count = random % 5 + 1;
                match allocator.reserve_chunks(count) {
                    Some(reserved_chunks) => {
                        for chunk in reserved_chunks.chunk_numbers.clone() {
                            assert!(!occupied[chunk as usize]);
                            occupied[chunk as usize] = true;
                        }
                        reservations.push(reserved_chunks);
                    }
                    None => assert!(!occupied.windows(count as usize).any(|run| run.iter().all(|used| !used))),
                }
            }
            assert_eq!(
                allocator.available_bytes(),
                occupied.iter().filter(|&&used| !used).count() as u64 * CHUNK_BYTES
            );
        }
        drop(reservations);
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
                        let reserved_chunks = allocator.reserve_chunks(2).unwrap();
                        drop(reserved_chunks);
                    }
                });
            }
        });
        assert_all_chunks_free(&allocator, capacity);
    }
}
