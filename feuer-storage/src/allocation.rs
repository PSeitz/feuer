//! Whole-chunk allocation for immutable batch writes; see `disk-prototype.md`.

use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) const BLOCK_BYTES: u64 = 4096;
pub(super) const CHUNK_BYTES: u64 = 1024 * 1024;

/// Free whole chunks in one independently allocated disk range.
#[derive(Clone, Debug)]
pub(super) struct DiskAllocator {
    free: Arc<Mutex<FreeSpace>>,
}

#[derive(Debug)]
struct FreeSpace {
    /// Coalesced free runs, represented as chunk number -> exclusive end.
    chunks: BTreeMap<u64, u64>,
    available_chunks: u64,
}

impl FreeSpace {
    fn take_chunk(&mut self) -> u64 {
        let (start, end) = self.chunks.pop_first().unwrap();
        if start + 1 < end {
            self.chunks.insert(start + 1, end);
        }
        self.available_chunks -= 1;
        start
    }

    fn release_chunk(&mut self, chunk: u64) {
        let mut start = chunk;
        let mut end = chunk + 1;
        if let Some((&previous_start, &previous_end)) = self.chunks.range(..chunk).next_back() {
            assert!(previous_end <= chunk);
            if previous_end == chunk {
                start = previous_start;
                self.chunks.remove(&previous_start);
            }
        }
        if let Some((&next_start, &next_end)) = self.chunks.range(chunk..).next() {
            assert!(next_start >= end);
            if next_start == end {
                end = next_end;
                self.chunks.remove(&next_start);
            }
        }
        self.chunks.insert(start, end);
        self.available_chunks += 1;
    }
}

impl DiskAllocator {
    #[cfg(test)]
    fn new(capacity: u64) -> Option<Self> {
        Self::for_disk_range(0..capacity)
    }

    #[cfg(test)]
    pub(super) fn available_bytes(&self) -> u64 {
        self.free.lock().unwrap().available_chunks * CHUNK_BYTES
    }

    pub(super) fn for_disk_range(disk_range: Range<u64>) -> Option<Self> {
        if disk_range.start >= disk_range.end
            || disk_range.end > i64::MAX as u64
            || !disk_range.start.is_multiple_of(CHUNK_BYTES)
            || !disk_range.end.is_multiple_of(CHUNK_BYTES)
        {
            return None;
        }
        Some(Self {
            free: Arc::new(Mutex::new(FreeSpace {
                chunks: BTreeMap::from([(disk_range.start / CHUNK_BYTES, disk_range.end / CHUNK_BYTES)]),
                available_chunks: (disk_range.end - disk_range.start) / CHUNK_BYTES,
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
    free: Arc<Mutex<FreeSpace>>,
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
        self.state.reusable.store(false, Ordering::Relaxed);
    }
}

/// Prevents its containing chunk from being overwritten or reused while a read depends on this region.
#[derive(Debug)]
pub(super) struct DiskRegionReadGuard {
    region: DiskRegion,
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
