use super::*;
use crate::test_metrics::{registry, value};

fn download(length: usize) -> Download {
    Download::new(0, Bytes::from(vec![7; length])).unwrap()
}

async fn measured_cache(capacity: u64) -> (tempfile::TempDir, DiskRangeCache, prometheus::Registry) {
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let cache = DiskRangeCache::open_with_metrics(
        directory.path(),
        capacity,
        IoMetrics::new(&backend),
        Arc::new(ObjectAccessHistories::new()),
        DiskMetrics::new(&backend),
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    (directory, cache, registry)
}

#[tokio::test]
async fn recovery_counts_indexed_entries_not_writes_or_reads() {
    let capacity = 8 * CHUNK_BYTES;
    let (directory, cache, registry) = measured_cache(capacity).await;
    let inputs = vec![
        ("small-a".to_owned(), download(4)),
        ("small-b".to_owned(), download(8)),
        ("large".to_owned(), download(2 * CHUNK_BYTES as usize + 17)),
    ];
    cache.insert_batch(inputs.clone()).await.unwrap();
    assert_eq!(value(&registry, "feuer_disk_recovery_entries_total", &[]), 0.0);
    cache.disk.save_recovery_ends().unwrap();
    let metrics = cache.disk.metrics.clone();
    drop(cache);

    let cache = DiskRangeCache::open_with_metrics(
        directory.path(),
        capacity,
        IoMetrics::noop(),
        Arc::new(ObjectAccessHistories::new()),
        metrics,
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    recovery::tests::wait_for_recovery(&cache).await;
    assert_eq!(value(&registry, "feuer_disk_recovery_entries_total", &[]), 3.0);
    for (key, source) in inputs {
        assert_eq!(
            cache.get(&key, source.downloaded_range()).await.unwrap(),
            source.bytes()
        );
    }
    drop(cache);
    assert_eq!(value(&registry, "feuer_disk_recovery_entries_total", &[]), 3.0);
}

#[tokio::test]
async fn concurrent_slices_share_one_read_and_survive_initializer_cancellation() {
    use std::{future::Future, task::Poll};

    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = "object".to_owned();
    let source = Download::new(10, Bytes::from_static(b"abcdefgh")).unwrap();
    cache.insert_batch(vec![(key.clone(), source)]).await.unwrap();

    // Hold initialization pending so both lookups deterministically join the same read.
    let result = Arc::new(OnceCell::new());
    {
        let mut index = cache.disk.shards[0].entry_index.lock().unwrap();
        index
            .ranges_by_key
            .get_mut(&key)
            .unwrap()
            .get_mut(&10)
            .unwrap()
            .read_result = Arc::downgrade(&result);
    }
    let mut initializer = Box::pin(result.get_or_init(std::future::pending));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(initializer.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let mut first = Box::pin(cache.get(&key, ByteRange::new(11, 14).unwrap()));
    let mut second = Box::pin(cache.get(&key, ByteRange::new(15, 18).unwrap()));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(first.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(Arc::strong_count(&result), 3);
    let weak = Arc::downgrade(&result);
    drop(initializer);
    drop(result);

    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.unwrap(), Bytes::from_static(b"bcd"));
    assert_eq!(second.unwrap(), Bytes::from_static(b"fgh"));
    assert!(weak.upgrade().is_none(), "completed reads must not retain the payload");
    let reads = || {
        value(
            &registry,
            "feuer_disk_io_total",
            &[("operation", "read"), ("outcome", "success")],
        )
    };
    assert_eq!(reads(), 1.0);
    assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 2.0);

    // A later lookup must issue another read rather than use a hidden memory cache.
    assert_eq!(
        cache.get(&key, ByteRange::new(10, 11).unwrap()).await.unwrap(),
        b"a"[..]
    );
    assert_eq!(reads(), 2.0);
}

#[tokio::test]
async fn records_write_outcomes_packing_and_index_usage() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = "object".to_owned();
    let request = ByteRange::new(0, 1).unwrap();
    let outcome = |label| value(&registry, "feuer_disk_write_entries_total", &[("outcome", label)]);
    assert_eq!(
        cache
            .insert_batch(vec![(key.clone(), download(4)), ("neighbor".into(), download(8))])
            .await
            .unwrap(),
        2
    );
    assert_eq!(outcome("published"), 2.0);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 12.0);
    assert_eq!(
        value(&registry, "feuer_disk_batch_bytes_total", &[("kind", "payload")]),
        12.0
    );
    assert_eq!(
        value(&registry, "feuer_disk_batch_bytes_total", &[("kind", "chunk")]),
        CHUNK_BYTES as f64
    );
    assert_eq!(cache.insert_batch(vec![(key.clone(), download(4))]).await.unwrap(), 0);
    assert_eq!(outcome("already_covered"), 1.0);
    assert_eq!(
        cache
            .insert_batch_checked(vec![("stale".into(), download(4), ())], |_, _| {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(outcome("stale"), 1.0);
    assert_eq!(value(&registry, "feuer_disk_written_entries_total", &[]), 3.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 1.0);
    assert_eq!(
        cache
            .insert_batch(vec![("oversized".into(), download(4 * CHUNK_BYTES as usize))])
            .await
            .unwrap(),
        0
    );
    assert_eq!(outcome("no_capacity"), 1.0);
    assert!(cache.get(&key, request).await.is_some());
    assert!(cache.get(&"absent".into(), request).await.is_none());
    for label in ["hit", "absent"] {
        assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", label)]), 1.0);
    }
    assert_eq!(
        value(&registry, "feuer_disk_lookup_duration_seconds", &[("outcome", "hit")]),
        1.0
    );
    drop(cache);
    for name in ["feuer_disk_entries", "feuer_disk_payload_bytes"] {
        assert_eq!(value(&registry, name, &[]), 0.0);
    }
    for state in ["free", "allocated"] {
        assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", state)]), 0.0);
    }
}

#[tokio::test]
async fn eviction_triggering_insertions_count_entries_not_victims_or_batches() {
    let (_directory, cache, registry) = measured_cache(CHUNK_BYTES).await;
    let triggering = || value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]);
    cache
        .insert_batch(vec![("first".into(), download(4)), ("second".into(), download(4))])
        .await
        .unwrap();
    assert_eq!(triggering(), 0.0);
    cache
        .insert_batch(vec![("third".into(), download(4)), ("fourth".into(), download(4))])
        .await
        .unwrap();
    // The first incoming entry evicts both old chunk owners. Its batch neighbor evicts nothing.
    assert!(!cache.contains(&"first".into(), ByteRange::new(0, 1).unwrap()));
    assert!(!cache.contains(&"second".into(), ByteRange::new(0, 1).unwrap()));
    assert_eq!(triggering(), 1.0);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
        4.0
    );
    cache.insert_batch(vec![("third".into(), download(4))]).await.unwrap();
    assert_eq!(triggering(), 1.0);

    // Evictions still count when a read guard prevents reuse and insertion fails.
    let guard =
        cache.disk.shards[0].entry_index.lock().unwrap().ranges_by_key["third"][&0].payload_regions[0].read_guard();
    assert_eq!(
        cache.insert_batch(vec![("blocked".into(), download(4))]).await.unwrap(),
        0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    assert_eq!(triggering(), 2.0);
    assert_eq!(
        value(
            &registry,
            "feuer_disk_write_entries_total",
            &[("outcome", "no_capacity")]
        ),
        1.0
    );
    // No victims remain, so an unsuccessful eviction search must not count.
    cache
        .insert_batch(vec![("still-blocked".into(), download(4))])
        .await
        .unwrap();
    assert_eq!(triggering(), 2.0);
    drop(guard);
}

#[tokio::test]
async fn distinguishes_checksum_failures_from_io_errors_and_removes_index_usage() {
    let (directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
    let request = ByteRange::new(0, 1).unwrap();
    let key = "corrupt".to_owned();
    cache.insert_batch(vec![(key.clone(), download(4))]).await.unwrap();
    let address = cache.disk.shards[0].entry_index.lock().unwrap().ranges_by_key[&key][&0].payload_regions[0]
        .range()
        .start;
    cache
        .disk
        .file
        .write_at(address, &Bytes::from(vec![0; 4096]))
        .await
        .unwrap();
    assert!(cache.get(&key, request).await.is_none());
    assert_eq!(
        value(
            &registry,
            "feuer_disk_lookup_total",
            &[("outcome", "checksum_failed")]
        ),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    cache.insert_batch(vec![(key.clone(), download(4))]).await.unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(cache.get(&key, request).await.is_none());
    assert_eq!(
        value(&registry, "feuer_disk_lookup_total", &[("outcome", "io_error")]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 2.0);
}

#[tokio::test]
async fn pressure_eviction_is_not_replacement_and_failed_writes_are_not_published() {
    let (_directory, cache, registry) = measured_cache(CHUNK_BYTES).await;
    cache.insert_batch(vec![("first".into(), download(4))]).await.unwrap();
    cache.insert_batch(vec![("second".into(), download(4))]).await.unwrap();
    assert_eq!(
        value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    // As in the existing write-failure test, let allocation exceed the actual file.
    let mut disk = Arc::try_unwrap(cache.disk).ok().unwrap();
    disk.shards[0].entry_index.lock().unwrap().remove("second", 0);
    disk.shards[0].allocator = DiskChunkAllocator::with_metrics(0..3 * CHUNK_BYTES, disk.metrics.clone()).unwrap();
    let cache = DiskRangeCache { disk: Arc::new(disk) };
    assert!(
        cache
            .insert_batch(vec![
                ("small".into(), download(1)),
                ("large".into(), download(CHUNK_BYTES as usize))
            ])
            .await
            .is_err()
    );
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "failed")]),
        2.0
    );
    assert_eq!(value(&registry, "feuer_disk_written_entries_total", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 3.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
}

#[test]
fn abandoned_write_attempts_count_once_as_canceled() {
    let (registry, backend) = registry();
    let metrics = DiskMetrics::new(&backend);
    let mut canceled = DiskWriteAttempt::new(metrics.clone());
    canceled.evicted = true;
    drop(canceled);
    assert_eq!(
        value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]),
        1.0
    );
    let mut finished = DiskWriteAttempt::new(metrics);
    finished.set_outcome(DiskWriteOutcome::Published);
    drop(finished);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "canceled")]),
        1.0
    );
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
        1.0
    );
}
