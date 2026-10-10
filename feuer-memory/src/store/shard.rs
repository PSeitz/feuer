use std::{collections::BTreeMap, sync::Arc};

use bytes::Bytes;
use feuer_historian::ObjectAccessHistories;
use feuer_types::{ByteRange, Download, ObjectKeyHash};
use rustc_hash::FxHashMap;

use super::{MemoryCache, range_trim::plan_range_trim};
use crate::{BufferPool, retention::sample_candidates};

/// Minimum requests across all keys since admission before trimming a range.
pub(super) const MIN_REQUESTS_BEFORE_RANGE_TRIM: u64 = 64;

/// An entry's payload bytes, object range, allocation charge, candidate index, and request count at insertion.
struct MemoryEntry {
    /// Exact object interval, payload, and allocation charge. Lookup results share this allocation.
    download: Download,
    /// Index of this entry's candidate in the list sampled for trimming or eviction.
    candidate_index: usize,
    /// Total request count when this entry was inserted.
    request_count_at_insertion: u64,
}

impl MemoryEntry {
    fn range(&self) -> ByteRange {
        self.download.downloaded_range()
    }
}

/// One object's entries, indexed by their starting byte offset.
///
/// No entry's range fully contains another. Partial overlaps remain indexed.
/// Because starts and ends both increase, the predecessor of a request start
/// is the only possible covering entry.
#[derive(Default)]
struct ObjectEntries {
    /// Entries ordered by exact start for predecessor-based covering lookup.
    by_start: BTreeMap<u64, MemoryEntry>,
}

impl ObjectEntries {
    /// Finds the entry whose byte range covers the entire request.
    fn covering_entry(&self, range: ByteRange) -> Option<&MemoryEntry> {
        let (_, entry) = self.by_start.range(..=range.start()).next_back()?;
        entry.range().contains(range).then_some(entry)
    }

    /// Iterates contained entries, including empty entries at the range's end.
    fn contained_entries(&self, range: ByteRange) -> impl Iterator<Item = &MemoryEntry> {
        self.by_start
            .range(range.start()..)
            .map(|(_, entry)| entry)
            .take_while(move |entry| range.contains(entry.range()))
    }
}

/// Source download and replacement ranges for trimming outside the shard lock.
pub(super) struct RangeTrimSource {
    object_key: ObjectKeyHash,
    download: Download,
    replacement_ranges: Box<[ByteRange]>,
}

impl RangeTrimSource {
    /// Copies payloads for the replacement ranges into separate allocations outside the shard metadata lock.
    pub(super) fn copy_replacement_payloads(self) -> RangeTrimReplacement {
        let payloads = self
            .replacement_ranges
            .iter()
            .map(|range| {
                let bytes = Bytes::copy_from_slice(&self.download.bytes_in_range(*range));
                Download::new(range.start(), bytes).expect("trimmed range is contained in the source")
            })
            .collect();
        RangeTrimReplacement {
            object_key: self.object_key,
            source_range: self.download.downloaded_range(),
            payloads,
        }
    }
}

/// Replacement payloads from trimming a cached range, awaiting the source-range check.
pub(super) struct RangeTrimReplacement {
    pub(super) object_key: ObjectKeyHash,
    source_range: ByteRange,
    pub(super) payloads: Vec<Download>,
}

/// Result of trying to insert a download or reclaim space:
/// finished, retry with eviction accounting, or trim a range.
pub(super) enum InsertOrReclaimResult {
    Complete(bool),
    Retry { evicted: bool },
    Trim(RangeTrimSource),
}

/// One memory-cache shard's range indexes and allocation accounting.
pub(super) struct MemoryCacheShard {
    capacity: u64,
    used_bytes: u64,
    entries_by_key: FxHashMap<ObjectKeyHash, ObjectEntries>,
    /// One key and range start per indexed entry, sampled in rotation for trimming or eviction.
    reclaim_candidates: Vec<(ObjectKeyHash, u64)>,
    next_sample_start: usize,
    buffer_pool: Arc<BufferPool>,
}

impl MemoryCacheShard {
    pub(super) fn new(capacity: u64, buffer_pool: Arc<BufferPool>) -> Self {
        Self {
            capacity,
            used_bytes: 0,
            entries_by_key: FxHashMap::default(),
            reclaim_candidates: Vec::new(),
            next_sample_start: 0,
            buffer_pool,
        }
    }

    #[cfg(test)]
    pub(super) const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    pub(super) fn get(&self, object_key: &ObjectKeyHash, requested: Option<ByteRange>) -> Option<Bytes> {
        self.entries_by_key
            .get(object_key)?
            .covering_entry(requested.unwrap_or_else(|| ByteRange::new(0, 0).unwrap()))
            .map(|entry| entry.download.bytes_in_range(requested.unwrap_or(entry.range())))
    }

    /// Tries to admit the download or reclaim space using the cache's access history and sample size.
    /// Entries whose ranges are fully contained in the incoming download will be replaced on insertion.
    /// Their bytes are already subtracted when checking capacity, so eviction sampling skips them.
    /// If a sample has no eligible victim, the caller releases the lock before trying the next sample
    /// to bound sampling work per lock hold.
    pub(super) fn try_admit_or_reclaim(
        &mut self,
        object_key: &ObjectKeyHash,
        download: &Download,
        cache: &MemoryCache,
        allow_range_trim: bool,
    ) -> InsertOrReclaimResult {
        let range = download.downloaded_range();
        let contained_allocation_bytes: u64 = match self.entries_by_key.get(object_key) {
            Some(entries) if entries.covering_entry(range).is_some() => {
                self.buffer_pool.metrics.record_redundant();
                return InsertOrReclaimResult::Complete(false);
            }
            Some(entries) => entries
                .contained_entries(range)
                .map(|entry| entry.download.allocation_charge() as u64)
                .sum(),
            None => 0,
        };
        let used_bytes_without_contained_entries = self.used_bytes - contained_allocation_bytes;
        let max_existing_bytes = self.capacity.saturating_sub(download.allocation_charge() as u64);

        if used_bytes_without_contained_entries <= max_existing_bytes {
            let replaced = self.insert_entry(*object_key, download.clone(), cache.access_histories.request_count());

            self.buffer_pool.metrics.record_insert(replaced);
            return InsertOrReclaimResult::Complete(true);
        }

        let Some((key, entry)) = self.select_reclaim_candidate(object_key, range, cache) else {
            // The sample cursor advanced, but no eligible victim was found.
            // Return to the caller so it can release the lock before sampling again.
            return InsertOrReclaimResult::Retry { evicted: false };
        };
        if allow_range_trim && let Some(source) = Self::prepare_range_trim(key, entry, &cache.access_histories) {
            return InsertOrReclaimResult::Trim(source);
        }

        let range = entry.range();
        self.remove_entry(&key, range.start());
        InsertOrReclaimResult::Retry { evicted: true }
    }

    /// Inserts an entry not already covered, removes entries it contains, and returns whether any were replaced.
    fn insert_entry(&mut self, object_key: ObjectKeyHash, download: Download, request_count: u64) -> bool {
        let range = download.downloaded_range();
        let allocation_charge = download.allocation_charge() as u64;
        let mut replaced = false;
        while let Some(contained_range) = self
            .entries_by_key
            .get(&object_key)
            .and_then(|entries| entries.contained_entries(range).next().map(|entry| entry.range()))
        {
            self.remove_entry(&object_key, contained_range.start());
            replaced = true;
        }
        self.used_bytes += allocation_charge;
        self.buffer_pool.add_entry_bytes(allocation_charge);
        self.buffer_pool.metrics.entries.increase(1);
        let candidate_index = self.reclaim_candidates.len();
        self.reclaim_candidates.push((object_key, range.start()));
        let entries = self.entries_by_key.entry(object_key).or_default();
        let entry = MemoryEntry {
            download,
            candidate_index,
            request_count_at_insertion: request_count,
        };

        entries.by_start.insert(range.start(), entry);
        replaced
    }

    pub(super) fn contains_entry(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
        self.entries_by_key
            .get(key)
            .and_then(|entries| entries.by_start.get(&range.start()))
            .is_some_and(|entry| entry.range() == range)
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKeyHash, range: ByteRange) -> bool {
        if !self.contains_entry(object_key, range) {
            return false;
        }
        self.remove_entry(object_key, range.start());
        self.buffer_pool.metrics.record_remove();
        true
    }

    /// Removes an entry known to exist at this key and start, updating shard, buffer-pool, and metric usage.
    fn remove_entry(&mut self, object_key: &ObjectKeyHash, start: u64) {
        let entries = self.entries_by_key.get_mut(object_key).unwrap();
        let removed_entry = entries.by_start.remove(&start).unwrap();
        if entries.by_start.is_empty() {
            self.entries_by_key.remove(object_key);
        }
        self.reclaim_candidates.swap_remove(removed_entry.candidate_index);
        if let Some((key, start)) = self.reclaim_candidates.get(removed_entry.candidate_index) {
            self.entries_by_key
                .get_mut(key)
                .unwrap()
                .by_start
                .get_mut(start)
                .unwrap()
                .candidate_index = removed_entry.candidate_index;
        }
        let removed_bytes = removed_entry.download.allocation_charge() as u64;
        self.used_bytes -= removed_bytes;
        self.buffer_pool.remove_entry_bytes(removed_bytes);
        self.buffer_pool.metrics.entries.decrease(1);
    }

    /// Samples eligible entries and selects the lowest retention score, breaking ties by key and range.
    fn select_reclaim_candidate(
        &mut self,
        admitting_key: &ObjectKeyHash,
        admitting_range: ByteRange,
        cache: &MemoryCache,
    ) -> Option<(ObjectKeyHash, &MemoryEntry)> {
        sample_candidates(
            &mut self.next_sample_start,
            &self.reclaim_candidates,
            cache.reclaim_sample_size,
        )
        .map(|(key, start)| (key, &self.entries_by_key[key].by_start[start]))
        .filter(|(key, entry)| **key != *admitting_key || !admitting_range.contains(entry.range()))
        .map(|(key, entry)| {
            let score = cache.retention_scorer.score(
                key,
                entry.range(),
                entry.download.allocation_charge() as u64,
                &cache.access_histories,
            );
            (key, entry, score)
        })
        .min_by(
            |(left_key, left_entry, left_score), (right_key, right_entry, right_score)| {
                left_score
                    .total_cmp(right_score)
                    .then_with(|| left_key.cmp(right_key))
                    .then_with(|| left_entry.range().cmp(&right_entry.range()))
            },
        )
        .map(|(object_key, entry, _)| (*object_key, entry))
    }

    /// Prepares a range trim by retaining its source bytes and plan after the grace period.
    fn prepare_range_trim(
        object_key: ObjectKeyHash,
        entry: &MemoryEntry,
        access_histories: &ObjectAccessHistories,
    ) -> Option<RangeTrimSource> {
        let request_count = access_histories.request_count();
        if request_count.saturating_sub(entry.request_count_at_insertion) < MIN_REQUESTS_BEFORE_RANGE_TRIM {
            return None;
        }
        let replacement_ranges = plan_range_trim(entry.range(), access_histories.recent_requested_ranges(&object_key))?;
        Some(RangeTrimSource {
            object_key,
            download: entry.download.clone(),
            replacement_ranges,
        })
    }

    /// Publishes copied range trimming output only if the exact source range is still cached.
    /// Retains only newly published payloads for the compaction callback.
    pub(super) fn publish_range_trim(&mut self, replacement: &mut RangeTrimReplacement, request_count: u64) -> bool {
        // Objects are immutable: reinsertion and neighboring-range changes do not invalidate the bytes.
        // New requests may change the desirability of the plan, but do not invalidate it.
        let used_bytes_before_trim = self.used_bytes;
        if !self.contains_entry(&replacement.object_key, replacement.source_range) {
            return false;
        }
        self.remove_entry(&replacement.object_key, replacement.source_range.start());

        replacement.payloads.retain(|download| {
            if self
                .entries_by_key
                .get(&replacement.object_key)
                .and_then(|entries| entries.covering_entry(download.downloaded_range()))
                .is_some()
            {
                return false;
            }
            self.insert_entry(replacement.object_key, download.clone(), request_count);
            true
        });

        let reclaimed_bytes = used_bytes_before_trim - self.used_bytes;
        self.buffer_pool.metrics.record_range_trim(reclaimed_bytes);
        true
    }

    pub(super) fn entry_count(&self) -> usize {
        self.reclaim_candidates.len()
    }
}

impl Drop for MemoryCacheShard {
    fn drop(&mut self) {
        self.buffer_pool.metrics.entries.decrease(self.entry_count() as u64);
        self.buffer_pool.remove_entry_bytes(self.used_bytes);
    }
}
