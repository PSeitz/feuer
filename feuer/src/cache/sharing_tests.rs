use super::*;
use crate::test_metrics::{registry, value};
use std::{
    future::pending,
    task::{Context, Poll, Waker},
};
use tokio::sync::Notify;

fn cache() -> (TieredMemoryDiskCache, prometheus::Registry) {
    let (registry, backend) = registry();
    let cache = TieredMemoryDiskCache {
        inner: Arc::new(TieredMemoryDiskCacheInner {
            config: CacheConfig::new("unused", 0, 8 << 20).unwrap(),
            memory: MemoryCache::with_metrics(8 << 20, feuer_memory::MemoryMetrics::new(&backend), 7),
            metrics: LookupMetrics::new(&backend),
            pending_fetches: PendingFetches::default(),
            #[cfg(target_os = "linux")]
            disk: None,
        }),
        trace: None,
    };
    (cache, registry)
}

fn assert_pending(future: std::pin::Pin<&mut impl Future>) {
    assert!(future.poll(&mut Context::from_waker(Waker::noop())).is_pending());
}

fn pending_count(cache: &TieredMemoryDiskCache) -> usize {
    cache.inner.pending_fetches.lock().unwrap().len()
}

#[tokio::test]
async fn shares_successes_and_errors_but_not_completed_failures() {
    let requested = ByteRange::new(0, 2).unwrap();
    for download in [
        Ok(Download::new(0, Bytes::from_static(b"ok")).unwrap()),
        Err("source failed"),
        Ok(Download::new(0, Bytes::from_static(b"x")).unwrap()),
    ] {
        let (cache, registry) = cache();
        let release = Notify::new();
        let mut first = Box::pin(cache.get_or_fetch("object".into(), requested, || async {
            release.notified().await;
            download.clone()
        }));
        let mut second = Box::pin(cache.get_or_fetch("object".into(), requested, || {
            std::future::ready(Err::<Download, &str>("waiter must not execute"))
        }));
        assert_pending(first.as_mut());
        assert_pending(second.as_mut());
        assert_eq!(pending_count(&cache), 1);
        release.notify_one();
        let first = first.await;
        let second = second.await;
        assert_eq!(first, second);
        let outcome = match &first {
            Ok(bytes) => {
                assert_eq!(bytes, "ok");
                assert_eq!(bytes.as_ptr(), second.as_ref().unwrap().as_ptr());
                "callback"
            }
            Err(GetOrFetchError::Callback(error)) => {
                assert_eq!(*error, "source failed");
                "callback_error"
            }
            Err(GetOrFetchError::DownloadDoesNotCover { .. }) => "invalid_download",
        };
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), 2.0);
        assert_eq!(
            value(&registry, "feuer_lookup_bytes_total", &[("source", "callback")]),
            if first.is_ok() { 4.0 } else { 0.0 }
        );
        let history = cache.inner.memory.access_histories();
        assert_eq!(history.request_count(), 2);
        assert_eq!(
            history.recent_requested_ranges(&ObjectKeyHash::from("object")),
            vec![requested; 2]
        );
        assert_eq!(pending_count(&cache), 0);
        if first.is_err() {
            assert_eq!(
                cache
                    .get_or_fetch("object".into(), requested, || async {
                        Err::<Download, &str>("new attempt")
                    })
                    .await,
                Err(GetOrFetchError::Callback("new attempt"))
            );
        }
    }
}

#[tokio::test]
async fn different_error_types_ranges_and_keys_do_not_share() {
    let (cache, _) = cache();
    let range = ByteRange::new(0, 1).unwrap();
    let mut first = Box::pin(cache.get_or_fetch("object".into(), range, pending::<Result<Download, &str>>));
    let mut different_error = Box::pin(cache.get_or_fetch("object".into(), range, pending::<Result<Download, String>>));
    let mut different_range = Box::pin(cache.get_or_fetch(
        "object".into(),
        ByteRange::new(1, 2).unwrap(),
        pending::<Result<Download, &str>>,
    ));
    let mut different_key = Box::pin(cache.get_or_fetch("other".into(), range, pending::<Result<Download, &str>>));
    assert_pending(first.as_mut());
    assert_pending(different_error.as_mut());
    assert_pending(different_range.as_mut());
    assert_pending(different_key.as_mut());
    assert_eq!(pending_count(&cache), 4);
    drop((first, different_error, different_range, different_key));
    assert_eq!(pending_count(&cache), 0);
}

#[tokio::test]
async fn cancellation_allows_takeover_and_cleans_up_in_either_order() {
    for order in 0..3 {
        let (cache, _) = cache();
        let mut first = Box::pin(cache.get_or_fetch_object("object".into(), pending::<Result<Bytes, &str>>));
        let mut second = Box::pin(cache.get_or_fetch_object("object".into(), || async {
            Ok::<_, &str>(Bytes::from_static(b"replacement"))
        }));
        assert_pending(first.as_mut());
        assert_pending(second.as_mut());
        match order {
            0 => {
                drop(first);
                assert_eq!(pending_count(&cache), 1);
                assert_eq!(second.await.unwrap(), "replacement");
            }
            1 => {
                drop(first);
                drop(second);
            }
            _ => {
                drop(second);
                drop(first);
            }
        }
        assert_eq!(pending_count(&cache), 0);
    }
}

#[tokio::test]
async fn old_waiter_cannot_remove_a_new_attempt() {
    let (cache, _) = cache();
    let release = Notify::new();
    let mut first = Box::pin(cache.get_or_fetch_object("object".into(), || async {
        release.notified().await;
        Err::<Bytes, &str>("old failure")
    }));
    let mut waiter = Box::pin(cache.get_or_fetch_object("object".into(), pending::<Result<Bytes, &str>>));
    assert_pending(first.as_mut());
    assert_pending(waiter.as_mut());
    release.notify_one();
    assert_eq!(first.await, Err(GetOrFetchError::Callback("old failure")));
    let mut replacement = Box::pin(cache.get_or_fetch_object("object".into(), pending::<Result<Bytes, &str>>));
    assert_pending(replacement.as_mut());
    assert_eq!(waiter.await, Err(GetOrFetchError::Callback("old failure")));
    assert_eq!(pending_count(&cache), 1);
    drop(replacement);
    assert_eq!(pending_count(&cache), 0);
}

#[test]
fn simultaneous_cancellation_removes_the_last_callers() {
    let pending = PendingFetches::default();
    let first = PendingFetch::<&str>::join(&pending, ObjectKeyHash::from("object"), None);
    let second = PendingFetch::<&str>::join(&pending, ObjectKeyHash::from("object"), None);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            drop(first);
        });
        scope.spawn(|| {
            barrier.wait();
            drop(second);
        });
    });
    assert!(pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn lookup_only_checks_cached_bytes_without_waiting_for_a_callback() {
    let (cache, registry) = cache();
    let mut fetch = Box::pin(cache.get_or_fetch_object("object".into(), pending::<Result<Bytes, &str>>));
    assert_pending(fetch.as_mut());
    assert_eq!(
        Box::pin(cache.get_object("object".into()))
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(None)
    );
    for (key, bytes) in [("object", Bytes::from_static(b"object")), ("empty", Bytes::new())] {
        cache
            .inner
            .memory
            .insert(ObjectKeyHash::from(key), Download::new(0, bytes.clone()).unwrap());
        assert_eq!(cache.get_object(key.into()).await, Some(bytes));
    }
    assert_eq!(cache.inner.memory.access_histories().request_count(), 4);
    assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", "miss")]), 1.0);
    assert_eq!(
        value(&registry, "feuer_lookup_total", &[("outcome", "memory_hit")]),
        2.0
    );
    drop(fetch);
    assert_eq!(pending_count(&cache), 0);
}
