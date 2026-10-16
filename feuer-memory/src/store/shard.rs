use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use bytes::Bytes;
use feuer_historian::ObjectAccessHistories;
use feuer_types::{ByteRange, Download, ObjectKeyHash};
use rustc_hash::FxHashMap;

use super::{MemoryCache, range_trim::plan_range_trim};
use crate::{BufferPool, retention::sample_candidates};

/// Minimum requests across all keys since admission before trimming a range.
pub(super) const MIN_REQUESTS_BEFORE_RANGE_TRIM: u64 = 64;

/// An entry's download and position in the eviction candidates.
struct MemoryEntry {
    /// Exact object interval, payload, and allocation charge. Lookup results share this allocation.
    download: Download,
    /// Position in the list sampled for eviction.
    eviction_index: usize,
}

/// One object's entries, indexed by their exact byte ranges.
///
/// Compaction can publish overlapping and contained request ranges independently.
#[derive(Default)]
struct ObjectEntries {
    by_range: BTreeMap<ByteRange, MemoryEntry>,
}

impl ObjectEntries {
    /// Prefers an exact entry, otherwise finds one covering the entire request.
    fn covering_entry(&self, range: ByteRange) -> Option<&MemoryEntry> {
        self.by_range.get(&range).or_else(|| {
            self.by_range
                .range(..=ByteRange::new(range.start(), u64::MAX).unwrap())
                .rev()
                .map(|(_, entry)| entry)
                .find(|entry| entry.download.downloaded_range().contains(range))
        })
    }

    /// Iterates contained entries, including empty entries at the range's end.
    fn contained_entries(&self, range: ByteRange) -> impl Iterator<Item = &MemoryEntry> {
        self.by_range
            .range(
                ByteRange::new(range.start(), range.start()).unwrap()
                    ..=ByteRange::new(range.end(), range.end()).unwrap(),
            )
            .map(|(_, entry)| entry)
            .filter(move |entry| range.contains(entry.download.downloaded_range()))
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

/// Result of trying to insert a download:
/// finished, retry after an eviction attempt, or compaction plans for the caller to process.
pub(super) enum InsertResult {
    Inserted,
    AlreadyCovered,
    Retry { evicted: bool },
    Trim(Vec<RangeTrimSource>),
}

/// One memory-cache shard's range indexes and allocation accounting.
pub(super) struct MemoryCacheShard {
    capacity: u64,
    used_bytes: u64,
    entries_by_key: FxHashMap<ObjectKeyHash, ObjectEntries>,
    /// One key and range per entry, sampled in rotation for eviction.
    eviction_candidates: Vec<(ObjectKeyHash, ByteRange)>,
    /// Insertion request count, key, and range awaiting one compaction check, oldest first.
    compaction_candidates: VecDeque<(u64, ObjectKeyHash, ByteRange)>,
    next_sample_start: usize,
    buffer_pool: Arc<BufferPool>,
}

impl MemoryCacheShard {
    pub(super) fn new(capacity: u64, buffer_pool: Arc<BufferPool>) -> Self {
        Self {
            capacity,
            used_bytes: 0,
            entries_by_key: FxHashMap::default(),
            eviction_candidates: Vec::new(),
            compaction_candidates: VecDeque::new(),
            next_sample_start: 0,
            buffer_pool,
        }
    }

    #[cfg(test)]
    pub(super) const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    #[cfg(test)]
    pub(super) fn compaction_candidate_count(&self) -> usize {
        self.compaction_candidates.len()
    }

    pub(super) fn get(&self, object_key: &ObjectKeyHash, requested: Option<ByteRange>) -> Option<Bytes> {
        let download = &self
            .entries_by_key
            .get(object_key)?
            .covering_entry(requested.unwrap_or_else(|| ByteRange::new(0, 0).unwrap()))?
            .download;
        Some(download.bytes_in_range(requested.unwrap_or(download.downloaded_range())))
    }

    /// Tries to insert the download, preparing compaction plans or sampling entries for eviction if space is needed.
    /// Entries contained in the download are subtracted from usage and skipped during reclamation.
    /// The caller checks compaction candidates once per insertion and releases the lock between eviction samples.
    pub(super) fn try_insert(
        &mut self,
        object_key: &ObjectKeyHash,
        download: &Download,
        cache: &MemoryCache,
        compact_before_eviction: bool,
    ) -> InsertResult {
        let range = download.downloaded_range();
        let contained_allocation_bytes: u64 = match self.entries_by_key.get(object_key) {
            Some(entries) if entries.covering_entry(range).is_some() => {
                self.buffer_pool.metrics.record_redundant();
                return InsertResult::AlreadyCovered;
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
            return InsertResult::Inserted;
        }

        if compact_before_eviction {
            let request_count = cache.access_histories.request_count();
            let ready_count = self.compaction_candidates.partition_point(|(inserted, _, _)| {
                request_count.saturating_sub(*inserted) >= MIN_REQUESTS_BEFORE_RANGE_TRIM
            });
            let sources: Vec<_> = self
                .compaction_candidates
                .drain(..ready_count)
                .filter(|(_, key, candidate_range)| key != object_key || !range.contains(*candidate_range))
                .filter_map(|(_, key, range)| {
                    let replacement_ranges =
                        plan_range_trim(range, cache.access_histories.recent_requested_ranges(&key))?;
                    Some(RangeTrimSource {
                        object_key: key,
                        download: self.entries_by_key[&key].by_range[&range].download.clone(),
                        replacement_ranges,
                    })
                })
                .collect();
            if !sources.is_empty() {
                return InsertResult::Trim(sources);
            }
        }
        let entry_to_evict = sample_candidates(
            &mut self.next_sample_start,
            &self.eviction_candidates,
            cache.reclaim_sample_size,
        )
        .filter(|(key, candidate_range)| key != object_key || !range.contains(*candidate_range))
        .map(|(key, range)| {
            let entry = &self.entries_by_key[key].by_range[range];
            let score = cache.retention_scorer.score(
                key,
                *range,
                entry.download.allocation_charge() as u64,
                &cache.access_histories,
            );
            (score, *key, *range)
        })
        .min_by(|(score, key, range), (other_score, other_key, other_range)| {
            score
                .total_cmp(other_score)
                .then_with(|| (key, range).cmp(&(other_key, other_range)))
        });
        let Some((_, key, range)) = entry_to_evict else {
            // The caller releases the shard lock before searching the next sample.
            return InsertResult::Retry { evicted: false };
        };
        self.remove_entry(&key, range);
        InsertResult::Retry { evicted: true }
    }

    /// Inserts an entry not already covered, removes entries it contains, and returns whether any were replaced.
    fn insert_entry(&mut self, object_key: ObjectKeyHash, download: Download, request_count: u64) -> bool {
        let range = download.downloaded_range();
        let mut replaced = false;
        while let Some(contained_range) = self.entries_by_key.get(&object_key).and_then(|entries| {
            entries
                .contained_entries(range)
                .next()
                .map(|entry| entry.download.downloaded_range())
        }) {
            self.remove_entry(&object_key, contained_range);
            replaced = true;
        }
        self.add_entry(object_key, download, request_count);
        replaced
    }

    /// Adds one exact range known not to be indexed, without removing contained entries.
    fn add_entry(&mut self, object_key: ObjectKeyHash, download: Download, request_count: u64) {
        let range = download.downloaded_range();
        let allocation_charge = download.allocation_charge() as u64;
        self.used_bytes += allocation_charge;
        self.buffer_pool.add_entry_bytes(allocation_charge);
        self.buffer_pool.metrics.entries.increase(1);
        let eviction_index = self.eviction_candidates.len();
        self.eviction_candidates.push((object_key, range));
        self.compaction_candidates.push_back((request_count, object_key, range));
        let entries = self.entries_by_key.entry(object_key).or_default();
        let entry = MemoryEntry {
            download,
            eviction_index,
        };

        entries.by_range.insert(range, entry);
    }

    pub(super) fn contains_entry(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
        self.entries_by_key
            .get(key)
            .is_some_and(|entries| entries.by_range.contains_key(&range))
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKeyHash, range: ByteRange) -> bool {
        if !self.contains_entry(object_key, range) {
            return false;
        }
        self.remove_entry(object_key, range);
        self.buffer_pool.metrics.record_remove();
        true
    }

    /// Removes an entry known to exist at this key and range, updating shard, buffer-pool, and metric usage.
    fn remove_entry(&mut self, object_key: &ObjectKeyHash, range: ByteRange) {
        let entries = self.entries_by_key.get_mut(object_key).unwrap();
        let removed_entry = entries.by_range.remove(&range).unwrap();
        if entries.by_range.is_empty() {
            self.entries_by_key.remove(object_key);
        }
        self.compaction_candidates
            .retain(|(_, key, candidate_range)| key != object_key || *candidate_range != range);
        self.eviction_candidates.swap_remove(removed_entry.eviction_index);
        if let Some(&(key, range)) = self.eviction_candidates.get(removed_entry.eviction_index) {
            let entries = &mut self.entries_by_key.get_mut(&key).unwrap().by_range;
            entries.get_mut(&range).unwrap().eviction_index = removed_entry.eviction_index;
        }
        let removed_bytes = removed_entry.download.allocation_charge() as u64;
        self.used_bytes -= removed_bytes;
        self.buffer_pool.remove_entry_bytes(removed_bytes);
        self.buffer_pool.metrics.entries.decrease(1);
    }

    /// Publishes copied range trimming output only if the exact source range is still cached.
    /// Retains only newly published payloads for the compaction callback.
    pub(super) fn publish_range_trim(
        &mut self,
        replacement: &mut RangeTrimReplacement,
        histories: &ObjectAccessHistories,
    ) -> bool {
        // Read under the shard lock to preserve candidate order.
        let request_count = histories.request_count();
        // Objects are immutable: reinsertion and neighboring-range changes do not invalidate the bytes.
        // New requests may change the desirability of the plan, but do not invalidate it.
        let used_bytes_before_trim = self.used_bytes;
        if !self.contains_entry(&replacement.object_key, replacement.source_range) {
            return false;
        }
        self.remove_entry(&replacement.object_key, replacement.source_range);

        replacement.payloads.retain(|download| {
            if self.contains_entry(&replacement.object_key, download.downloaded_range()) {
                return false;
            }
            self.add_entry(replacement.object_key, download.clone(), request_count);
            true
        });

        let reclaimed_bytes = used_bytes_before_trim - self.used_bytes;
        self.buffer_pool.metrics.record_range_trim(reclaimed_bytes);
        true
    }

    pub(super) fn entry_count(&self) -> usize {
        self.eviction_candidates.len()
    }
}

impl Drop for MemoryCacheShard {
    fn drop(&mut self) {
        self.buffer_pool.metrics.entries.decrease(self.entry_count() as u64);
        self.buffer_pool.remove_entry_bytes(self.used_bytes);
    }
}
