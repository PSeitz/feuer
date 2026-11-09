use super::*;
use crate::test_metrics::{registry, value};

#[test]
fn insert_needs_no_runtime_and_records_no_lookup_demand() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let (registry, backend) = registry();
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .with_metrics(&backend)
        .open()
        .now_or_never()
        .unwrap()
        .unwrap();
    for (key, bytes) in [("object", Bytes::from_static(b"payload")), ("empty", Bytes::new())] {
        let key = (key, 100u64..200);
        cache.insert(&key, bytes.clone());
        assert_eq!(cache.inner.memory.access_histories().request_count(), 0);
        assert_eq!(
            value(&registry, "feuer_lookup_total", &[("outcome", "memory_hit")]),
            0.0
        );
        assert_eq!(cache.inner.memory.get(&ObjectKeyHash::from_hash(&key)), Some(bytes));
    }
    assert_eq!(cache.inner.memory.used_bytes(), 7);
    assert_eq!(
        cache
            .get(&("object".to_owned(), 100u64..200))
            .now_or_never()
            .unwrap()
            .unwrap(),
        b"payload"[..]
    );
    assert_eq!(cache.inner.memory.access_histories().request_count(), 1);
}

#[test]
fn insert_preserves_buffer_charge_and_duplicate_memory_entry() {
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .open()
        .now_or_never()
        .unwrap()
        .unwrap();
    let mut buffer = cache.allocate_buffer(4).unwrap();
    buffer.as_mut().copy_from_slice(b"abcd");
    let address = buffer.as_ref().as_ptr();
    let capacity = buffer.capacity();
    cache.insert("object", buffer);
    assert_eq!(cache.inner.memory.used_bytes(), capacity as u64);
    cache.insert("object", Bytes::from_static(b"abcd"));
    assert_eq!(cache.inner.memory.used_bytes(), capacity as u64);
    let hit = cache
        .get_or_fetch("object", || async { Err::<Bytes, _>("must hit memory") })
        .now_or_never()
        .unwrap()
        .unwrap();
    assert_eq!(hit.as_ref(), b"abcd");
    assert_eq!(hit.as_ptr(), address);
    assert!(cache.inner.memory.remove(&ObjectKeyHash::from("object")));
    assert_eq!(cache.inner.memory.used_bytes(), 0);
    assert_eq!(hit.as_ref(), b"abcd");
}

#[test]
fn insert_during_fetch_keeps_inserted_bytes_and_does_not_add_demand() {
    let cache = Cache::builder()
        .memory_capacity(1 << 20)
        .open()
        .now_or_never()
        .unwrap()
        .unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let mut fetch = Box::pin(cache.get_or_fetch("object", || receiver));
    assert!(fetch.as_mut().now_or_never().is_none());
    let inserted = Bytes::from_static(b"payload");
    cache.insert("object", inserted.clone());
    assert_eq!(cache.inner.memory.access_histories().request_count(), 1);
    let downloaded = Bytes::copy_from_slice(&inserted);
    sender.send(downloaded.clone()).unwrap();
    let result = fetch.now_or_never().unwrap().unwrap();
    assert_eq!(result, inserted);
    assert_eq!(result.as_ptr(), downloaded.as_ptr());
    let cached = cache.inner.memory.get(&ObjectKeyHash::from("object")).unwrap();
    assert_eq!(cached.as_ptr(), inserted.as_ptr());
    assert_eq!(cache.inner.memory.used_bytes(), inserted.len() as u64);
}

#[cfg(all(target_os = "linux", feature = "disk"))]
#[test]
fn insert_without_entering_runtime_enqueues_disk_and_keeps_memory_when_queue_is_full() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let cache = runtime
        .block_on(
            Cache::builder()
                .directory(directory.path())
                .disk_capacity(4 << 20)
                .memory_capacity(1 << 20)
                .with_metrics(&backend)
                .open(),
        )
        .unwrap();
    assert!(tokio::runtime::Handle::try_current().is_err());
    let initial_memory_charge = cache.inner.memory.used_bytes();
    // The current-thread runtime is stopped, so the writer cannot drain the queue.
    for index in 0..513 {
        cache.insert(&index.to_string(), Bytes::from_static(b"abcd"));
    }
    cache.insert("0", Bytes::from_static(b"abcd"));
    for (outcome, expected) in [("queued", 512.0), ("queue_full", 1.0), ("redundant", 1.0)] {
        assert_eq!(
            value(&registry, "feuer_disk_write_queue_total", &[("outcome", outcome)]),
            expected
        );
    }
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 512.0);
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 512.0 * 4.0);
    assert_eq!(cache.inner.memory.used_bytes(), initial_memory_charge + 513 * 4);
    assert_eq!(cache.inner.memory.access_histories().request_count(), 0);
    for outcome in ["hit", "absent"] {
        assert_eq!(
            value(&registry, "feuer_disk_lookup_total", &[("outcome", outcome)]),
            0.0
        );
    }
    assert_eq!(
        cache.inner.memory.get(&ObjectKeyHash::from("512")).unwrap(),
        b"abcd"[..]
    );
    runtime.block_on(async {
        let disk = cache.inner.disk.as_ref().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !disk.contains(&ObjectKeyHash::from("511")) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        disk.flush().await.unwrap();
        assert!(cache.inner.memory.remove(&ObjectKeyHash::from("0")));
        assert_eq!(cache.get("0").await.unwrap(), b"abcd"[..]);
        assert!(!disk.contains(&ObjectKeyHash::from("512")));
    });
    // A disk-resident entry still gets synchronous memory admission without a disk lookup.
    assert!(cache.inner.memory.remove(&ObjectKeyHash::from("0")));
    let payload = Bytes::copy_from_slice(b"abcd");
    cache.insert("0", payload.clone());
    assert_eq!(
        cache.inner.memory.get(&ObjectKeyHash::from("0")).unwrap().as_ptr(),
        payload.as_ptr()
    );
    assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 1.0);
    assert_eq!(
        value(&registry, "feuer_disk_write_queue_total", &[("outcome", "queued")]),
        513.0
    );
}
