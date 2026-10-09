//! Ownership and reuse of consecutive whole chunks.

use std::{
    collections::{BTreeMap, hash_map::Entry},
    ops::Range,
    sync::{Arc, Mutex},
};

use rustc_hash::FxHashMap;

use crate::DiskMetrics;

pub(super) const CHUNK_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub(super) struct DiskChunkAllocator {
    free: Arc<Mutex<FreeChunks>>,
    pub(super) chunk_capacity: u64,
    /// First chunk number -> reserved chunks and number of payloads using them.
    payload_chunks: Mutex<FxHashMap<u64, (ReservedChunks, usize)>>,
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
    fn remove_free_chunks(&mut self, chunks: &Range<u64>) -> Option<()> {
        let (&start, count) = self.free_chunk_count_by_start.range_mut(..=chunks.start).next_back()?;
        let end = start + *count;
        if chunks.start >= chunks.end || chunks.end > end {
            return None;
        }
        *count = chunks.start - start;
        if *count == 0 {
            self.free_chunk_count_by_start.remove(&start);
        }
        if chunks.end < end {
            self.free_chunk_count_by_start.insert(chunks.end, end - chunks.end);
        }
        let count = chunks.end - chunks.start;
        self.metrics.free_chunks.decrease(count);
        self.metrics.allocated_chunks.increase(count);
        Some(())
    }

    /// Returns this allocator's reserved chunks to the free map, merging adjacent runs.
    fn release(&mut self, chunks: &Range<u64>) {
        let released_chunk_count = chunks.end - chunks.start;
        let merged_chunk_count = released_chunk_count + self.free_chunk_count_by_start.remove(&chunks.end).unwrap_or(0);
        if let Some((&previous, length)) = self.free_chunk_count_by_start.range_mut(..chunks.start).next_back()
            && previous + *length == chunks.start
        {
            *length += merged_chunk_count;
        } else {
            self.free_chunk_count_by_start.insert(chunks.start, merged_chunk_count);
        }
        self.metrics.free_chunks.increase(released_chunk_count);
        self.metrics.allocated_chunks.decrease(released_chunk_count);
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
    fn new(capacity: u64) -> Self {
        Self::for_disk_range(0..capacity)
    }

    #[cfg(test)]
    pub(super) fn available_bytes(&self) -> u64 {
        let free = self.free.lock().unwrap();
        free.free_chunk_count_by_start.values().sum::<u64>() * CHUNK_BYTES
    }

    #[cfg(test)]
    pub(super) fn for_disk_range(disk_range: Range<u64>) -> Self {
        Self::with_metrics(disk_range, DiskMetrics::noop())
    }

    /// Creates an allocator for a shard's nonempty, chunk-aligned disk byte range.
    /// The disk cache validates capacity before dividing it into shard ranges.
    pub(super) fn with_metrics(disk_range: Range<u64>, metrics: Arc<DiskMetrics>) -> Self {
        let chunk_capacity = (disk_range.end - disk_range.start) / CHUNK_BYTES;
        metrics.free_chunks.increase(chunk_capacity);
        Self {
            chunk_capacity,
            payload_chunks: Mutex::default(),
            free: Arc::new(Mutex::new(FreeChunks {
                free_chunk_count_by_start: BTreeMap::from([(disk_range.start / CHUNK_BYTES, chunk_capacity)]),
                metrics,
            })),
        }
    }

    /// Reserves consecutive chunks starting at the specified chunk number.
    pub(super) fn reserve_chunks_at(&self, first_chunk_number: u64, count: u64) -> Option<ReservedChunks> {
        let chunk_numbers = first_chunk_number..first_chunk_number.checked_add(count)?;
        self.reserve_chunk_numbers(&mut self.free.lock().unwrap(), chunk_numbers)
    }

    /// Keeps a recovered payload's chunks reserved until that payload is released.
    /// Packed payloads share one chunk. Standalone payloads start at a chunk boundary.
    /// Rejects conflicts and payloads outside this allocator's shard.
    pub(super) fn hold_chunks_for_recovered_payload(&self, payload_range: &Range<u64>) -> Option<()> {
        let first_chunk_number = payload_range.start / CHUNK_BYTES;
        let mut payload_chunks = self.payload_chunks.lock().unwrap();
        match payload_chunks.entry(first_chunk_number) {
            Entry::Occupied(entry) => {
                let (reserved_chunks, payload_count) = entry.into_mut();
                (payload_range.end <= reserved_chunks.disk_byte_range().end).then(|| *payload_count += 1)
            }
            Entry::Vacant(entry) => {
                let mut reserved_chunks = self.reserve_chunks_at(
                    first_chunk_number,
                    payload_range.end.div_ceil(CHUNK_BYTES) - first_chunk_number,
                )?;
                reserved_chunks.mark_recovered();
                entry.insert((reserved_chunks, 1));
                Some(())
            }
        }
    }

    /// Reserves one contiguous run of whole chunks. Failure consumes no space.
    pub(super) fn reserve_chunks(&self, count: u64) -> Option<ReservedChunks> {
        let mut free = self.free.lock().unwrap();
        let (&start, _) = free
            .free_chunk_count_by_start
            .iter()
            .find(|(_, length)| **length >= count)?;
        let chunk_numbers = start..start + count;
        self.reserve_chunk_numbers(&mut free, chunk_numbers)
    }

    /// Keeps these chunks reserved until all their payloads have been released.
    pub(super) fn hold_chunks_until_payloads_released(&self, reserved_chunks: ReservedChunks, payload_count: usize) {
        self.payload_chunks
            .lock()
            .unwrap()
            .insert(reserved_chunks.chunk_numbers.start, (reserved_chunks, payload_count));
    }

    /// Releases one held payload's use of its chunks, freeing them after their last payload.
    /// The caller must supply the payload's start address, once per held payload.
    pub(super) fn release_payload(&self, payload_address: u64) {
        let first_chunk_number = payload_address / CHUNK_BYTES;
        let mut payload_chunks = self.payload_chunks.lock().unwrap();
        match payload_chunks.entry(first_chunk_number) {
            Entry::Occupied(entry) if entry.get().1 == 1 => drop(entry.remove()),
            Entry::Occupied(mut entry) => entry.get_mut().1 -= 1,
            Entry::Vacant(_) => unreachable!("released payload must be held"),
        }
    }

    /// Reserves the specified chunk numbers while the caller holds the free-chunk lock.
    fn reserve_chunk_numbers(&self, free: &mut FreeChunks, chunk_numbers: Range<u64>) -> Option<ReservedChunks> {
        free.remove_free_chunks(&chunk_numbers)?;
        Some(ReservedChunks {
            free: self.free.clone(),
            chunk_numbers,
            recovered: false,
        })
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

    /// Counts recovered chunks until their reservation is released. Called once per accepted reservation.
    pub(super) fn mark_recovered(&mut self) {
        self.free
            .lock()
            .unwrap()
            .metrics
            .recovered_chunks
            .increase(self.chunk_count());
        self.recovered = true;
    }
}

impl Drop for ReservedChunks {
    fn drop(&mut self) {
        let mut free = self.free.lock().unwrap();
        if self.recovered {
            free.metrics.recovered_chunks.decrease(self.chunk_count());
        }
        free.release(&self.chunk_numbers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_metrics::{registry, value};
    use std::sync::Barrier;

    #[test]
    fn allocator_releases_shared_chunks_after_the_last_payload() {
        let allocator = DiskChunkAllocator::new(CHUNK_BYTES);
        let reserved_chunks = allocator.reserve_chunks(1).unwrap();
        allocator.hold_chunks_until_payloads_released(reserved_chunks, 2);
        allocator.release_payload(0);
        assert_eq!(allocator.available_bytes(), 0);
        allocator.release_payload(4096);
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    }

    #[test]
    fn recovery_reservations_respect_shard_bounds_and_chunk_ownership() {
        let allocator = DiskChunkAllocator::for_disk_range(CHUNK_BYTES..5 * CHUNK_BYTES);
        let _metadata = allocator.reserve_chunks_at(1, 1).unwrap();
        for (payload, accepted) in [
            (0..4096, false),
            (CHUNK_BYTES..CHUNK_BYTES + 4096, false),
            (5 * CHUNK_BYTES..5 * CHUNK_BYTES + 4096, false),
            (2 * CHUNK_BYTES..4 * CHUNK_BYTES, true),
            (3 * CHUNK_BYTES..3 * CHUNK_BYTES + 4096, false),
            (2 * CHUNK_BYTES + 4096..2 * CHUNK_BYTES + 8192, true),
            (2 * CHUNK_BYTES..6 * CHUNK_BYTES, false),
        ] {
            assert_eq!(
                allocator.hold_chunks_for_recovered_payload(&payload).is_some(),
                accepted,
                "{payload:?}"
            );
        }
        allocator.release_payload(2 * CHUNK_BYTES);
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
        allocator.release_payload(2 * CHUNK_BYTES + 4096);
        assert_eq!(allocator.available_bytes(), 3 * CHUNK_BYTES);
    }

    #[test]
    fn writes_and_recovery_cannot_reserve_the_same_chunks_concurrently() {
        for _ in 0..100 {
            let allocator = DiskChunkAllocator::new(2 * CHUNK_BYTES);
            let barrier = Barrier::new(2);
            std::thread::scope(|scope| {
                let task = scope.spawn(|| {
                    barrier.wait();
                    allocator.reserve_chunks(2)
                });
                barrier.wait();
                let recovered = allocator.reserve_chunks_at(0, 2);
                let written = task.join().unwrap();
                assert_ne!(recovered.is_some(), written.is_some());
            });
        }
    }

    #[test]
    fn recovered_chunk_metrics_follow_allocator_ownership_until_reuse() {
        let (registry, backend) = registry();
        let allocator = DiskChunkAllocator::with_metrics(0..3 * CHUNK_BYTES, DiskMetrics::new(&backend));
        let recovered_chunks = || value(&registry, "feuer_disk_recovered_chunks", &[]);
        let inspected = allocator.reserve_chunks_at(0, 3).unwrap();
        assert_eq!(recovered_chunks(), 0.0);
        drop(inspected);
        for payload in [0..3 * CHUNK_BYTES, 4096..8192] {
            allocator.hold_chunks_for_recovered_payload(&payload).unwrap();
            assert_eq!(recovered_chunks(), 3.0);
        }
        allocator.release_payload(0);
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
        let allocator = DiskChunkAllocator::with_metrics(0..2 * CHUNK_BYTES, metrics.clone());
        let other = DiskChunkAllocator::with_metrics(2 * CHUNK_BYTES..3 * CHUNK_BYTES, metrics);
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
        let allocator = DiskChunkAllocator::new(capacity);
        for count in [1, 2, 40, 100, 102] {
            let reserved_chunks = allocator.reserve_chunks(count).unwrap();
            assert_eq!(reserved_chunks.disk_byte_range(), 0..count * CHUNK_BYTES);
            assert_eq!(reserved_chunks.chunk_count(), count);
            drop(reserved_chunks);
            assert_all_chunks_free(&allocator, capacity);
        }
    }

    #[test]
    fn reservations_split_and_restore_free_chunks_at_nonzero_addresses() {
        let allocator = DiskChunkAllocator::for_disk_range(7 * CHUNK_BYTES..12 * CHUNK_BYTES);
        let reserved_chunks = allocator.reserve_chunks_at(9, 2).unwrap();
        assert_eq!(reserved_chunks.disk_byte_range(), 9 * CHUNK_BYTES..11 * CHUNK_BYTES);
        assert!(allocator.reserve_chunks_at(8, 2).is_none());
        assert_eq!(
            allocator.free.lock().unwrap().free_chunk_count_by_start,
            BTreeMap::from([(7, 2), (11, 1)])
        );
        drop(reserved_chunks);
        assert_eq!(
            allocator.free.lock().unwrap().free_chunk_count_by_start,
            BTreeMap::from([(7, 5)])
        );
    }

    #[test]
    fn rejects_unavailable_chunk_counts_without_consuming_space() {
        let allocator = DiskChunkAllocator::new(2 * CHUNK_BYTES);
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
        let allocator = DiskChunkAllocator::new(capacity);
        let large = allocator.reserve_chunks(100).unwrap();
        assert_eq!(allocator.free.lock().unwrap().free_chunk_count_by_start.len(), 1);
        drop(large);
        assert_all_chunks_free(&allocator, capacity);
    }

    #[test]
    fn fragmented_space_is_not_combined_and_coalesces_after_release() {
        let allocator = DiskChunkAllocator::new(5 * CHUNK_BYTES);
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
        let allocator = DiskChunkAllocator::new(capacity);
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
        let allocator = DiskChunkAllocator::new(capacity);
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
