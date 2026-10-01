use std::{fmt, future::Future, sync::Arc, time::Instant};

#[cfg(target_os = "linux")]
use crate::disk_write::DiskWriteQueue;
use bytes::Bytes;
use feuer_memory::MemoryCache;
#[cfg(target_os = "linux")]
use feuer_memory::MemoryMetrics;
#[cfg(target_os = "linux")]
use feuer_storage::{DiskRangeCache, DiskRangeCacheError, IoMetrics};
use feuer_types::{ByteRange, Download, ObjectKeyHash, retention::ObjectAccessHistories};
#[cfg(target_os = "linux")]
use mixtrics::metrics::BoxedRegistry;
use thiserror::Error;

use crate::{
    CacheConfig,
    metrics::{LookupMetrics, LookupOutcome},
};

/// Configuration and both tiers shared by cloned cache handles.
struct CacheState {
    config: CacheConfig,
    memory: Arc<MemoryCache>,
    access_histories: Arc<ObjectAccessHistories>,
    metrics: LookupMetrics,
    #[cfg(target_os = "linux")]
    disk: DiskRangeCache,
    #[cfg(target_os = "linux")]
    disk_write_queue: DiskWriteQueue,
}

/// A cloneable handle to one Feuer cache.
///
/// Lookups check memory, then integrity-checked disk, then the per-call callback.
/// Disk writes are bounded and best-effort. Opening requires Linux direct I/O
/// and io_uring. Background recovery adds disk entries incrementally without gating lookups or writes.
#[derive(Clone)]
pub struct TieredMemoryDiskCache {
    state: Arc<CacheState>,
}

impl fmt::Debug for TieredMemoryDiskCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TieredMemoryDiskCache")
            .field("memory_capacity", &self.state.config.memory_capacity())
            .field("disk_capacity", &self.state.config.disk_capacity())
            .finish_non_exhaustive()
    }
}

impl TieredMemoryDiskCache {
    /// Opens an exclusively locked disk cache and starts its best-effort writer.
    /// Disk capacity must be a positive multiple of 1 MiB. Requires a Tokio runtime,
    /// usable io_uring and direct I/O; no memory-only or buffered fallback is used.
    /// Existing disk entries recover incrementally in the background while lookups and writes proceed.
    #[cfg(target_os = "linux")]
    pub async fn open(config: CacheConfig) -> Result<Self, DiskRangeCacheError> {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::open_with_metrics(config, &registry).await
    }

    /// Opens a cache with metrics registered through `mixtrics`.
    /// Labels are bounded and contain no object identities or cache names. Caches
    /// sharing a registry contribute to the same counters and aggregate gauges.
    #[cfg(target_os = "linux")]
    pub async fn open_with_metrics(config: CacheConfig, registry: &BoxedRegistry) -> Result<Self, DiskRangeCacheError> {
        let access_histories = Arc::new(ObjectAccessHistories::new());
        let memory = Arc::new(
            MemoryCache::with_access_histories(
                config.memory_capacity(),
                MemoryMetrics::new(registry),
                access_histories.clone(),
            )
            .with_reclaim_sample_size(config.reclaim_sample_size()),
        );
        let disk = DiskRangeCache::open_with_buffer_pool(
            config.directory(),
            config.disk_capacity(),
            IoMetrics::new(registry),
            access_histories.clone(),
            feuer_storage::DiskMetrics::new(registry),
            config.reclaim_sample_size(),
            memory.buffer_pool(),
        )
        .await?;
        let disk_write_queue = DiskWriteQueue::with_metrics(memory.clone(), disk.clone(), registry);
        Ok(Self {
            state: Arc::new(CacheState {
                config,
                memory,
                access_histories,
                metrics: LookupMetrics::new(registry),
                disk,
                disk_write_queue,
            }),
        })
    }

    /// Returns this cache's configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.state.config
    }

    /// Returns the requested bytes from memory, disk, or this call's callback.
    ///
    /// A covering memory range is checked first, then disk. A disk hit promotes
    /// only the requested bytes to memory. On a miss in both tiers, `callback` is
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
        let started = Instant::now();
        let object_key = ObjectKeyHash::from(object_key);
        self.state.access_histories.record_access(&object_key, requested_range);
        let metrics = &self.state.metrics;
        if let Some(bytes) = self.state.memory.get(&object_key, requested_range) {
            metrics.record(LookupOutcome::MemoryHit, started.elapsed(), requested_range.len());
            return Ok(bytes);
        }

        #[cfg(target_os = "linux")]
        if let Some((bytes, capacity)) = self.state.disk.get_with_capacity(&object_key, requested_range).await {
            self.state.memory.insert_with_capacity(
                object_key,
                Download::new(requested_range.start(), bytes.clone()).expect("disk result covers the request"),
                capacity,
            );
            metrics.record(LookupOutcome::DiskHit, started.elapsed(), requested_range.len());
            return Ok(bytes);
        }

        let download = match callback().await {
            Ok(download) => download,
            Err(error) => {
                metrics.record(LookupOutcome::CallbackError, started.elapsed(), 0);
                return Err(GetOrFetchError::Callback(error));
            }
        };
        let downloaded_range = download.downloaded_range();
        if !downloaded_range.contains(requested_range) {
            metrics.record(LookupOutcome::InvalidDownload, started.elapsed(), 0);
            return Err(GetOrFetchError::DownloadDoesNotCover {
                requested_range,
                downloaded_range,
            });
        }

        let requested_bytes = requested_slice(download.bytes(), downloaded_range, requested_range);
        #[cfg(target_os = "linux")]
        if self.state.disk.contains(&object_key, downloaded_range) {
            self.state.disk_write_queue.record_already_covered();
            metrics.record(LookupOutcome::Callback, started.elapsed(), requested_range.len());
            return Ok(requested_bytes);
        }
        let entry_id = self.state.memory.insert(object_key, download.clone());
        #[cfg(target_os = "linux")]
        if let Some(entry_id) = entry_id {
            self.state
                .disk_write_queue
                .enqueue_if_capacity(object_key, download, entry_id);
        } else {
            self.state.disk_write_queue.record_already_in_memory();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = entry_id;

        metrics.record(LookupOutcome::Callback, started.elapsed(), requested_range.len());
        Ok(requested_bytes)
    }
}

/// A failed [`TieredMemoryDiskCache::get_or_fetch`] operation.
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

fn requested_slice(bytes: &Bytes, downloaded_range: ByteRange, requested_range: ByteRange) -> Bytes {
    debug_assert!(downloaded_range.contains(requested_range));
    let start = usize::try_from(requested_range.start() - downloaded_range.start())
        .expect("an offset within a callback Bytes payload must fit in usize");
    let end = usize::try_from(requested_range.end() - downloaded_range.start())
        .expect("an offset within a callback Bytes payload must fit in usize");
    bytes.slice(start..end)
}

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
    async fn public_registry_observes_memory_disk_callbacks_and_writes() {
        let directory = tempfile::tempdir().unwrap();
        let (registry, backend) = registry();
        let cache = TieredMemoryDiskCache::open_with_metrics(
            CacheConfig::new(directory.path(), 4 << 20, 32).unwrap(),
            &backend,
        )
        .await
        .unwrap();
        let key = "object".to_owned();
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
        wait_for_disk(&cache, &key, range(0, 4)).await;
        cache
            .get_or_fetch(key.clone(), range(0, 2), || async {
                Err::<Download, _>("memory hit must not fetch")
            })
            .await
            .unwrap();
        cache.state.memory.insert(
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
        for outcome in ["memory_hit", "disk_hit", "callback"] {
            assert_eq!(
                value(&registry, "feuer_lookup_duration_seconds", &[("outcome", outcome)]),
                1.0
            );
        }
        for source in ["memory", "disk", "callback"] {
            assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 2.0);
        }
        assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 1.0);
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "write"), ("outcome", "success")]
            ),
            1.0
        );
        assert_eq!(
            value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
            1.0
        );
        assert_eq!(
            cache.state.access_histories.clock(),
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
        let cache = TieredMemoryDiskCache::open(CacheConfig::new(directory.path(), 4 << 20, memory_capacity).unwrap())
            .await
            .unwrap();
        (directory, cache)
    }

    async fn wait_for_disk(cache: &TieredMemoryDiskCache, key: &str, range: ByteRange) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !cache.state.disk.contains(&ObjectKeyHash::from(key), range) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("disk write did not finish");
    }

    #[tokio::test]
    async fn disk_hit_after_memory_pressure_promotes_only_request_and_records_once() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let history = &cache.state.access_histories;
        let source = Download::new(10, Bytes::from_static(b"abcdefghij")).unwrap();
        let result = cache
            .get_or_fetch(key.clone(), range(13, 17), || async {
                Ok::<_, Infallible>(source.clone())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"defg"));
        wait_for_disk(&cache, &key, source.downloaded_range()).await;
        assert_eq!(history.clock(), 1);

        // The same key selects the same memory shard; an oversized disjoint range
        // forces the original download out without contributing an access.
        cache.state.memory.insert(
            ObjectKeyHash::from(key.as_str()),
            Download::new(100, Bytes::from(vec![0; 64])).unwrap(),
        );
        assert!(
            cache
                .state
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
            assert_eq!(history.clock(), accesses);
        }
        assert_eq!(cache.state.memory.used_bytes(), 32 * 1024);
    }

    #[tokio::test]
    async fn disk_promotion_charges_the_whole_rounded_allocation_for_a_small_slice() {
        let (_directory, cache) = cache(32).await;
        let key = "large-entry".to_owned();
        cache
            .state
            .disk
            .insert_batch(vec![(
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
        assert_eq!(cache.state.memory.used_bytes(), 256 * 1024);
        assert_eq!(
            cache
                .state
                .memory
                .get(&ObjectKeyHash::from(key.as_str()), range(5, 9))
                .unwrap()
                .as_ptr(),
            bytes.as_ptr()
        );
        assert_eq!(cache.state.memory.buffer_pool().idle_bytes(), 0);
        cache
            .state
            .memory
            .remove(&ObjectKeyHash::from(key.as_str()), range(5, 9));
        assert_eq!(cache.state.memory.used_bytes(), 0);
        assert_eq!(&bytes[..], &[0x77; 4]);
    }

    #[tokio::test]
    async fn callback_covered_by_a_racing_disk_write_is_not_inserted_again() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let history = &cache.state.access_histories;
        let bytes = cache
            .get_or_fetch(key.clone(), range(2, 4), || async {
                // Simulate another disk write finishing while this callback is pending.
                cache
                    .state
                    .disk
                    .insert_batch(vec![(
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
        assert_eq!(cache.state.memory.used_bytes(), 0);
        assert_eq!(history.clock(), 1);
    }

    #[tokio::test]
    async fn callback_cancellation_keeps_the_recorded_request_without_inserting() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("canceled");
        let history = &cache.state.access_histories;
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
        assert_eq!(history.clock(), 1, "recorded before the callback completes");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            cache
                .state
                .memory
                .get(&ObjectKeyHash::from(key.as_str()), range(0, 1))
                .is_none()
        );
        assert!(
            !cache
                .state
                .disk
                .contains(&ObjectKeyHash::from(key.as_str()), range(0, 1))
        );
        assert_eq!(history.clock(), 1);
    }

    #[tokio::test]
    async fn opening_validates_disk_capacity_and_exclusive_directory_ownership() {
        let (directory, cache) = cache(32).await;
        assert!(TieredMemoryDiskCache::open(cache.config().clone()).await.is_err());
        let invalid = CacheConfig::new(directory.path(), 1024, 32).unwrap();
        assert!(matches!(
            TieredMemoryDiskCache::open(invalid).await,
            Err(DiskRangeCacheError::InvalidCapacity)
        ));
    }

    #[test]
    fn cache_handle_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<TieredMemoryDiskCache>();
    }

    #[tokio::test]
    async fn callback_result_and_covering_memory_hit_return_the_exact_request() {
        let (_directory, cache) = cache(32).await;
        let key = String::from("object");
        let payload = Bytes::from_static(b"abcdefghij");
        let callback_count = Arc::new(AtomicU64::new(0));

        let count = callback_count.clone();
        let callback_payload = payload.clone();
        let result = cache
            .get_or_fetch(key.clone(), range(13, 17), move || async move {
                count.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Download::new(10, callback_payload).unwrap())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"defg"));
        assert_eq!(result.as_ptr(), payload.slice(3..).as_ptr());

        let count = callback_count.clone();
        let result = cache
            .get_or_fetch(key, range(11, 19), move || async move {
                count.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Download::new(11, Bytes::from_static(b"12345678")).unwrap())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"bcdefghi"));
        assert_eq!(callback_count.load(Ordering::Relaxed), 1);
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

        let unexpected_callback_count = Arc::new(AtomicU64::new(0));
        let count = unexpected_callback_count.clone();
        let result = cache
            .get_or_fetch(key, range(0, 5), move || async move {
                count.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Download::new(0, Bytes::from_static(b"XXXXX")).unwrap())
            })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"abcde"));
        assert_eq!(unexpected_callback_count.load(Ordering::Relaxed), 0);
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
        let unexpected_callback_count = Arc::new(AtomicU64::new(0));
        let count = unexpected_callback_count.clone();
        let cached = cache
            .get_or_fetch(key, range(2, 8), move || async move {
                count.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Download::new(2, Bytes::from_static(b"123456")).unwrap())
            })
            .await
            .unwrap();
        assert_eq!(cached, Bytes::from_static(b"cdefgh"));
        assert_eq!(unexpected_callback_count.load(Ordering::Relaxed), 0);
    }
}
