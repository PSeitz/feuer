use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use bytes::Bytes;
use feuer_types::{ByteRange, ObjectKey};
use rustc_hash::FxHashMap;

use super::{
    compaction::{CompactionPlan, plan_compaction},
    evidence::{ACCESSES_PER_EPOCH, AccessEvidence, MAX_EVIDENCE_AGE_EPOCHS, RetentionEvidence},
};
use crate::MemoryMetrics;

/// Successful shard-local accesses granted to a broader admission before compaction.
pub(super) const PREFETCH_GRACE_ACCESSES: u64 = 64;
/// Exact intervals remembered per extent solely for detecting novel prefetch use.
pub(super) const MAX_OBSERVED_INTERVALS_PER_ENTRY: usize = 8;
/// Demonstrated-prefetch events retained per extent.
pub(super) const MAX_REUSE_EVENTS_PER_ENTRY: usize = 4;
/// Entries retained in the payload-free re-admission history of one shard.
pub(super) const MAX_GHOST_ENTRIES_PER_SHARD: usize = 256;
/// Object-key bytes retained by the payload-free re-admission history of one shard.
pub(super) const MAX_GHOST_KEY_BYTES_PER_SHARD: usize = 256 * 1024;
/// Shard-local successful accesses for which an evicted downloaded range is remembered.
pub(super) const GHOST_LIFETIME_ACCESSES: u64 = 4_096;

/// Maximum entries inspected for one victim or compaction decision.
const POLICY_SAMPLE_SIZE: usize = 64;
/// Relative retention credit of one demonstrated-prefetch event.
const PREFETCH_REUSE_WEIGHT: u64 = 1;

/// The selected value policy plus a benchmark-only disabled control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompactionMode {
    /// Choose the available action with lower estimated value loss per byte.
    ValueAware,
    /// Retain and evict complete downloaded extents only.
    #[cfg(feature = "benchmark")]
    Disabled,
}

#[cfg(feature = "benchmark")]
impl CompactionMode {
    const fn is_disabled(self) -> bool {
        matches!(self, Self::Disabled)
    }
}

/// One retained downloaded range.
struct Entry {
    /// Unique identity used to revalidate lazy policy references.
    id: u64,
    /// Exact object interval represented by `bytes`.
    range: ByteRange,
    /// Retained payload; lookup results share slices of this allocation.
    bytes: Bytes,
    /// Stable age tie-breaker assigned at original admission.
    sequence: u64,
    /// Slot in the bounded-work policy candidate ring.
    candidate_slot: usize,
    /// Successful-access clock at original admission.
    admitted_at: u64,
    /// Whether an initial request has established this entry's observed coverage.
    has_observed_access: bool,
    /// Whether that initial request was smaller than the downloaded extent.
    broader_than_request: bool,
    /// Bounded merged intervals already observed on this particular extent.
    observed: ObservedCoverage,
    /// Bounded, ageable demonstrated-prefetch evidence.
    reuse: ReuseEvidence,
    /// A matching ghost should promote this entry once its first request is known.
    pending_ghost_promotion: bool,
    /// Re-admission recognized by ghost evidence does not repeat one-hit grace.
    probation_bypassed: bool,
}

impl Entry {
    fn admission(
        id: u64,
        range: ByteRange,
        bytes: Bytes,
        sequence: u64,
        candidate_slot: usize,
        admitted_at: u64,
        ghost_hit: bool,
    ) -> Self {
        Self {
            id,
            range,
            bytes,
            sequence,
            candidate_slot,
            admitted_at,
            has_observed_access: false,
            broader_than_request: false,
            observed: ObservedCoverage::default(),
            reuse: ReuseEvidence::default(),
            pending_ghost_promotion: ghost_hit,
            probation_bypassed: false,
        }
    }

    fn compacted(
        id: u64,
        range: ByteRange,
        bytes: Bytes,
        sequence: u64,
        candidate_slot: usize,
        admitted_at: u64,
    ) -> Self {
        let mut observed = ObservedCoverage::default();
        observed.observe(range, admitted_at);
        Self {
            id,
            range,
            bytes,
            sequence,
            candidate_slot,
            admitted_at,
            has_observed_access: true,
            broader_than_request: false,
            observed,
            reuse: ReuseEvidence::default(),
            pending_ghost_promotion: false,
            probation_bypassed: false,
        }
    }

    fn requested_bytes(&self, requested_range: ByteRange) -> Bytes {
        debug_assert!(self.range.contains(requested_range));
        let start = usize::try_from(requested_range.start() - self.range.start())
            .expect("an offset within a Bytes payload must fit in usize");
        let end = usize::try_from(requested_range.end() - self.range.start())
            .expect("an offset within a Bytes payload must fit in usize");
        self.bytes.slice(start..end)
    }

    fn observe_access(&mut self, requested_range: ByteRange, access_clock: u64, epoch: u64) -> PrefetchObservation {
        debug_assert!(self.range.contains(requested_range));
        if !self.has_observed_access {
            self.has_observed_access = true;
            self.broader_than_request = self.range != requested_range;
            self.observed.observe(requested_range, access_clock);

            let ghost_promoted = self.pending_ghost_promotion && self.broader_than_request;
            if ghost_promoted {
                self.reuse.record(epoch);
                self.probation_bypassed = true;
            }
            self.pending_ghost_promotion = false;
            return PrefetchObservation {
                demonstrated: false,
                ghost_promoted,
            };
        }

        let demonstrated = self.broader_than_request && !self.observed.contains(requested_range);
        if demonstrated {
            self.reuse.record(epoch);
        }
        self.observed.observe(requested_range, access_clock);
        PrefetchObservation {
            demonstrated,
            ghost_promoted: false,
        }
    }

    fn is_in_probation(&self, access_clock: u64) -> bool {
        self.broader_than_request
            && !self.probation_bypassed
            && access_clock.saturating_sub(self.admitted_at) < PREFETCH_GRACE_ACCESSES
    }

    fn compaction_eligible(&self, access_clock: u64) -> bool {
        self.broader_than_request
            && self.has_observed_access
            && (self.probation_bypassed || access_clock.saturating_sub(self.admitted_at) >= PREFETCH_GRACE_ACCESSES)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PrefetchObservation {
    demonstrated: bool,
    ghost_promoted: bool,
}

#[derive(Clone, Copy)]
struct ObservedInterval {
    range: ByteRange,
    last_access: u64,
}

/// Small per-entry union used only to identify newly useful prefetched bytes.
#[derive(Default)]
struct ObservedCoverage {
    intervals: Vec<ObservedInterval>,
}

impl ObservedCoverage {
    fn contains(&self, requested: ByteRange) -> bool {
        self.intervals.iter().any(|interval| interval.range.contains(requested))
    }

    fn observe(&mut self, requested: ByteRange, access_clock: u64) {
        let mut start = requested.start();
        let mut end = requested.end();
        let mut newest = access_clock;
        let mut index = 0;
        while index < self.intervals.len() {
            let interval = self.intervals[index];
            if interval.range.end() < start || interval.range.start() > end {
                index += 1;
                continue;
            }
            start = start.min(interval.range.start());
            end = end.max(interval.range.end());
            newest = newest.max(interval.last_access);
            self.intervals.remove(index);
        }

        let range = ByteRange::new(start, end).expect("a union containing a non-empty request must be non-empty");
        let position = self
            .intervals
            .partition_point(|interval| interval.range.start() < range.start());
        self.intervals.insert(
            position,
            ObservedInterval {
                range,
                last_access: newest,
            },
        );

        if self.intervals.len() > MAX_OBSERVED_INTERVALS_PER_ENTRY {
            let oldest = self
                .intervals
                .iter()
                .enumerate()
                .min_by_key(|(_, interval)| (interval.last_access, interval.range))
                .map(|(index, _)| index)
                .expect("an overfull observed-coverage set cannot be empty");
            self.intervals.remove(oldest);
        }
    }
}

/// Bounded and epoch-aged promotion evidence for one broader extent.
#[derive(Default)]
struct ReuseEvidence {
    epochs: VecDeque<u64>,
}

impl ReuseEvidence {
    fn record(&mut self, epoch: u64) {
        if self.epochs.len() == MAX_REUSE_EVENTS_PER_ENTRY {
            self.epochs.pop_front();
        }
        self.epochs.push_back(epoch);
    }

    fn retention(&self, epoch: u64) -> RetentionEvidence {
        let mut retention = RetentionEvidence::default();
        for &observed_epoch in &self.epochs {
            let age = epoch.saturating_sub(observed_epoch);
            if age > MAX_EVIDENCE_AGE_EPOCHS {
                continue;
            }
            let shift =
                u32::try_from(MAX_EVIDENCE_AGE_EPOCHS - age).expect("an active demonstrated-reuse age must fit in u32");
            retention.weighted_frequency = retention.weighted_frequency.saturating_add(1_u64 << shift);
            retention.newest_epoch = Some(
                retention
                    .newest_epoch
                    .map_or(observed_epoch, |seen| seen.max(observed_epoch)),
            );
        }
        retention
    }

    #[cfg(test)]
    fn active_len(&self, epoch: u64) -> usize {
        self.epochs
            .iter()
            .filter(|&&observed| epoch.saturating_sub(observed) <= MAX_EVIDENCE_AGE_EPOCHS)
            .count()
    }
}

/// Ordered cached downloaded ranges for one cache key.
///
/// No cached range fully contains another. Partial overlaps remain indexed.
/// Because starts and ends both increase, the predecessor of a request start
/// is the only possible covering entry.
#[derive(Default)]
struct CachedRanges {
    /// Entries ordered by exact start for predecessor-based covering lookup.
    by_start: BTreeMap<u64, Entry>,
    /// Exact access evidence shared by this object's retained extents.
    accesses: AccessEvidence,
    /// Structural and access generation used by copy-outside-lock compaction.
    generation: u64,
}

impl CachedRanges {
    fn covering(&self, range: ByteRange) -> Option<&Entry> {
        let (_, entry) = self.by_start.range(..=range.start()).next_back()?;
        entry.range.contains(range).then_some(entry)
    }

    fn observe_covering<R>(
        &mut self,
        requested: ByteRange,
        access_clock: u64,
        epoch: u64,
        project: impl FnOnce(&Entry) -> R,
    ) -> Option<(R, PrefetchObservation)> {
        let (projected, observation) = {
            let (_, entry) = self.by_start.range_mut(..=requested.start()).next_back()?;
            if !entry.range.contains(requested) {
                return None;
            }
            let projected = project(entry);
            let observation = entry.observe_access(requested, access_clock, epoch);
            (projected, observation)
        };
        self.generation = self.generation.saturating_add(1);
        self.accesses.record(requested, epoch);
        Some((projected, observation))
    }

    fn superseded_by(&self, range: ByteRange) -> Superseded {
        let mut superseded = Superseded::default();
        for (_, entry) in self.by_start.range(range.start()..range.end()) {
            if range.contains(entry.range) {
                superseded.ranges.push(entry.range);
                superseded.bytes += entry.bytes.len() as u64;
            }
        }
        superseded
    }
}

/// Existing entries fully covered by one larger download.
#[derive(Default)]
struct Superseded {
    ranges: Vec<ByteRange>,
    bytes: u64,
}

/// Usage removed while admitting one download.
#[derive(Default)]
struct Removal {
    bytes: u64,
    entries: u64,
}

#[derive(Clone)]
struct CandidateRef {
    object_key: ObjectKey,
    start: u64,
    id: u64,
}

/// Dense candidate ring with O(1) registration/removal and rotating samples.
#[derive(Default)]
struct PolicyCandidates {
    entries: Vec<CandidateRef>,
    victim_cursor: usize,
    compaction_cursor: usize,
}

impl PolicyCandidates {
    fn register(&mut self, candidate: CandidateRef) -> usize {
        let slot = self.entries.len();
        self.entries.push(candidate);
        slot
    }

    /// Removes `slot` and returns the candidate moved into it, if any.
    fn remove(&mut self, slot: usize, expected_id: u64) -> Option<CandidateRef> {
        debug_assert_eq!(self.entries.get(slot).map(|candidate| candidate.id), Some(expected_id));
        let last = self.entries.len() - 1;
        self.entries.swap_remove(slot);
        let moved = (slot != last).then(|| self.entries[slot].clone());
        if self.entries.is_empty() {
            self.victim_cursor = 0;
            self.compaction_cursor = 0;
        } else {
            self.victim_cursor %= self.entries.len();
            self.compaction_cursor %= self.entries.len();
        }
        moved
    }

    fn victim_sample(&mut self) -> (usize, usize) {
        let count = self.entries.len().min(POLICY_SAMPLE_SIZE);
        if count == 0 {
            return (0, 0);
        }
        let start = self.victim_cursor % self.entries.len();
        self.victim_cursor = (start + count) % self.entries.len();
        (start, count)
    }

    fn compaction_sample(&mut self) -> (usize, usize) {
        let count = self.entries.len().min(POLICY_SAMPLE_SIZE);
        if count == 0 {
            return (0, 0);
        }
        let start = self.compaction_cursor % self.entries.len();
        self.compaction_cursor = (start + count) % self.entries.len();
        (start, count)
    }
}

#[derive(Clone)]
struct GhostEntry {
    object_key: ObjectKey,
    range: ByteRange,
    expires_at: u64,
}

/// Bounded payload-free evidence that an exact downloaded range recurred.
#[derive(Default)]
struct GhostStore {
    entries: VecDeque<GhostEntry>,
    key_bytes: usize,
}

impl GhostStore {
    fn record(&mut self, object_key: &ObjectKey, range: ByteRange, access_clock: u64) {
        if object_key.len() > MAX_GHOST_KEY_BYTES_PER_SHARD {
            return;
        }
        self.expire(access_clock);
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.object_key == *object_key && entry.range == range)
        {
            let old = self
                .entries
                .remove(index)
                .expect("the matching ghost index was just found");
            self.key_bytes -= old.object_key.len();
        }

        while self.entries.len() >= MAX_GHOST_ENTRIES_PER_SHARD
            || self.key_bytes.saturating_add(object_key.len()) > MAX_GHOST_KEY_BYTES_PER_SHARD
        {
            let Some(oldest) = self.entries.pop_front() else {
                break;
            };
            self.key_bytes -= oldest.object_key.len();
        }

        if self.key_bytes.saturating_add(object_key.len()) > MAX_GHOST_KEY_BYTES_PER_SHARD {
            return;
        }
        self.key_bytes += object_key.len();
        self.entries.push_back(GhostEntry {
            object_key: object_key.clone(),
            range,
            expires_at: access_clock.saturating_add(GHOST_LIFETIME_ACCESSES),
        });
    }

    fn take(&mut self, object_key: &ObjectKey, range: ByteRange, access_clock: u64) -> bool {
        self.expire(access_clock);
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.object_key == *object_key && entry.range == range)
        else {
            return false;
        };
        let entry = self
            .entries
            .remove(index)
            .expect("the matching ghost index was just found");
        self.key_bytes -= entry.object_key.len();
        true
    }

    fn expire(&mut self, access_clock: u64) {
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.expires_at < access_clock)
        {
            let expired = self.entries.pop_front().expect("the expired front ghost must exist");
            self.key_bytes -= expired.object_key.len();
        }
    }
}

/// One entry considered by the retention comparator.
struct Victim {
    object_key: ObjectKey,
    range: ByteRange,
    id: u64,
    bytes: u64,
    sequence: u64,
    evidence: RetentionEvidence,
    in_probation: bool,
}

/// One extent selected for shard-local compaction.
struct PlannedCompaction {
    object_key: ObjectKey,
    start: u64,
    id: u64,
    sequence: u64,
    generation: u64,
    prefetch_value: RetentionEvidence,
    plan: CompactionPlan,
}

/// A compaction source cloned under the metadata lock for copying afterward.
pub(super) struct CompactionWork {
    object_key: ObjectKey,
    start: u64,
    id: u64,
    sequence: u64,
    generation: u64,
    plan: CompactionPlan,
    source_bytes: Bytes,
}

impl CompactionWork {
    /// Copies retained payload while no shard metadata lock is held.
    pub(super) fn copy_payload(self) -> PreparedCompaction {
        let retained = self
            .plan
            .retained()
            .iter()
            .map(|range| {
                let start = usize::try_from(range.start() - self.plan.source().start())
                    .expect("a planned source offset must fit in usize");
                let end = usize::try_from(range.end() - self.plan.source().start())
                    .expect("a planned source offset must fit in usize");
                (*range, Bytes::copy_from_slice(&self.source_bytes[start..end]))
            })
            .collect();
        PreparedCompaction {
            object_key: self.object_key,
            start: self.start,
            id: self.id,
            sequence: self.sequence,
            generation: self.generation,
            plan: self.plan,
            retained,
        }
    }
}

/// Copied replacement payload waiting for generation-checked publication.
pub(super) struct PreparedCompaction {
    object_key: ObjectKey,
    start: u64,
    id: u64,
    sequence: u64,
    generation: u64,
    plan: CompactionPlan,
    retained: Vec<(ByteRange, Bytes)>,
}

/// One bounded admission action. Callers repeat until `Complete`.
pub(super) enum AdmissionStep {
    Complete,
    Retry,
    Compact(CompactionWork),
}

/// Why an indexed entry is being detached.
#[derive(Clone, Copy)]
enum RemovalCause {
    Explicit,
    Superseded,
    Evicted,
    Compacted,
}

/// Independently locked range indexes, access evidence, and payload accounting.
pub(super) struct Shard {
    capacity: u64,
    used_bytes: u64,
    ranges: FxHashMap<ObjectKey, Box<CachedRanges>>,
    access_clock: u64,
    insertion_sequence: u64,
    next_entry_id: u64,
    candidates: PolicyCandidates,
    ghosts: GhostStore,
    #[cfg(feature = "benchmark")]
    compaction_mode: CompactionMode,
    metrics: Arc<MemoryMetrics>,
}

impl Shard {
    pub(super) fn new(capacity: u64, metrics: Arc<MemoryMetrics>, _compaction_mode: CompactionMode) -> Self {
        Self {
            capacity,
            used_bytes: 0,
            ranges: FxHashMap::default(),
            access_clock: 0,
            insertion_sequence: 0,
            next_entry_id: 0,
            candidates: PolicyCandidates::default(),
            ghosts: GhostStore::default(),
            #[cfg(feature = "benchmark")]
            compaction_mode: _compaction_mode,
            metrics,
        }
    }

    pub(super) const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    pub(super) fn get(&mut self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes> {
        let access_clock = self.access_clock.saturating_add(1);
        let epoch = access_clock / ACCESSES_PER_EPOCH;
        let accessed = self.ranges.get_mut(object_key).and_then(|entries| {
            entries.observe_covering(requested_range, access_clock, epoch, |entry| {
                entry.requested_bytes(requested_range)
            })
        });
        let Some((bytes, observation)) = accessed else {
            self.metrics.record_lookup(false);
            return None;
        };

        self.access_clock = access_clock;
        self.record_access_metrics(observation);
        self.metrics.record_lookup(true);
        Some(bytes)
    }

    pub(super) fn record_access(&mut self, object_key: &ObjectKey, requested_range: ByteRange) {
        self.record_successful_access(object_key, requested_range);
    }

    fn record_successful_access(&mut self, object_key: &ObjectKey, requested_range: ByteRange) {
        self.access_clock = self.access_clock.saturating_add(1);
        let epoch = self.current_epoch();
        let observation = self
            .ranges
            .get_mut(object_key)
            .and_then(|entries| entries.observe_covering(requested_range, self.access_clock, epoch, |_| ()))
            .map_or_else(PrefetchObservation::default, |(_, observation)| observation);
        self.record_access_metrics(observation);
    }

    fn record_access_metrics(&self, observation: PrefetchObservation) {
        self.metrics.record_access();
        if observation.demonstrated {
            self.metrics.record_prefetch_reuse();
        }
        if observation.ghost_promoted {
            self.metrics.record_ghost_promotion();
        }
    }

    /// Advances one bounded admission action while holding the shard lock.
    pub(super) fn admission_step(
        &mut self,
        object_key: &ObjectKey,
        range: ByteRange,
        bytes: &Bytes,
        requested_range: Option<ByteRange>,
        allow_compaction: bool,
    ) -> AdmissionStep {
        let superseded = match self.ranges.get(object_key) {
            Some(entries) if entries.covering(range).is_some() => {
                self.metrics.record_redundant();
                if let Some(requested_range) = requested_range {
                    self.record_successful_access(object_key, requested_range);
                }
                return AdmissionStep::Complete;
            }
            Some(entries) => entries.superseded_by(range),
            None => Superseded::default(),
        };
        let added_bytes = bytes.len() as u64;
        let effective_used = self.used_bytes - superseded.bytes;
        let target = self.capacity.saturating_sub(added_bytes);

        if effective_used <= target {
            let removal = self.remove_superseded(object_key, &superseded.ranges);
            debug_assert_eq!(removal.bytes, superseded.bytes);
            let ghost_hit = self.ghosts.take(object_key, range, self.access_clock);
            self.insert_admission(object_key.clone(), range, bytes.clone(), ghost_hit);

            if removal.entries != 0 {
                self.metrics.decrease_usage(removal.bytes, removal.entries);
            }
            self.metrics.increase_usage(added_bytes, 1);
            self.metrics.record_insert(removal.entries != 0);
            if ghost_hit {
                self.metrics.record_ghost_hit();
            }
            if let Some(requested_range) = requested_range {
                self.record_successful_access(object_key, requested_range);
            }
            return AdmissionStep::Complete;
        }

        let victim = self.eviction_victim(object_key, range);
        let compaction = if !allow_compaction {
            None
        } else {
            #[cfg(feature = "benchmark")]
            {
                if self.compaction_mode.is_disabled() {
                    None
                } else {
                    self.best_compaction(object_key, range)
                }
            }
            #[cfg(not(feature = "benchmark"))]
            {
                self.best_compaction(object_key, range)
            }
        };

        let compact = match (&compaction, &victim) {
            (Some(compaction), Some(victim)) => compare_action_loss(compaction, victim) != Ordering::Greater,
            (Some(_), None) => true,
            _ => false,
        };
        if compact {
            return AdmissionStep::Compact(
                self.compaction_work(compaction.expect("the selected compaction candidate must exist")),
            );
        }

        let Some(victim) = victim else {
            // A rotating bounded sample can temporarily consist entirely of
            // entries superseded by this admission. Advancing its cursor and
            // retrying gives amortized full coverage without one long scan.
            return AdmissionStep::Retry;
        };
        let removed = self
            .detach_entry(
                &victim.object_key,
                victim.range,
                Some(victim.id),
                victim.object_key == *object_key,
                RemovalCause::Evicted,
            )
            .expect("a sampled victim cannot disappear while its shard is locked");
        self.metrics.decrease_usage(removed, 1);
        self.metrics.record_evictions(1);
        if victim.in_probation {
            self.metrics.record_probation_eviction();
        }
        AdmissionStep::Retry
    }

    fn insert_admission(&mut self, object_key: ObjectKey, range: ByteRange, bytes: Bytes, ghost_hit: bool) {
        self.used_bytes += bytes.len() as u64;
        self.insertion_sequence = self.insertion_sequence.saturating_add(1);
        let sequence = self.insertion_sequence;
        let id = self.allocate_entry_id();
        let candidate_slot = self.candidates.register(CandidateRef {
            object_key: object_key.clone(),
            start: range.start(),
            id,
        });
        let entry = Entry::admission(id, range, bytes, sequence, candidate_slot, self.access_clock, ghost_hit);

        let entries = self.ranges.entry(object_key).or_default();
        entries.generation = entries.generation.saturating_add(1);
        let replaced = entries.by_start.insert(range.start(), entry);
        debug_assert!(replaced.is_none());
    }

    fn insert_compacted(&mut self, object_key: &ObjectKey, range: ByteRange, bytes: Bytes, sequence: u64) {
        let id = self.allocate_entry_id();
        let candidate_slot = self.candidates.register(CandidateRef {
            object_key: object_key.clone(),
            start: range.start(),
            id,
        });
        let entry = Entry::compacted(id, range, bytes, sequence, candidate_slot, self.access_clock);
        let entries = self.ranges.entry(object_key.clone()).or_default();
        entries.generation = entries.generation.saturating_add(1);
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

    fn remove_superseded(&mut self, object_key: &ObjectKey, ranges: &[ByteRange]) -> Removal {
        let mut removal = Removal::default();
        for &range in ranges {
            let bytes = self
                .detach_entry(object_key, range, None, true, RemovalCause::Superseded)
                .expect("the superseded entry was just found");
            removal.bytes += bytes;
            removal.entries += 1;
        }
        removal
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKey, range: ByteRange) -> bool {
        let Some(bytes) = self.detach_entry(object_key, range, None, false, RemovalCause::Explicit) else {
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
        preserve_access: bool,
        cause: RemovalCause,
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
            entries.generation = entries.generation.saturating_add(1);
            let object_is_empty = entries.by_start.is_empty();
            (entry, object_is_empty)
        };

        self.unregister_candidate(entry.candidate_slot, entry.id);
        if matches!(cause, RemovalCause::Evicted | RemovalCause::Compacted)
            && entry.has_observed_access
            && entry.broader_than_request
        {
            self.ghosts.record(object_key, entry.range, self.access_clock);
        }
        if object_is_empty && !preserve_access {
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

    fn eviction_victim(&mut self, admitting_key: &ObjectKey, admitting_range: ByteRange) -> Option<Victim> {
        let epoch = self.current_epoch();
        let (sample_start, sample_count) = self.candidates.victim_sample();
        let candidate_count = self.candidates.entries.len();
        let mut best: Option<Victim> = None;

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

            let exact = entries.accesses.retention(entry.range, epoch);
            let evidence = combine_retention(exact, entry.reuse.retention(epoch));
            let victim = Victim {
                object_key: candidate.object_key.clone(),
                range: entry.range,
                id: entry.id,
                bytes: entry.bytes.len() as u64,
                sequence: entry.sequence,
                evidence,
                in_probation: entry.is_in_probation(self.access_clock),
            };
            if best
                .as_ref()
                .is_none_or(|current| compare_retention(&victim, current).is_lt())
            {
                best = Some(victim);
            }
        }
        best
    }

    fn best_compaction(&mut self, admitting_key: &ObjectKey, admitting_range: ByteRange) -> Option<PlannedCompaction> {
        let epoch = self.current_epoch();
        let (sample_start, sample_count) = self.candidates.compaction_sample();
        let candidate_count = self.candidates.entries.len();
        let mut best: Option<PlannedCompaction> = None;

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
            if !entry.compaction_eligible(self.access_clock)
                || (candidate.object_key == *admitting_key && admitting_range.contains(entry.range))
            {
                continue;
            }
            let Some(plan) = plan_compaction(entry.range, entries.accesses.active_ranges(epoch)) else {
                continue;
            };
            let planned = PlannedCompaction {
                object_key: candidate.object_key.clone(),
                start: candidate.start,
                id: candidate.id,
                sequence: entry.sequence,
                generation: entries.generation,
                prefetch_value: scale_retention(entry.reuse.retention(epoch), PREFETCH_REUSE_WEIGHT),
                plan,
            };
            if best
                .as_ref()
                .is_none_or(|current| compare_compaction(&planned, current).is_lt())
            {
                best = Some(planned);
            }
        }
        best
    }

    fn compaction_work(&self, candidate: PlannedCompaction) -> CompactionWork {
        let entry = self
            .ranges
            .get(&candidate.object_key)
            .and_then(|entries| entries.by_start.get(&candidate.start))
            .filter(|entry| entry.id == candidate.id)
            .expect("a selected compaction source cannot disappear while its shard is locked");
        CompactionWork {
            object_key: candidate.object_key,
            start: candidate.start,
            id: candidate.id,
            sequence: candidate.sequence,
            generation: candidate.generation,
            plan: candidate.plan,
            source_bytes: entry.bytes.clone(),
        }
    }

    /// Publishes copied compaction output only if source metadata is unchanged.
    pub(super) fn publish_compaction(&mut self, prepared: PreparedCompaction) -> bool {
        let valid = self.ranges.get(&prepared.object_key).is_some_and(|entries| {
            entries.generation == prepared.generation
                && entries
                    .by_start
                    .get(&prepared.start)
                    .is_some_and(|entry| entry.id == prepared.id && entry.range == prepared.plan.source())
        });
        if !valid {
            return false;
        }

        let source_bytes = self
            .detach_entry(
                &prepared.object_key,
                prepared.plan.source(),
                Some(prepared.id),
                true,
                RemovalCause::Compacted,
            )
            .expect("the compaction source was revalidated immediately before removal");
        let mut retained_bytes = 0_u64;
        let mut retained_entries = 0_u64;

        for (retained_range, bytes) in prepared.retained {
            if self
                .ranges
                .get(&prepared.object_key)
                .and_then(|entries| entries.covering(retained_range))
                .is_some()
            {
                continue;
            }
            retained_bytes += bytes.len() as u64;
            retained_entries += 1;
            self.used_bytes += bytes.len() as u64;
            self.insert_compacted(&prepared.object_key, retained_range, bytes, prepared.sequence);
        }

        let reclaimed = source_bytes - retained_bytes;
        debug_assert!(reclaimed >= prepared.plan.reclaimed_bytes());
        self.metrics.decrease_usage(source_bytes, 1);
        if retained_entries != 0 {
            self.metrics.increase_usage(retained_bytes, retained_entries);
        }
        self.metrics.record_compaction(reclaimed);
        true
    }

    const fn current_epoch(&self) -> u64 {
        self.access_clock / ACCESSES_PER_EPOCH
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
    pub(super) fn access_evidence_len(&self, object_key: &ObjectKey) -> usize {
        self.ranges.get(object_key).map_or(0, |entries| entries.accesses.len())
    }

    #[cfg(test)]
    pub(super) fn active_reuse_events(&self, object_key: &ObjectKey, range: ByteRange) -> usize {
        let epoch = self.current_epoch();
        self.ranges
            .get(object_key)
            .and_then(|entries| entries.by_start.get(&range.start()))
            .filter(|entry| entry.range == range)
            .map_or(0, |entry| entry.reuse.active_len(epoch))
    }

    #[cfg(test)]
    pub(super) fn is_compaction_eligible(&self, object_key: &ObjectKey, range: ByteRange) -> bool {
        self.ranges
            .get(object_key)
            .and_then(|entries| entries.by_start.get(&range.start()))
            .filter(|entry| entry.range == range)
            .is_some_and(|entry| entry.compaction_eligible(self.access_clock))
    }

    #[cfg(test)]
    pub(super) fn ghost_count(&mut self) -> usize {
        self.ghosts.expire(self.access_clock);
        self.ghosts.entries.len()
    }

    #[cfg(test)]
    pub(super) fn candidate_count(&self) -> usize {
        self.candidates.entries.len()
    }
}

fn combine_retention(exact: RetentionEvidence, reuse: RetentionEvidence) -> RetentionEvidence {
    let reuse = scale_retention(reuse, PREFETCH_REUSE_WEIGHT);
    RetentionEvidence {
        weighted_frequency: exact.weighted_frequency.saturating_add(reuse.weighted_frequency),
        newest_epoch: exact.newest_epoch.max(reuse.newest_epoch),
    }
}

fn scale_retention(mut evidence: RetentionEvidence, weight: u64) -> RetentionEvidence {
    evidence.weighted_frequency = evidence.weighted_frequency.saturating_mul(weight);
    evidence
}

/// Lower density, older evidence, then older admission wins victim selection.
fn compare_retention(left: &Victim, right: &Victim) -> Ordering {
    compare_density(left.evidence, left.bytes, right.evidence, right.bytes)
        .then_with(|| left.sequence.cmp(&right.sequence))
        .then_with(|| left.object_key.cmp(&right.object_key))
        .then_with(|| left.range.cmp(&right.range))
}

/// Lower demonstrated-prefetch value per reclaimable byte is compacted first.
fn compare_compaction(left: &PlannedCompaction, right: &PlannedCompaction) -> Ordering {
    compare_density(
        left.prefetch_value,
        left.plan.reclaimed_bytes(),
        right.prefetch_value,
        right.plan.reclaimed_bytes(),
    )
    .then_with(|| left.sequence.cmp(&right.sequence))
    .then_with(|| left.plan.retained_bytes().cmp(&right.plan.retained_bytes()))
    .then_with(|| left.object_key.cmp(&right.object_key))
    .then_with(|| left.start.cmp(&right.start))
}

/// Compares value lost per byte between partial compaction and complete eviction.
fn compare_action_loss(compaction: &PlannedCompaction, victim: &Victim) -> Ordering {
    compare_density(
        compaction.prefetch_value,
        compaction.plan.reclaimed_bytes(),
        victim.evidence,
        victim.bytes,
    )
}

fn compare_density(left: RetentionEvidence, left_bytes: u64, right: RetentionEvidence, right_bytes: u64) -> Ordering {
    let left_density = u128::from(left.weighted_frequency) * u128::from(right_bytes);
    let right_density = u128::from(right.weighted_frequency) * u128::from(left_bytes);
    left_density
        .cmp(&right_density)
        .then_with(|| left.newest_epoch.cmp(&right.newest_epoch))
}

impl Drop for Shard {
    fn drop(&mut self) {
        let entries = self.entry_count() as u64;
        if entries != 0 {
            self.metrics.decrease_usage(self.used_bytes, entries);
        }
    }
}
