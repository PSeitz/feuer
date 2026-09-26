//! Test-only two-level allocation candidate; see `disk-prototype.md`.

use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const BLOCK_BYTES: u64 = 4096;
const UNIT_BYTES: u64 = 1024 * 1024;
const BLOCKS_PER_UNIT: u16 = (UNIT_BYTES / BLOCK_BYTES) as u16;
const PAYLOAD_BLOCKS_PER_UNIT: u16 = BLOCKS_PER_UNIT - 1;

/// Free storage in one arena. The first block of every unit belongs to metadata.
#[derive(Clone, Debug)]
struct DiskAllocator {
    free: Arc<Mutex<FreeSpace>>,
}

#[derive(Debug)]
struct FreeSpace {
    /// Whole free units, represented as start -> exclusive end.
    units: BTreeMap<u64, u64>,
    /// Present only when a unit has both allocated and free payload blocks.
    blocks: BTreeMap<u64, FreeBlocks>,
    available_blocks: u64,
}

/// Free payload blocks in one subdivided unit; bit zero is always clear.
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
        while end < BLOCKS_PER_UNIT && u64::from(end - start) < requested && self.contains(end) {
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
    fn take_unit(&mut self) -> u64 {
        let (start, end) = self.units.pop_first().unwrap();
        if start + 1 < end {
            self.units.insert(start + 1, end);
        }
        start
    }

    fn release_unit(&mut self, unit: u64) {
        let mut start = unit;
        let mut end = unit + 1;
        if let Some((&previous_start, &previous_end)) = self.units.range(..unit).next_back() {
            assert!(previous_end <= unit);
            if previous_end == unit {
                start = previous_start;
                self.units.remove(&previous_start);
            }
        }
        if let Some((&next_start, &next_end)) = self.units.range(unit..).next() {
            assert!(next_start >= end);
            if next_start == end {
                end = next_end;
                self.units.remove(&next_start);
            }
        }
        self.units.insert(start, end);
    }

    fn release(&mut self, range: &Range<u64>) {
        let unit = range.start / UNIT_BYTES;
        let first = ((range.start % UNIT_BYTES) / BLOCK_BYTES) as u16;
        let count = ((range.end - range.start) / BLOCK_BYTES) as u16;
        if count == PAYLOAD_BLOCKS_PER_UNIT {
            assert!(!self.blocks.contains_key(&unit));
            self.release_unit(unit);
        } else {
            let blocks = self.blocks.entry(unit).or_insert(FreeBlocks::NONE);
            blocks.release(first..first + count);
            if blocks.0 == FreeBlocks::ALL.0 {
                self.blocks.remove(&unit);
                self.release_unit(unit);
            }
        }
        self.available_blocks += u64::from(count);
    }
}

impl DiskAllocator {
    fn new(capacity: u64) -> Option<Self> {
        if capacity == 0 || capacity > i64::MAX as u64 || !capacity.is_multiple_of(UNIT_BYTES) {
            return None;
        }
        let units = capacity / UNIT_BYTES;
        Some(Self {
            free: Arc::new(Mutex::new(FreeSpace {
                units: BTreeMap::from([(0, units)]),
                blocks: BTreeMap::new(),
                available_blocks: units * u64::from(PAYLOAD_BLOCKS_PER_UNIT),
            })),
        })
    }

    /// Reserves aligned storage, in logical byte order, without requiring physical adjacency.
    /// The caller must separately charge entry descriptors and other overflow metadata.
    /// A failed reservation does not change free-space state.
    fn reserve(&self, bytes: u64) -> Option<Vec<DiskRegion>> {
        let mut remaining = bytes.div_ceil(BLOCK_BYTES);
        let mut free = self.free.lock().unwrap();
        if remaining == 0 || remaining > free.available_blocks {
            return None;
        }
        free.available_blocks -= remaining;
        let mut regions = Vec::new();
        while remaining != 0 {
            let (unit, blocks) = if remaining >= u64::from(PAYLOAD_BLOCKS_PER_UNIT) && !free.units.is_empty() {
                (free.take_unit(), 1..BLOCKS_PER_UNIT)
            } else {
                if free.blocks.is_empty() {
                    let unit = free.take_unit();
                    free.blocks.insert(unit, FreeBlocks::ALL);
                }
                let (&unit, blocks) = free.blocks.first_key_value().unwrap();
                debug_assert!(blocks.0 != FreeBlocks::NONE.0);
                let blocks = free.blocks.get_mut(&unit).unwrap();
                let taken = blocks.take(remaining);
                if blocks.0 == FreeBlocks::NONE.0 {
                    free.blocks.remove(&unit);
                }
                (unit, taken)
            };
            remaining -= u64::from(blocks.end - blocks.start);
            regions.push(DiskRegion {
                state: Arc::new(DiskRegionState {
                    free: self.free.clone(),
                    range: unit * UNIT_BYTES + u64::from(blocks.start) * BLOCK_BYTES
                        ..unit * UNIT_BYTES + u64::from(blocks.end) * BLOCK_BYTES,
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
struct DiskRegion {
    state: Arc<DiskRegionState>,
}

#[derive(Debug)]
struct DiskRegionState {
    free: Arc<Mutex<FreeSpace>>,
    range: Range<u64>,
    reusable: AtomicBool,
}

impl DiskRegion {
    fn range(&self) -> Range<u64> {
        self.state.range.clone()
    }

    /// Called by the range index only after successful, revalidated publication.
    fn read_guard(&self) -> DiskRegionReadGuard {
        DiskRegionReadGuard {
            state: self.state.clone(),
        }
    }

    /// Unknown write completion forbids reuse for the lifetime of this allocator.
    fn quarantine(self) {
        self.state.reusable.store(false, Ordering::Relaxed);
    }
}

/// Prevents overwrite or reuse while a read depends on this region's contents.
#[derive(Debug)]
struct DiskRegionReadGuard {
    state: Arc<DiskRegionState>,
}

impl DiskRegionReadGuard {
    fn range(&self) -> Range<u64> {
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
