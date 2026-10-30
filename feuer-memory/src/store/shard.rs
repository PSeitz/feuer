use std::sync::Arc;

use bytes::Bytes;
use feuer_types::ObjectKeyHash;
use rustc_hash::FxHashMap;

use super::MemoryCache;
use crate::{BufferPool, retention::sample_candidates};

/// An entry's bytes, allocation charge, and position in the eviction candidates.
struct MemoryEntry {
    /// Payload shared with lookup results.
    bytes: Bytes,
    allocation_charge: usize,
    /// Position in the list sampled for eviction.
    candidate_index: usize,
}

/// Whether an insertion attempt inserted the object, found its key, or evicted an entry.
pub(super) enum InsertResult {
    Inserted,
    AlreadyPresent,
    Evicted,
}

/// A memory-cache shard's object index and allocation accounting.
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
        Some(self.entries_by_key.get(object_key)?.bytes.clone())
    }

    /// Tries to insert the object, sampling entries for eviction if space is needed.
    /// The caller releases the lock between eviction samples.
    pub(super) fn try_insert(
        &mut self,
        object_key: &ObjectKeyHash,
        bytes: &Bytes,
        allocation_charge: usize,
        cache: &MemoryCache,
    ) -> InsertResult {
        if self.entries_by_key.contains_key(object_key) {
            self.buffer_pool.metrics.redundant_insertions.increase(1);
            return InsertResult::AlreadyPresent;
        }
        let remaining_capacity = self.capacity.saturating_sub(allocation_charge as u64);
        if self.used_bytes <= remaining_capacity {
            self.add_entry(*object_key, bytes.clone(), allocation_charge);
            self.buffer_pool.metrics.insertions.increase(1);
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
                entry.bytes.len() as u64,
                entry.allocation_charge as u64,
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
    fn add_entry(&mut self, object_key: ObjectKeyHash, bytes: Bytes, allocation_charge: usize) {
        self.used_bytes += allocation_charge as u64;
        self.buffer_pool.add_entry_bytes(allocation_charge as u64);
        self.buffer_pool.metrics.entries.increase(1);
        let candidate_index = self.eviction_candidates.len();
        self.eviction_candidates.push(object_key);
        self.entries_by_key.insert(
            object_key,
            MemoryEntry {
                bytes,
                allocation_charge,
                candidate_index,
            },
        );
    }

    pub(super) fn contains_entry(&self, key: &ObjectKeyHash) -> bool {
        self.entries_by_key.contains_key(key)
    }

    pub(super) fn remove(&mut self, object_key: &ObjectKeyHash) -> bool {
        if !self.remove_entry(object_key) {
            return false;
        }
        self.buffer_pool.metrics.removals.increase(1);
        true
    }

    /// Removes an object if present, updating shard, buffer-pool, and metric usage.
    fn remove_entry(&mut self, object_key: &ObjectKeyHash) -> bool {
        let Some(removed_entry) = self.entries_by_key.remove(object_key) else {
            return false;
        };
        self.eviction_candidates.swap_remove(removed_entry.candidate_index);
        if let Some(key) = self.eviction_candidates.get(removed_entry.candidate_index) {
            self.entries_by_key.get_mut(key).unwrap().candidate_index = removed_entry.candidate_index;
        }
        let removed_bytes = removed_entry.allocation_charge as u64;
        self.used_bytes -= removed_bytes;
        self.buffer_pool.remove_entry_bytes(removed_bytes);
        self.buffer_pool.metrics.entries.decrease(1);
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
