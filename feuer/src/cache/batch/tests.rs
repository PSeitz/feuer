use super::*;
use crate::test_metrics::{registry, value};
use std::{
    future::pending,
    pin::Pin,
    task::{Context, Waker},
};
use tokio::sync::Notify;

async fn cache() -> Cache {
    Cache::builder().memory_capacity(8 << 20).open().await.unwrap()
}

async fn unexpected_callback(_: Vec<usize>) -> Vec<Result<Bytes, &'static str>> {
    panic!("cache hits and joined fetches must not invoke a callback")
}

fn assert_pending(future: Pin<&mut impl Future>) {
    assert!(future.poll(&mut Context::from_waker(Waker::noop())).is_pending());
}

#[tokio::test]
async fn full_miss_batch_duplicates_hits_and_exact_memory_charge() {
    let (registry, backend) = registry();
    let cache = Cache::builder()
        .memory_capacity(8 << 20)
        .with_metrics(&backend)
        .open()
        .await
        .unwrap();
    let mut keys: Vec<_> = (0..32).map(|index| format!("key-{index}")).collect();
    keys.push(keys[7].clone());
    let result = cache
        .get_or_fetch_many(&keys, |indices| async move {
            assert_eq!(indices, (0..32).collect::<Vec<_>>());
            indices
                .into_iter()
                .map(|index| Ok::<_, &str>(Bytes::from(vec![index as u8; 3])))
                .collect()
        })
        .await;
    for (index, bytes) in result.iter().enumerate() {
        assert_eq!(
            bytes.as_ref().unwrap().as_ref(),
            &[if index == 32 { 7 } else { index as u8 }; 3]
        );
    }
    assert_eq!(
        result[7].as_ref().unwrap().as_ptr(),
        result[32].as_ref().unwrap().as_ptr()
    );
    assert_eq!(cache.inner.memory.used_bytes(), 32 * 3);
    assert_eq!(cache.inner.memory.access_histories().request_count(), 33);
    assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", "callback")]), 33.0);
    let hits = cache.get_or_fetch_many(&keys, unexpected_callback).await;
    assert_eq!(hits, result);
    assert_eq!(cache.inner.memory.access_histories().request_count(), 66);
    assert_eq!(
        value(&registry, "feuer_lookup_total", &[("outcome", "memory_hit")]),
        33.0
    );
    assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mixed_hits_errors_empty_payloads_and_retry() {
    let cache = cache().await;
    cache
        .get_or_fetch("hit", || async { Ok::<_, &str>(Bytes::from_static(b"cached")) })
        .await
        .unwrap();
    let keys = ["hit", "error", "empty", "error"];
    let result = cache
        .get_or_fetch_many(&keys, |indices| async move {
            assert_eq!(indices, [1, 2]);
            vec![Err("failed"), Ok(Bytes::new())]
        })
        .await;
    assert_eq!(
        result,
        [
            Ok(Bytes::from_static(b"cached")),
            Err(GetOrFetchError::Callback("failed")),
            Ok(Bytes::new()),
            Err(GetOrFetchError::Callback("failed"))
        ]
    );
    let retry = cache
        .get_or_fetch_many(&keys, |indices| async move {
            assert_eq!(indices, [1]);
            vec![Ok::<_, &str>(Bytes::from_static(b"retry"))]
        })
        .await;
    assert_eq!(retry[1], Ok(Bytes::from_static(b"retry")));
    assert_eq!(retry[1], retry[3]);
    assert_eq!(cache.inner.memory.access_histories().request_count(), 9);
    let empty = cache.get_or_fetch_many(&[] as &[&str], unexpected_callback).await;
    assert!(empty.is_empty());
}

#[tokio::test]
async fn scalar_waiter_drives_original_batch_after_owner_cancels() {
    let cache = cache().await;
    let release = Arc::new(Notify::new());
    let fetch_release = release.clone();
    let guard = Arc::new(());
    let weak = Arc::downgrade(&guard);
    let mut batch = Box::pin(cache.get_or_fetch_many(&["one", "two"], move |indices| async move {
        assert_eq!(indices, [0, 1]);
        fetch_release.notified().await;
        drop(guard);
        vec![
            Ok::<_, &str>(Bytes::from_static(b"one")),
            Ok(Bytes::from_static(b"two")),
        ]
    }));
    assert_pending(batch.as_mut());
    let mut scalar = Box::pin(cache.get_or_fetch("two", || async { Err::<Bytes, &str>("must share") }));
    assert_pending(scalar.as_mut());
    drop(batch);
    assert!(weak.upgrade().is_some());
    release.notify_one();
    assert_eq!(scalar.await.unwrap(), "two");
    assert!(weak.upgrade().is_none());
    assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
    assert_eq!(cache.inner.memory.access_histories().request_count(), 3);
}

#[tokio::test]
async fn last_waiter_cancels_whole_source_batch_in_either_drop_order() {
    for owner_first in [false, true] {
        let cache = cache().await;
        let guard = Arc::new(());
        let weak = Arc::downgrade(&guard);
        let mut first = Box::pin(cache.get_or_fetch_many(&["a", "b"], move |_| async move {
            let _guard = guard;
            pending::<Vec<Result<Bytes, &str>>>().await
        }));
        let mut second = Box::pin(cache.get_or_fetch_many(&["b", "a"], unexpected_callback));
        assert_pending(first.as_mut());
        assert_pending(second.as_mut());
        if owner_first {
            drop(first);
            assert!(weak.upgrade().is_some());
            drop(second);
        } else {
            drop(second);
            assert!(weak.upgrade().is_some());
            drop(first);
        }
        assert!(weak.upgrade().is_none());
        assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
        assert_eq!(cache.inner.memory.access_histories().request_count(), 4);
        assert_eq!(cache.inner.memory.used_bytes(), 0);
    }
}

#[tokio::test]
async fn overlapping_batches_forward_new_misses_before_old_fetch_completes() {
    let cache = cache().await;
    let release = Arc::new(Notify::new());
    let fetch_release = release.clone();
    let mut first = Box::pin(cache.get_or_fetch_many(&["a", "b"], move |indices| async move {
        assert_eq!(indices, [0, 1]);
        fetch_release.notified().await;
        vec![Ok::<_, &str>(Bytes::from_static(b"a")), Ok(Bytes::from_static(b"b"))]
    }));
    assert_pending(first.as_mut());
    let mut second = Box::pin(cache.get_or_fetch_many(&["b", "c", "a", "d"], |indices| async move {
        assert_eq!(indices, [1, 3]);
        vec![Ok::<_, &str>(Bytes::from_static(b"c")), Err("d failed")]
    }));
    assert_pending(second.as_mut());
    assert_eq!(cache.inner.memory.get(&ObjectKeyHash::from("c")).unwrap(), "c");
    // Completed errors are retryable even while this batch still waits on an older fetch.
    assert_eq!(
        cache
            .get_or_fetch("d", || async { Err::<Bytes, &str>("fresh failure") })
            .await,
        Err(GetOrFetchError::Callback("fresh failure"))
    );
    drop(first);
    release.notify_one();
    let result = second.await;
    assert_eq!(
        result,
        [
            Ok(Bytes::from_static(b"b")),
            Ok(Bytes::from_static(b"c")),
            Ok(Bytes::from_static(b"a")),
            Err(GetOrFetchError::Callback("d failed"))
        ]
    );
    assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn batch_waiter_preserves_scalar_fetch_and_shares_errors() {
    let cache = cache().await;
    let release = Arc::new(Notify::new());
    let fetch_release = release.clone();
    let mut scalar = Box::pin(cache.get_or_fetch("a", move || async move {
        fetch_release.notified().await;
        Err::<Bytes, &str>("original scalar error")
    }));
    assert_pending(scalar.as_mut());
    let mut batch = Box::pin(cache.get_or_fetch_many(&["a", "b", "a"], |indices| async move {
        assert_eq!(indices, [1]);
        vec![Ok::<_, &str>(Bytes::from_static(b"b"))]
    }));
    assert_pending(batch.as_mut());
    drop(scalar);
    release.notify_one();
    assert_eq!(
        batch.await,
        [
            Err(GetOrFetchError::Callback("original scalar error")),
            Ok(Bytes::from_static(b"b")),
            Err(GetOrFetchError::Callback("original scalar error"))
        ]
    );
    assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn aligned_buffers_share_with_byte_callbacks_and_charge_capacity() {
    let cache = cache().await;
    let mut buffer = cache.allocate_buffer(3).unwrap();
    let capacity = buffer.capacity();
    let release = Arc::new(Notify::new());
    let fetch_release = release.clone();
    let mut batch = Box::pin(cache.get_or_fetch_many(&["buffer"], move |_| async move {
        fetch_release.notified().await;
        buffer.as_mut().copy_from_slice(b"abc");
        vec![Ok::<_, &str>(buffer)]
    }));
    assert_pending(batch.as_mut());
    let mut scalar = Box::pin(cache.get_or_fetch("buffer", || async { Err::<Bytes, &str>("must share") }));
    assert_pending(scalar.as_mut());
    release.notify_one();
    let result = batch.await;
    let bytes = scalar.await.unwrap();
    assert_eq!(bytes, "abc");
    assert_eq!(result[0].as_ref().unwrap().as_ptr(), bytes.as_ptr());
    assert_eq!(cache.inner.memory.used_bytes(), capacity as u64);
    assert_eq!(cache.inner.memory.access_histories().request_count(), 2);
}

#[tokio::test]
async fn completed_batch_waiter_cannot_remove_a_retry() {
    let cache = cache().await;
    let release = Arc::new(Notify::new());
    let fetch_release = release.clone();
    let mut first = Box::pin(cache.get_or_fetch_many(&["a", "b"], move |_| async move {
        fetch_release.notified().await;
        vec![Err::<Bytes, &str>("old"), Err("old")]
    }));
    let mut waiter = Box::pin(cache.get_or_fetch_many(&["b", "a"], unexpected_callback));
    assert_pending(first.as_mut());
    assert_pending(waiter.as_mut());
    release.notify_one();
    assert!(first.await.iter().all(Result::is_err));
    let mut retry = Box::pin(cache.get_or_fetch_many(&["a", "b"], |_| pending::<Vec<Result<Bytes, &str>>>()));
    assert_pending(retry.as_mut());
    assert!(waiter.await.iter().all(Result::is_err));
    assert_eq!(cache.inner.pending_fetches.lock().unwrap().len(), 2);
    drop(retry);
    assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
}

#[tokio::test]
#[should_panic(expected = "batch callback must return one result per miss")]
async fn incorrect_callback_cardinality_is_a_contract_violation() {
    cache()
        .await
        .get_or_fetch_many(&["a"], |_| async { Vec::<Result<Bytes, &str>>::new() })
        .await;
}
