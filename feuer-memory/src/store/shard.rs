use std::{cmp::Ordering, collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use feuer_types::{ByteRange, ObjectKey};
use rustc_hash::FxHashMap;

use super::{
    access_history::RangeAccessHistory,
    range_trim::{RangeTrimPlan, plan_range_trim},
};
use crate::MemoryMetrics;

/// Successful shard-local accesses allowed before a cached range can be trimmed.
pub(super) const RANGE_TRIM_GRACE_ACCESSES: u64 = 64;
/// Maximum entries inspected for one pressure decision.
const RECLAIM_SAMPLE_SIZE: usize = 64;

/// One retained downloaded range.
struct CachedRange {
    /// Monotonic identity used for revalidation and policy tie-breaking.
    id: u64,
    /// Exact object interval represented by `bytes`.
    range: ByteRange,
    /// Retained payload; lookup results share slices of this allocation.
    bytes: Bytes,
    /// Slot in the bounded-work policy candidate ring.
    candidate_slot: usize,
    /// Successful-access clock at admission.
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
    /// Exact access history shared by this object's cached ranges.
    accesses: RangeAccessHistory,
    /// Structural and access generation used by copy-outside-lock range trimming.
    object_generation: u64,
}

impl ObjectCachedRanges {
    fn covering(&self, range: ByteRange) -> Option<&CachedRange> {
        let (_, entry) = self.by_start.range(..=range.start()).next_back()?;
        entry.range.contains(range).then_some(entry)
    }

    fn record_covering_access<R>(
        &mut self,
        requested: ByteRange,
        access_clock: u64,
        project: impl FnOnce(&CachedRange) -> R,
    ) -> Option<R> {
        let projected = {
            let (_, entry) = self.by_start.range(..=requested.start()).next_back()?;
            if !entry.range.contains(requested) {
                return None;
            }
            project(entry)
        };
        self.object_generation = self.object_generation.saturating_add(1);
        self.accesses.record(requested, access_clock);
        Some(projected)
    }

    fn superseded_by(&self, range: ByteRange) -> SupersededRanges {
        let mut superseded = SupersededRanges::default();
        for (_, entry) in self.by_start.range(range.start()..range.end()) {
            if range.contains(entry.range) {
                superseded.ranges.push(entry.range);
                superseded.payload_bytes += entry.bytes.len() as u64;
            }
        }
        superseded
    }
}

/// Existing entries fully covered by one larger download.
#[derive(Default)]
struct SupersededRanges {
    ranges: Vec<ByteRange>,
    payload_bytes: u64,
}

/// Usage removed while admitting one download.
#[derive(Default)]
struct RemovedCacheUsage {
    payload_bytes: u64,
    entry_count: u64,
}

/// An object's cached range identity: object key, start offset, and entry ID.
#[derive(Clone)]
struct CachedRangeIdentity {
    object_key: ObjectKey,
    start: u64,
    id: u64,
}

/// Rotating ring of cached-range candidates for reclaiming memory by trimming or eviction.
#[derive(Default)]
struct ReclaimCandidateRing {
    entries: Vec<CachedRangeIdentity>,
    cursor: usize,
}

impl ReclaimCandidateRing {
    fn register(&mut self, candidate: CachedRangeIdentity) -> usize {
        let slot = self.entries.len();
        self.entries.push(candidate);
        slot
    }

    /// Removes `slot` and returns the candidate moved into it, if any.
    fn remove(&mut self, slot: usize, expected_id: u64) -> Option<CachedRangeIdentity> {
        debug_assert_eq!(self.entries.get(slot).map(|candidate| candidate.id), Some(expected_id));
        let last = self.entries.len() - 1;
        self.entries.swap_remove(slot);
        let moved = (slot != last).then(|| self.entries[slot].clone());
        if self.entries.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor %= self.entries.len();
        }
        moved
    }

    fn sample(&mut self) -> (usize, usize) {
        let count = self.entries.len().min(RECLAIM_SAMPLE_SIZE);
        if count == 0 {
            return (0, 0);
        }
        let start = self.cursor % self.entries.len();
        self.cursor = (start + count) % self.entries.len();
        (start, count)
    }
}

/// A cached range considered for reclaiming memory, with its retrieval cost and retained size.
struct ReclaimCandidate {
    object_key: ObjectKey,
    range: ByteRange,
    id: u64,
    retained_bytes: u64,
    retrieval_cost: u64,
}

/// Source payload, plan, and identity for trimming a cached range outside the shard lock.
pub(super) struct RangeTrimSource {
    object_key: ObjectKey,
    start: u64,
    id: u64,
    object_generation: u64,
    plan: RangeTrimPlan,
    source_bytes: Bytes,
}

impl RangeTrimSource {
    /// Copies retained payload while no shard metadata lock is held.
    pub(super) fn copy_payload(self) -> RangeTrimReplacement {
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
    object_key: ObjectKey,
    start: u64,
    id: u64,
    object_generation: u64,
    plan: RangeTrimPlan,
    retained_payloads: Vec<(ByteRange, Bytes)>,
}

/// Progress after a bounded admission attempt: complete, retry, or trim before retrying.
pub(super) enum AdmissionProgress {
    Complete,
    Retry,
    Trim(RangeTrimSource),
}

/// One memory-cache shard's range indexes, access history, and payload accounting.
pub(super) struct MemoryCacheShard {
    capacity: u64,
    used_bytes: u64,
    ranges: FxHashMap<ObjectKey, ObjectCachedRanges>,
    access_clock: u64,
    next_entry_id: u64,
    candidates: ReclaimCandidateRing,
    metrics: Arc<MemoryMetrics>,
}

impl MemoryCacheShard {
    pub(super) fn new(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self {
        Self {
            capacity,
            used_bytes: 0,
            ranges: FxHashMap::default(),
            access_clock: 0,
            next_entry_id: 0,
            candidates: ReclaimCandidateRing::default(),
            metrics,
        }
    }

    pub(super) const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    pub(super) fn get(&mut self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes> {
        let access_clock = self.access_clock.saturating_add(1);
        let accessed = self.ranges.get_mut(object_key).and_then(|entries| {
            entries.record_covering_access(requested_range, access_clock, |entry| {
                entry.requested_bytes(requested_range)
            })
        });
        let Some(bytes) = accessed else {
            self.metrics.record_lookup(false);
            return None;
        };

        self.access_clock = access_clock;
        self.metrics.record_access();
        self.metrics.record_lookup(true);
        Some(bytes)
    }

    pub(super) fn record_access(&mut self, object_key: &ObjectKey, requested_range: ByteRange) {
        self.record_successful_access(object_key, requested_range);
    }

    fn record_successful_access(&mut self, object_key: &ObjectKey, requested_range: ByteRange) {
        self.access_clock = self.access_clock.saturating_add(1);
        self.ranges
            .get_mut(object_key)
            .and_then(|entries| entries.record_covering_access(requested_range, self.access_clock, |_| ()));
        self.metrics.record_access();
    }

    /// Advances one bounded admission action while holding the shard lock.
    pub(super) fn advance_admission(
        &mut self,
        object_key: &ObjectKey,
        range: ByteRange,
        bytes: &Bytes,
        requested_range: Option<ByteRange>,
        allow_range_trim: bool,
    ) -> AdmissionProgress {
        let superseded = match self.ranges.get(object_key) {
            Some(entries) if entries.covering(range).is_some() => {
                self.metrics.record_redundant();
                if let Some(requested_range) = requested_range {
                    self.record_successful_access(object_key, requested_range);
                }
                return AdmissionProgress::Complete;
            }
            Some(entries) => entries.superseded_by(range),
            None => SupersededRanges::default(),
        };
        let added_bytes = bytes.len() as u64;
        let used_bytes_without_superseded = self.used_bytes - superseded.payload_bytes;
        let max_existing_bytes = self.capacity.saturating_sub(added_bytes);

        if used_bytes_without_superseded <= max_existing_bytes {
            let removal = self.remove_superseded(object_key, &superseded.ranges);
            debug_assert_eq!(removal.payload_bytes, superseded.payload_bytes);
            self.insert_admission(object_key.clone(), range, bytes.clone());

            if removal.entry_count != 0 {
                self.metrics.decrease_usage(removal.payload_bytes, removal.entry_count);
            }
            self.metrics.increase_usage(added_bytes, 1);
            self.metrics.record_insert(removal.entry_count != 0);
            if let Some(requested_range) = requested_range {
                self.record_successful_access(object_key, requested_range);
            }
            return AdmissionProgress::Complete;
        }

        let Some(candidate) = self.select_reclaim_candidate(object_key, range) else {
            // A rotating bounded sample can temporarily consist entirely of
            // entries superseded by this admission. Advancing its cursor and
            // retrying gives amortized full coverage without one long scan.
            return AdmissionProgress::Retry;
        };
        if allow_range_trim && let Some(source) = self.range_trim_source(&candidate) {
            return AdmissionProgress::Trim(source);
        }

        let removed = self
            .detach_entry(
                &candidate.object_key,
                candidate.range,
                Some(candidate.id),
                candidate.object_key == *object_key,
            )
            .expect("a sampled pressure candidate cannot disappear while its shard is locked");
        self.metrics.decrease_usage(removed, 1);
        self.metrics.record_evictions(1);
        AdmissionProgress::Retry
    }

    fn insert_admission(&mut self, object_key: ObjectKey, range: ByteRange, bytes: Bytes) {
        self.used_bytes += bytes.len() as u64;
        let id = self.allocate_entry_id();
        let candidate_slot = self.candidates.register(CachedRangeIdentity {
            object_key: object_key.clone(),
            start: range.start(),
            id,
        });
        let entry = CachedRange {
            id,
            range,
            bytes,
            candidate_slot,
            admitted_at_access: self.access_clock,
        };

        let entries = self.ranges.entry(object_key).or_default();
        entries.object_generation = entries.object_generation.saturating_add(1);
        let replaced = entries.by_start.insert(range.start(), entry);
        debug_assert!(replaced.is_none());
    }

    fn insert_trimmed(&mut self, object_key: &ObjectKey, range: ByteRange, bytes: Bytes) {
        let id = self.allocate_entry_id();
        let candidate_slot = self.candidates.register(CachedRangeIdentity {
            object_key: object_key.clone(),
            start: range.start(),
            id,
        });
        let entry = CachedRange {
            id,
            range,
            bytes,
            candidate_slot,
            admitted_at_access: self.access_clock,
        };
        let entries = self.ranges.entry(object_key.clone()).or_default();
        entries.object_generation = entries.object_generation.saturating_add(1);
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

    fn remove_superseded(&mut self, object_key: &ObjectKey, ranges: &[ByteRange]) -> RemovedCacheUsage {
        let mut removal = RemovedCacheUsage::default();
        for &range in ranges {
            let bytes = self
                .detach_entry(object_key, range, None, true)
                .expect("the superseded entry was just found");
            removal.payload_bytes += bytes;
            removal.entry_count += 1;
        }
        removal
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKey, range: ByteRange) -> bool {
        let Some(bytes) = self.detach_entry(object_key, range, None, false) else {
            return false;
        };
        self.metrics.decrease_usage(bytes, 1);
        self.metrics.record_remove();
        true
    }

    fn detach_entry(
        &mut self,
        object_key: &ObjectKey,
        range: ByteRange,
        expected_id: Option<u64>,
        preserve_access_history: bool,
    ) -> Option<u64> {
        let (entry, object_is_empty) = {
            let entries = self.ranges.get_mut(object_key)?;
            let current = entries.by_start.get(&range.start())?;
            if current.range != range || expected_id.is_some_and(|id| id != current.id) {
                return None;
            }
            let entry = entries
                .by_start
                .remove(&range.start())
                .expect("the exact entry was checked immediately before removal");
            entries.object_generation = entries.object_generation.saturating_add(1);
            let object_is_empty = entries.by_start.is_empty();
            (entry, object_is_empty)
        };

        self.unregister_candidate(entry.candidate_slot, entry.id);
        if object_is_empty && !preserve_access_history {
            self.ranges.remove(object_key);
        }
        let bytes = entry.bytes.len() as u64;
        self.used_bytes -= bytes;
        Some(bytes)
    }

    fn unregister_candidate(&mut self, slot: usize, expected_id: u64) {
        let moved = self.candidates.remove(slot, expected_id);
        let Some(moved) = moved else {
            return;
        };
        let entry = self
            .ranges
            .get_mut(&moved.object_key)
            .and_then(|entries| entries.by_start.get_mut(&moved.start))
            .filter(|entry| entry.id == moved.id)
            .expect("a moved policy candidate must still refer to a live entry");
        entry.candidate_slot = slot;
    }

    /// Selects the lowest recent retrieval value per retained byte in a bounded sample.
    fn select_reclaim_candidate(
        &mut self,
        admitting_key: &ObjectKey,
        admitting_range: ByteRange,
    ) -> Option<ReclaimCandidate> {
        let (sample_start, sample_count) = self.candidates.sample();
        let candidate_count = self.candidates.entries.len();
        let mut selected: Option<ReclaimCandidate> = None;

        for offset in 0..sample_count {
            let candidate = &self.candidates.entries[(sample_start + offset) % candidate_count];
            let entries = self
                .ranges
                .get(&candidate.object_key)
                .expect("every policy candidate must have an object index");
            let entry = entries
                .by_start
                .get(&candidate.start)
                .filter(|entry| entry.id == candidate.id)
                .expect("every policy candidate must identify a live entry");
            if candidate.object_key == *admitting_key && admitting_range.contains(entry.range) {
                continue;
            }

            let sampled = ReclaimCandidate {
                object_key: candidate.object_key.clone(),
                range: entry.range,
                id: entry.id,
                retained_bytes: entry.bytes.len() as u64,
                retrieval_cost: entries.accesses.covered_retrieval_cost(entry.range, self.access_clock),
            };
            if selected
                .as_ref()
                .is_none_or(|current| compare_retrieval_cost_per_byte(&sampled, current).is_lt())
            {
                selected = Some(sampled);
            }
        }
        selected
    }

    /// Plans range trimming only for the selected candidate, after its grace expires.
    fn range_trim_source(&self, candidate: &ReclaimCandidate) -> Option<RangeTrimSource> {
        let entries = self
            .ranges
            .get(&candidate.object_key)
            .expect("a selected pressure candidate must have an object index");
        let entry = entries
            .by_start
            .get(&candidate.range.start())
            .filter(|entry| entry.id == candidate.id)
            .expect("a selected pressure candidate must identify a live entry");
        if self.access_clock.saturating_sub(entry.admitted_at_access) < RANGE_TRIM_GRACE_ACCESSES {
            return None;
        }
        let plan = plan_range_trim(entry.range, entries.accesses.active_ranges(self.access_clock))?;
        Some(RangeTrimSource {
            object_key: candidate.object_key.clone(),
            start: candidate.range.start(),
            id: candidate.id,
            object_generation: entries.object_generation,
            plan,
            source_bytes: entry.bytes.clone(),
        })
    }

    /// Publishes copied range trimming output only if source metadata is unchanged.
    pub(super) fn publish_range_trim(&mut self, replacement: RangeTrimReplacement) -> bool {
        let valid = self.ranges.get(&replacement.object_key).is_some_and(|entries| {
            entries.object_generation == replacement.object_generation
                && entries
                    .by_start
                    .get(&replacement.start)
                    .is_some_and(|entry| entry.id == replacement.id && entry.range == replacement.plan.source_range())
        });
        if !valid {
            return false;
        }

        let removed_bytes = self
            .detach_entry(
                &replacement.object_key,
                replacement.plan.source_range(),
                Some(replacement.id),
                true,
            )
            .expect("the compaction source was revalidated immediately before removal");
        let mut retained_bytes = 0_u64;
        let mut retained_entries = 0_u64;

        for (retained_range, bytes) in replacement.retained_payloads {
            if self
                .ranges
                .get(&replacement.object_key)
                .and_then(|entries| entries.covering(retained_range))
                .is_some()
            {
                continue;
            }
            retained_bytes += bytes.len() as u64;
            retained_entries += 1;
            self.used_bytes += bytes.len() as u64;
            self.insert_trimmed(&replacement.object_key, retained_range, bytes);
        }

        let reclaimed = removed_bytes - retained_bytes;
        debug_assert!(reclaimed >= replacement.plan.reclaimed_bytes());
        self.metrics.decrease_usage(removed_bytes, 1);
        if retained_entries != 0 {
            self.metrics.increase_usage(retained_bytes, retained_entries);
        }
        self.metrics.record_range_trim(reclaimed);
        true
    }

    pub(super) fn entry_count(&self) -> usize {
        self.candidates.entries.len()
    }

    #[cfg(test)]
    pub(super) fn accessed_ranges(&self, object_key: &ObjectKey) -> Vec<ByteRange> {
        self.ranges
            .get(object_key)
            .map_or_else(Vec::new, |entries| entries.accesses.ranges())
    }

    #[cfg(test)]
    pub(super) fn access_history_len(&self, object_key: &ObjectKey) -> usize {
        self.ranges.get(object_key).map_or(0, |entries| entries.accesses.len())
    }

    #[cfg(test)]
    pub(super) fn candidate_count(&self) -> usize {
        self.candidates.entries.len()
    }
}

/// Lower retrieval-value density, then the older entry wins victim selection.
fn compare_retrieval_cost_per_byte(left: &ReclaimCandidate, right: &ReclaimCandidate) -> Ordering {
    compare_cost_per_byte(
        left.retrieval_cost,
        left.retained_bytes,
        right.retrieval_cost,
        right.retained_bytes,
    )
    .then_with(|| left.id.cmp(&right.id))
    .then_with(|| left.object_key.cmp(&right.object_key))
    .then_with(|| left.range.cmp(&right.range))
}

fn compare_cost_per_byte(left_cost: u64, left_bytes: u64, right_cost: u64, right_bytes: u64) -> Ordering {
    (u128::from(left_cost) * u128::from(right_bytes)).cmp(&(u128::from(right_cost) * u128::from(left_bytes)))
}

impl Drop for MemoryCacheShard {
    fn drop(&mut self) {
        let entries = self.entry_count() as u64;
        if entries != 0 {
            self.metrics.decrease_usage(self.used_bytes, entries);
        }
    }
}
