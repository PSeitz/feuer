use super::*;
use crate::test_metrics::{registry, value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn assert_filter_counts(registry: &prometheus::Registry, tier: &str, accepted: u64, rejected: u64) {
    for (outcome, count) in [("accepted", accepted), ("rejected", rejected)] {
        assert_eq!(
            value(
                registry,
                "feuer_admission_filter_total",
                &[("tier", tier), ("outcome", outcome)]
            ),
            count as f64,
        );
    }
}

#[test]
fn unconfigured_filters_record_no_decisions() {
    let (registry, backend) = registry();
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .with_metrics(&backend)
        .open()
        .unwrap();
    cache.insert("object", Bytes::from_static(b"abcd"));
    assert_filter_counts(&registry, "memory", 0, 0);
    #[cfg(all(target_os = "linux", feature = "disk"))]
    assert_filter_counts(&registry, "disk", 0, 0);
}

#[tokio::test]
async fn rejected_fetches_return_bytes_and_keep_demand_for_later_admission() {
    let (registry, backend) = registry();
    let calls = Arc::new(AtomicUsize::new(0));
    let filter_calls = calls.clone();
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .with_metrics(&backend)
        .with_memory_admission_filter(move |key, payload_len, history| {
            assert_eq!(*key, ObjectKeyHash::from("object"));
            assert_eq!(payload_len, 4);
            filter_calls.fetch_add(1, Ordering::Relaxed);
            history.decayed_request_count(key) > 1.5
        })
        .with_disk_admission_filter(|_, _, _| panic!("disk is disabled"))
        .open()
        .unwrap();
    let clone = cache.clone();
    for (cache, admitted) in [(&cache, false), (&clone, true)] {
        let bytes = cache
            .get_or_fetch("object", || async { Ok::<_, &str>(Bytes::from_static(b"abcd")) })
            .await
            .unwrap();
        assert_eq!(bytes, b"abcd"[..]);
        assert_eq!(
            cache.inner.memory.get(&ObjectKeyHash::from("object")).is_some(),
            admitted
        );
    }
    assert_eq!(cache.get("object").await.unwrap(), b"abcd"[..]);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(cache.inner.memory.access_histories().request_count(), 3);
    assert_filter_counts(&registry, "memory", 1, 1);
    #[cfg(all(target_os = "linux", feature = "disk"))]
    assert_filter_counts(&registry, "disk", 0, 0);
}

#[test]
fn insert_filter_receives_payload_length_and_preserves_existing_entries() {
    let accept = Arc::new(AtomicBool::new(true));
    let filter_accept = accept.clone();
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .with_memory_admission_filter(move |key, payload_len, history| {
            assert_eq!(history.request_count(), 0);
            assert_eq!(payload_len, if *key == ObjectKeyHash::from("empty") { 0 } else { 4 });
            filter_accept.load(Ordering::Relaxed)
        })
        .open()
        .unwrap();
    let mut buffer = cache.allocate_buffer(4).unwrap();
    buffer.as_mut().copy_from_slice(b"abcd");
    let capacity = buffer.capacity();
    let address = buffer.as_ref().as_ptr();
    cache.insert("object", buffer);
    accept.store(false, Ordering::Relaxed);
    cache.insert("object", Bytes::from_static(b"abcd"));
    cache.insert("rejected", Bytes::from_static(b"abcd"));
    cache.insert("empty", Bytes::new());
    assert_eq!(
        cache.inner.memory.get(&ObjectKeyHash::from("object")).unwrap().as_ptr(),
        address
    );
    assert_eq!(cache.inner.memory.used_bytes(), capacity as u64);
    assert!(cache.inner.memory.get(&ObjectKeyHash::from("rejected")).is_none());
    assert!(cache.inner.memory.get(&ObjectKeyHash::from("empty")).is_none());
}

#[test]
fn rejected_shared_fetch_runs_admission_once_and_errors_skip_admission() {
    let (registry, backend) = registry();
    let calls = Arc::new(AtomicUsize::new(0));
    let filter_calls = calls.clone();
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .with_metrics(&backend)
        .with_memory_admission_filter(move |_, _, history| {
            assert_eq!(history.request_count(), 2);
            filter_calls.fetch_add(1, Ordering::Relaxed);
            false
        })
        .open()
        .unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut first = Box::pin(cache.get_or_fetch("object", || receiver));
    assert!(first.as_mut().now_or_never().is_none());
    let mut second = Box::pin(cache.get_or_fetch(
        "object",
        || -> std::future::Ready<Result<Bytes, tokio::sync::oneshot::error::RecvError>> { panic!("must share fetch") },
    ));
    assert!(second.as_mut().now_or_never().is_none());
    let bytes = Bytes::from_static(b"abcd");
    sender.send(bytes.clone()).unwrap();
    assert_eq!(first.now_or_never().unwrap().unwrap(), bytes);
    assert_eq!(second.now_or_never().unwrap().unwrap(), bytes);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(cache.inner.memory.get(&ObjectKeyHash::from("object")).is_none());
    assert_eq!(
        cache
            .get_or_fetch("object", || async { Err::<Bytes, _>("failed") })
            .now_or_never()
            .unwrap(),
        Err(GetOrFetchError::Callback("failed"))
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_filter_counts(&registry, "memory", 0, 1);
}

#[cfg(all(target_os = "linux", feature = "disk"))]
#[tokio::test]
async fn memory_and_disk_admission_are_independent_for_inserts_and_fetches() {
    for admit_memory in [false, true] {
        for admit_disk in [false, true] {
            for fetch in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                let (registry, backend) = registry();
                let cache = Cache::builder()
                    .directory(directory.path())
                    .memory_capacity(1 << 20)
                    .disk_capacity(4 << 20)
                    .with_metrics(&backend)
                    .with_memory_admission_filter(move |key, payload_len, history| {
                        assert_eq!(*key, ObjectKeyHash::from("object"));
                        assert_eq!(payload_len, 4);
                        assert_eq!(history.request_count(), u64::from(fetch));
                        admit_memory
                    })
                    .with_disk_admission_filter(move |key, payload_len, history| {
                        assert_eq!(*key, ObjectKeyHash::from("object"));
                        assert_eq!(payload_len, 4);
                        assert_eq!(history.request_count(), u64::from(fetch));
                        admit_disk
                    })
                    .open()
                    .unwrap();
                let bytes = Bytes::from_static(b"abcd");
                if fetch {
                    assert_eq!(
                        cache
                            .get_or_fetch("object", || async { Ok::<_, &str>(bytes) })
                            .await
                            .unwrap(),
                        b"abcd"[..]
                    );
                } else {
                    cache.insert("object", bytes);
                }
                let hash = ObjectKeyHash::from("object");
                assert_eq!(cache.inner.memory.get(&hash).is_some(), admit_memory);
                assert_filter_counts(&registry, "memory", u64::from(admit_memory), u64::from(!admit_memory));
                assert_filter_counts(&registry, "disk", u64::from(admit_disk), u64::from(!admit_disk));
                assert_eq!(
                    value(&registry, "feuer_disk_write_queue_total", &[("outcome", "queued")]),
                    if admit_disk { 1.0 } else { 0.0 }
                );
                let disk = cache.inner.disk.as_ref().unwrap();
                if admit_disk {
                    tokio::time::timeout(std::time::Duration::from_secs(10), async {
                        while !disk.contains(&hash) {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .unwrap();
                    disk.flush().await.unwrap();
                }
                assert_eq!(disk.contains(&hash), admit_disk);
                if admit_memory {
                    cache.insert("object", Bytes::from_static(b"abcd"));
                    assert_filter_counts(&registry, "memory", 2, 0);
                    assert_filter_counts(&registry, "disk", u64::from(admit_disk), u64::from(!admit_disk));
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", feature = "disk"))]
#[tokio::test]
async fn disk_hits_apply_only_memory_admission() {
    let (registry, backend) = registry();
    let directory = tempfile::tempdir().unwrap();
    let key = ObjectKeyHash::from("object");
    let accept = Arc::new(AtomicBool::new(false));
    let filter_accept = accept.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let filter_calls = calls.clone();
    let cache = Cache::builder()
        .directory(directory.path())
        .memory_capacity(1 << 20)
        .disk_capacity(4 << 20)
        .with_metrics(&backend)
        .with_memory_admission_filter(move |hash, payload_len, history| {
            assert_eq!(*hash, key);
            assert_eq!(payload_len, 4);
            assert!(history.decayed_request_count(hash) >= 1.0);
            filter_calls.fetch_add(1, Ordering::Relaxed);
            filter_accept.load(Ordering::Relaxed)
        })
        .with_disk_admission_filter(|_, _, _| panic!("hits must skip disk admission"))
        .open()
        .unwrap();
    cache
        .inner
        .disk
        .as_ref()
        .unwrap()
        .insert_batch(vec![(key, Bytes::from_static(b"abcd"))])
        .await
        .unwrap();
    assert_eq!(cache.get("object").await.unwrap(), b"abcd"[..]);
    assert!(cache.inner.memory.get(&key).is_none());
    assert_eq!(
        cache
            .get_or_fetch("object", || async { Err::<Bytes, _>("must hit disk") })
            .await
            .unwrap(),
        b"abcd"[..]
    );
    assert!(cache.inner.memory.get(&key).is_none());
    accept.store(true, Ordering::Relaxed);
    assert_eq!(cache.get("object").await.unwrap(), b"abcd"[..]);
    assert_eq!(cache.inner.memory.get(&key).unwrap(), b"abcd"[..]);
    assert_eq!(cache.get("object").await.unwrap(), b"abcd"[..]);
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    assert_filter_counts(&registry, "memory", 1, 2);
    assert_filter_counts(&registry, "disk", 0, 0);
}
