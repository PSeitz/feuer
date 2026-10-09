use std::{fmt, future::Future, sync::Arc, time::Instant};

use bytes::Bytes;
use feuer_memory::MemoryCache;
#[cfg(target_os = "linux")]
use feuer_memory::{
    MemoryMetrics,
    retention::{RetentionScorer, RetrievalCostScorer},
};
#[cfg(target_os = "linux")]
use feuer_storage::{DiskCache, DiskCacheError, DiskCacheOptions, IoMetrics, IoQueues};
use feuer_types::{ByteRange, Download, ObjectKeyHash};
#[cfg(target_os = "linux")]
use mixtrics::metrics::BoxedRegistry;
use thiserror::Error;

use crate::{
    CacheConfig,
    access_trace::Trace,
    metrics::{LookupMetrics, LookupOutcome},
};

/// Configuration and cache tiers shared by cloned handles.
struct TieredMemoryDiskCacheInner {
    config: CacheConfig,
    memory: MemoryCache,
    metrics: LookupMetrics,
    #[cfg(target_os = "linux")]
    disk: Option<DiskCache>,
}

/// A cloneable handle to one Feuer cache.
///
/// Lookups check memory, then integrity-checked disk, then the per-call callback.
/// Disk writes are best-effort; a full 512-entry queue skips new writes. Zero disk capacity disables disk;
/// otherwise opening requires Linux direct I/O and io_uring, and waits for metadata recovery.
#[derive(Clone)]
pub struct TieredMemoryDiskCache {
    inner: Arc<TieredMemoryDiskCacheInner>,
    trace: Option<Trace>,
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
    /// Otherwise requires Tokio, io_uring and direct I/O, locks the backing file,
    /// and waits for metadata recovery. Disk failures never fall back to memory.
    ///
    /// `metrics_registry: None` disables metrics; `io_queues: None` creates dedicated queues.
    /// Metrics and queues can be supplied independently. A shared registry aggregates metrics;
    /// labels contain no object identities or cache names.
    /// Pass clones of the same [`IoQueues`] to caches on the same SSD to share I/O concurrency.
    /// Contents, capacities, eviction, buffer pools, file locks, and background-write queues
    /// remain independent. Zero disk capacity ignores the queues.
    #[cfg(target_os = "linux")]
    pub async fn open(
        config: CacheConfig,
        metrics_registry: Option<&BoxedRegistry>,
        io_queues: Option<IoQueues>,
    ) -> Result<Self, DiskCacheError> {
        let scorer = Arc::new(RetrievalCostScorer {
            fixed_retrieval_equivalent_bytes: config.fixed_retrieval_equivalent_bytes(),
        });
        Self::open_with_retention_scorer(config, metrics_registry, io_queues, scorer).await
    }

    /// Opens a cache with one application-provided scorer shared by memory and disk.
    /// See [`Self::open`] for resource configuration and [`RetentionScorer`] for the scoring contract.
    #[cfg(target_os = "linux")]
    pub async fn open_with_retention_scorer(
        config: CacheConfig,
        metrics_registry: Option<&BoxedRegistry>,
        io_queues: Option<IoQueues>,
        retention_scorer: Arc<dyn RetentionScorer>,
    ) -> Result<Self, DiskCacheError> {
        let noop_metrics_registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        let metrics_registry = metrics_registry.unwrap_or(&noop_metrics_registry);
        let memory = MemoryCache::with_metrics(
            config.memory_capacity(),
            MemoryMetrics::new(metrics_registry),
            config.idle_buffer_pool_percent(),
        )
        .with_reclaim_sample_size(config.reclaim_sample_size())
        .with_retention_scorer(retention_scorer.clone());
        let disk = if config.disk_capacity() == 0 {
            None
        } else {
            Some(
                DiskCache::open(
                    config.directory(),
                    config.disk_capacity(),
                    DiskCacheOptions {
                        file_name: config.file_name().to_owned(),
                        io_metrics: IoMetrics::new(metrics_registry),
                        metrics: feuer_storage::DiskMetrics::new(metrics_registry),
                        access_histories: memory.access_histories().clone(),
                        reclaim_sample_size: config.reclaim_sample_size(),
                        buffer_pool: memory.buffer_pool(),
                        io_queues,
                        retention_scorer,
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
                disk,
            }),
            trace: None,
        })
    }

    /// Delivers complete, uncompressed binary trace packages as [`Bytes`] for this handle and its clones.
    /// Requires Tokio. Persist bytes unchanged; the receiver owns retries and error reporting.
    pub fn with_trace<F, Fut>(mut self, callback: F) -> Self
    where
        F: FnMut(Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = std::io::Result<()>> + Send + 'static,
    {
        self.trace = Some(Trace::new(callback));
        self
    }

    /// Returns this cache's configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.inner.config
    }

    /// Acquires an aligned buffer from the same pool used by disk reads.
    /// Fill its exposed slice, then call [`feuer_memory::AlignedBuffer::into_download`]
    /// to hand it to [`Self::get_or_fetch`] without copying. It returns to the pool
    /// after its last owner releases it, subject to the idle capacity limit.
    pub fn allocate_buffer(&self, length: usize) -> std::io::Result<feuer_memory::AlignedBuffer> {
        self.inner.memory.buffer_pool().allocate(length)
    }

    /// Returns the complete object from memory, disk, or this call's callback.
    ///
    /// The callback returns the entire immutable object as [`Bytes`], including empty objects.
    /// Do not mix this API and [`Self::get_or_fetch`] for the same key,
    /// including after reopening the disk cache. The stored length is treated as the object length.
    pub async fn get_or_fetch_object<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Bytes, E>>,
    {
        self.lookup(object_key, None, || async {
            callback()
                .await
                .map(|bytes| Download::new(0, bytes).expect("an object length fits in u64"))
        })
        .await
    }

    /// Returns the requested bytes from memory, disk, or this call's callback.
    ///
    /// Do not mix this API and [`Self::get_or_fetch_object`] for the same key,
    /// including after reopening the disk cache.
    ///
    /// A covering memory range is checked first, then disk. A disk hit promotes
    /// only the requested bytes to memory, copying if the destination allocation
    /// saves at least 25% of backing capacity. Otherwise, or if allocation fails,
    /// it retains the original slice. On a miss in both tiers, `callback` is
    /// invoked exactly once by this call; Feuer performs no leader election,
    /// waiter coordination, or source retry. A successful callback must return
    /// one valid [`Download`] covering `requested_range`. The memory target is
    /// soft, so callback results are not rejected solely for exceeding it.
    ///
    /// Every started request records `requested_range` exactly once, before checking either tier.
    /// Failures and cancellation after the request starts still count. Downloaded-range
    /// insertion is separate and creates no access.
    /// The returned [`Bytes`] contains exactly the request and may share the
    /// download's allocation.
    ///
    /// Tracing can backpressure lookups but never changes their results.
    pub async fn get_or_fetch<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        requested_range: ByteRange,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Download, E>>,
    {
        self.lookup(object_key, Some(requested_range), callback).await
    }

    async fn lookup<F, Fut, E>(
        &self,
        object_key: crate::ObjectKey,
        requested: Option<ByteRange>,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Download, E>>,
    {
        let object_key = ObjectKeyHash::from(object_key);
        self.inner
            .memory
            .access_histories()
            .record_access(&object_key, requested);
        let access = match &self.trace {
            Some(trace) => {
                let range = requested.unwrap_or_else(|| ByteRange::new(0, u64::MAX).unwrap());
                Some(trace.send_request_start(object_key, range).await)
            }
            None => None,
        };
        let started = Instant::now();
        let (outcome, downloaded_range, result) = self.fetch(object_key, requested, callback).await;
        let bytes = result.as_ref().map_or(0, |bytes| bytes.len() as u64);
        self.inner.metrics.record(outcome, started.elapsed(), bytes);
        if let Some(access) = access {
            access.send_outcome(outcome, downloaded_range).await;
        }
        result
    }

    /// Returns the lookup outcome, the downloaded range of a callback result, and the requested bytes.
    async fn fetch<F, Fut, E>(
        &self,
        object_key: ObjectKeyHash,
        requested: Option<ByteRange>,
        callback: F,
    ) -> (LookupOutcome, Option<ByteRange>, Result<Bytes, GetOrFetchError<E>>)
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Download, E>>,
    {
        if let Some(bytes) = self.inner.memory.get(&object_key, requested) {
            return (LookupOutcome::MemoryHit, None, Ok(bytes));
        }
        #[cfg(target_os = "linux")]
        if let Some(disk) = &self.inner.disk
            && let Some((bytes, buffer_capacity)) = disk.fetch_from_disk(&object_key, requested).await
        {
            let download = Download::new(requested.map_or(0, ByteRange::start), bytes.clone())
                .expect("disk result covers the request");
            self.inner
                .memory
                .insert(object_key, download.with_allocation_charge(buffer_capacity));
            return (LookupOutcome::DiskHit, None, Ok(bytes));
        }

        let download = match callback().await {
            Ok(download) => download,
            Err(error) => {
                return (
                    LookupOutcome::CallbackError,
                    None,
                    Err(GetOrFetchError::Callback(error)),
                );
            }
        };
        let downloaded_range = download.downloaded_range();
        let requested_range = requested.unwrap_or(downloaded_range);
        if !downloaded_range.contains(requested_range) {
            let error = GetOrFetchError::DownloadDoesNotCover {
                requested_range,
                downloaded_range,
            };
            return (LookupOutcome::InvalidDownload, Some(downloaded_range), Err(error));
        }

        let requested_bytes = download.bytes_in_range(requested_range);
        #[cfg(target_os = "linux")]
        if let Some(disk) = &self.inner.disk {
            let metrics = &self.inner.metrics;
            if disk.covers_range(&object_key, downloaded_range) {
                metrics.disk_write_already_covered.increase(1);
            } else if self.inner.memory.insert(object_key, download.clone()) {
                disk.enqueue_if_space_available(object_key, download);
            } else {
                metrics.disk_write_redundant.increase(1);
            }
            return (LookupOutcome::Callback, Some(downloaded_range), Ok(requested_bytes));
        }
        self.inner.memory.insert(object_key, download);
        (LookupOutcome::Callback, Some(downloaded_range), Ok(requested_bytes))
    }
}

/// A failed cache lookup or download operation.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GetOrFetchError<E> {
    /// The callback supplied to this lookup failed.
    #[error("download callback failed: {0}")]
    Callback(#[source] E),
    /// The returned download did not contain this call's exact request.
    #[error("downloaded range {downloaded_range:?} does not cover requested range {requested_range:?}")]
    DownloadDoesNotCover {
        /// The exact range requested by the lookup.
        requested_range: ByteRange,
        /// The exact range returned by the callback.
        downloaded_range: ByteRange,
    },
}

#[cfg(test)]
mod download_buffer_tests;

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

    use tokio::sync::{Barrier, Notify};

    use super::*;
    use crate::test_metrics::{registry, value};

    #[tokio::test]
    async fn custom_scorer_and_history_are_shared_by_both_tiers() {
        use crate::ObjectAccessHistories;
        use std::sync::Mutex;

        #[derive(Default)]
        struct ScoredEntries(Mutex<Vec<(ObjectKeyHash, ByteRange, u64, usize)>>);

        impl RetentionScorer for ScoredEntries {
            fn score(
                &self,
                key: &ObjectKeyHash,
                cached_range: ByteRange,
                charged_bytes: u64,
                histories: &ObjectAccessHistories,
            ) -> f64 {
                self.0.lock().unwrap().push((
                    *key,
                    cached_range,
                    charged_bytes,
                    histories as *const ObjectAccessHistories as usize,
                ));
                if cached_range.start() == 0 { 2.0 } else { 1.0 }
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let scorer = Arc::new(ScoredEntries::default());
        let cache = TieredMemoryDiskCache::open_with_retention_scorer(
            CacheConfig::new(directory.path(), 3 << 20, 1).unwrap(),
            None,
            None,
            scorer.clone(),
        )
        .await
        .unwrap();
        let key = ObjectKeyHash::from("computed");
        let disk = cache.inner.disk.as_ref().unwrap();
        for start in [0, 10, 20] {
            let download = Download::new(start, Bytes::from_static(b"x"))
                .unwrap()
                .with_allocation_charge(100);
            cache.inner.memory.insert(key, download.clone());
            disk.insert_batch(vec![(key, download)]).await.unwrap();
        }
        let history = Arc::as_ptr(cache.inner.memory.access_histories()) as usize;
        assert_eq!(
            *scorer.0.lock().unwrap(),
            [
                (key, range(0, 1), 100, history),
                (key, range(10, 11), 100, history),
                (key, range(0, 1), 1, history),
                (key, range(10, 11), 1, history),
            ]
        );
        assert!(disk.get(&key, range(0, 1)).await.is_some());
        assert!(disk.get(&key, range(10, 11)).await.is_none());
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
                .get_or_fetch(key.clone(), range(0, 2), || async { Err::<Download, _>("failed") })
                .await
                .is_err()
        );
        assert!(
            cache
                .get_or_fetch(key.clone(), range(0, 2), || async {
                    Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"x")).unwrap())
                })
                .await
                .is_err()
        );
        cache
            .get_or_fetch(key.clone(), range(1, 3), || async {
                Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"abcd")).unwrap())
            })
            .await
            .unwrap();
        wait_for_buffered(&cache, &key, range(0, 4)).await;
        cache.inner.disk.as_ref().unwrap().flush().await.unwrap();
        cache
            .get_or_fetch(key.clone(), range(0, 2), || async {
                Err::<Download, _>("memory hit must not fetch")
            })
            .await
            .unwrap();
        cache.inner.memory.insert(
            ObjectKeyHash::from(key.as_str()),
            Download::new(100, Bytes::from(vec![0; 64])).unwrap(),
        );
        cache
            .get_or_fetch(key.clone(), range(1, 3), || async {
                Err::<Download, _>("disk hit must not fetch")
            })
            .await
            .unwrap();
        for outcome in [
            "memory_hit",
            "disk_hit",
            "callback",
            "callback_error",
            "invalid_download",
        ] {
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
            ) < 3.0
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
            3.0 // One payload chunk, one metadata record page, and one next-chunk link.
        );
        assert_eq!(
            value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
            1.0
        );
        assert_eq!(
            cache.inner.memory.access_histories().request_count(),
            5,
            "every request is recorded, including errors and invalid downloads"
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

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
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

    async fn wait_for_buffered(cache: &TieredMemoryDiskCache, key: &str, range: ByteRange) {
        let disk = cache.inner.disk.as_ref().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !disk.covers_range(&ObjectKeyHash::from(key), range) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("disk write was not buffered");
    }

    #[tokio::test]
    async fn buffered_hit_after_memory_pressure_promotes_only_request_and_records_once() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let history = cache.inner.memory.access_histories();
        let source = Download::new(10, Bytes::from_static(b"abcdefghij")).unwrap();
        let result = cache
            .get_or_fetch(key.clone(), range(13, 17), || async {
                Ok::<_, Infallible>(source.clone())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"defg"));
        wait_for_buffered(&cache, &key, source.downloaded_range()).await;
        assert_eq!(history.request_count(), 1);

        // The same key selects the same memory shard; an oversized disjoint range
        // forces the original download out without contributing an access.
        cache.inner.memory.insert(
            ObjectKeyHash::from(key.as_str()),
            Download::new(100, Bytes::from(vec![0; 64])).unwrap(),
        );
        assert!(
            cache
                .inner
                .memory
                .get(&ObjectKeyHash::from(key.as_str()), range(13, 17))
                .is_none()
        );
        for accesses in [2, 3] {
            let hit = cache
                .get_or_fetch(key.clone(), range(13, 17), || async {
                    Err::<Download, _>("callback must not run")
                })
                .await
                .unwrap();
            assert_eq!(hit, result);
            assert_eq!(history.request_count(), accesses);
        }
        assert_eq!(cache.inner.memory.used_bytes(), 4);
    }

    #[tokio::test]
    async fn shared_io_queues_preserve_independent_whole_object_caches_in_one_directory() {
        let directory = tempfile::tempdir().unwrap();
        let (metrics_registry, backend) = registry();
        let queues = IoQueues::new().unwrap();
        let first = TieredMemoryDiskCache::open(
            CacheConfig::new(directory.path(), 4 << 20, 1 << 20).unwrap(),
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
        let key = "object-v1";
        let hash = ObjectKeyHash::from(key);
        let first_bytes = Bytes::from_static(b"first object");
        let second_bytes = Bytes::from_static(b"second object");
        let (first_result, second_result) = tokio::join!(
            first.get_or_fetch_object(key.to_owned(), || async { Ok::<_, Infallible>(first_bytes.clone()) }),
            second.get_or_fetch_object(key.to_owned(), || async { Ok::<_, Infallible>(second_bytes.clone()) }),
        );
        assert_eq!(first_result.unwrap(), first_bytes);
        assert_eq!(second_result.unwrap(), second_bytes);
        assert_eq!(
            value(&metrics_registry, "feuer_lookup_total", &[("outcome", "callback")]),
            1.0
        );
        wait_for_buffered(&first, key, range(0, first_bytes.len() as u64)).await;
        wait_for_buffered(&second, key, range(0, second_bytes.len() as u64)).await;
        let (first_flush, second_flush) = tokio::join!(
            first.inner.disk.as_ref().unwrap().flush(),
            second.inner.disk.as_ref().unwrap().flush(),
        );
        first_flush.unwrap();
        second_flush.unwrap();
        assert!(first.inner.memory.remove(&hash, range(0, first_bytes.len() as u64)));
        assert!(second.inner.memory.remove(&hash, range(0, second_bytes.len() as u64)));
        let (first_result, second_result) = tokio::join!(
            first.get_or_fetch_object(key.to_owned(), || async { Err::<Bytes, _>("must hit first disk") }),
            second.get_or_fetch_object(key.to_owned(), || async { Err::<Bytes, _>("must hit second disk") }),
        );
        assert_eq!(first_result.unwrap(), first_bytes);
        assert_eq!(second_result.unwrap(), second_bytes);
        assert_eq!(
            value(&metrics_registry, "feuer_lookup_total", &[("outcome", "disk_hit")]),
            1.0
        );
        drop(first);
        assert!(second.inner.memory.remove(&hash, range(0, second_bytes.len() as u64)));
        assert_eq!(
            second
                .get_or_fetch_object(key.to_owned(), || async { Err::<Bytes, _>("must hit surviving disk") })
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
            .get_or_fetch_object(key.clone(), || async { Ok::<_, &str>(payload.clone()) })
            .await
            .unwrap();
        wait_for_buffered(&cache, &key, range(0, 6)).await;
        for flush in [false, true] {
            assert!(cache.inner.memory.remove(&hash, range(0, 6)));
            if flush {
                cache.inner.disk.as_ref().unwrap().flush().await.unwrap();
            }
            let bytes = cache
                .get_or_fetch_object(key.clone(), || async { Err::<Bytes, _>("must hit disk") })
                .await
                .unwrap();
            assert_eq!(bytes, payload);
            assert_eq!(cache.inner.memory.get(&hash, None).unwrap().as_ptr(), bytes.as_ptr());
        }
    }

    #[tokio::test]
    async fn disk_promotion_charges_the_smaller_allocation_for_a_small_slice() {
        let (_directory, cache) = cache(32).await;
        let key = "large-entry".to_owned();
        let disk = cache.inner.disk.as_ref().unwrap();
        disk.insert_batch(vec![(
            ObjectKeyHash::from(key.as_str()),
            Download::new(0, Bytes::from(vec![0x77; 40 * 1024])).unwrap(),
        )])
        .await
        .unwrap();
        let bytes = cache
            .get_or_fetch(key.clone(), range(5, 9), || async {
                Err::<Download, _>("disk hit must not fetch")
            })
            .await
            .unwrap();
        assert_eq!(&bytes[..], &[0x77; 4]);
        assert_eq!(cache.inner.memory.used_bytes(), 32 * 1024);
        assert_eq!(
            cache
                .inner
                .memory
                .get(&ObjectKeyHash::from(key.as_str()), range(5, 9))
                .unwrap()
                .as_ptr(),
            bytes.as_ptr()
        );
        assert_eq!(cache.inner.memory.buffer_pool().idle_bytes(), 0);
        cache
            .inner
            .memory
            .remove(&ObjectKeyHash::from(key.as_str()), range(5, 9));
        assert_eq!(cache.inner.memory.used_bytes(), 0);
        assert_eq!(&bytes[..], &[0x77; 4]);
    }

    #[tokio::test]
    async fn concurrent_disk_promotions_preserve_each_requested_slice() {
        let (_directory, cache) = cache(8 << 20).await;
        let key = "shared-entry".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let mut payload = vec![0x77; 40 * 1024];
        payload[13..21].copy_from_slice(b"abcdefgh");
        let disk = cache.inner.disk.as_ref().unwrap();
        disk.insert_batch(vec![(hash, Download::new(0, Bytes::from(payload)).unwrap())])
            .await
            .unwrap();
        disk.flush().await.unwrap();
        let (first, second) = tokio::join!(
            cache.get_or_fetch(key.clone(), range(13, 17), || async {
                Err::<Download, _>("disk hit must not fetch")
            }),
            cache.get_or_fetch(key, range(17, 21), || async {
                Err::<Download, _>("disk hit must not fetch")
            }),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(&first[..], b"abcd");
        assert_eq!(&second[..], b"efgh");
        for (range, bytes) in [(range(13, 17), &first), (range(17, 21), &second)] {
            assert_eq!(cache.inner.memory.get(&hash, range).unwrap().as_ptr(), bytes.as_ptr());
        }
        assert_eq!(
            cache.inner.memory.used_bytes(),
            cache.inner.memory.buffer_pool().idle_bytes() + 2 * 32 * 1024
        );
    }

    #[tokio::test]
    async fn callback_covered_by_a_racing_disk_write_is_not_inserted_again() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let history = cache.inner.memory.access_histories();
        let bytes = cache
            .get_or_fetch(key.clone(), range(2, 4), || async {
                // Simulate another disk write finishing while this callback is pending.
                let disk = cache.inner.disk.as_ref().unwrap();
                disk.insert_batch(vec![(
                    ObjectKeyHash::from(key.as_str()),
                    Download::new(0, Bytes::from_static(b"abcdefgh")).unwrap(),
                )])
                .await
                .unwrap();
                Ok::<_, Infallible>(Download::new(2, Bytes::from_static(b"cd")).unwrap())
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
                    .get_or_fetch(key, range(0, 1), || async move {
                        entered.notify_one();
                        std::future::pending::<Result<Download, Infallible>>().await
                    })
                    .await
            })
        };
        entered.notified().await;
        assert_eq!(history.request_count(), 1, "recorded before the callback completes");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            cache
                .inner
                .memory
                .get(&ObjectKeyHash::from(key.as_str()), range(0, 1))
                .is_none()
        );
        let disk = cache.inner.disk.as_ref().unwrap();
        assert!(!disk.covers_range(&ObjectKeyHash::from(key.as_str()), range(0, 1)));
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
    async fn callback_result_and_covering_memory_hit_return_the_exact_request() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unused");
        let cache = TieredMemoryDiskCache::open(CacheConfig::new(&path, 0, 32).unwrap(), None, None)
            .await
            .unwrap();
        let key = String::from("object");
        let payload = Bytes::from_static(b"abcdefghij");
        let result = cache
            .get_or_fetch(key.clone(), range(13, 17), || async {
                Ok::<_, Infallible>(Download::new(10, payload.clone()).unwrap())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"defg"));
        assert_eq!(result.as_ptr(), payload.slice(3..).as_ptr());

        let result = cache
            .get_or_fetch(key, range(11, 19), || async {
                Err::<Download, _>("memory hit must not fetch")
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"bcdefghi"));
        assert!(cache.inner.disk.is_none());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn every_concurrent_miss_invokes_its_own_callback() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let barrier = Arc::new(Barrier::new(3));
        let callback_count = Arc::new(AtomicU64::new(0));
        let mut tasks = Vec::new();

        for _ in 0..2 {
            let cache = cache.clone();
            let key = key.clone();
            let barrier = barrier.clone();
            let callback_count = callback_count.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_fetch(key, range(2, 5), move || async move {
                        callback_count.fetch_add(1, Ordering::Relaxed);
                        barrier.wait().await;
                        Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"abcdefgh")).unwrap())
                    })
                    .await
                    .unwrap()
            }));
        }

        barrier.wait().await;
        for task in tasks {
            assert_eq!(task.await.unwrap(), Bytes::from_static(b"cde"));
        }
        assert_eq!(callback_count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn callback_errors_are_returned_without_retry_or_insertion() {
        let (_directory, cache) = cache(8).await;
        let key = String::from("object");
        let callback_count = Arc::new(AtomicU64::new(0));

        for expected_count in 1..=2 {
            let invocation_count = callback_count.clone();
            let error = cache
                .get_or_fetch(key.clone(), range(0, 1), move || async move {
                    invocation_count.fetch_add(1, Ordering::Relaxed);
                    Err::<Download, _>("source unavailable")
                })
                .await
                .unwrap_err();

            assert_eq!(error, GetOrFetchError::Callback("source unavailable"));
            assert_eq!(callback_count.load(Ordering::Relaxed), expected_count);
        }
    }

    #[tokio::test]
    async fn rejects_noncovering_but_retains_oversized_callback_results() {
        let (_directory, cache) = cache(4).await;
        let key = String::from("object");

        let error = cache
            .get_or_fetch(key.clone(), range(2, 4), || async {
                Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"abc")).unwrap())
            })
            .await
            .unwrap_err();
        assert_eq!(
            error,
            GetOrFetchError::DownloadDoesNotCover {
                requested_range: range(2, 4),
                downloaded_range: range(0, 3),
            }
        );

        let result = cache
            .get_or_fetch(key.clone(), range(2, 4), || async {
                Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"abcde")).unwrap())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"cd"));

        let result = cache
            .get_or_fetch(key, range(0, 5), || async {
                Err::<Download, _>("memory hit must not fetch")
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"abcde"));
    }

    #[tokio::test]
    async fn a_racing_contained_download_is_discarded_but_returns_its_own_bytes() {
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
                    .get_or_fetch(key, range(3, 5), move || async move {
                        callback_entered.notify_one();
                        release_callback.notified().await;
                        Ok::<_, Infallible>(Download::new(2, Bytes::from_static(b"XXXXXX")).unwrap())
                    })
                    .await
                    .unwrap()
            })
        };

        callback_entered.notified().await;
        cache
            .get_or_fetch(key.clone(), range(0, 10), || async {
                Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"abcdefghij")).unwrap())
            })
            .await
            .unwrap();
        release_callback.notify_one();

        assert_eq!(pending.await.unwrap(), Bytes::from_static(b"XX"));
        let cached = cache
            .get_or_fetch(key, range(2, 8), || async {
                Err::<Download, _>("memory hit must not fetch")
            })
            .await
            .unwrap();
        assert_eq!(cached, Bytes::from_static(b"cdefgh"));
    }
}
