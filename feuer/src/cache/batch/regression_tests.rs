use super::tests::{assert_pending, cache, unexpected_callback};
use super::*;
use std::{
    future::pending,
    sync::{Barrier, Weak},
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::sync::Notify;

#[tokio::test]
async fn canceled_owner_keeps_unobserved_members_discoverable() {
    for batch_joiner in [false, true] {
        let cache = cache().await;
        let release = Arc::new(Notify::new());
        let source_release = release.clone();
        let mut owner = Box::pin(cache.get_or_fetch_many(&["a", "b"], move |indices| async move {
            assert_eq!(indices, [0, 1]);
            source_release.notified().await;
            vec![Ok::<_, &str>(Bytes::from_static(b"a")), Ok(Bytes::from_static(b"b"))]
        }));
        assert_pending(owner.as_mut());
        let mut survivor = Box::pin(cache.get_or_fetch("b", || async { Err::<Bytes, &str>("must join") }));
        assert_pending(survivor.as_mut());
        drop(owner);
        let mut late = if batch_joiner {
            async { cache.get_or_fetch_many(&["a"], unexpected_callback).await }.boxed()
        } else {
            async {
                vec![
                    cache
                        .get_or_fetch("a", || async { Err::<Bytes, &str>("a is still being fetched") })
                        .await,
                ]
            }
            .boxed()
        };
        assert_pending(late.as_mut());
        release.notify_one();
        assert_eq!(late.await, [Ok(Bytes::from_static(b"a"))]);
        assert_eq!(survivor.await.unwrap(), "b");
        assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
    }
}

// Bytes owns this wrapper, and Weak observes the actual payload allocation.
struct Payload(Arc<Vec<u8>>);
impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

fn payload(value: u8) -> (Bytes, Weak<Vec<u8>>) {
    let allocation = Arc::new(vec![value; 64 << 10]);
    let weak = Arc::downgrade(&allocation);
    (Bytes::from_owner(Payload(allocation)), weak)
}

#[tokio::test]
async fn completed_payloads_drop_independently_after_eviction_or_unretained_admission() {
    for already_cached in [false, true] {
        let cache = cache().await;
        let keys: Vec<_> = (0..16).map(|index| format!("payload-{index}")).collect();
        let (bytes, allocations): (Vec<_>, Vec<_>) = (0..16).map(payload).unzip();
        let release = Arc::new(Notify::new());
        let source_release = release.clone();
        let mut owner = Box::pin(cache.get_or_fetch_many(&keys, move |indices| async move {
            assert_eq!(indices.len(), 16);
            source_release.notified().await;
            bytes.into_iter().map(Ok::<_, &str>).collect()
        }));
        assert_pending(owner.as_mut());
        let mut first = Box::pin(cache.get_or_fetch_many(&["slow-1", "payload-0"], |indices| async move {
            assert_eq!(indices, [0]);
            pending::<Vec<Result<Bytes, &str>>>().await
        }));
        let mut second = Box::pin(cache.get_or_fetch_many(&["slow-2", "payload-1"], |indices| async move {
            assert_eq!(indices, [0]);
            pending::<Vec<Result<Bytes, &str>>>().await
        }));
        assert_pending(first.as_mut());
        assert_pending(second.as_mut());
        let mut driver = Box::pin(cache.get_or_fetch("payload-0", || async { Err::<Bytes, &str>("must join") }));
        assert_pending(driver.as_mut());
        drop(owner);
        if already_cached {
            // Racing insertions make admission leave every downloaded allocation unretained.
            for key in &keys {
                assert!(cache.inner.memory.insert(
                    ObjectKeyHash::from(key.as_str()),
                    Bytes::from_static(b"existing"),
                    8
                ));
            }
        }
        release.notify_one();
        drop(driver.await.unwrap());
        for key in &keys {
            let hash = ObjectKeyHash::from(key.as_str());
            if already_cached {
                assert_eq!(cache.inner.memory.get(&hash).unwrap(), "existing");
            }
            assert!(cache.inner.memory.remove(&hash));
        }
        assert_eq!(cache.inner.memory.used_bytes(), 0);
        // Leave both joined callers unpolled after producer completion. Each must
        // retain only its requested payload even before it observes completion.
        assert!(allocations[0].upgrade().is_some());
        assert!(allocations[1].upgrade().is_some());
        assert!(allocations[2..].iter().all(|allocation| allocation.upgrade().is_none()));
        drop(first);
        assert!(allocations[0].upgrade().is_none());
        assert!(allocations[1].upgrade().is_some());
        drop(second);
        assert!(allocations.iter().all(|allocation| allocation.upgrade().is_none()));
        assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn joined_independent_producers_progress_concurrently_after_owners_cancel() {
    for scalar_first in [false, true] {
        let cache = cache().await;
        let release_a = Arc::new(Notify::new());
        let release_b = Arc::new(Notify::new());
        let rendezvous = Arc::new(tokio::sync::Barrier::new(2));
        let source_release = release_a.clone();
        let source_rendezvous = rendezvous.clone();
        let source_a = async move {
            source_release.notified().await;
            source_rendezvous.wait().await;
            Ok::<_, &str>(Bytes::from_static(b"a"))
        };
        let mut first = if scalar_first {
            async { vec![cache.get_or_fetch("a", move || source_a).await] }.boxed()
        } else {
            cache
                .get_or_fetch_many(&["a", "a-unused"], move |_| async move {
                    vec![source_a.await, Ok(Bytes::from_static(b"unused"))]
                })
                .boxed()
        };
        let source_release = release_b.clone();
        let mut second = Box::pin(cache.get_or_fetch_many(&["b", "b-unused"], move |_| async move {
            source_release.notified().await;
            rendezvous.wait().await;
            vec![
                Ok::<_, &str>(Bytes::from_static(b"b")),
                Ok(Bytes::from_static(b"unused")),
            ]
        }));
        assert_pending(first.as_mut());
        assert_pending(second.as_mut());
        let mut survivor = Box::pin(cache.get_or_fetch_many(&["a", "b"], unexpected_callback));
        assert_pending(survivor.as_mut());
        drop((first, second));
        release_a.notify_one();
        release_b.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), survivor).await.unwrap(),
            [Ok(Bytes::from_static(b"a")), Ok(Bytes::from_static(b"b"))]
        );
        assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn cross_thread_join_completion_and_cancellation() {
    for _ in 0..64 {
        let cache = cache().await;
        let release = Arc::new(Notify::new());
        let source_release = release.clone();
        let mut owner = Box::pin(cache.get_or_fetch_many(&["a", "b"], move |_| async move {
            source_release.notified().await;
            vec![Ok::<_, &str>(Bytes::from_static(b"a")), Ok(Bytes::from_static(b"b"))]
        }));
        assert_pending(owner.as_mut());
        let mut survivor = Box::pin(cache.get_or_fetch_many(&["b"], unexpected_callback));
        assert_pending(survivor.as_mut());
        let start = Barrier::new(3);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                drop(owner);
            });
            scope.spawn(|| {
                start.wait();
                release.notify_one();
                // Another thread may briefly be polling the shared producer.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    assert!(Instant::now() < deadline, "surviving batch stalled");
                    match survivor.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                        Poll::Ready(result) => {
                            assert_eq!(result, [Ok(Bytes::from_static(b"b"))]);
                            break;
                        }
                        Poll::Pending => std::thread::yield_now(),
                    }
                }
            });
            scope.spawn(|| {
                start.wait();
                let mut late = Box::pin(cache.get_or_fetch_many(&["a", "b", "a"], unexpected_callback));
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    assert!(Instant::now() < deadline, "joining batch stalled");
                    match late.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                        Poll::Ready(result) => {
                            assert_eq!(
                                result,
                                [
                                    Ok(Bytes::from_static(b"a")),
                                    Ok(Bytes::from_static(b"b")),
                                    Ok(Bytes::from_static(b"a"))
                                ]
                            );
                            break;
                        }
                        Poll::Pending => std::thread::yield_now(),
                    }
                }
            });
        });
        assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn cross_thread_last_batch_waiters_cancel_the_producer() {
    for _ in 0..64 {
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
        let start = Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                drop(first);
            });
            scope.spawn(|| {
                start.wait();
                drop(second);
            });
        });
        assert!(weak.upgrade().is_none());
        assert!(cache.inner.pending_fetches.lock().unwrap().is_empty());
    }
}
