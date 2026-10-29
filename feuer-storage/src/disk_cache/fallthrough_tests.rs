use std::{future::Future, pin::Pin, task::Poll};

use super::*;
use crate::{
    IoQueues,
    test_metrics::{registry, value},
};

async fn open() -> (tempfile::TempDir, DiskCache, IoQueues, prometheus::Registry) {
    let directory = tempfile::tempdir().unwrap();
    let queues = IoQueues::new().unwrap();
    let (registry, backend) = registry();
    let cache = DiskCache::open(
        directory.path(),
        4 * CHUNK_BYTES,
        DiskCacheOptions {
            io_queues: Some(queues.clone()),
            io_metrics: IoMetrics::new(&backend),
            metrics: DiskMetrics::new(&backend),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (directory, tests::with_manual_metadata_writes(cache), queues, registry)
}

async fn assert_pending(mut future: Pin<&mut impl Future>) {
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
            .await
            .is_pending()
    );
}

#[tokio::test]
async fn reaching_fallthrough_threshold_checks_earlier_reads_without_issuing_io() {
    let (_directory, cache, queues, registry) = open().await;
    let source = tests::download(0, 8192);
    let keys: Vec<_> = (0..64).map(ObjectKeyHash).collect();
    cache
        .insert_batch(keys.iter().map(|key| (*key, source.clone())).collect())
        .await
        .unwrap();
    let before = value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]);
    let read_permits = queues.hold_all_read_permits();
    let calls = std::cell::Cell::new(0);
    let mut reads: Vec<_> = keys
        .iter()
        .map(|key| {
            Box::pin(cache.get_with_fallthrough(key, || {
                calls.set(calls.get() + 1);
                true
            }))
        })
        .collect();
    let last = reads.pop().unwrap();
    for read in &mut reads {
        assert_pending(read.as_mut()).await;
    }
    assert_eq!(calls.get(), 0);
    assert!(last.await.is_none());
    for read in reads {
        assert!(read.await.is_none());
    }
    assert_eq!(calls.get(), keys.len());
    assert_eq!(
        value(&registry, "feuer_disk_lookup_total", &[("outcome", "fallthrough")]),
        keys.len() as f64
    );
    assert_eq!(
        value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]),
        before
    );
    for key in keys {
        assert!(cache.contains(&key));
    }
    drop(read_permits);
}

#[tokio::test]
async fn declining_fallthrough_reads_disk() {
    let (_directory, cache, queues, _registry) = open().await;
    let key = ObjectKeyHash::from("object");
    let source = tests::download(0, 8192);
    cache.insert(key, source.clone()).await.unwrap();
    let held_reads = queues.hold_queued_reads();
    let (bytes, _) = cache
        .get_with_fallthrough(&key, || {
            drop(held_reads);
            false
        })
        .await
        .unwrap();
    assert_eq!(bytes, source);
}

#[tokio::test]
async fn absent_buffered_and_uncongested_reads_do_not_consult_fallthrough() {
    let (_directory, cache, queues, _registry) = open().await;
    let key = ObjectKeyHash::from("object");
    let source = tests::download(0, 8192);
    cache.insert(key, source.clone()).await.unwrap();
    let (bytes, _) = cache
        .get_with_fallthrough(&key, || panic!("queue has room"))
        .await
        .unwrap();
    assert_eq!(bytes, source);

    let held_reads = queues.hold_queued_reads();
    let missing = ObjectKeyHash::from("missing");
    assert!(
        cache
            .get_with_fallthrough(&missing, || panic!("no disk entry"))
            .await
            .is_none()
    );
    let buffered = ObjectKeyHash::from("buffered");
    cache.enqueue_if_space_available(buffered, source.clone());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !cache.contains(&buffered) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (bytes, _) = cache
        .get_with_fallthrough(&buffered, || panic!("buffered hit"))
        .await
        .unwrap();
    assert_eq!(bytes, source);
    drop(held_reads);
}

#[tokio::test]
async fn shared_read_waiter_does_not_consult_fallthrough() {
    let (_directory, cache, queues, _registry) = open().await;
    let key = ObjectKeyHash::from("shared");
    let source = tests::download(0, 8192);
    cache.insert(key, source.clone()).await.unwrap();
    let held_reads = queues.hold_queued_reads();
    let mut initializer = Box::pin(cache.get_with_fallthrough(&key, || false));
    assert_pending(initializer.as_mut()).await;
    let mut waiter = Box::pin(cache.get_with_fallthrough(&key, || panic!("shared read")));
    assert_pending(waiter.as_mut()).await;
    drop(held_reads);
    let (first, second) = tokio::join!(initializer, waiter);
    assert_eq!(first.unwrap().0, source);
    assert_eq!(second.unwrap().0, source);
}

#[tokio::test]
async fn canceled_initializer_allows_independent_waiter_decisions() {
    let (_directory, cache, queues, _registry) = open().await;
    let key = ObjectKeyHash::from("shared");
    let source = tests::download(0, 8192);
    cache.insert(key, source.clone()).await.unwrap();
    let held_reads = queues.hold_queued_reads();
    let mut initializer = Box::pin(cache.get_with_fallthrough(&key, || false));
    assert_pending(initializer.as_mut()).await;
    let mut fallthrough = Box::pin(cache.get_with_fallthrough(&key, || true));
    let mut reader = Box::pin(cache.get_with_fallthrough(&key, || {
        drop(held_reads);
        false
    }));
    assert_pending(fallthrough.as_mut()).await;
    assert_pending(reader.as_mut()).await;
    drop(initializer);
    assert!(fallthrough.await.is_none());
    // Fallthrough must not become a shared miss for the caller that chooses disk.
    assert_eq!(reader.await.unwrap().0, source);
}
