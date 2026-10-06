mod range_trim;
mod shard;
#[cfg(test)]
mod tests;

use std::{fmt, hash::BuildHasher, sync::Arc};

use bytes::Bytes;
use feuer_types::retention::RECLAIM_SAMPLE_SIZE;
use feuer_types::{ByteRange, Download, ObjectKeyHash, retention::ObjectAccessHistories};
use parking_lot::Mutex;
use rustc_hash::FxBuildHasher;

use self::shard::{InsertOrReclaimResult, MemoryCacheShard};
use crate::{BufferPool, MemoryMetrics};

/// Caps lock partitioning to avoid excessive per-cache metadata.
const MAX_SHARDS: usize = 64;

/// A sharded cache of downloaded object ranges with Foyer-style soft capacity.
///
/// All ranges for one [`ObjectKeyHash`] share a shard and are kept
/// in an ordered index. Partial overlaps coexist and are treated no differently
/// from disjoint ranges. A lookup succeeds only when one entry covers
/// the exact request and returns a [`Bytes`] slice containing only those
/// requested bytes. The result can share the entry's allocation and never
/// holds an entry guard. Request history is supplied separately: cache operations
/// consult it for retention decisions but never record accesses or remove history.
///
/// Cached allocations and idle read buffers share the configured capacity.
/// Each shard evicts locally against its share before insertion; admission also
/// frees idle buffers when needed. An allocation larger than its shard's target
/// is cached after that shard is emptied, so total usage can exceed capacity.
/// Victims are selected shard-locally by recent modeled retrieval value per charged allocation byte.
/// A rotating sample selects one victim;
/// if its observed requests form a useful smaller payload, Feuer trims that victim
/// outside the shard lock.
pub struct MemoryCache {
    /// Independently locked partitions selected by complete object identity.
    shards: Box<[Mutex<MemoryCacheShard>]>,
    access_histories: Arc<ObjectAccessHistories>,
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
    /// Use [`Self::with_access_histories`] to supply request history for access-aware retention.
    pub fn new(capacity: u64) -> Self {
        Self::with_metrics(capacity, MemoryMetrics::noop())
    }

    /// Creates a cache with a shared allocation-byte target, empty history, and registered metrics.
    /// Use [`Self::with_access_histories`] to supply request history for access-aware retention.
    pub fn with_metrics(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self {
        Self::with_access_histories(capacity, metrics, Arc::new(ObjectAccessHistories::new()))
    }

    /// Creates a cache that consults standalone request history for retention decisions.
    /// The caller records requests directly in that history before lookup, independently of cache operations.
    pub fn with_access_histories(
        capacity: u64,
        metrics: Arc<MemoryMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
    ) -> Self {
        Self::with_shard_count(capacity, metrics, default_shard_count(), access_histories)
    }

    /// Creates a cache with an explicit shard count for controlled benchmarks.
    #[cfg(feature = "benchmark")]
    #[doc(hidden)]
    pub fn with_shards_for_benchmark(
        capacity: u64,
        num_shards: usize,
        access_histories: Arc<ObjectAccessHistories>,
    ) -> Self {
        Self::with_shard_count(capacity, MemoryMetrics::noop(), num_shards, access_histories)
    }

    fn with_shard_count(
        capacity: u64,
        metrics: Arc<MemoryMetrics>,
        num_shards: usize,
        access_histories: Arc<ObjectAccessHistories>,
    ) -> Self {
        assert!(num_shards > 0, "memory cache requires at least one shard");
        let buffer_pool = BufferPool::new(capacity, metrics);
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
            access_histories,
            reclaim_sample_size: RECLAIM_SAMPLE_SIZE,
            buffer_pool,
        }
    }

    /// Sets the maximum candidates inspected per decision. Panics if zero.
    pub fn with_reclaim_sample_size(mut self, sample_size: usize) -> Self {
        assert!(sample_size > 0, "reclaim sample size must be greater than zero");
        self.reclaim_sample_size = sample_size;
        self
    }

    /// Returns the shared capacity for cached allocations and idle buffers.
    pub fn capacity(&self) -> u64 {
        self.buffer_pool.capacity
    }

    /// Returns cached allocation capacity plus idle buffer capacity.
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

    /// Looks up one covering range without recording an access.
    pub fn get(&self, object_key: &ObjectKeyHash, requested_range: ByteRange) -> Option<Bytes> {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index].lock().get(object_key, requested_range)
    }

    /// Caches one downloaded range without creating an access, using its allocation charge.
    ///
    /// If an existing entry contains the download, the supplied payload is
    /// discarded. A larger download replaces entries it fully contains, while
    /// partial overlaps coexist.
    /// Returns whether the download was inserted rather than already covered.
    pub fn insert(&self, object_key: ObjectKeyHash, download: Download) -> bool {
        let shard_index = self.shard_index(&object_key);
        let mut allow_range_trim = true;
        let mut evicted_any_entry = false;
        loop {
            let insert_or_reclaim_result =
                self.shards[shard_index]
                    .lock()
                    .try_admit_or_reclaim(&object_key, &download, self, allow_range_trim);
            match insert_or_reclaim_result {
                InsertOrReclaimResult::Complete(inserted) => {
                    if evicted_any_entry {
                        self.buffer_pool.metrics.eviction_triggering_insertions.increase(1);
                    }
                    return inserted;
                }
                InsertOrReclaimResult::Retry { evicted } => evicted_any_entry |= evicted,
                InsertOrReclaimResult::Trim(source) => {
                    // Payload copying is deliberately outside the shard lock.
                    // Publication checks that the exact source range is still cached, not history.
                    let replacement = source.copy_replacement_payloads();
                    let request_count = self.access_histories.request_count();
                    // If the source disappeared, fall back to eviction so admission cannot starve.
                    allow_range_trim = self.shards[shard_index]
                        .lock()
                        .publish_range_trim(replacement, request_count);
                }
            }
        }
    }

    /// Inserts downloaded bytes using the supplied allocation-byte charge, even if they are a smaller slice.
    /// Shared allocations are conservatively charged once per cached entry.
    pub fn insert_with_allocation_charge(
        &self,
        object_key: ObjectKeyHash,
        download: Download,
        allocation_charge: usize,
    ) -> bool {
        self.insert(object_key, download.with_allocation_charge(allocation_charge))
    }

    /// Checks whether the exact key and range are cached, without retaining bytes or recording an access.
    pub fn contains_entry(&self, object_key: &ObjectKeyHash, range: ByteRange) -> bool {
        self.shards[self.shard_index(object_key)]
            .lock()
            .contains_entry(object_key, range)
    }

    /// Removes one entry with exactly the supplied key and range.
    pub fn remove(&self, object_key: &ObjectKeyHash, range: ByteRange) -> bool {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index].lock().remove(object_key, range)
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
    std::thread::available_parallelism().map_or(4, |cpus| cpus.get().min(MAX_SHARDS / 4) * 4)
}
