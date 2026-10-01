use std::{collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use feuer_types::{
    ByteRange, ObjectKeyHash,
    retention::{ObjectAccessHistories, RECLAIM_SAMPLE_SIZE, compare_cost_per_byte, sample_candidates},
};
use rustc_hash::FxHashMap;

use super::range_trim::{RangeTrimPlan, plan_range_trim};
use crate::{BufferPool, MemoryMetrics};

/// Minimum requests across all keys since admission before payload compaction is allowed.
pub(super) const MIN_ACCESSES_BEFORE_PAYLOAD_COMPACTION: u64 = 64;

/// One retained downloaded range.
struct CachedRange {
    /// Monotonic identity used for revalidation and policy tie-breaking.
    id: u64,
    /// Exact object interval represented by `bytes`.
    range: ByteRange,
    /// Retained payload; lookup results share slices of this allocation.
    bytes: Bytes,
    /// Capacity of the backing allocation, possibly larger than the visible payload.
    capacity: u64,
    /// Slot in the bounded-work policy candidate ring.
    candidate_slot: usize,
    /// Request clock at admission.
    admitted_at_access: u64,
}

impl CachedRange {
    fn requested_bytes(&self, requested_range: ByteRange) -> Bytes {
        debug_assert!(self.range.contains(requested_range));
        let start = usize::try_from(requested_range.start() - self.range.start())
            .expect("an offset within a Bytes payload must fit in usize");
        let end = usize::try_from(requested_range.end() - self.range.start())
            .expect("an offset within a Bytes payload must fit in usize");
        self.bytes.slice(start..end)
    }
}

/// Ordered cached downloaded ranges for one cache key.
///
/// No cached range fully contains another. Partial overlaps remain indexed.
/// Because starts and ends both increase, the predecessor of a request start
/// is the only possible covering entry.
#[derive(Default)]
struct ObjectCachedRanges {
    /// Entries ordered by exact start for predecessor-based covering lookup.
    by_start: BTreeMap<u64, CachedRange>,
    /// Structural generation used by copy-outside-lock range trimming.
    object_generation: u64,
}

impl ObjectCachedRanges {
    /// Finds the cached range covering the entire requested range.
    fn covering_range(&self, range: ByteRange) -> Option<&CachedRange> {
        let (_, entry) = self.by_start.range(..=range.start()).next_back()?;
        entry.range.contains(range).then_some(entry)
    }

    /// Collects cached ranges contained by the incoming range and their allocation charges.
    fn ranges_contained_by(&self, range: ByteRange) -> ContainedCachedRanges {
        let mut contained_ranges = ContainedCachedRanges::default();
        for (_, entry) in self.by_start.range(range.start()..range.end()) {
            if range.contains(entry.range) {
                contained_ranges.ranges.push(entry.range);
                contained_ranges.allocation_bytes += entry.capacity;
            }
        }
        contained_ranges
    }
}

/// Cached ranges fully contained in an incoming download, with their allocation charges.
#[derive(Default)]
struct ContainedCachedRanges {
    ranges: Vec<ByteRange>,
    allocation_bytes: u64,
}

/// Usage removed while admitting one download.
#[derive(Default)]
struct RemovedCacheUsage {
    allocation_bytes: u64,
    entry_count: u64,
}

/// Object key and range start of a cached entry.
#[derive(Clone)]
struct ObjectKeyAndRangeStart {
    object_key: ObjectKeyHash,
    start: u64,
}

/// Rotating ring of cached-range candidates for reclaiming memory by trimming or eviction.
#[derive(Default)]
struct ReclaimCandidateRing {
    entries: Vec<ObjectKeyAndRangeStart>,
    cursor: usize,
}

impl ReclaimCandidateRing {
    fn register(&mut self, candidate: ObjectKeyAndRangeStart) -> usize {
        let slot = self.entries.len();
        self.entries.push(candidate);
        slot
    }

    /// Removes `slot` and returns the candidate moved into it, if any.
    fn remove(&mut self, slot: usize) -> Option<ObjectKeyAndRangeStart> {
        let last_slot = self.entries.len() - 1;
        self.entries.swap_remove(slot);
        let moved_candidate = (slot != last_slot).then(|| self.entries[slot].clone());
        if self.entries.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor %= self.entries.len();
        }
        moved_candidate
    }

    fn sample(&mut self, sample_size: usize) -> (usize, usize) {
        sample_candidates(&mut self.cursor, self.entries.len(), sample_size)
    }
}

/// The cached range selected for reclaiming memory.
struct ReclaimCandidate {
    object_key: ObjectKeyHash,
    range: ByteRange,
    id: u64,
}

/// Source payload, plan, and identity for trimming a cached range outside the shard lock.
pub(super) struct RangeTrimSource {
    object_key: ObjectKeyHash,
    start: u64,
    id: u64,
    object_generation: u64,
    plan: RangeTrimPlan,
    source_bytes: Bytes,
}

impl RangeTrimSource {
    /// Copies the retained payloads into separate allocations while no shard metadata lock is held.
    pub(super) fn copy_retained_payloads(self) -> RangeTrimReplacement {
        let retained_payloads = self
            .plan
            .retained_ranges()
            .iter()
            .map(|range| {
                let start = usize::try_from(range.start() - self.plan.source_range().start())
                    .expect("a planned source offset must fit in usize");
                let end = usize::try_from(range.end() - self.plan.source_range().start())
                    .expect("a planned source offset must fit in usize");
                (*range, Bytes::copy_from_slice(&self.source_bytes[start..end]))
            })
            .collect();
        RangeTrimReplacement {
            object_key: self.object_key,
            start: self.start,
            id: self.id,
            object_generation: self.object_generation,
            plan: self.plan,
            retained_payloads,
        }
    }
}

/// Replacement payloads from trimming a cached range, awaiting identity/generation checks.
pub(super) struct RangeTrimReplacement {
    object_key: ObjectKeyHash,
    start: u64,
    id: u64,
    object_generation: u64,
    plan: RangeTrimPlan,
    retained_payloads: Vec<(ByteRange, Bytes)>,
}

/// Result of trying to insert a download or reclaim space:
/// finished, evicted an entry, retry, or trim a range.
pub(super) enum InsertOrReclaimResult {
    Complete(Option<u64>),
    Evicted,
    Retry,
    Trim(RangeTrimSource),
}

/// One memory-cache shard's range indexes and allocation accounting.
pub(super) struct MemoryCacheShard {
    capacity: u64,
    pub(super) reclaim_sample_size: usize,
    used_bytes: u64,
    ranges: FxHashMap<ObjectKeyHash, ObjectCachedRanges>,
    next_entry_id: u64,
    /// Object keys and range starts, sampled in rotation to choose a range to trim or evict.
    candidates: ReclaimCandidateRing,
    metrics: Arc<MemoryMetrics>,
    buffer_pool: Arc<BufferPool>,
}

impl MemoryCacheShard {
    pub(super) fn new(capacity: u64, metrics: Arc<MemoryMetrics>, buffer_pool: Arc<BufferPool>) -> Self {
        Self {
            capacity,
            reclaim_sample_size: RECLAIM_SAMPLE_SIZE,
            used_bytes: 0,
            ranges: FxHashMap::default(),
            next_entry_id: 0,
            candidates: ReclaimCandidateRing::default(),
            metrics,
            buffer_pool,
        }
    }

    #[cfg(test)]
    pub(super) const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    pub(super) fn get(&self, object_key: &ObjectKeyHash, requested_range: ByteRange) -> Option<Bytes> {
        self.ranges
            .get(object_key)?
            .covering_range(requested_range)
            .map(|entry| entry.requested_bytes(requested_range))
    }

    /// Tries to admit the download or reclaim space by evicting one range or preparing a trim.
    /// Cached ranges fully contained in the incoming download will be replaced on insertion.
    /// Their bytes are already subtracted when checking capacity, so eviction sampling skips them.
    /// If a sample has no eligible victim, the caller releases the lock before trying the next sample
    /// to bound sampling work per lock hold.
    pub(super) fn try_admit_or_reclaim(
        &mut self,
        object_key: &ObjectKeyHash,
        range: ByteRange,
        bytes: &Bytes,
        capacity: u64,
        access_histories: &ObjectAccessHistories,
        allow_range_trim: bool,
    ) -> InsertOrReclaimResult {
        let contained_ranges = match self.ranges.get(object_key) {
            Some(entries) if entries.covering_range(range).is_some() => {
                self.metrics.record_redundant();
                return InsertOrReclaimResult::Complete(None);
            }
            Some(entries) => entries.ranges_contained_by(range),
            None => ContainedCachedRanges::default(),
        };
        let added_bytes = capacity;
        let used_bytes_without_contained_ranges = self.used_bytes - contained_ranges.allocation_bytes;
        let max_existing_bytes = self.capacity.saturating_sub(added_bytes);

        if used_bytes_without_contained_ranges <= max_existing_bytes {
            let removal = self.remove_contained_ranges(object_key, &contained_ranges.ranges);
            debug_assert_eq!(removal.allocation_bytes, contained_ranges.allocation_bytes);
            let id =
                self.insert_downloaded_range(*object_key, range, bytes.clone(), capacity, access_histories.clock());

            if removal.entry_count != 0 {
                self.metrics
                    .decrease_usage(removal.allocation_bytes, removal.entry_count);
            }
            self.metrics.increase_usage(added_bytes, 1);
            self.metrics.record_insert(removal.entry_count != 0);
            return InsertOrReclaimResult::Complete(Some(id));
        }

        let Some(candidate) = self.select_reclaim_candidate(object_key, range, access_histories) else {
            // The sample cursor advanced, but no eligible victim was found.
            // Return to the caller so it can release the lock before sampling again.
            return InsertOrReclaimResult::Retry;
        };
        if allow_range_trim && let Some(source) = self.prepare_range_trim(&candidate, access_histories) {
            return InsertOrReclaimResult::Trim(source);
        }

        let removed_bytes = self
            .remove_entry(&candidate.object_key, candidate.range, Some(candidate.id))
            .expect("a sampled pressure candidate cannot disappear while its shard is locked");
        self.metrics.decrease_usage(removed_bytes, 1);
        InsertOrReclaimResult::Evicted
    }

    /// Inserts a downloaded range, charging its backing allocation capacity.
    fn insert_downloaded_range(
        &mut self,
        object_key: ObjectKeyHash,
        range: ByteRange,
        bytes: Bytes,
        capacity: u64,
        access_clock: u64,
    ) -> u64 {
        self.used_bytes += capacity;
        self.buffer_pool.add_cached(capacity);
        let id = self.allocate_entry_id();
        let candidate_slot = self.candidates.register(ObjectKeyAndRangeStart {
            object_key,
            start: range.start(),
        });
        let entries = self.ranges.entry(object_key).or_default();
        let entry = CachedRange {
            id,
            range,
            bytes,
            capacity,
            candidate_slot,
            admitted_at_access: access_clock,
        };

        entries.object_generation += 1;
        let replaced = entries.by_start.insert(range.start(), entry);
        debug_assert!(replaced.is_none());
        id
    }

    pub(super) fn contains_entry(&self, key: &ObjectKeyHash, range: ByteRange, id: u64) -> bool {
        self.ranges
            .get(key)
            .and_then(|entries| entries.by_start.get(&range.start()))
            .is_some_and(|entry| entry.id == id && entry.range == range)
    }

    /// Inserts a trimmed range into the index and candidate ring; the caller accounts for its bytes.
    fn insert_trimmed_range(&mut self, object_key: &ObjectKeyHash, range: ByteRange, bytes: Bytes, access_clock: u64) {
        let capacity = bytes.len() as u64;
        self.buffer_pool.add_cached(capacity);
        let id = self.allocate_entry_id();
        let candidate_slot = self.candidates.register(ObjectKeyAndRangeStart {
            object_key: *object_key,
            start: range.start(),
        });
        let entries = self.ranges.entry(*object_key).or_default();
        let entry = CachedRange {
            id,
            range,
            bytes,
            capacity,
            candidate_slot,
            admitted_at_access: access_clock,
        };
        entries.object_generation += 1;
        let replaced = entries.by_start.insert(range.start(), entry);
        debug_assert!(replaced.is_none());
    }

    fn allocate_entry_id(&mut self) -> u64 {
        self.next_entry_id = self
            .next_entry_id
            .checked_add(1)
            .expect("a shard exhausted its in-process entry identities");
        self.next_entry_id
    }

    /// Removes cached ranges already found fully contained in the incoming download.
    fn remove_contained_ranges(&mut self, object_key: &ObjectKeyHash, ranges: &[ByteRange]) -> RemovedCacheUsage {
        let mut removal = RemovedCacheUsage::default();
        for &range in ranges {
            let removed_bytes = self
                .remove_entry(object_key, range, None)
                .expect("the contained cached range was just found");
            removal.allocation_bytes += removed_bytes;
            removal.entry_count += 1;
        }
        removal
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKeyHash, range: ByteRange) -> bool {
        let Some(removed_bytes) = self.remove_entry(object_key, range, None) else {
            return false;
        };
        self.metrics.decrease_usage(removed_bytes, 1);
        self.metrics.record_remove();
        true
    }

    /// Removes one entry from the shard and returns its allocation charge.
    /// Subtracts those bytes from shard usage; the caller updates metrics.
    fn remove_entry(
        &mut self,
        object_key: &ObjectKeyHash,
        range: ByteRange,
        expected_entry_id: Option<u64>,
    ) -> Option<u64> {
        let (removed_range, object_has_no_cached_ranges) = {
            let object_cached_ranges = self.ranges.get_mut(object_key)?;
            let cached_range = object_cached_ranges.by_start.get(&range.start())?;
            if cached_range.range != range || expected_entry_id.is_some_and(|id| id != cached_range.id) {
                return None;
            }
            let removed_range = object_cached_ranges
                .by_start
                .remove(&range.start())
                .expect("the exact entry was checked immediately before removal");
            object_cached_ranges.object_generation += 1;
            let object_has_no_cached_ranges = object_cached_ranges.by_start.is_empty();
            (removed_range, object_has_no_cached_ranges)
        };

        self.remove_eviction_candidate(removed_range.candidate_slot);
        if object_has_no_cached_ranges {
            self.ranges.remove(object_key);
        }
        let removed_bytes = removed_range.capacity;
        self.used_bytes -= removed_bytes;
        self.buffer_pool.remove_cached(removed_bytes);
        Some(removed_bytes)
    }

    fn remove_eviction_candidate(&mut self, slot: usize) {
        let moved_candidate = self.candidates.remove(slot);
        let Some(moved_candidate) = moved_candidate else {
            return;
        };
        let entry = self
            .ranges
            .get_mut(&moved_candidate.object_key)
            .and_then(|entries| entries.by_start.get_mut(&moved_candidate.start))
            .expect("a moved policy candidate must still refer to a live entry");
        entry.candidate_slot = slot;
    }

    /// Selects the lowest retrieval-cost density from a rotating, bounded sample.
    fn select_reclaim_candidate(
        &mut self,
        admitting_key: &ObjectKeyHash,
        admitting_range: ByteRange,
        access_histories: &ObjectAccessHistories,
    ) -> Option<ReclaimCandidate> {
        let (sample_start, sample_count) = self.candidates.sample(self.reclaim_sample_size);
        let candidate_count = self.candidates.entries.len();
        let mut selected_candidate: Option<(&ObjectKeyHash, &CachedRange, f64)> = None;

        for offset in 0..sample_count {
            let candidate = &self.candidates.entries[(sample_start + offset) % candidate_count];
            let entries = self
                .ranges
                .get(&candidate.object_key)
                .expect("every policy candidate must have an object index");
            let entry = entries
                .by_start
                .get(&candidate.start)
                .expect("every policy candidate must identify a live entry");
            if candidate.object_key == *admitting_key && admitting_range.contains(entry.range) {
                continue;
            }

            let retrieval_cost = access_histories.retention_score(&candidate.object_key, entry.range);
            if selected_candidate.is_none_or(|(selected_key, selected_entry, selected_cost)| {
                compare_cost_per_byte(retrieval_cost, entry.capacity, selected_cost, selected_entry.capacity)
                    .then_with(|| entry.id.cmp(&selected_entry.id))
                    .then_with(|| candidate.object_key.cmp(selected_key))
                    .then_with(|| entry.range.cmp(&selected_entry.range))
                    .is_lt()
            }) {
                selected_candidate = Some((&candidate.object_key, entry, retrieval_cost));
            }
        }
        selected_candidate.map(|(object_key, entry, _)| ReclaimCandidate {
            object_key: *object_key,
            range: entry.range,
            id: entry.id,
        })
    }

    /// Prepares a range trim by retaining its source bytes, plan, and identity after the grace period.
    fn prepare_range_trim(
        &self,
        candidate: &ReclaimCandidate,
        access_histories: &ObjectAccessHistories,
    ) -> Option<RangeTrimSource> {
        let entries = self
            .ranges
            .get(&candidate.object_key)
            .expect("a selected pressure candidate must have an object index");
        let entry = entries
            .by_start
            .get(&candidate.range.start())
            .filter(|entry| entry.id == candidate.id)
            .expect("a selected pressure candidate must identify a live entry");
        let access_clock = access_histories.clock();
        if access_clock.saturating_sub(entry.admitted_at_access) < MIN_ACCESSES_BEFORE_PAYLOAD_COMPACTION {
            return None;
        }
        let plan = plan_range_trim(entry.range, access_histories.active_ranges(&candidate.object_key))?;
        Some(RangeTrimSource {
            object_key: candidate.object_key,
            start: candidate.range.start(),
            id: candidate.id,
            object_generation: entries.object_generation,
            plan,
            source_bytes: entry.bytes.clone(),
        })
    }

    /// Publishes copied range trimming output only if source metadata is unchanged.
    pub(super) fn publish_range_trim(&mut self, replacement: RangeTrimReplacement, access_clock: u64) -> bool {
        // Only cached-range changes invalidate the copy. New requests may change the desirability
        // of the trimming plan, but not the correctness of the retained bytes.
        let source_unchanged = self.ranges.get(&replacement.object_key).is_some_and(|entries| {
            entries.object_generation == replacement.object_generation
                // The source must still be the same insertion, not a replacement at the same range.
                && entries
                    .by_start
                    .get(&replacement.start)
                    .is_some_and(|entry| entry.id == replacement.id && entry.range == replacement.plan.source_range())
        });
        if !source_unchanged {
            return false;
        }

        let removed_bytes = self
            .remove_entry(
                &replacement.object_key,
                replacement.plan.source_range(),
                Some(replacement.id),
            )
            .expect("the compaction source was revalidated immediately before removal");
        let mut retained_bytes = 0_u64;
        let mut retained_entries = 0_u64;

        for (retained_range, bytes) in replacement.retained_payloads {
            if self
                .ranges
                .get(&replacement.object_key)
                .and_then(|entries| entries.covering_range(retained_range))
                .is_some()
            {
                continue;
            }
            retained_bytes += bytes.len() as u64;
            retained_entries += 1;
            self.used_bytes += bytes.len() as u64;
            self.insert_trimmed_range(&replacement.object_key, retained_range, bytes, access_clock);
        }

        let reclaimed_bytes = removed_bytes - retained_bytes;
        debug_assert!(reclaimed_bytes >= replacement.plan.reclaimed_bytes());
        self.metrics.decrease_usage(removed_bytes, 1);
        if retained_entries != 0 {
            self.metrics.increase_usage(retained_bytes, retained_entries);
        }
        self.metrics.record_range_trim(reclaimed_bytes);
        true
    }

    pub(super) fn entry_count(&self) -> usize {
        self.candidates.entries.len()
    }

    #[cfg(test)]
    pub(super) fn candidate_count(&self) -> usize {
        self.candidates.entries.len()
    }
}

impl Drop for MemoryCacheShard {
    fn drop(&mut self) {
        let entry_count = self.entry_count() as u64;
        if entry_count != 0 {
            self.metrics.decrease_usage(self.used_bytes, entry_count);
            self.buffer_pool.remove_cached(self.used_bytes);
        }
    }
}
