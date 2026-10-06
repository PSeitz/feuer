use std::{fmt, future::Future, sync::Arc, time::Instant};

use bytes::Bytes;
use feuer_memory::MemoryCache;
#[cfg(target_os = "linux")]
use feuer_memory::MemoryMetrics;
#[cfg(target_os = "linux")]
use feuer_storage::{DiskCache, DiskCacheError, IoMetrics};
use feuer_types::{ByteRange, Download, ObjectKeyHash};
#[cfg(target_os = "linux")]
use mixtrics::metrics::BoxedRegistry;
use thiserror::Error;

use crate::{
    CacheConfig,
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
/// Disk writes are best-effort; a full 256-entry queue skips new writes. Zero disk capacity disables disk;
/// otherwise opening requires Linux direct I/O and io_uring, and waits for metadata recovery.
#[derive(Clone)]
pub struct TieredMemoryDiskCache {
    inner: Arc<TieredMemoryDiskCacheInner>,
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
    /// Otherwise requires Tokio, io_uring and direct I/O, locks the directory,
    /// and waits for metadata recovery. Disk failures never fall back to memory.
    #[cfg(target_os = "linux")]
    pub async fn open(config: CacheConfig) -> Result<Self, DiskCacheError> {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::open_with_metrics(config, &registry).await
    }

    /// Opens a cache with metrics registered through `mixtrics`.
    /// Label values are defined by Feuer and contain no object identities or cache names. Caches
    /// sharing a registry contribute to the same counters and aggregate gauges.
    #[cfg(target_os = "linux")]
    pub async fn open_with_metrics(config: CacheConfig, registry: &BoxedRegistry) -> Result<Self, DiskCacheError> {
        let memory = MemoryCache::with_metrics(config.memory_capacity(), MemoryMetrics::new(registry))
            .with_reclaim_sample_size(config.reclaim_sample_size());
        let disk = if config.disk_capacity() == 0 {
            None
        } else {
            Some(
                DiskCache::open_with_buffer_pool(
                    config.directory(),
                    config.disk_capacity(),
                    IoMetrics::new(registry),
                    memory.access_histories().clone(),
                    feuer_storage::DiskMetrics::new(registry),
                    config.reclaim_sample_size(),
                    memory.buffer_pool(),
                )
                .await?,
            )
        };
        Ok(Self {
            inner: Arc::new(TieredMemoryDiskCacheInner {
                config,
                memory,
                metrics: LookupMetrics::new(registry),
                disk,
            }),
        })
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

    /// Returns the requested bytes from memory, disk, or this call's callback.
    ///
    /// A covering memory range is checked first, then disk. A disk hit promotes
    /// only the requested bytes to memory, copying if the pool offers a smaller
    /// backing allocation. On a miss in both tiers, `callback` is
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
        let object_key = ObjectKeyHash::from(object_key);
        let history = self.inner.memory.access_histories();
        history.record_access(&object_key, requested_range);
        let metrics = &self.inner.metrics;
        if let Some(bytes) = self.inner.memory.get(&object_key, requested_range) {
            metrics.record(LookupOutcome::MemoryHit, None, requested_range.len());
            return Ok(bytes);
        }

        let started = Instant::now();
        #[cfg(target_os = "linux")]
        if let Some(disk) = &self.inner.disk
            && let Some((bytes, buffer_capacity)) = disk.fetch_from_disk(&object_key, requested_range).await
        {
            self.inner.memory.insert_with_allocation_charge(
                object_key,
                Download::new(requested_range.start(), bytes.clone()).expect("disk result covers the request"),
                buffer_capacity,
            );
            metrics.record(LookupOutcome::DiskHit, Some(started.elapsed()), requested_range.len());
            return Ok(bytes);
        }

        let download = callback().await.map_err(|error| {
            metrics.record(LookupOutcome::CallbackError, None, 0);
            GetOrFetchError::Callback(error)
        })?;
        let downloaded_range = download.downloaded_range();
        if !downloaded_range.contains(requested_range) {
            metrics.record(LookupOutcome::InvalidDownload, None, 0);
            return Err(GetOrFetchError::DownloadDoesNotCover {
                requested_range,
                downloaded_range,
            });
        }

        let requested_bytes = download.bytes_in_range(requested_range);
        #[cfg(target_os = "linux")]
        if let Some(disk) = &self.inner.disk {
            if disk.covers_range(&object_key, downloaded_range) {
                metrics.disk_write_already_covered.increase(1);
            } else if self.inner.memory.insert(object_key, download.clone()) {
                disk.enqueue_if_space_available(object_key, download);
            } else {
                metrics.disk_write_redundant.increase(1);
            }
        } else {
            self.inner.memory.insert(object_key, download);
        }
        #[cfg(not(target_os = "linux"))]
        self.inner.memory.insert(object_key, download);

        metrics.record(LookupOutcome::Callback, Some(started.elapsed()), requested_range.len());
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
        let cache = TieredMemoryDiskCache::open(CacheConfig::new(directory.path(), 4 << 20, memory_capacity).unwrap())
            .await
            .unwrap();
        (directory, cache)
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
    async fn opening_validates_disk_capacity_and_exclusive_directory_ownership() {
        let (directory, cache) = cache(32).await;
        assert!(TieredMemoryDiskCache::open(cache.config().clone()).await.is_err());
        let invalid = CacheConfig::new(directory.path(), 1024, 32).unwrap();
        assert!(matches!(
            TieredMemoryDiskCache::open(invalid).await,
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
        let cache = TieredMemoryDiskCache::open(CacheConfig::new(&path, 0, 32).unwrap())
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
