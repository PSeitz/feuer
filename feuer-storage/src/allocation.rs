//! Two-level allocation candidate; see `disk-prototype.md`.

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
const BLOCKS_PER_CHUNK: u16 = (CHUNK_BYTES / BLOCK_BYTES) as u16;
const PAYLOAD_BLOCKS_PER_CHUNK: u16 = BLOCKS_PER_CHUNK - 1;

/// Free storage in one arena. The first block of every chunk belongs to metadata.
#[derive(Clone, Debug)]
pub(super) struct DiskAllocator {
    free: Arc<Mutex<FreeSpace>>,
}

#[derive(Debug)]
struct FreeSpace {
    /// Whole free chunks, represented as start -> exclusive end.
    chunks: BTreeMap<u64, u64>,
    /// Present only when a chunk has both allocated and free payload blocks.
    blocks: BTreeMap<u64, FreeBlocks>,
    available_blocks: u64,
}

/// Free payload blocks in one subdivided chunk; bit zero is always clear.
#[derive(Debug)]
struct FreeBlocks([u64; 4]);

impl FreeBlocks {
    const ALL: Self = Self([u64::MAX - 1, u64::MAX, u64::MAX, u64::MAX]);
    const NONE: Self = Self([0; 4]);

    fn contains(&self, block: u16) -> bool {
        self.0[usize::from(block / 64)] & (1 << (block % 64)) != 0
    }

    fn take(&mut self, requested: u64) -> Range<u16> {
        let word = self.0.iter().position(|&word| word != 0).unwrap();
        let start = (word * 64) as u16 + self.0[word].trailing_zeros() as u16;
        let mut end = start;
        while end < BLOCKS_PER_CHUNK && u64::from(end - start) < requested && self.contains(end) {
            self.0[usize::from(end / 64)] &= !(1 << (end % 64));
            end += 1;
        }
        start..end
    }

    fn release(&mut self, blocks: Range<u16>) {
        for block in blocks {
            assert!(!self.contains(block));
            self.0[usize::from(block / 64)] |= 1 << (block % 64);
        }
    }
}

impl FreeSpace {
    fn take_chunk(&mut self) -> u64 {
        let (start, end) = self.chunks.pop_first().unwrap();
        if start + 1 < end {
            self.chunks.insert(start + 1, end);
        }
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
    }

    fn release(&mut self, range: &Range<u64>) {
        let chunk = range.start / CHUNK_BYTES;
        let first = ((range.start % CHUNK_BYTES) / BLOCK_BYTES) as u16;
        let count = ((range.end - range.start) / BLOCK_BYTES) as u16;
        if count == PAYLOAD_BLOCKS_PER_CHUNK {
            assert!(!self.blocks.contains_key(&chunk));
            self.release_chunk(chunk);
        } else {
            let blocks = self.blocks.entry(chunk).or_insert(FreeBlocks::NONE);
            blocks.release(first..first + count);
            if blocks.0 == FreeBlocks::ALL.0 {
                self.blocks.remove(&chunk);
                self.release_chunk(chunk);
            }
        }
        self.available_blocks += u64::from(count);
    }
}

impl DiskAllocator {
    #[cfg(test)]
    fn new(capacity: u64) -> Option<Self> {
        Self::for_disk_range(0..capacity)
    }

    #[cfg(test)]
    pub(super) fn available_bytes(&self) -> u64 {
        self.free.lock().unwrap().available_blocks * BLOCK_BYTES
    }

    pub(super) fn for_disk_range(disk_range: Range<u64>) -> Option<Self> {
        if disk_range.start >= disk_range.end
            || disk_range.end > i64::MAX as u64
            || !disk_range.start.is_multiple_of(CHUNK_BYTES)
            || !disk_range.end.is_multiple_of(CHUNK_BYTES)
        {
            return None;
        }
        let chunks = (disk_range.end - disk_range.start) / CHUNK_BYTES;
        Some(Self {
            free: Arc::new(Mutex::new(FreeSpace {
                chunks: BTreeMap::from([(disk_range.start / CHUNK_BYTES, disk_range.end / CHUNK_BYTES)]),
                blocks: BTreeMap::new(),
                available_blocks: chunks * u64::from(PAYLOAD_BLOCKS_PER_CHUNK),
            })),
        })
    }

    /// Reserves aligned storage, in logical byte order, without requiring physical adjacency.
    /// The caller must separately charge entry metadata and other overflow metadata.
    /// A failed reservation does not change free-space state.
    pub(super) fn reserve(&self, bytes: u64) -> Option<Vec<DiskRegion>> {
        let mut remaining = bytes.div_ceil(BLOCK_BYTES);
        let mut free = self.free.lock().unwrap();
        if remaining == 0 || remaining > free.available_blocks {
            return None;
        }
        free.available_blocks -= remaining;
        let mut regions = Vec::new();
        while remaining != 0 {
            let (chunk, blocks) = if remaining >= u64::from(PAYLOAD_BLOCKS_PER_CHUNK) && !free.chunks.is_empty() {
                (free.take_chunk(), 1..BLOCKS_PER_CHUNK)
            } else {
                if free.blocks.is_empty() {
                    let chunk = free.take_chunk();
                    free.blocks.insert(chunk, FreeBlocks::ALL);
                }
                let (&chunk, blocks) = free.blocks.first_key_value().unwrap();
                debug_assert!(blocks.0 != FreeBlocks::NONE.0);
                let blocks = free.blocks.get_mut(&chunk).unwrap();
                let taken = blocks.take(remaining);
                if blocks.0 == FreeBlocks::NONE.0 {
                    free.blocks.remove(&chunk);
                }
                (chunk, taken)
            };
            remaining -= u64::from(blocks.end - blocks.start);
            regions.push(DiskRegion {
                state: Arc::new(DiskRegionState {
                    free: self.free.clone(),
                    range: chunk * CHUNK_BYTES + u64::from(blocks.start) * BLOCK_BYTES
                        ..chunk * CHUNK_BYTES + u64::from(blocks.end) * BLOCK_BYTES,
                    reusable: AtomicBool::new(true),
                }),
            });
        }
        Some(regions)
    }
}

/// A reserved byte range in the backing file. A writer must retain ownership through completion.
/// Shared packed entries can own an `Arc<DiskRegion>` until whole-block eviction.
#[derive(Debug)]
pub(super) struct DiskRegion {
    state: Arc<DiskRegionState>,
}

#[derive(Debug)]
struct DiskRegionState {
    free: Arc<Mutex<FreeSpace>>,
    range: Range<u64>,
    reusable: AtomicBool,
}

impl DiskRegion {
    pub(super) fn range(&self) -> Range<u64> {
        self.state.range.clone()
    }

    /// Called by the range index only after successful, revalidated publication.
    pub(super) fn read_guard(&self) -> DiskRegionReadGuard {
        DiskRegionReadGuard {
            state: self.state.clone(),
        }
    }

    /// Unknown write completion forbids reuse for the lifetime of this allocator.
    pub(super) fn quarantine(self) {
        self.state.reusable.store(false, Ordering::Relaxed);
    }
}

/// Prevents overwrite or reuse while a read depends on this region's contents.
#[derive(Debug)]
pub(super) struct DiskRegionReadGuard {
    state: Arc<DiskRegionState>,
}

impl DiskRegionReadGuard {
    pub(super) fn range(&self) -> Range<u64> {
        self.state.range.clone()
    }
}

impl Drop for DiskRegionState {
    fn drop(&mut self) {
        if *self.reusable.get_mut() {
            self.free.lock().unwrap().release(&self.range);
        }
    }
}

#[cfg(test)]
mod tests;
