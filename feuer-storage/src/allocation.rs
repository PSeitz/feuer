//! Ownership and reuse of consecutive whole chunks and entry metadata.

use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use crate::DiskMetrics;

pub(super) const CHUNK_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct DiskChunkAllocator {
    free: Arc<Mutex<DiskAvailability>>,
    pub(super) chunk_capacity: u64,
    /// First chunk address -> reserved region and number of entries using it.
    payload_chunks: Arc<Mutex<BTreeMap<u64, (DiskRegion, usize)>>>,
}

/// Free disk chunks and entry metadata, with chunk accounting.
#[derive(Debug)]
struct DiskAvailability {
    /// Consecutive free chunks: first chunk number -> count. Adjacent runs are merged.
    free_chunk_count_by_start: BTreeMap<u64, u64>,
    available_chunks: u64,
    /// Metadata available for overwrite, identified by (metadata chunk index, entry metadata index).
    /// Only these indexes are tracked here; disk space is reserved in whole chunks.
    available_metadata: Vec<(usize, usize)>,
    metrics: Arc<DiskMetrics>,
}

impl DiskAvailability {
    /// Removes the specified chunks from the free-chunk map and decreases the available chunk count.
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
        self.available_chunks -= count;
        self.metrics.free_chunks.decrease(count);
        self.metrics.allocated_chunks.increase(count);
        Some(())
    }

    fn release(&mut self, chunks: Range<u64>) {
        let count = chunks.end - chunks.start;
        let mut start = chunks.start;
        let mut end = chunks.end;
        if let Some((&previous, &length)) = self.free_chunk_count_by_start.range(..start).next_back() {
            assert!(previous + length <= start);
            if previous + length == start {
                start = previous;
                self.free_chunk_count_by_start.remove(&previous);
            }
        }
        if let Some((&next, &length)) = self.free_chunk_count_by_start.range(chunks.start..).next() {
            assert!(next >= end);
            if next == end {
                end += length;
                self.free_chunk_count_by_start.remove(&next);
            }
        }
        self.free_chunk_count_by_start.insert(start, end - start);
        self.available_chunks += count;
        self.metrics.free_chunks.increase(count);
        self.metrics.allocated_chunks.decrease(count);
    }
}

impl Drop for DiskAvailability {
    fn drop(&mut self) {
        self.metrics.free_chunks.decrease(self.available_chunks);
    }
}

impl DiskChunkAllocator {
    #[cfg(test)]
    fn new(capacity: u64) -> Option<Self> {
        Self::for_disk_range(0..capacity)
    }

    #[cfg(test)]
    pub(super) fn available_bytes(&self) -> u64 {
        self.free.lock().unwrap().available_chunks * CHUNK_BYTES
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
            free: Arc::new(Mutex::new(DiskAvailability {
                free_chunk_count_by_start: BTreeMap::from([(disk_range.start / CHUNK_BYTES, chunk_capacity)]),
                available_chunks: chunk_capacity,
                available_metadata: Vec::new(),
                metrics,
            })),
        })
    }

    /// Makes entry metadata in a newly initialized metadata chunk available for overwrite.
    pub(super) fn add_metadata_chunk(&self, chunk_index: usize, entry_count: usize) {
        let mut free = self.free.lock().unwrap();
        free.available_metadata.extend(
            (0..entry_count)
                .rev()
                .map(|entry_metadata_index| (chunk_index, entry_metadata_index)),
        );
    }

    pub(super) fn available_metadata_count(&self) -> usize {
        self.free.lock().unwrap().available_metadata.len()
    }

    /// Reserves space to write metadata. Returns (metadata chunk index, entry metadata index).
    pub(super) fn reserve_metadata(&self) -> Option<(usize, usize)> {
        self.free.lock().unwrap().available_metadata.pop()
    }

    /// Allows existing metadata to be overwritten. Does not erase bytes or release chunks.
    pub(super) fn allow_metadata_overwrite(&self, metadata: (usize, usize)) {
        self.free.lock().unwrap().available_metadata.push(metadata);
    }

    /// Reserves chunks at a known address during startup, before writes are allowed.
    pub(super) fn reserve_for_recovery(&self, first: u64, count: u64) -> Option<DiskRegion> {
        let chunks = first..first.checked_add(count)?;
        let mut free = self.free.lock().unwrap();
        free.remove_free_chunks(chunks.clone())?;
        Some(self.region(chunks))
    }

    /// Reserves a payload's chunks, sharing an existing reservation when it contains the payload.
    /// Rejects conflicts and payloads outside this allocator's shard.
    pub(super) fn reserve_payload_chunks(&self, payload_range: &Range<u64>) -> Option<()> {
        let mut payload_chunks = self.payload_chunks.lock().unwrap();
        if let Some((_, (region, entry_count))) = payload_chunks.range_mut(..=payload_range.start).next_back()
            && region.range().contains(&payload_range.start)
        {
            if payload_range.end > region.range().end {
                return None;
            }
            *entry_count += 1;
        } else {
            let first_chunk_number = payload_range.start / CHUNK_BYTES;
            let mut region = self.reserve_for_recovery(
                first_chunk_number,
                payload_range.end.div_ceil(CHUNK_BYTES) - first_chunk_number,
            )?;
            region.mark_recovered();
            payload_chunks.insert(region.range().start, (region, 1));
        }
        Some(())
    }

    /// Reserves one contiguous run of whole chunks. Failure consumes no space.
    pub(super) fn reserve_chunks(&self, count: u64) -> Option<DiskRegion> {
        let mut free = self.free.lock().unwrap();
        if count == 0 || count > free.available_chunks {
            return None;
        }
        let (&start, _) = free
            .free_chunk_count_by_start
            .iter()
            .find(|(_, length)| **length >= count)?;
        let chunks = start..start + count;
        free.remove_free_chunks(chunks.clone())?;
        Some(self.region(chunks))
    }

    /// Retains a written chunk run until all its entries are removed.
    pub(super) fn retain_payloads(&self, region: DiskRegion, entries: usize) {
        self.payload_chunks
            .lock()
            .unwrap()
            .insert(region.range().start, (region, entries));
    }

    /// Removes one entry's use of its chunks, releasing the run after its last entry.
    pub(super) fn remove_payload(&self, address: u64) {
        let mut chunks = self.payload_chunks.lock().unwrap();
        if let Some((&start, (region, entries))) = chunks.range_mut(..=address).next_back()
            && region.range().contains(&address)
        {
            *entries -= 1;
            if *entries == 0 {
                chunks.remove(&start);
            }
        }
    }

    fn region(&self, chunks: Range<u64>) -> DiskRegion {
        DiskRegion {
            free: self.free.clone(),
            chunks,
            recovered: false,
        }
    }
}

/// One owner's reservation of consecutive whole chunks in the backing file.
#[derive(Debug)]
pub(super) struct DiskRegion {
    free: Arc<Mutex<DiskAvailability>>,
    chunks: Range<u64>,
    recovered: bool,
}

impl DiskRegion {
    pub(super) fn range(&self) -> Range<u64> {
        self.chunks.start * CHUNK_BYTES..self.chunks.end * CHUNK_BYTES
    }

    pub(super) fn chunk_count(&self) -> u64 {
        self.chunks.end - self.chunks.start
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

impl Drop for DiskRegion {
    fn drop(&mut self) {
        let mut free = self.free.lock().unwrap();
        if self.recovered {
            free.metrics
                .recovered_chunks
                .decrease(self.chunks.end - self.chunks.start);
        }
        free.release(self.chunks.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_metrics::{registry, value};
    use std::sync::Barrier;

    #[test]
    fn allocator_releases_shared_chunks_after_the_last_entry() {
        let allocator = DiskChunkAllocator::new(CHUNK_BYTES).unwrap();
        let region = allocator.reserve_chunks(1).unwrap();
        allocator.retain_payloads(region, 2);
        allocator.remove_payload(0);
        assert_eq!(allocator.available_bytes(), 0);
        allocator.remove_payload(4096);
        assert_eq!(allocator.available_bytes(), CHUNK_BYTES);
    }

    #[test]
    fn recovery_reservations_respect_shard_bounds_and_chunk_ownership() {
        let allocator = DiskChunkAllocator::for_disk_range(CHUNK_BYTES..3 * CHUNK_BYTES).unwrap();
        let _metadata = allocator.reserve_for_recovery(1, 1).unwrap();
        for payload in [
            0..4096,
            CHUNK_BYTES..CHUNK_BYTES + 4096,
            2 * CHUNK_BYTES..4 * CHUNK_BYTES,
            3 * CHUNK_BYTES..3 * CHUNK_BYTES + 4096,
        ] {
            assert!(allocator.reserve_payload_chunks(&payload).is_none());
        }
        assert!(
            allocator
                .reserve_payload_chunks(&(2 * CHUNK_BYTES..2 * CHUNK_BYTES + 4096))
                .is_some()
        );
        assert!(
            allocator
                .reserve_payload_chunks(&(2 * CHUNK_BYTES..2 * CHUNK_BYTES + 8192))
                .is_some()
        );
        assert!(
            allocator
                .reserve_payload_chunks(&(2 * CHUNK_BYTES..4 * CHUNK_BYTES))
                .is_none()
        );
        allocator.remove_payload(2 * CHUNK_BYTES);
        assert_eq!(allocator.available_bytes(), 0);
        allocator.remove_payload(2 * CHUNK_BYTES);
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
            let recovered = allocator.reserve_for_recovery(0, 2);
            let written = task.join().unwrap();
            assert_ne!(recovered.is_some(), written.is_some());
        }
    }

    #[test]
    fn recovered_chunk_metrics_follow_allocator_ownership_until_reuse() {
        let (registry, backend) = registry();
        let allocator = DiskChunkAllocator::with_metrics(0..3 * CHUNK_BYTES, DiskMetrics::new(&backend)).unwrap();
        let recovered_chunks = || value(&registry, "feuer_disk_recovered_chunks", &[]);
        let inspected = allocator.reserve_for_recovery(0, 3).unwrap();
        assert_eq!(recovered_chunks(), 0.0);
        drop(inspected);
        let mut recovered = allocator.reserve_for_recovery(0, 3).unwrap();
        recovered.mark_recovered();
        recovered.mark_recovered();
        assert_eq!(recovered_chunks(), 3.0);
        allocator.retain_payloads(recovered, 1);
        assert_eq!(recovered_chunks(), 3.0);
        allocator.remove_payload(4096);
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
        allocator.retain_payloads(region, 1);
        assert_eq!(chunks("free"), 1.0);
        allocator.remove_payload(4096);
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
                        drop(region);
                    }
                });
            }
        });
        assert_all_chunks_free(&allocator, capacity);
    }
}
