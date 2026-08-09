use std::{fmt, future::Future, sync::Arc};

use bytes::Bytes;
use feuer_memory::MemoryCache;
use feuer_types::{ByteRange, Download, ObjectKey};
use thiserror::Error;

use crate::Config;

struct Inner {
    config: Config,
    memory: MemoryCache,
}

/// A cloneable handle to one Feuer cache.
///
/// This implementation slice provides the complete per-call callback and
/// covering-memory path. Disk population and recovery are added by later
/// slices; constructing a handle does not yet open or modify the configured
/// cache directory.
#[derive(Clone)]
pub struct Cache {
    inner: Arc<Inner>,
}

impl fmt::Debug for Cache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cache")
            .field("memory_capacity", &self.inner.config.memory_capacity())
            .field("disk_capacity", &self.inner.config.disk_capacity())
            .finish_non_exhaustive()
    }
}

impl Cache {
    /// Creates a cache handle from a validated configuration.
    pub fn new(config: Config) -> Self {
        let memory = MemoryCache::new(config.memory_capacity());
        Self {
            inner: Arc::new(Inner { config, memory }),
        }
    }

    /// Returns this cache's configuration.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// Returns the requested bytes from memory or invokes this call's callback.
    ///
    /// One covering memory range is checked first. On a miss, `callback` is
    /// invoked exactly once by this call; Feuer performs no leader election,
    /// waiter coordination, or source retry. A successful callback must return
    /// one valid [`Download`] covering `requested_range`. The memory target is
    /// soft, so callback results are not rejected solely for exceeding it.
    ///
    /// Every success records `requested_range` exactly once. Downloaded-range
    /// population is separate and creates no access.
    /// The returned [`Bytes`] contains exactly the request and may share the
    /// download's allocation.
    pub async fn get_or_fetch<F, Fut, E>(
        &self,
        object_key: ObjectKey,
        requested_range: ByteRange,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Download, E>>,
    {
        if let Some(bytes) = self.inner.memory.get(&object_key, requested_range) {
            return Ok(bytes);
        }

        let download = callback().await.map_err(GetOrFetchError::Callback)?;
        let downloaded_range = download.downloaded_range();
        if !downloaded_range.contains(requested_range) {
            return Err(GetOrFetchError::DownloadDoesNotCover {
                requested_range,
                downloaded_range,
            });
        }

        let requested_bytes = requested_slice(download.bytes(), downloaded_range, requested_range);
        self.inner
            .memory
            .insert_and_record(object_key, download, requested_range);

        Ok(requested_bytes)
    }
}

/// A failed [`Cache::get_or_fetch`] operation.
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

#[cfg(test)]
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

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    fn cache(memory_capacity: u64) -> Cache {
        Cache::new(Config::new("unused", 1024, memory_capacity).unwrap())
    }

    #[test]
    fn cache_handle_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<Cache>();
    }

    #[tokio::test]
    async fn callback_result_and_covering_memory_hit_return_the_exact_request() {
        let cache = cache(32);
        let key = ObjectKey::from("object");
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
        let cache = cache(32);
        let key = ObjectKey::from("object");
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
    async fn callback_errors_are_returned_without_retry_or_population() {
        let cache = cache(8);
        let key = ObjectKey::from("object");
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
        let cache = cache(4);
        let key = ObjectKey::from("object");

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
        let cache = cache(32);
        let key = ObjectKey::from("object");
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
