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
    // Sticky during recovery, including chunks freed after writes or recovery claimed them.
    recovery_claims: Option<(u64, Vec<u64>)>,
    metrics: Arc<DiskMetrics>,
}

impl DiskChunkAvailability {
    /// Reserves the first free chunk by disk address and updates chunk accounting.
    fn reserve_first_free_chunk(&mut self) -> u64 {
        let (first_chunk, chunk_count) = self.free_chunk_count_by_start.pop_first().unwrap();
        if chunk_count > 1 {
            self.free_chunk_count_by_start.insert(first_chunk + 1, chunk_count - 1);
        }
        self.mark_claimed(first_chunk);
        self.account_reservation();
        first_chunk
    }

    fn account_reservation(&mut self) {
        self.available_chunks -= 1;
        self.metrics.free_chunks.decrease(1);
        self.metrics.allocated_chunks.increase(1);
    }

    fn mark_claimed(&mut self, chunk: u64) {
        if let Some((start, words)) = &mut self.recovery_claims
            && let Some(word) = words.get_mut(((chunk - *start) / 64) as usize)
        {
            *word |= 1 << ((chunk - *start) % 64);
        }
    }

    fn release_chunk(&mut self, chunk_number: u64) {
        let mut first_chunk = chunk_number;
        let mut chunk_count = 1;
        if let Some((&previous_start, &previous_count)) =
            self.free_chunk_count_by_start.range(..chunk_number).next_back()
        {
            assert!(previous_start + previous_count <= chunk_number);
            if previous_start + previous_count == chunk_number {
                first_chunk = previous_start;
                chunk_count += previous_count;
                self.free_chunk_count_by_start.remove(&previous_start);
            }
        }
        if let Some((&next_start, &next_count)) = self.free_chunk_count_by_start.range(chunk_number..).next() {
            assert!(next_start > chunk_number);
            if next_start == chunk_number + 1 {
                chunk_count += next_count;
                self.free_chunk_count_by_start.remove(&next_start);
            }
        }
        self.free_chunk_count_by_start.insert(first_chunk, chunk_count);
        self.available_chunks += 1;
        self.metrics.free_chunks.increase(1);
        self.metrics.allocated_chunks.decrease(1);
    }
}

impl Drop for DiskChunkAvailability {
    fn drop(&mut self) {
        // The last reservation has gone; only free chunks remain.
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
                recovery_claims: None,
                metrics,
            })),
        })
    }

    /// Must run before exposing the allocator to writes. Only the startup scan needs these bits.
    pub(super) fn start_recovery(&self, start: u64, end: u64) {
        self.free.lock().unwrap().recovery_claims = Some((
            start / CHUNK_BYTES,
            vec![0; ((end - start) / CHUNK_BYTES).div_ceil(64) as usize],
        ));
    }

    pub(super) fn finish_recovery(&self) {
        self.free.lock().unwrap().recovery_claims = None;
    }

    /// Reserves an old chunk for inspection without marking it claimed. The reservation prevents
    /// writes from overwriting metadata during I/O. Previously claimed chunks never recover again.
    pub(super) fn reserve_for_recovery(&self, chunk: u64) -> Option<DiskRegion> {
        let mut free = self.free.lock().unwrap();
        let (start, words) = free.recovery_claims.as_ref()?;
        let relative = chunk.checked_sub(*start)?;
        if words.get((relative / 64) as usize)? & (1 << (relative % 64)) != 0 {
            return None;
        }
        let (&run_start, &count) = free.free_chunk_count_by_start.range(..=chunk).next_back()?;
        if chunk >= run_start + count {
            return None;
        }
        free.free_chunk_count_by_start.remove(&run_start);
        if chunk > run_start {
            free.free_chunk_count_by_start.insert(run_start, chunk - run_start);
        }
        if chunk + 1 < run_start + count {
            free.free_chunk_count_by_start
                .insert(chunk + 1, run_start + count - chunk - 1);
        }
        free.account_reservation();
        Some(DiskRegion {
            range: chunk * CHUNK_BYTES..(chunk + 1) * CHUNK_BYTES,
            reservation: Arc::new(ChunkReservation {
                free: self.free.clone(),
                chunk_number: chunk,
                recovered: AtomicBool::new(false),
            }),
        })
    }

    /// Reserves whole chunks, not necessarily adjacent. Failure consumes no space.
    pub(super) fn reserve_chunks(&self, chunk_count: u64) -> Option<Vec<DiskRegion>> {
        let mut free = self.free.lock().unwrap();
        if chunk_count == 0 || chunk_count > free.available_chunks {
            return None;
        }
        Some(
            (0..chunk_count)
                .map(|_| {
                    let chunk_number = free.reserve_first_free_chunk();
                    DiskRegion {
                        range: chunk_number * CHUNK_BYTES..(chunk_number + 1) * CHUNK_BYTES,
                        reservation: Arc::new(ChunkReservation {
                            free: self.free.clone(),
                            chunk_number,
                            recovered: AtomicBool::new(false),
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
    reservation: Arc<ChunkReservation>,
}

/// Ownership of one whole chunk, shared by its entry regions and read guards.
#[derive(Debug)]
struct ChunkReservation {
    free: Arc<Mutex<DiskChunkAvailability>>,
    chunk_number: u64,
    recovered: AtomicBool,
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
            reservation: self.reservation.clone(),
        }
    }

    /// Counts this successfully recovered chunk until its last owner/read guard releases it,
    /// and prevents its old contents from being recovered again during this scan.
    pub(super) fn mark_recovered(&self) {
        let mut free = self.reservation.free.lock().unwrap();
        free.mark_claimed(self.reservation.chunk_number);
        if !self.reservation.recovered.swap(true, Ordering::Relaxed) {
            free.metrics.recovered_chunks.increase(1);
        }
    }

    pub(super) fn read_guard(&self) -> DiskRegionReadGuard {
        DiskRegionReadGuard {
            region: self.slice(self.range()),
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
        let mut free = self.free.lock().unwrap();
        if *self.recovered.get_mut() {
            free.metrics.recovered_chunks.decrease(1);
        }
        free.release_chunk(self.chunk_number);
    }
}

#[cfg(test)]
mod tests;
