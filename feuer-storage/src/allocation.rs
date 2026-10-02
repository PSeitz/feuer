//! Ownership and reuse of consecutive whole chunks.

use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::DiskMetrics;

/// Payload chunks stay immutable while owned; metadata updates require caller-owned synchronization.
pub(super) const CHUNK_BYTES: u64 = 1024 * 1024;

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
    // First chunk number and recovery claim bits. Releasing chunks does not clear their bits,
    // so recovery cannot reserve a previously claimed chunk again.
    recovery_claims: Option<(u64, Vec<u64>)>,
    metrics: Arc<DiskMetrics>,
}

impl DiskChunkAvailability {
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
        Some(())
    }

    fn mark_claimed(&mut self, chunks: Range<u64>) {
        if let Some((start, words)) = &mut self.recovery_claims {
            for chunk in chunks {
                if let Some(relative) = chunk.checked_sub(*start)
                    && let Some(word) = words.get_mut((relative / 64) as usize)
                {
                    *word |= 1 << (relative % 64);
                }
            }
        }
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

impl Drop for DiskChunkAvailability {
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
            free: Arc::new(Mutex::new(DiskChunkAvailability {
                free_chunk_count_by_start: BTreeMap::from([(disk_range.start / CHUNK_BYTES, chunk_capacity)]),
                available_chunks: chunk_capacity,
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

    /// Reserves old chunks for inspection without marking them claimed. Previously claimed chunks
    /// never recover again, even after their new owners release them.
    pub(super) fn reserve_for_recovery(&self, first: u64, count: u64) -> Option<DiskRegion> {
        let chunks = first..first.checked_add(count)?;
        let mut free = self.free.lock().unwrap();
        let (start, words) = free.recovery_claims.as_ref()?;
        for chunk in chunks.clone() {
            let relative = chunk.checked_sub(*start)?;
            if words.get((relative / 64) as usize)? & (1 << (relative % 64)) != 0 {
                return None;
            }
        }
        free.remove_free_chunks(chunks.clone())?;
        free.metrics.free_chunks.decrease(count);
        free.metrics.allocated_chunks.increase(count);
        Some(self.region(chunks, false))
    }

    /// Startup payload recovery retains every region and reports metrics for each batch.
    /// Metadata is already reserved; retained regions prevent payload chunks from being claimed twice.
    pub(super) fn recover_payload_chunks(&self, first: u64, count: u64) -> Option<DiskRegion> {
        let chunks = first..first.checked_add(count)?;
        self.free.lock().unwrap().remove_free_chunks(chunks.clone())?;
        Some(self.region(chunks, true))
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
        free.mark_claimed(chunks.clone());
        free.metrics.free_chunks.decrease(count);
        free.metrics.allocated_chunks.increase(count);
        Some(self.region(chunks, false))
    }

    fn region(&self, chunks: Range<u64>, recovered: bool) -> DiskRegion {
        DiskRegion {
            range: chunks.start * CHUNK_BYTES..chunks.end * CHUNK_BYTES,
            reservation: Arc::new(ChunkReservation {
                free: self.free.clone(),
                chunks,
                recovered: AtomicBool::new(recovered),
            }),
        }
    }
}

/// A reserved byte range in the backing file. Subranges retain all reserved chunks.
#[derive(Debug)]
pub(super) struct DiskRegion {
    range: Range<u64>,
    reservation: Arc<ChunkReservation>,
}

/// Ownership of consecutive whole chunks, shared by entry regions and read guards.
#[derive(Debug)]
struct ChunkReservation {
    free: Arc<Mutex<DiskChunkAvailability>>,
    chunks: Range<u64>,
    recovered: AtomicBool,
}

impl DiskRegion {
    pub(super) fn range(&self) -> Range<u64> {
        self.range.clone()
    }

    pub(super) fn chunk_count(&self) -> u64 {
        self.reservation.chunks.end - self.reservation.chunks.start
    }

    /// Creates a subrange sharing ownership of all reserved chunks.
    pub(super) fn slice(&self, range: Range<u64>) -> Self {
        assert!(self.range.start <= range.start && range.start < range.end && range.end <= self.range.end);
        Self {
            range,
            reservation: self.reservation.clone(),
        }
    }

    /// Counts all recovered chunks until their last owner/read guard releases them.
    pub(super) fn mark_recovered(&self) {
        let mut free = self.reservation.free.lock().unwrap();
        free.mark_claimed(self.reservation.chunks.clone());
        if !self.reservation.recovered.swap(true, Ordering::Relaxed) {
            free.metrics.recovered_chunks.increase(self.chunk_count());
        }
    }

    pub(super) fn read_guard(&self) -> DiskRegionReadGuard {
        DiskRegionReadGuard {
            region: self.slice(self.range()),
        }
    }
}

/// Prevents all reserved chunks from being reused while a read depends on this region.
/// In-place metadata updates require separate caller-owned read/write synchronization.
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
            free.metrics
                .recovered_chunks
                .decrease(self.chunks.end - self.chunks.start);
        }
        free.release(self.chunks.clone());
    }
}

#[cfg(test)]
mod tests;
