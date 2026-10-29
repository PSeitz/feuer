use super::*;
use crate::test_metrics::{registry, value};
use prometheus::Registry;
use std::{
    future::{pending, ready},
    pin::Pin,
    sync::Barrier,
    task::{Context, Poll, Waker},
    thread::scope,
};
use tokio::sync::Notify;

async fn cache() -> (Cache, Registry) {
    let (registry, backend) = registry();
    let cache = Cache::open(CacheConfig::new("unused", 0, 8 << 20).unwrap(), Some(&backend), None)
        .await
        .unwrap();
    (cache, registry)
}

fn assert_pending(future: Pin<&mut impl Future>) {
    assert!(future.poll(&mut Context::from_waker(Waker::noop())).is_pending());
}

fn pending_count(cache: &Cache) -> usize {
    cache.inner.pending_fetches.lock().unwrap().len()
}

#[tokio::test]
async fn lookup_methods_share_successes_and_errors_but_not_completed_failures() {
    for success in [true, false] {
        let (cache, registry) = cache().await;
        let release = Notify::new();
        let mut first = Box::pin(cache.get_or_fetch_buffer("object".into(), || async {
            release.notified().await;
            if success {
                let mut buffer = cache.allocate_buffer(2).unwrap();
                buffer.as_mut_slice().copy_from_slice(b"ok");
                Ok(buffer)
            } else {
                Err("source failed")
            }
        }));
        let mut second =
            Box::pin(cache.get_or_fetch("object".into(), || ready(Err::<Bytes, &str>("waiter must not execute"))));
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
        };
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), 2.0);
        assert_eq!(
            value(&registry, "feuer_lookup_bytes_total", &[("source", "callback")]),
            if first.is_ok() { 4.0 } else { 0.0 }
        );
        let history = cache.inner.memory.access_histories();
        assert_eq!(history.request_count(), 2);
        assert!(history.decayed_request_count(&ObjectKeyHash::from("object")) > 1.0);
        assert_eq!(pending_count(&cache), 0);
        if first.is_err() {
            assert_eq!(
                cache
                    .get_or_fetch("object".into(), || async { Err::<Bytes, &str>("new attempt") })
                    .await,
                Err(GetOrFetchError::Callback("new attempt"))
            );
        }
    }
}

#[tokio::test]
async fn different_error_types_and_keys_do_not_share() {
    let (cache, _) = cache().await;
    let mut first = Box::pin(cache.get_or_fetch("object".into(), pending::<Result<Bytes, &str>>));
    let mut different_error = Box::pin(cache.get_or_fetch("object".into(), pending::<Result<Bytes, String>>));
    let mut different_key = Box::pin(cache.get_or_fetch("other".into(), pending::<Result<Bytes, &str>>));
    assert_pending(first.as_mut());
    assert_pending(different_error.as_mut());
    assert_pending(different_key.as_mut());
    assert_eq!(pending_count(&cache), 3);
    drop((first, different_error, different_key));
    assert_eq!(pending_count(&cache), 0);
}

#[tokio::test]
async fn cancellation_allows_takeover_and_cleans_up_in_either_order() {
    for order in 0..3 {
        let (cache, _) = cache().await;
        let mut first = Box::pin(cache.get_or_fetch_buffer("object".into(), pending::<Result<AlignedBuffer, &str>>));
        let mut second = Box::pin(cache.get_or_fetch("object".into(), || async {
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
    let (cache, _) = cache().await;
    let release = Notify::new();
    let mut first = Box::pin(cache.get_or_fetch("object".into(), || async {
        release.notified().await;
        Err::<Bytes, &str>("old failure")
    }));
    let mut waiter = Box::pin(cache.get_or_fetch_buffer("object".into(), pending::<Result<AlignedBuffer, &str>>));
    assert_pending(first.as_mut());
    assert_pending(waiter.as_mut());
    release.notify_one();
    assert_eq!(first.await, Err(GetOrFetchError::Callback("old failure")));
    let mut replacement = Box::pin(cache.get_or_fetch("object".into(), pending::<Result<Bytes, &str>>));
    assert_pending(replacement.as_mut());
    assert_eq!(waiter.await, Err(GetOrFetchError::Callback("old failure")));
    assert_eq!(pending_count(&cache), 1);
    drop(replacement);
    assert_eq!(pending_count(&cache), 0);
}

#[test]
fn simultaneous_cancellation_removes_the_last_callers() {
    let pending = PendingFetches::default();
    let first = PendingFetch::<&str>::join(&pending, ObjectKeyHash::from("object"));
    let second = PendingFetch::<&str>::join(&pending, ObjectKeyHash::from("object"));
    let barrier = Barrier::new(2);
    scope(|scope| {
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
    let (cache, registry) = cache().await;
    let mut fetch = Box::pin(cache.get_or_fetch("object".into(), pending::<Result<Bytes, &str>>));
    assert_pending(fetch.as_mut());
    assert_eq!(
        Box::pin(cache.get("object".into()))
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Ready(None)
    );
    for (key, bytes) in [("object", Bytes::from_static(b"object")), ("empty", Bytes::new())] {
        cache
            .inner
            .memory
            .insert(ObjectKeyHash::from(key), bytes.clone(), bytes.len());
        assert_eq!(cache.get(key.into()).await, Some(bytes));
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
