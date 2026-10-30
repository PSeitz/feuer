use std::{fmt, future::Future, sync::Arc, time::Instant};

use bytes::Bytes;
use feuer_memory::{
    AlignedBuffer, MemoryCache, MemoryMetrics,
    retention::{AvoidedRetrievalCostScorer, RetentionScorer},
};
#[cfg(target_os = "linux")]
use feuer_storage::{DiskCache, DiskCacheOptions, IoMetrics};
use feuer_types::ObjectKeyHash;
use mixtrics::metrics::BoxedRegistry;
use thiserror::Error;

use crate::{
    CacheConfig, DiskCacheError, IoQueues,
    access_trace::AccessTrace,
    metrics::{LookupMetrics, LookupOutcome},
};

mod pending_fetches;
use pending_fetches::{PendingFetch, PendingFetches};

type FetchResult<E> = (LookupOutcome, Result<Bytes, GetOrFetchError<E>>);

/// Configuration and cache tiers shared by cloned handles.
struct TieredMemoryDiskCacheInner {
    config: CacheConfig,
    memory: MemoryCache,
    metrics: LookupMetrics,
    pending_fetches: PendingFetches,
    #[cfg(target_os = "linux")]
    disk: Option<DiskCache>,
}

/// A cloneable handle to a Feuer cache.
///
/// Lookups check memory, then integrity-checked disk, then the per-call callback.
/// Disk writes are best-effort. A full 512-entry queue skips new writes. Zero disk capacity disables disk.
/// Otherwise, opening requires Linux direct I/O and io_uring and waits for metadata recovery.
#[derive(Clone)]
pub struct TieredMemoryDiskCache {
    inner: Arc<TieredMemoryDiskCacheInner>,
    access_trace: Option<AccessTrace>,
}

impl fmt::Debug for TieredMemoryDiskCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TieredMemoryDiskCache")
            .field("memory_capacity", &self.inner.config.memory_capacity())
            .field("disk_capacity", &self.inner.config.disk_capacity())
            .finish_non_exhaustive()
    }
}

impl TieredMemoryDiskCache {
    /// Opens a cache, using only memory when disk capacity is zero.
    /// Memory-only operation works on every platform without a Tokio runtime.
    /// Nonzero disk capacity returns an error on platforms other than Linux.
    /// On Linux, disk caching requires Tokio, io_uring and direct I/O, locks the backing file,
    /// and waits for metadata recovery. Disk failures never fall back to memory.
    ///
    /// `metrics_registry: None` disables metrics. `io_queues: None` creates dedicated queues.
    /// Metrics and queues can be supplied independently. A shared registry aggregates metrics.
    /// Labels contain no object identities or cache names.
    /// Pass clones of the same [`IoQueues`] to caches on the same SSD to share I/O concurrency.
    /// Contents, capacities, eviction, buffer pools, file locks, and background-write queues
    /// remain independent. Zero disk capacity ignores the queues.
    /// Pass `None` for queues on platforms other than Linux.
    pub async fn open(
        config: CacheConfig,
        metrics_registry: Option<&BoxedRegistry>,
        io_queues: Option<IoQueues>,
    ) -> Result<Self, DiskCacheError> {
        let scorer = Arc::new(AvoidedRetrievalCostScorer::default());
        Self::open_with_retention_scorer(config, metrics_registry, io_queues, scorer).await
    }

    /// Opens a cache with an application-provided scorer shared by memory and disk.
    /// See [`Self::open`] for resource configuration and [`RetentionScorer`] for the scoring contract.
    pub async fn open_with_retention_scorer(
        config: CacheConfig,
        metrics_registry: Option<&BoxedRegistry>,
        io_queues: Option<IoQueues>,
        retention_scorer: Arc<dyn RetentionScorer>,
    ) -> Result<Self, DiskCacheError> {
        Self::open_with_retention_scorers(
            config,
            metrics_registry,
            io_queues,
            retention_scorer.clone(),
            retention_scorer,
        )
        .await
    }

    /// Opens a cache with separate memory and disk retention scorers.
    /// Both receive the same caller-demand history, including memory hits.
    /// Scorers cannot inspect the other tier's residency by re-entering the cache.
    /// See [`Self::open`] for resources and [`RetentionScorer`] for the scoring contract.
    pub async fn open_with_retention_scorers(
        config: CacheConfig,
        metrics_registry: Option<&BoxedRegistry>,
        io_queues: Option<IoQueues>,
        memory_scorer: Arc<dyn RetentionScorer>,
        disk_scorer: Arc<dyn RetentionScorer>,
    ) -> Result<Self, DiskCacheError> {
        #[cfg(not(target_os = "linux"))]
        {
            if config.disk_capacity() != 0 {
                return Err(DiskCacheError::UnsupportedPlatform);
            }
            let _ = (io_queues, disk_scorer);
        }
        let noop_metrics_registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        let metrics_registry = metrics_registry.unwrap_or(&noop_metrics_registry);
        let memory = MemoryCache::with_metrics(
            config.memory_capacity(),
            MemoryMetrics::new(metrics_registry),
            config.idle_buffer_pool_percent(),
        )
        .with_access_histories(Arc::new(feuer_historian::ObjectAccessHistories::with_half_life(
            config.access_count_half_life(),
        )))
        .with_reclaim_sample_size(config.eviction().sample_size())
        .with_retention_scorer(memory_scorer);
        #[cfg(target_os = "linux")]
        let disk = if config.disk_capacity() == 0 {
            None
        } else {
            Some(
                DiskCache::open(
                    config.directory(),
                    config.disk_capacity(),
                    DiskCacheOptions {
                        file_name: config.file_name().to_owned(),
                        payload_version: config.payload_version(),
                        io_metrics: IoMetrics::new(metrics_registry),
                        metrics: feuer_storage::DiskMetrics::new(metrics_registry),
                        access_histories: memory.access_histories().clone(),
                        reclaim_sample_size: config.eviction().sample_size(),
                        buffer_pool: memory.buffer_pool(),
                        io_queues,
                        retention_scorer: disk_scorer,
                    },
                )
                .await?,
            )
        };
        Ok(Self {
            inner: Arc::new(TieredMemoryDiskCacheInner {
                config,
                memory,
                metrics: LookupMetrics::new(metrics_registry),
                pending_fetches: PendingFetches::default(),
                #[cfg(target_os = "linux")]
                disk,
            }),
            access_trace: None,
        })
    }

    /// Delivers uncompressed binary access-trace packages as [`Bytes`] for this handle and its clones.
    /// Requires Tokio. Persist packages unchanged. The callback owns retries and error reporting.
    pub fn with_trace<F, Fut>(mut self, deliver_package: F) -> Self
    where
        F: FnMut(Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = std::io::Result<()>> + Send + 'static,
    {
        self.access_trace = Some(AccessTrace::new(deliver_package));
        self
    }

    /// Returns this cache's configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.inner.config
    }

    /// Acquires an aligned buffer from the same pool used by disk reads.
    /// Fill its exposed slice and return it from [`Self::get_or_fetch_buffer`]'s callback
    /// without copying. It returns to the pool after its last owner releases it,
    /// subject to the idle capacity limit.
    pub fn allocate_buffer(&self, length: usize) -> std::io::Result<feuer_memory::AlignedBuffer> {
        self.inner.memory.buffer_pool().allocate(length)
    }

    /// Returns an object from memory or disk, or `None` on a miss.
    /// Does not invoke or wait for source callbacks. Disk hits are promoted to memory.
    pub async fn get_object(&self, object_key: crate::ObjectKey) -> Option<Bytes> {
        let object_key = ObjectKeyHash::from(object_key);
        self.inner.memory.access_histories().record_access(&object_key);
        let event_bytes = match &self.access_trace {
            Some(access_trace) => Some(access_trace.send_request_start(object_key).await),
            None => None,
        };
        let started = Instant::now();
        let (outcome, bytes) = self.lookup_cached(object_key, || false).await;
        let payload_length = bytes.as_ref().map_or(0, |bytes| bytes.len() as u64);
        self.inner.metrics.record(outcome, started.elapsed(), payload_length);
        if let (Some(access_trace), Some(event_bytes)) = (&self.access_trace, event_bytes) {
            access_trace.send_outcome(event_bytes, outcome, payload_length).await;
        }
        bytes
    }

    /// Returns the object from memory, disk, or a shared callback.
    ///
    /// Disk hits promote the object to memory. Concurrent memory misses with
    /// the same key and exact error type share one disk-or-callback operation, including errors.
    /// Callbacks for matching requests must be interchangeable. Each callback runs at most once.
    /// Cancellation of the executing caller allows a waiter to take over with its own callback.
    /// Completed failures are not cached for later requests.
    ///
    /// The callback returns immutable [`Bytes`], including empty objects, without copying.
    /// Memory is charged by payload length. Retained backing capacity can be larger.
    /// Use [`Self::get_or_fetch_buffer`] to charge a pool buffer's capacity instead.
    /// The memory target is soft.
    ///
    /// Every started request records demand exactly once, before checking either tier.
    /// Failures and cancellation still count. Insertion creates no access event.
    /// Callback results are queued to disk immediately, subject to queue capacity.
    /// Tracing can backpressure lookups but never changes their results.
    pub async fn get_or_fetch<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Bytes, E>>,
    {
        self.get_or_fetch_with_disk_fallthrough(object_key, || false, callback)
            .await
    }

    /// Like [`Self::get_or_fetch`], with the per-request disk fallthrough decision described in
    /// [`Self::get_or_fetch_buffer_with_disk_fallthrough`].
    pub async fn get_or_fetch_with_disk_fallthrough<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        // why is this a closure? it it expected that some should_fall_through are expensive to compute, and 
        // making it lazy is an improvement then?
        should_fall_through: impl FnOnce() -> bool,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Bytes, E>>,
    {
        self.lookup_by_hash(ObjectKeyHash::from(object_key), should_fall_through, || async {
            callback().await.map(|bytes| {
                let charge = bytes.len();
                (bytes, charge)
            })
        })
        .await
    }

    /// Like [`Self::get_or_fetch`], but the callback returns an [`AlignedBuffer`].
    /// Memory is charged by buffer capacity. The buffer becomes shared [`Bytes`] without copying.
    /// Concurrent misses share work across both callback types when their error types match.
    pub async fn get_or_fetch_buffer<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<AlignedBuffer, E>>,
    {
        self.get_or_fetch_buffer_with_disk_fallthrough(object_key, || false, callback)
            .await
    }

    /// Like [`Self::get_or_fetch_buffer`], but may fetch from the source instead of a busy disk.
    ///
    /// Caches sharing `IoQueues` share 64 payload read permits. At 64 waiting reads,
    /// every unchecked waiter is scheduled to call `should_fall_through` before I/O,
    /// even if earlier fallthroughs have since reduced the number of waiting reads.
    /// Each request is checked at most once. True invokes `callback`, false waits for disk.
    /// The synchronous decision runs in the caller's task, should be cheap, and can capture
    /// request details and current source load.
    /// Memory hits, buffered disk hits, disk misses, admitted reads, and shared-read waiters
    /// do not call the decision callback. Callers sharing a fetch follow its executing caller's
    /// disk-versus-source decision. After cancellation, a waiter taking over uses its own decision.
    ///
    /// Fallthrough leaves the disk entry intact. Successful source downloads are admitted to
    /// memory. Objects already on disk are not queued for another write.
    /// Source failures are returned without retrying disk.
    /// Each request updates access history before checking either cache tier.
    pub async fn get_or_fetch_buffer_with_disk_fallthrough<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        should_fall_through: impl FnOnce() -> bool,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<AlignedBuffer, E>>,
    {
        self.lookup_by_hash(ObjectKeyHash::from(object_key), should_fall_through, || async {
            callback().await.map(|buffer| {
                let capacity = buffer.capacity();
                (buffer.into_bytes(), capacity)
            })
        })
        .await
    }

    /// Replays a recorded object identity without hashing it a second time.
    /// Otherwise identical to [`Self::get_or_fetch_buffer`].
    #[cfg(feature = "benchmark")]
    #[doc(hidden)]
    pub async fn get_or_fetch_buffer_by_hash<F, Fut, E>(
        &self,
        object_key: ObjectKeyHash,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<AlignedBuffer, E>>,
    {
        self.lookup_by_hash(
            object_key,
            || false,
            || async {
                callback().await.map(|buffer| {
                    let capacity = buffer.capacity();
                    (buffer.into_bytes(), capacity)
                })
            },
        )
        .await
    }

    async fn lookup_by_hash<F, Fut, E>(
        &self,
        object_key: ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(Bytes, usize), E>>,
    {
        self.inner.memory.access_histories().record_access(&object_key);
        let event_bytes = match &self.access_trace {
            Some(access_trace) => Some(access_trace.send_request_start(object_key).await),
            None => None,
        };
        let started = Instant::now();
        let (outcome, result) = if let Some(bytes) = self.inner.memory.get(&object_key) {
            (LookupOutcome::MemoryHit, Ok(bytes))
        } else {
            let fetch = PendingFetch::<E>::join(&self.inner.pending_fetches, object_key);
            let result = fetch
                .result
                .get_or_init(|| self.fetch(object_key, should_fall_through, callback))
                .await
                .clone();
            fetch.remove_from_registry();
            result
        };
        let payload_length = result.as_ref().map_or(0, |bytes| bytes.len() as u64);
        self.inner.metrics.record(outcome, started.elapsed(), payload_length);
        if let (Some(access_trace), Some(event_bytes)) = (&self.access_trace, event_bytes) {
            access_trace.send_outcome(event_bytes, outcome, payload_length).await;
        }
        result
    }

    async fn lookup_cached(
        &self,
        object_key: ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool,
    ) -> (LookupOutcome, Option<Bytes>) {
        if let Some(bytes) = self.inner.memory.get(&object_key) {
            return (LookupOutcome::MemoryHit, Some(bytes));
        }
        #[cfg(not(target_os = "linux"))]
        let _ = should_fall_through;
        #[cfg(target_os = "linux")]
        if let Some(disk) = &self.inner.disk
            // TODO check if disk.get_with_fallthrough uses AlignedBuffer
            && let Some((bytes, buffer_capacity)) = disk.get_with_fallthrough(&object_key, should_fall_through).await
        {
            self.inner.memory.insert(object_key, bytes.clone(), buffer_capacity);
            return (LookupOutcome::DiskHit, Some(bytes));
        }
        (LookupOutcome::Miss, None)
    }

    /// Rechecks memory before fetching from disk or the callback.
    /// Another fetch may have populated memory before this caller joined the registry.
    async fn fetch<F, Fut, E>(
        &self,
        object_key: ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool,
        callback: F,
    ) -> FetchResult<E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(Bytes, usize), E>>,
    {
        let mut fell_through = false;
        // this read from memory (expected by doc comment), AND from the fisk
        let (outcome, bytes) = self
            .lookup_cached(object_key, || {
                fell_through = should_fall_through();
                fell_through
            })
            .await;
        if let Some(bytes) = bytes {
            return (outcome, Ok(bytes));
        }

        let (bytes, allocation_charge) = match callback().await {
            Ok(result) => result,
            Err(error) => {
                return (LookupOutcome::CallbackError, Err(GetOrFetchError::Callback(error)));
            }
        };
        #[cfg(target_os = "linux")]
        if let Some(disk) = &self.inner.disk {
            let metrics = &self.inner.metrics;
            if disk.contains(&object_key) {
                metrics.disk_write_already_covered.increase(1);
                if fell_through {
                    self.inner.memory.insert(object_key, bytes.clone(), allocation_charge);
                }
            } else if self.inner.memory.insert(object_key, bytes.clone(), allocation_charge) {
                disk.enqueue_if_space_available(object_key, bytes.clone());
            } else {
                metrics.disk_write_redundant.increase(1);
            }
            return (LookupOutcome::Callback, Ok(bytes));
        }
        self.inner.memory.insert(object_key, bytes.clone(), allocation_charge);
        (LookupOutcome::Callback, Ok(bytes))
    }
}

/// A failed cache lookup or download operation.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GetOrFetchError<E> {
    /// The callback used by this lookup's shared fetch failed.
    #[error("download callback failed: {0}")]
    Callback(#[source] E),
}

#[cfg(all(test, target_os = "linux"))]
mod disk_fallthrough_tests;

#[cfg(test)]
mod portable_tests;

#[cfg(test)]
mod sharing_tests;

#[cfg(test)]
mod download_buffer_tests;

#[cfg(test)]
mod object_allocation_charge_tests;

#[cfg(all(test, not(target_os = "linux")))]
mod request_history_tests;

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        convert::Infallible,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
    };

    use tokio::sync::Notify;

    use super::*;
    use crate::test_metrics::{registry, value};

    #[tokio::test]
    async fn tier_scorers_receive_entry_sizes_and_shared_history() {
        use crate::ObjectAccessHistories;
        use std::sync::Mutex;

        #[derive(Default)]
        struct ScoredEntries(Mutex<Vec<(ObjectKeyHash, u64, u64, usize)>>);

        impl RetentionScorer for ScoredEntries {
            fn score(
                &self,
                key: &ObjectKeyHash,
                payload_len: u64,
                entry_size_bytes: u64,
                histories: &ObjectAccessHistories,
            ) -> f64 {
                self.0.lock().unwrap().push((
                    *key,
                    payload_len,
                    entry_size_bytes,
                    histories as *const ObjectAccessHistories as usize,
                ));
                1.0
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let scorer = Arc::new(ScoredEntries::default());
        let disk_scorer = Arc::new(ScoredEntries::default());
        let cache = TieredMemoryDiskCache::open_with_retention_scorers(
            CacheConfig::new(directory.path(), 4 << 20, 1).unwrap(),
            None,
            None,
            scorer.clone(),
            disk_scorer.clone(),
        )
        .await
        .unwrap();
        // More entries than memory shards force the memory scorer to run.
        for index in 0..65 {
            cache.inner.memory.insert(
                ObjectKeyHash::from(format!("memory-{index}")),
                Bytes::from_static(b"x"),
                100,
            );
        }
        let disk = cache.inner.disk.as_ref().unwrap();
        for index in 0..3 {
            disk.insert_batch(vec![(
                ObjectKeyHash::from(format!("disk-{index}")),
                Bytes::from(vec![0; 1 << 20]),
            )])
            .await
            .unwrap();
        }
        let history = Arc::as_ptr(cache.inner.memory.access_histories()) as usize;
        let memory_scores = scorer.0.lock().unwrap();
        assert!(!memory_scores.is_empty());
        assert!(
            memory_scores.iter().all(|(_, payload_len, charge, histories)| {
                (*payload_len, *charge, *histories) == (1, 100, history)
            })
        );
        let disk_scores = disk_scorer.0.lock().unwrap();
        assert!(!disk_scores.is_empty());
        assert!(disk_scores.iter().all(|(_, payload_len, charge, histories)| {
            (*payload_len, *charge, *histories) == (1 << 20, 1 << 20, history)
        }));
    }

    #[tokio::test]
    async fn public_registry_observes_memory_disk_callbacks_and_writes() {
        let directory = tempfile::tempdir().unwrap();
        let (registry, backend) = registry();
        let cache = TieredMemoryDiskCache::open(
            CacheConfig::new(directory.path(), 4 << 20, 32).unwrap(),
            Some(&backend),
            None,
        )
        .await
        .unwrap();
        let key = "object".to_owned();
        assert!(Arc::ptr_eq(
            cache.inner.memory.access_histories(),
            &cache.inner.disk.as_ref().unwrap().access_histories()
        ));
        assert!(
            cache
                .get_or_fetch(key.clone(), || async { Err::<Bytes, _>("failed") })
                .await
                .is_err()
        );
        cache
            .get_or_fetch(key.clone(), || async { Ok::<_, Infallible>(Bytes::from_static(b"bc")) })
            .await
            .unwrap();
        wait_for_buffered(&cache, &key).await;
        cache.inner.disk.as_ref().unwrap().flush().await.unwrap();
        cache
            .get_or_fetch(key.clone(), || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert!(cache.inner.memory.remove(&ObjectKeyHash::from(key.as_str())));
        cache
            .get_or_fetch(key.clone(), || async { Err::<Bytes, _>("disk hit must not fetch") })
            .await
            .unwrap();
        for outcome in ["memory_hit", "disk_hit", "callback", "callback_error"] {
            assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), 1.0);
        }
        for outcome in ["disk_hit", "callback"] {
            assert_eq!(
                value(&registry, "feuer_lookup_duration_seconds", &[("outcome", outcome)]),
                1.0
            );
        }
        for source in ["memory", "disk", "callback"] {
            assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 2.0);
        }
        assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 1.0);
        // Metadata is written periodically, independently of payload publication.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "write"), ("outcome", "success")],
            ) < 4.0
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "write"), ("outcome", "success")]
            ),
            4.0 // Global header, one payload chunk, one metadata page, and one next-chunk link.
        );
        assert_eq!(
            value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
            1.0
        );
        assert_eq!(
            cache.inner.memory.access_histories().request_count(),
            4,
            "every request is recorded, including errors"
        );
        // Closing the cache also releases the detached disk-write worker's gauges.
        drop(cache);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while value(&registry, "feuer_disk_chunks", &[("state", "free")]) != 0.0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 0.0);
    }

    async fn cache(memory_capacity: u64) -> (tempfile::TempDir, TieredMemoryDiskCache) {
        let directory = tempfile::tempdir().unwrap();
        let cache = TieredMemoryDiskCache::open(
            CacheConfig::new(directory.path(), 4 << 20, memory_capacity).unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
        (directory, cache)
    }

    #[tokio::test]
    async fn configured_idle_buffer_pool_percent_controls_retention() {
        let size = 32 * 1024;
        for (percent, expected) in [(0, 0), (25, size as u64)] {
            let config = CacheConfig::new("unused", 0, 4 * size as u64)
                .unwrap()
                .with_idle_buffer_pool_percent(percent)
                .unwrap();
            let cache = TieredMemoryDiskCache::open(config, None, None).await.unwrap();
            let buffers: Vec<_> = (0..2).map(|_| cache.allocate_buffer(size).unwrap()).collect();
            drop(buffers);
            assert_eq!(cache.inner.memory.buffer_pool().idle_bytes(), expected);
        }
    }

    async fn wait_for_buffered(cache: &TieredMemoryDiskCache, key: &str) {
        let disk = cache.inner.disk.as_ref().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !disk.contains(&ObjectKeyHash::from(key)) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("disk write was not buffered");
    }

    #[tokio::test]
    async fn shared_io_queues_preserve_independent_whole_object_caches_in_one_directory() {
        let directory = tempfile::tempdir().unwrap();
        let (metrics_registry, backend) = registry();
        let queues = IoQueues::new().unwrap();
        let first = TieredMemoryDiskCache::open(
            CacheConfig::new(directory.path(), 4 << 20, 1 << 20)
                .unwrap()
                .with_access_count_half_life(2)
                .unwrap(),
            Some(&backend),
            Some(queues.clone()),
        )
        .await
        .unwrap();
        // Queue sharing does not require the caches to use the same metrics configuration.
        let second = TieredMemoryDiskCache::open(
            CacheConfig::new(directory.path(), 4 << 20, 1 << 20)
                .unwrap()
                .with_file_name("objects")
                .unwrap()
                .with_access_count_half_life(4)
                .unwrap(),
            None,
            Some(queues),
        )
        .await
        .unwrap();
        assert!(
            TieredMemoryDiskCache::open(second.config().clone(), None, None)
                .await
                .is_err()
        );
        assert!(!Arc::ptr_eq(
            &first.inner.memory.buffer_pool(),
            &second.inner.memory.buffer_pool()
        ));
        assert!(!Arc::ptr_eq(
            first.inner.memory.access_histories(),
            second.inner.memory.access_histories()
        ));
        for (cache, half_life) in [(&first, 2), (&second, 4)] {
            assert_eq!(cache.inner.memory.access_histories().half_life(), half_life);
            assert_eq!(
                cache.inner.disk.as_ref().unwrap().access_histories().half_life(),
                half_life
            );
        }
        let key = "object-v1";
        let hash = ObjectKeyHash::from(key);
        let first_bytes = Bytes::from_static(b"first object");
        let second_bytes = Bytes::from_static(b"second object");
        let (first_result, second_result) = tokio::join!(
            first.get_or_fetch(key.to_owned(), || async { Ok::<_, Infallible>(first_bytes.clone()) }),
            second.get_or_fetch(key.to_owned(), || async { Ok::<_, Infallible>(second_bytes.clone()) }),
        );
        assert_eq!(first_result.unwrap(), first_bytes);
        assert_eq!(second_result.unwrap(), second_bytes);
        assert_eq!(
            value(&metrics_registry, "feuer_lookup_total", &[("outcome", "callback")]),
            1.0
        );
        wait_for_buffered(&first, key).await;
        wait_for_buffered(&second, key).await;
        let (first_flush, second_flush) = tokio::join!(
            first.inner.disk.as_ref().unwrap().flush(),
            second.inner.disk.as_ref().unwrap().flush(),
        );
        first_flush.unwrap();
        second_flush.unwrap();
        assert!(first.inner.memory.remove(&hash));
        assert!(second.inner.memory.remove(&hash));
        let (first_result, second_result) = tokio::join!(
            first.get_or_fetch(key.to_owned(), || async { Err::<Bytes, _>("must hit first disk") }),
            second.get_or_fetch(key.to_owned(), || async { Err::<Bytes, _>("must hit second disk") }),
        );
        assert_eq!(first_result.unwrap(), first_bytes);
        assert_eq!(second_result.unwrap(), second_bytes);
        assert_eq!(
            value(&metrics_registry, "feuer_lookup_total", &[("outcome", "disk_hit")]),
            1.0
        );
        drop(first);
        assert!(second.inner.memory.remove(&hash));
        assert_eq!(
            second
                .get_or_fetch(key.to_owned(), || async { Err::<Bytes, _>("must hit surviving disk") })
                .await
                .unwrap(),
            second_bytes,
        );
    }

    #[tokio::test]
    async fn whole_object_buffer_and_disk_hits_promote_all_bytes() {
        let (_directory, cache) = cache(32).await;
        let key = "whole-object".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let payload = Bytes::from_static(b"abcdef");
        cache
            .get_or_fetch(key.clone(), || async { Ok::<_, &str>(payload.clone()) })
            .await
            .unwrap();
        wait_for_buffered(&cache, &key).await;
        for flush in [false, true] {
            if flush {
                cache.inner.disk.as_ref().unwrap().flush().await.unwrap();
            }
            for lookup_only in [false, true] {
                assert!(cache.inner.memory.remove(&hash));
                let bytes = if lookup_only {
                    cache.get_object(key.clone()).await.unwrap()
                } else {
                    cache
                        .get_or_fetch(key.clone(), || async { Err::<Bytes, _>("must hit disk") })
                        .await
                        .unwrap()
                };
                assert_eq!(bytes, payload);
                assert_eq!(cache.inner.memory.get(&hash).unwrap().as_ptr(), bytes.as_ptr());
            }
        }
    }

    #[tokio::test]
    async fn disk_promotion_charges_buffer_capacity_and_preserves_returned_bytes_after_removal() {
        let (_directory, cache) = cache(32).await;
        let key = "large-entry".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let disk = cache.inner.disk.as_ref().unwrap();
        disk.insert_batch(vec![(hash, Bytes::from(vec![0x77; 40 * 1024]))])
            .await
            .unwrap();
        let bytes = cache
            .get_or_fetch(key.clone(), || async { Err::<Bytes, _>("disk hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(&bytes[..], &[0x77; 40 * 1024]);
        assert_eq!(cache.inner.memory.used_bytes(), 256 * 1024);
        assert_eq!(cache.inner.memory.get(&hash).unwrap().as_ptr(), bytes.as_ptr());
        assert_eq!(cache.inner.memory.buffer_pool().idle_bytes(), 0);
        assert!(cache.inner.memory.remove(&hash));
        assert_eq!(cache.inner.memory.used_bytes(), 0);
        assert_eq!(&bytes[..], &[0x77; 40 * 1024]);
    }

    #[tokio::test]
    async fn concurrent_disk_promotions_share_the_object_and_charge_once() {
        let (_directory, cache) = cache(8 << 20).await;
        let key = "shared-entry".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let payload = Bytes::from(vec![0x77; 40 * 1024]);
        let disk = cache.inner.disk.as_ref().unwrap();
        disk.insert_batch(vec![(hash, payload.clone())]).await.unwrap();
        disk.flush().await.unwrap();
        let (first, second) = tokio::join!(
            cache.get_or_fetch(key.clone(), || async { Err::<Bytes, _>("disk hit must not fetch") }),
            cache.get_or_fetch(key, || async { Err::<Bytes, _>("disk hit must not fetch") }),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first, payload);
        assert_eq!(second, payload);
        assert_eq!(first.as_ptr(), second.as_ptr());
        assert_eq!(cache.inner.memory.get(&hash).unwrap().as_ptr(), first.as_ptr());
        assert_eq!(
            cache.inner.memory.used_bytes(),
            cache.inner.memory.buffer_pool().idle_bytes() + 256 * 1024
        );
    }

    #[tokio::test]
    async fn callback_with_a_racing_disk_write_is_not_inserted_again() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let history = cache.inner.memory.access_histories();
        let bytes = cache
            .get_or_fetch(key.clone(), || async {
                // Simulate another disk write finishing while this callback is pending.
                let disk = cache.inner.disk.as_ref().unwrap();
                disk.insert_batch(vec![(ObjectKeyHash::from(key.as_str()), Bytes::from_static(b"cd"))])
                    .await
                    .unwrap();
                Ok::<_, Infallible>(Bytes::from_static(b"cd"))
            })
            .await
            .unwrap();
        assert_eq!(bytes, Bytes::from_static(b"cd"));
        assert_eq!(cache.inner.memory.used_bytes(), 0);
        assert_eq!(history.request_count(), 1);
    }

    #[tokio::test]
    async fn callback_cancellation_keeps_the_recorded_request_without_inserting() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("canceled");
        let history = cache.inner.memory.access_histories();
        let entered = Arc::new(Notify::new());
        let task = {
            let cache = cache.clone();
            let key = key.clone();
            let entered = entered.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch(key, || async move {
                        entered.notify_one();
                        std::future::pending::<Result<Bytes, Infallible>>().await
                    })
                    .await
            })
        };
        entered.notified().await;
        assert_eq!(history.request_count(), 1, "recorded before the callback completes");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(cache.inner.memory.get(&ObjectKeyHash::from(key.as_str())).is_none());
        let disk = cache.inner.disk.as_ref().unwrap();
        assert!(!disk.contains(&ObjectKeyHash::from(key.as_str())));
        assert_eq!(history.request_count(), 1);
    }

    #[tokio::test]
    async fn opening_validates_disk_capacity_and_exclusive_file_ownership() {
        let (directory, cache) = cache(32).await;
        assert!(
            TieredMemoryDiskCache::open(cache.config().clone(), None, None)
                .await
                .is_err()
        );
        let invalid = CacheConfig::new(directory.path(), 1024, 32).unwrap();
        assert!(matches!(
            TieredMemoryDiskCache::open(invalid, None, None).await,
            Err(DiskCacheError::InvalidCapacity)
        ));
    }

    #[test]
    fn cache_handle_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<TieredMemoryDiskCache>();
    }

    #[tokio::test]
    async fn callback_and_memory_hit_share_the_object_without_creating_disk_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unused");
        let cache = TieredMemoryDiskCache::open(CacheConfig::new(&path, 0, 32).unwrap(), None, None)
            .await
            .unwrap();
        let key = String::from("object");
        let payload = Bytes::from_static(b"abcdefghij");
        let result = cache
            .get_or_fetch(key.clone(), || async { Ok::<_, Infallible>(payload.clone()) })
            .await
            .unwrap();
        assert_eq!(result, payload);
        assert_eq!(result.as_ptr(), payload.as_ptr());

        let result = cache
            .get_or_fetch(key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(result, payload);
        assert_eq!(result.as_ptr(), payload.as_ptr());
        assert!(cache.inner.disk.is_none());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn callback_errors_are_returned_without_retry_or_insertion() {
        let (_directory, cache) = cache(8).await;
        let key = String::from("object");
        let callback_count = Arc::new(AtomicU64::new(0));

        for expected_count in 1..=2 {
            let invocation_count = callback_count.clone();
            let error = cache
                .get_or_fetch(key.clone(), move || async move {
                    invocation_count.fetch_add(1, Ordering::Relaxed);
                    Err::<Bytes, _>("source unavailable")
                })
                .await
                .unwrap_err();

            assert_eq!(error, GetOrFetchError::Callback("source unavailable"));
            assert_eq!(callback_count.load(Ordering::Relaxed), expected_count);
        }
    }

    #[tokio::test]
    async fn retains_oversized_callback_results() {
        let (_directory, cache) = cache(4).await;
        let key = String::from("object");
        let result = cache
            .get_or_fetch(key.clone(), || async {
                Ok::<_, Infallible>(Bytes::from_static(b"abcde"))
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"abcde"));
        assert_eq!(cache.inner.memory.used_bytes(), 5);
        let hit = cache
            .get_or_fetch(key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(hit, result);
        assert_eq!(hit.as_ptr(), result.as_ptr());
    }

    #[tokio::test]
    async fn a_racing_memory_insertion_is_not_replaced_by_the_callback() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let callback_entered = Arc::new(Notify::new());
        let release_callback = Arc::new(Notify::new());

        let pending = {
            let cache = cache.clone();
            let key = key.clone();
            let callback_entered = callback_entered.clone();
            let release_callback = release_callback.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch(key, move || async move {
                        callback_entered.notify_one();
                        release_callback.notified().await;
                        Ok::<_, Infallible>(Bytes::from(vec![b'x'; 6]))
                    })
                    .await
                    .unwrap()
            })
        };

        callback_entered.notified().await;
        let payload = Bytes::from_static(b"xxxxxx");
        cache
            .inner
            .memory
            .insert(ObjectKeyHash::from(key.as_str()), payload.clone(), payload.len());
        release_callback.notify_one();

        let returned = pending.await.unwrap();
        assert_eq!(returned, payload);
        let cached = cache
            .get_or_fetch(key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(cached, payload);
        assert_eq!(cached.as_ptr(), payload.as_ptr());
        assert_ne!(returned.as_ptr(), cached.as_ptr());
        assert_eq!(cache.inner.memory.used_bytes(), payload.len() as u64);
    }
}
