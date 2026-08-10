mod compaction;
mod evidence;
mod shard;
#[cfg(test)]
mod tests;

use std::{
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
};

use bytes::Bytes;
use feuer_types::{ByteRange, Download, ObjectKey};
use parking_lot::Mutex;

use self::shard::{AdmissionStep, CompactionMode, Shard};
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
/// holds an entry guard. Bounded, ageable exact access evidence is recorded
/// separately from downloaded-range population.
///
/// The configured capacity is divided among independently locked shards. Each
/// shard evicts locally before insertion. A payload larger than its shard's
/// target is retained after that shard is emptied, so total usage can exceed the
/// configured capacity. Victims are selected shard-locally from aged expected
/// retrieval value per retained byte. Broader admissions receive probation measured
/// in successful same-shard accesses, novel containment reuse grants bounded
/// promotion, and bounded ghost state recognizes exact re-admission. Under
/// pressure, rotating samples choose the action with lower estimated value loss
/// per reclaimed byte; retained copies contain only observed requests and are
/// produced outside the shard metadata lock.
pub struct MemoryCache {
    /// Total soft target divided among the shards.
    capacity: u64,
    /// Independently locked partitions selected by complete object identity.
    shards: Box<[Mutex<Shard>]>,
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
        Self::with_shard_count_and_mode(capacity, MemoryMetrics::noop(), shard_count, CompactionMode::ValueAware)
    }

    /// Creates a benchmark cache with compaction disabled.
    #[cfg(feature = "benchmark")]
    #[doc(hidden)]
    pub fn with_compaction_disabled_for_benchmark(capacity: u64, shard_count: usize) -> Self {
        Self::with_shard_count_and_mode(capacity, MemoryMetrics::noop(), shard_count, CompactionMode::Disabled)
    }

    fn with_shard_count(capacity: u64, metrics: Arc<MemoryMetrics>, shard_count: usize) -> Self {
        Self::with_shard_count_and_mode(capacity, metrics, shard_count, CompactionMode::ValueAware)
    }

    fn with_shard_count_and_mode(
        capacity: u64,
        metrics: Arc<MemoryMetrics>,
        shard_count: usize,
        compaction_mode: CompactionMode,
    ) -> Self {
        assert!(shard_count > 0, "memory cache requires at least one shard");
        let shards = (0..shard_count)
            .map(|index| {
                Mutex::new(Shard::new(
                    shard_capacity_for(capacity, shard_count, index),
                    metrics.clone(),
                    compaction_mode,
                ))
            })
            .collect();
        Self { capacity, shards }
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
        self.insert_inner(object_key, downloaded_range, bytes, None);
    }

    /// Caches one callback download and records its successful request atomically.
    ///
    /// Population and access remain distinct policy events, but sharing one
    /// shard lock prevents an intervening admission from losing the callback's
    /// attribution. Containment suppression still records the access.
    pub fn insert_and_record(&self, object_key: ObjectKey, download: Download, requested_range: ByteRange) {
        let (downloaded_range, bytes) = download.into_parts();
        debug_assert!(downloaded_range.contains(requested_range));
        self.insert_inner(object_key, downloaded_range, bytes, Some(requested_range));
    }

    fn insert_inner(
        &self,
        object_key: ObjectKey,
        downloaded_range: ByteRange,
        bytes: Bytes,
        requested_range: Option<ByteRange>,
    ) {
        let shard_index = self.shard_index(&object_key);
        let mut allow_compaction = true;
        loop {
            let step = self.shards[shard_index].lock().admission_step(
                &object_key,
                downloaded_range,
                &bytes,
                requested_range,
                allow_compaction,
            );
            match step {
                AdmissionStep::Complete => return,
                AdmissionStep::Retry => continue,
                AdmissionStep::Compact(work) => {
                    // Payload copying is deliberately outside the shard lock.
                    // Publication revalidates both the source and its object's
                    // access/structure generation before changing the index.
                    let prepared = work.copy_payload();
                    if !self.shards[shard_index].lock().publish_compaction(prepared) {
                        // A hot source can invalidate every copy. Fall back to
                        // bounded eviction for this admission so it cannot
                        // starve while concurrent lookups keep succeeding.
                        allow_compaction = false;
                    }
                }
            }
        }
    }

    /// Records one successful lookup's exact requested range.
    ///
    /// This event is independent of the downloaded extent that satisfied the
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
        let mut hasher = DefaultHasher::new();
        object_key.hash(&mut hasher);
        (hasher.finish() % self.shards.len() as u64) as usize
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
