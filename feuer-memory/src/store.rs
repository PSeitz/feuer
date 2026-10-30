mod shard;
#[cfg(test)]
mod tests;

use std::{fmt, hash::BuildHasher, sync::Arc, thread::available_parallelism};

use bytes::Bytes;
use feuer_historian::ObjectAccessHistories;
use feuer_types::ObjectKeyHash;
use parking_lot::Mutex;
use rustc_hash::FxBuildHasher;

use self::shard::{InsertResult, MemoryCacheShard};
#[cfg(feature = "benchmark")]
use crate::read_idle_buffer_pool_percent;
use crate::{
    BufferPool, MemoryMetrics,
    buffer::DEFAULT_IDLE_BUFFER_POOL_PERCENT,
    retention::{AvoidedRetrievalCostScorer, RECLAIM_SAMPLE_SIZE, RetentionScorer},
};

/// Caps lock partitioning to avoid excessive per-cache metadata.
const MAX_SHARDS: usize = 64;

/// A sharded object cache with soft capacity.
///
/// Each [`ObjectKeyHash`] identifies an immutable object. Lookups return its
/// payload, sharing the entry's allocation without holding an entry guard.
/// Request history is supplied separately. Cache operations consult it for
/// retention decisions but never record requests or remove history.
///
/// Cached allocation charges and idle read buffers share the configured capacity.
/// Each shard evicts locally against its share before insertion. Admission also
/// frees idle buffers when needed. An allocation charge larger than its shard's
/// target is admitted once that shard's charged usage reaches zero, so total usage
/// can exceed capacity.
/// Under pressure, a rotating sample selects the lowest-scoring entry for eviction.
/// Default scores model recent retrieval value per charged allocation byte.
pub struct MemoryCache {
    /// Independently locked partitions selected by object identity.
    shards: Box<[Mutex<MemoryCacheShard>]>,
    access_histories: Arc<ObjectAccessHistories>,
    retention_scorer: Arc<dyn RetentionScorer>,
    reclaim_sample_size: usize,
    buffer_pool: Arc<BufferPool>,
}

impl fmt::Debug for MemoryCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryCache")
            .field("capacity", &self.capacity())
            .field("used_bytes", &self.used_bytes())
            .field("shards", &self.shards.len())
            .finish_non_exhaustive()
    }
}

impl MemoryCache {
    /// Creates a cache with a shared allocation-byte target, empty history, and no-op metrics.
    pub fn new(capacity: u64) -> Self {
        Self::with_metrics(capacity, MemoryMetrics::noop(), DEFAULT_IDLE_BUFFER_POOL_PERCENT)
    }

    /// Creates a cache with registered metrics and an idle buffer ceiling (0–100) of its capacity.
    /// Zero disables idle retention. Panics if the percentage exceeds 100.
    pub fn with_metrics(capacity: u64, metrics: Arc<MemoryMetrics>, idle_buffer_pool_percent: u64) -> Self {
        Self::with_shard_count(
            BufferPool::with_idle_buffer_pool_percent(capacity, metrics, idle_buffer_pool_percent),
            default_shard_count(),
        )
    }

    /// Consults the supplied request history for retention decisions.
    /// The caller records requests directly in that history before lookup, independently of cache operations.
    pub fn with_access_histories(mut self, access_histories: Arc<ObjectAccessHistories>) -> Self {
        self.access_histories = access_histories;
        self
    }

    /// Creates a cache with an explicit shard count for controlled benchmarks.
    /// Reads `FEUER_IDLE_BUFFER_POOL_PERCENT`. Panics if it is invalid.
    #[cfg(feature = "benchmark")]
    #[doc(hidden)]
    pub fn with_shards_for_benchmark(capacity: u64, num_shards: usize) -> Self {
        let percent = read_idle_buffer_pool_percent().unwrap_or_else(|error| panic!("{error}"));
        let buffer_pool = BufferPool::with_idle_buffer_pool_percent(capacity, MemoryMetrics::noop(), percent);
        Self::with_shard_count(buffer_pool, num_shards)
    }

    fn with_shard_count(buffer_pool: Arc<BufferPool>, num_shards: usize) -> Self {
        assert!(num_shards > 0, "memory cache requires at least one shard");
        let capacity = buffer_pool.capacity;
        let shards = (0..num_shards)
            .map(|shard_index| {
                Mutex::new(MemoryCacheShard::new(
                    shard_capacity_for(capacity, num_shards, shard_index),
                    buffer_pool.clone(),
                ))
            })
            .collect();
        Self {
            shards,
            access_histories: Arc::new(ObjectAccessHistories::new()),
            retention_scorer: Arc::new(AvoidedRetrievalCostScorer::default()),
            reclaim_sample_size: RECLAIM_SAMPLE_SIZE,
            buffer_pool,
        }
    }

    /// Uses application-provided retention scores.
    pub fn with_retention_scorer(mut self, scorer: Arc<dyn RetentionScorer>) -> Self {
        self.retention_scorer = scorer;
        self
    }

    /// Sets the maximum candidates inspected per eviction decision. Panics if zero.
    pub fn with_reclaim_sample_size(mut self, sample_size: usize) -> Self {
        assert!(sample_size > 0, "reclaim sample size must be greater than zero");
        self.reclaim_sample_size = sample_size;
        self
    }

    /// Returns the shared capacity for cached allocations and idle buffers.
    pub fn capacity(&self) -> u64 {
        self.buffer_pool.capacity
    }

    /// Returns cached allocation charges plus idle buffer capacity.
    /// Active reads and caller-only results are excluded.
    pub fn used_bytes(&self) -> u64 {
        self.buffer_pool.used_bytes()
    }

    /// The request history consulted by this cache for retention decisions.
    pub fn access_histories(&self) -> &Arc<ObjectAccessHistories> {
        &self.access_histories
    }

    /// The aligned buffer pool shared by this cache's storage readers.
    pub fn buffer_pool(&self) -> Arc<BufferPool> {
        self.buffer_pool.clone()
    }

    /// Looks up an object without recording a request.
    pub fn get(&self, object_key: &ObjectKeyHash) -> Option<Bytes> {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index].lock().get(object_key)
    }

    /// Caches an object without recording a request, using its allocation charge.
    /// An existing entry for the key is retained and the supplied bytes are discarded.
    /// Returns whether the object was inserted.
    ///
    /// # Panics
    ///
    /// Panics if `allocation_charge` is smaller than the payload length.
    pub fn insert(&self, object_key: ObjectKeyHash, bytes: Bytes, allocation_charge: usize) -> bool {
        assert!(allocation_charge >= bytes.len());
        let shard = &self.shards[self.shard_index(&object_key)];
        let mut evicted_any_entry = false;
        let inserted = loop {
            let insert_result = shard.lock().try_insert(&object_key, &bytes, allocation_charge, self);
            match insert_result {
                InsertResult::Inserted => break true,
                InsertResult::AlreadyPresent => break false,
                InsertResult::Evicted => evicted_any_entry = true,
            }
        };
        if evicted_any_entry {
            self.buffer_pool.metrics.eviction_triggering_insertions.increase(1);
        }
        inserted
    }

    /// Checks whether an object is cached without retaining bytes or recording a request.
    pub fn contains_entry(&self, object_key: &ObjectKeyHash) -> bool {
        self.shards[self.shard_index(object_key)]
            .lock()
            .contains_entry(object_key)
    }

    /// Removes the entry for the supplied object key.
    pub fn remove(&self, object_key: &ObjectKeyHash) -> bool {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index].lock().remove(object_key)
    }

    /// Selects this process's in-memory shard for an object.
    ///
    /// The shard assignment is process-local and must never be persisted.
    fn shard_index(&self, object_key: &ObjectKeyHash) -> usize {
        (FxBuildHasher.hash_one(object_key) % self.shards.len() as u64) as usize
    }

    #[cfg(test)]
    fn entry_count(&self) -> u64 {
        self.shards.iter().map(|shard| shard.lock().entry_count() as u64).sum()
    }
}

fn shard_capacity_for(total_capacity: u64, num_shards: usize, shard_index: usize) -> u64 {
    let num_shards = num_shards as u64;
    total_capacity / num_shards + u64::from((shard_index as u64) < total_capacity % num_shards)
}

fn default_shard_count() -> usize {
    available_parallelism()
        .map(|cpus| cpus.get().min(MAX_SHARDS / 4) * 4)
        .unwrap_or(4)
}
