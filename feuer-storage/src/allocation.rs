//! Whole-chunk allocation for immutable batch writes; see `disk-prototype.md`.

use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::DiskMetrics;

/// Size in bytes of a whole-chunk reservation. Written chunks stay immutable until
/// all entry owners and read guards release them; individual holes cannot be reused.
pub(super) const CHUNK_BYTES: u64 = 1024 * 1024;

/// Allocator reserving whole disk chunks in one independently allocated disk range.
#[derive(Clone, Debug)]
pub(super) struct DiskChunkAllocator {
    free: Arc<Mutex<DiskChunkAvailability>>,
    pub(super) chunk_capacity: u64,
}

/// Free disk-chunk ranges and availability accounting.
#[derive(Debug)]
struct DiskChunkAvailability {
    /// Consecutive free chunks: first chunk number -> count. Adjacent runs are merged.
    free_chunk_count_by_start: BTreeMap<u64, u64>,
    available_chunks: u64,
    quarantined_chunks: u64,
    metrics: Arc<DiskMetrics>,
}

impl DiskChunkAvailability {
    fn take_chunk(&mut self) -> u64 {
        let (start, count) = self.free_chunk_count_by_start.pop_first().unwrap();
        if count > 1 {
            self.free_chunk_count_by_start.insert(start + 1, count - 1);
        }
        self.available_chunks -= 1;
        self.metrics.free_chunks.decrease(1);
        self.metrics.reserved_chunks.increase(1);
        start
    }

    fn release_chunk(&mut self, chunk: u64) {
        let mut start = chunk;
        let mut count = 1;
        if let Some((&previous_start, &previous_count)) = self.free_chunk_count_by_start.range(..chunk).next_back() {
            assert!(previous_start + previous_count <= chunk);
            if previous_start + previous_count == chunk {
                start = previous_start;
                count += previous_count;
                self.free_chunk_count_by_start.remove(&previous_start);
            }
        }
        if let Some((&next_start, &next_count)) = self.free_chunk_count_by_start.range(chunk..).next() {
            assert!(next_start > chunk);
            if next_start == chunk + 1 {
                count += next_count;
                self.free_chunk_count_by_start.remove(&next_start);
            }
        }
        self.free_chunk_count_by_start.insert(start, count);
        self.available_chunks += 1;
        self.metrics.free_chunks.increase(1);
        self.metrics.reserved_chunks.decrease(1);
    }
}

impl Drop for DiskChunkAvailability {
    fn drop(&mut self) {
        // The last reservation has gone; only free or quarantined chunks remain.
        self.metrics.free_chunks.decrease(self.available_chunks);
        self.metrics.quarantined_chunks.decrease(self.quarantined_chunks);
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
        metrics
            .free_chunks
            .increase((disk_range.end - disk_range.start) / CHUNK_BYTES);
        Some(Self {
            chunk_capacity: (disk_range.end - disk_range.start) / CHUNK_BYTES,
            free: Arc::new(Mutex::new(DiskChunkAvailability {
                free_chunk_count_by_start: BTreeMap::from([(
                    disk_range.start / CHUNK_BYTES,
                    (disk_range.end - disk_range.start) / CHUNK_BYTES,
                )]),
                available_chunks: (disk_range.end - disk_range.start) / CHUNK_BYTES,
                quarantined_chunks: 0,
                metrics,
            })),
        })
    }

    /// Reserves whole chunks, not necessarily adjacent. Failure consumes no space.
    pub(super) fn reserve_chunks(&self, count: u64) -> Option<Vec<DiskRegion>> {
        let mut free = self.free.lock().unwrap();
        if count == 0 || count > free.available_chunks {
            return None;
        }
        Some(
            (0..count)
                .map(|_| {
                    let chunk = free.take_chunk();
                    DiskRegion {
                        range: chunk * CHUNK_BYTES..(chunk + 1) * CHUNK_BYTES,
                        state: Arc::new(ChunkReservation {
                            free: self.free.clone(),
                            chunk,
                            reusable: AtomicBool::new(true),
                        }),
                    }
                })
                .collect(),
        )
    }
}

/// A reserved byte range in the backing file. Subranges share ownership of their whole chunk.
#[derive(Debug)]
pub(super) struct DiskRegion {
    range: Range<u64>,
    state: Arc<ChunkReservation>,
}

/// Ownership of one whole chunk, shared by its entry regions and read guards.
#[derive(Debug)]
struct ChunkReservation {
    free: Arc<Mutex<DiskChunkAvailability>>,
    chunk: u64,
    reusable: AtomicBool,
}

impl DiskRegion {
    pub(super) fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    /// Reserves a subrange without permitting independent reuse of any part of the chunk.
    pub(super) fn slice(&self, range: Range<u64>) -> Self {
        assert!(self.range.start <= range.start && range.start < range.end && range.end <= self.range.end);
        Self {
            range,
            state: self.state.clone(),
        }
    }

    pub(super) fn read_guard(&self) -> DiskRegionReadGuard {
        DiskRegionReadGuard {
            region: self.slice(self.range()),
        }
    }

    /// Unknown write completion forbids reuse of the entire chunk for this allocator's lifetime.
    pub(super) fn quarantine(self) {
        if self.state.reusable.swap(false, Ordering::Relaxed) {
            let mut free = self.state.free.lock().unwrap();
            free.quarantined_chunks += 1;
            free.metrics.reserved_chunks.decrease(1);
            free.metrics.quarantined_chunks.increase(1);
        }
    }
}

/// Prevents its containing chunk from being overwritten or reused while a read depends on this region.
#[derive(Debug)]
pub(super) struct DiskRegionReadGuard {
    region: DiskRegion,
}

impl Clone for DiskRegionReadGuard {
    fn clone(&self) -> Self {
        self.region.read_guard()
    }
}

impl DiskRegionReadGuard {
    pub(super) fn range(&self) -> Range<u64> {
        self.region.range()
    }
}

impl Drop for ChunkReservation {
    fn drop(&mut self) {
        if *self.reusable.get_mut() {
            self.free.lock().unwrap().release_chunk(self.chunk);
        }
    }
}

#[cfg(test)]
mod tests;
