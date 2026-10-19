use std::sync::Arc;

use bytes::Bytes;
use feuer_types::{Download, ObjectKeyHash};
use rustc_hash::FxHashMap;

use super::MemoryCache;
use crate::{BufferPool, retention::sample_candidates};

/// An entry's download and position in the eviction candidates.
struct MemoryEntry {
    /// Payload and allocation charge. Lookup results share this allocation.
    download: Download,
    /// Position in the list sampled for eviction.
    eviction_index: usize,
}

/// Result of trying to insert a download, releasing the lock after each eviction.
pub(super) enum InsertResult {
    Inserted,
    AlreadyPresent,
    Evicted,
}

/// One memory-cache shard's object index and allocation accounting.
pub(super) struct MemoryCacheShard {
    capacity: u64,
    used_bytes: u64,
    entries_by_key: FxHashMap<ObjectKeyHash, MemoryEntry>,
    /// One key per entry, sampled in rotation for eviction.
    eviction_candidates: Vec<ObjectKeyHash>,
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
            next_sample_start: 0,
            buffer_pool,
        }
    }

    #[cfg(test)]
    pub(super) const fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    pub(super) fn get(&self, object_key: &ObjectKeyHash) -> Option<Bytes> {
        Some(self.entries_by_key.get(object_key)?.download.bytes().clone())
    }

    /// Tries to insert the object, sampling entries for eviction if space is needed.
    /// The caller releases the lock between eviction samples.
    pub(super) fn try_insert(
        &mut self,
        object_key: &ObjectKeyHash,
        download: &Download,
        cache: &MemoryCache,
    ) -> InsertResult {
        if self.entries_by_key.contains_key(object_key) {
            self.buffer_pool.metrics.record_redundant();
            return InsertResult::AlreadyPresent;
        }
        let max_existing_bytes = self.capacity.saturating_sub(download.allocation_charge() as u64);
        if self.used_bytes <= max_existing_bytes {
            self.add_entry(*object_key, download.clone());
            self.buffer_pool.metrics.record_insert();
            return InsertResult::Inserted;
        }

        let (_, key) = sample_candidates(
            &mut self.next_sample_start,
            &self.eviction_candidates,
            cache.reclaim_sample_size,
        )
        .map(|key| {
            let entry = &self.entries_by_key[key];
            let score = cache.retention_scorer.score(
                key,
                entry.download.bytes().len() as u64,
                entry.download.allocation_charge() as u64,
                &cache.access_histories,
            );
            (score, *key)
        })
        .min_by(|(score, key), (other_score, other_key)| score.total_cmp(other_score).then_with(|| key.cmp(other_key)))
        .expect("positive usage requires an eviction candidate");
        self.remove_entry(&key);
        InsertResult::Evicted
    }

    /// Adds an object known not to be indexed.
    fn add_entry(&mut self, object_key: ObjectKeyHash, download: Download) {
        let allocation_charge = download.allocation_charge() as u64;
        self.used_bytes += allocation_charge;
        self.buffer_pool.add_entry_bytes(allocation_charge);
        self.buffer_pool.metrics.entries.increase(1);
        let eviction_index = self.eviction_candidates.len();
        self.eviction_candidates.push(object_key);
        self.entries_by_key.insert(
            object_key,
            MemoryEntry {
                download,
                eviction_index,
            },
        );
    }

    pub(super) fn contains_entry(&self, key: &ObjectKeyHash) -> bool {
        self.entries_by_key.contains_key(key)
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKeyHash) -> bool {
        if !self.contains_entry(object_key) {
            return false;
        }
        self.remove_entry(object_key);
        self.buffer_pool.metrics.record_remove();
        true
    }

    /// Removes an indexed object, updating shard, buffer-pool, and metric usage.
    fn remove_entry(&mut self, object_key: &ObjectKeyHash) {
        let removed_entry = self.entries_by_key.remove(object_key).unwrap();
        self.eviction_candidates.swap_remove(removed_entry.eviction_index);
        if let Some(key) = self.eviction_candidates.get(removed_entry.eviction_index) {
            self.entries_by_key.get_mut(key).unwrap().eviction_index = removed_entry.eviction_index;
        }
        let removed_bytes = removed_entry.download.allocation_charge() as u64;
        self.used_bytes -= removed_bytes;
        self.buffer_pool.remove_entry_bytes(removed_bytes);
        self.buffer_pool.metrics.entries.decrease(1);
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
