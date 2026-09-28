mod range_trim;
mod shard;
#[cfg(test)]
mod tests;

use std::{fmt, sync::Arc};

use bytes::Bytes;
use feuer_types::{ByteRange, Download, EvictionPolicy, ObjectKey, retention::ObjectAccessHistories};
use parking_lot::Mutex;

use self::shard::{AdmissionProgress, MemoryCacheShard};
use crate::MemoryMetrics;

/// Caps lock partitioning to avoid excessive per-cache metadata.
const MAX_SHARDS: usize = 64;

/// A sharded cache of downloaded object ranges with Foyer-style soft capacity.
///
/// All ranges for one fully compared [`ObjectKey`] share a shard and are kept
/// in an ordered index. Partial overlaps coexist and are treated no differently
/// from disjoint ranges. A lookup succeeds only when one retained range covers
/// the exact request and returns a [`Bytes`] slice containing only those
/// requested bytes. The result can share the retained allocation and never
/// holds an entry guard. Decayed access counts and bounded range-trimming history
/// are recorded separately from downloaded-range population.
///
/// The configured capacity is divided among independently locked shards. Each
/// shard evicts locally before insertion. A payload larger than its shard's
/// target is retained after that shard is emptied, so total usage can exceed the
/// configured capacity. By default, victims are selected shard-locally by recent
/// modeled retrieval value per retained byte. A rotating sample selects one victim;
/// if its observed requests form a useful smaller payload, Feuer trims that victim
/// outside the shard lock. Optional S3-FIFO eviction uses small/main FIFO queues
/// and an exact-range ghost history instead, without range trimming.
pub struct MemoryCache {
    /// Total soft target divided among the shards.
    capacity: u64,
    /// Independently locked partitions selected by complete object identity.
    shards: Box<[Mutex<MemoryCacheShard>]>,
    access_histories: Arc<ObjectAccessHistories>,
}

impl fmt::Debug for MemoryCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryCache")
            .field("capacity", &self.capacity)
            .field("used_bytes", &self.used_bytes())
            .field("shards", &self.shards.len())
            .finish_non_exhaustive()
    }
}

impl MemoryCache {
    /// Creates a cache with a soft payload-byte target and no-op metrics.
    pub fn new(capacity: u64) -> Self {
        Self::with_metrics(capacity, MemoryMetrics::noop())
    }

    /// Creates a cache with a soft payload-byte target and registered metrics.
    pub fn with_metrics(capacity: u64, metrics: Arc<MemoryMetrics>) -> Self {
        Self::with_shard_count(capacity, metrics, default_shard_count())
    }

    /// Creates a cache with an explicit shard count for controlled benchmarks.
    #[cfg(feature = "benchmark")]
    #[doc(hidden)]
    pub fn with_shards_for_benchmark(capacity: u64, shard_count: usize) -> Self {
        Self::with_shard_count(capacity, MemoryMetrics::noop(), shard_count)
    }

    fn with_shard_count(capacity: u64, metrics: Arc<MemoryMetrics>, shard_count: usize) -> Self {
        assert!(shard_count > 0, "memory cache requires at least one shard");
        let access_histories = Arc::new(ObjectAccessHistories::new(shard_count));
        let shards = (0..shard_count)
            .map(|index| {
                Mutex::new(MemoryCacheShard::new(
                    shard_capacity_for(capacity, shard_count, index),
                    metrics.clone(),
                    access_histories.clone(),
                ))
            })
            .collect();
        Self {
            capacity,
            shards,
            access_histories,
        }
    }

    /// Selects the shard-local policy before population. Defaults to cost-aware.
    /// Panics if any shard already contains entries. S3-FIFO does not trim ranges.
    pub fn with_eviction_policy(mut self, policy: EvictionPolicy) -> Self {
        for shard in &mut self.shards {
            shard.get_mut().set_eviction_policy(policy);
        }
        self
    }

    /// Sets the maximum candidates inspected (or S3-FIFO queue steps) per decision. Panics if zero.
    pub fn with_reclaim_sample_size(mut self, sample_size: usize) -> Self {
        assert!(sample_size > 0, "reclaim sample size must be greater than zero");
        for shard in &mut self.shards {
            shard.get_mut().reclaim_sample_size = sample_size;
        }
        self
    }

    /// Shared per-object evidence for attaching a disk tier to this memory cache.
    pub fn access_histories(&self) -> Arc<ObjectAccessHistories> {
        self.access_histories.clone()
    }

    /// Returns the configured soft payload-byte target.
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Returns the sum of payload bytes currently retained by all shards.
    pub fn used_bytes(&self) -> u64 {
        self.shards.iter().map(|shard| shard.lock().used_bytes()).sum()
    }

    /// Looks up one covering range and records its exact request on a hit.
    pub fn get(&self, object_key: &ObjectKey, requested_range: ByteRange) -> Option<Bytes> {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index].lock().get(object_key, requested_range)
    }

    /// Caches one downloaded range without creating an access.
    ///
    /// If an existing entry contains the download, the supplied payload is
    /// discarded. A larger download replaces entries it fully contains, while
    /// partial overlaps coexist.
    pub fn insert(&self, object_key: ObjectKey, download: Download) {
        let (downloaded_range, bytes) = download.into_parts();
        self.admit_download(object_key, downloaded_range, bytes, None);
    }

    /// Caches one callback download and records its successful request atomically.
    ///
    /// Population and access remain distinct policy events, but sharing one
    /// shard lock prevents an intervening admission from losing the callback's
    /// attribution. A new entry starts with zero S3-FIFO reuse credits; the request
    /// still contributes to shared access history. Containment suppression records
    /// an access to the existing entry, including its S3-FIFO reuse counter.
    /// Returns the new shard-local entry identity, or `None` for a redundant download.
    pub fn insert_and_record(
        &self,
        object_key: ObjectKey,
        download: Download,
        requested_range: ByteRange,
    ) -> Option<u64> {
        let (downloaded_range, bytes) = download.into_parts();
        debug_assert!(downloaded_range.contains(requested_range));
        self.admit_download(object_key, downloaded_range, bytes, Some(requested_range))
    }

    /// Runs a short synchronous action only while this exact admission remains cached.
    /// Eviction, replacement and compaction cannot intervene before the action finishes.
    /// The action must not reenter this memory cache or perform I/O.
    pub fn with_current_entry<R>(
        &self,
        object_key: &ObjectKey,
        range: ByteRange,
        entry_id: u64,
        action: impl FnOnce() -> R,
    ) -> Option<R> {
        let shard = self.shards[self.shard_index(object_key)].lock();
        shard.contains_entry(object_key, range, entry_id).then(action)
    }

    fn admit_download(
        &self,
        object_key: ObjectKey,
        downloaded_range: ByteRange,
        bytes: Bytes,
        requested_range: Option<ByteRange>,
    ) -> Option<u64> {
        let shard_index = self.shard_index(&object_key);
        let mut allow_range_trim = true;
        loop {
            let step = self.shards[shard_index].lock().advance_admission(
                &object_key,
                downloaded_range,
                &bytes,
                requested_range,
                allow_range_trim,
            );
            match step {
                AdmissionProgress::Complete(id) => return id,
                AdmissionProgress::Retry => continue,
                AdmissionProgress::Trim(source) => {
                    // Payload copying is deliberately outside the shard lock.
                    // Publication revalidates both the source and its object's
                    // access/structure generation before changing the index.
                    let replacement = source.copy_payload();
                    if !self.shards[shard_index].lock().publish_range_trim(replacement) {
                        // A hot source can invalidate every copy. Fall back to
                        // bounded eviction for this admission so it cannot
                        // starve while concurrent lookups keep succeeding.
                        allow_range_trim = false;
                    }
                }
            }
        }
    }

    /// Records one successful lookup's exact requested range.
    ///
    /// This event is independent of the downloaded range that satisfied the
    /// lookup. Callers must invoke it exactly once for each successful lookup.
    pub fn record_access(&self, object_key: &ObjectKey, requested_range: ByteRange) {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index]
            .lock()
            .record_access(object_key, requested_range);
    }

    /// Removes one entry with exactly the supplied key and range.
    pub fn remove(&self, object_key: &ObjectKey, range: ByteRange) -> bool {
        let shard_index = self.shard_index(object_key);
        self.shards[shard_index].lock().remove(object_key, range)
    }

    /// Selects this process's in-memory shard for an object.
    ///
    /// The hash is not stable across Rust releases and must never be persisted.
    fn shard_index(&self, object_key: &ObjectKey) -> usize {
        self.access_histories.shard_index(object_key)
    }

    #[cfg(test)]
    fn entry_count(&self) -> u64 {
        self.shards.iter().map(|shard| shard.lock().entry_count() as u64).sum()
    }
}

fn shard_capacity_for(total: u64, shards: usize, index: usize) -> u64 {
    let shards = shards as u64;
    total / shards + u64::from((index as u64) < total % shards)
}

fn default_shard_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .saturating_mul(4)
        .clamp(1, MAX_SHARDS)
}
