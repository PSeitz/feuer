use super::*;
use crate::test_metrics::{registry, value};

fn download(length: usize) -> Download {
    Download::new(0, Bytes::from(vec![7; length])).unwrap()
}

async fn measured_cache(capacity: u64) -> (tempfile::TempDir, DiskCache, prometheus::Registry) {
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let cache = DiskCache::open_with_metrics(
        directory.path(),
        capacity,
        IoMetrics::new(&backend),
        Arc::new(ObjectAccessHistories::new()),
        DiskMetrics::new(&backend),
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    (directory, tests::with_manual_metadata_writes(cache), registry)
}

#[tokio::test]
async fn periodic_writer_combines_updates_into_page_writes() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    for key in [ObjectKeyHash(1), ObjectKeyHash(2)] {
        cache.insert(key, download(1)).await.unwrap();
    }
    let written_bytes = || value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]);
    let payload_bytes = 2 * CHUNK_BYTES;
    assert_eq!(written_bytes(), payload_bytes as f64);
    let weak = Arc::downgrade(&cache.disk);
    tokio::spawn(DiskCacheInner::write_metadata_periodically(weak.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        // Wait for both the shared record page and the link page, not a whole-chunk write.
        while written_bytes() < (payload_bytes + 2 * METADATA_PAGE_BYTES as u64) as f64 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(written_bytes(), (payload_bytes + 2 * METADATA_PAGE_BYTES as u64) as f64);
    drop(cache);
    assert!(
        weak.upgrade().is_none(),
        "the sleeping writer must not retain the cache"
    );
}

#[tokio::test]
async fn recovery_reads_one_full_metadata_chunk_independent_of_payload_size() {
    for (entries, payload_bytes, chunks) in [
        (1, 1024, 1),
        (84, 1024, 1),
        (85, 1024, 1),
        (168, 1024, 1),
        (169, 1024, 1),
        (252, 1024, 1),
        (1, CHUNK_BYTES as usize - METADATA_PAGE_BYTES, 1),
        (1, 2 * CHUNK_BYTES as usize, 1),
        (page_format::ENTRIES_PER_METADATA_CHUNK as u128 + 1, 1, 2),
    ] {
        let capacity = if chunks == 2 { 100 } else { 4 } * CHUNK_BYTES;
        let (directory, cache) = tests::open_test_cache(capacity).await;
        let source = download(payload_bytes);
        let inputs: Vec<_> = (0..entries).map(|key| (ObjectKeyHash(key), source.clone())).collect();
        assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), entries as usize);
        cache.write_dirty_metadata_pages().await;
        let capacity = cache.disk.file.capacity();
        drop(cache);

        let (registry, backend) = registry();
        let cache = DiskCache::open(directory.path(), capacity, IoMetrics::new(&backend))
            .await
            .unwrap();
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "read"), ("outcome", "success")]
            ),
            chunks as f64
        );
        assert_eq!(
            value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]),
            (chunks * CHUNK_BYTES) as f64
        );
        assert!(
            cache
                .disk
                .shards
                .iter()
                .all(|shard| shard.metadata_pages.lock().unwrap().dirty_pages.is_empty())
        );
        for (key, source) in inputs {
            assert!(cache.covers_range(&key, source.downloaded_range()));
        }
    }
}

#[tokio::test]
async fn recovered_chunk_gauge_counts_shared_and_multi_chunk_ownership() {
    let capacity = 8 * CHUNK_BYTES;
    let (directory, cache, registry) = measured_cache(capacity).await;
    let inputs = vec![
        (ObjectKeyHash::from("small-a"), download(4)),
        (ObjectKeyHash::from("small-b"), download(8)),
        (ObjectKeyHash::from("large"), download(2 * CHUNK_BYTES as usize + 17)),
    ];
    cache.insert_batch(inputs.clone()).await.unwrap();
    assert_eq!(
        value(
            &registry,
            "feuer_disk_io_total",
            &[("operation", "write"), ("outcome", "success")]
        ),
        2.0 // One shared payload run and one three-chunk payload run.
    );
    assert_eq!(
        value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]),
        (4 * CHUNK_BYTES) as f64
    );
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), 0.0);
    let written_chunks = value(&registry, "feuer_disk_chunks", &[("state", "allocated")]);
    assert_eq!(written_chunks, 5.0); // Metadata, one shared payload chunk, and three large-entry chunks.
    let metrics = cache.disk.metrics.clone();
    cache.write_dirty_metadata_pages().await;
    drop(cache);

    let cache = DiskCache::open_with_metrics(
        directory.path(),
        capacity,
        IoMetrics::noop(),
        Arc::new(ObjectAccessHistories::new()),
        metrics,
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), written_chunks);
    for (key, source) in inputs {
        assert_eq!(
            cache.get(&key, source.downloaded_range()).await.unwrap(),
            source.bytes()
        );
    }
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), written_chunks);
    drop(cache);
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), 0.0);
}

#[tokio::test]
async fn concurrent_slices_share_one_read_and_survive_initializer_cancellation() {
    use std::{future::Future, task::Poll};

    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    let source = Download::new(10, Bytes::from_static(b"abcdefgh")).unwrap();
    cache.insert_batch(vec![(key, source)]).await.unwrap();

    // Hold initialization pending so both lookups deterministically join the same read.
    let read = cache.disk.shards[0]
        .entry_index
        .lock()
        .unwrap()
        .covering_entry(&key, ByteRange::new(10, 18).unwrap())
        .unwrap()
        .share_payload_read();
    let mut initializer = Box::pin(read.result.get_or_init(std::future::pending));
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
    assert_eq!(Arc::strong_count(&read), 3);
    let weak = Arc::downgrade(&read);
    drop(initializer);
    drop(read);

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
    assert_eq!(reads(), 2.0); // One startup metadata read and one shared payload read.
    assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 2.0);

    // A later lookup must issue another read rather than use a hidden memory cache.
    assert_eq!(
        cache.get(&key, ByteRange::new(10, 11).unwrap()).await.unwrap(),
        b"a"[..]
    );
    assert_eq!(reads(), 3.0);
}

#[tokio::test]
async fn records_write_outcomes_packing_and_index_usage() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    let request = ByteRange::new(0, 1).unwrap();
    let outcome = |label| value(&registry, "feuer_disk_write_entries_total", &[("outcome", label)]);
    assert_eq!(
        cache
            .insert_batch(vec![(key, download(4)), ("neighbor".into(), download(8))])
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
    assert_eq!(cache.insert_batch(vec![(key, download(4))]).await.unwrap(), 0);
    assert_eq!(outcome("already_covered"), 1.0);
    assert_eq!(value(&registry, "feuer_disk_written_entries_total", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 2.0);
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
    let (_directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
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
    assert!(!cache.covers_range(&"first".into(), ByteRange::new(0, 1).unwrap()));
    assert!(!cache.covers_range(&"second".into(), ByteRange::new(0, 1).unwrap()));
    assert_eq!(triggering(), 1.0);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
        4.0
    );
    cache.insert_batch(vec![("third".into(), download(4))]).await.unwrap();
    assert_eq!(triggering(), 1.0);

    assert_eq!(cache.insert_batch(vec![("next".into(), download(4))]).await.unwrap(), 1);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    assert_eq!(triggering(), 2.0);
}

#[tokio::test]
async fn checksum_failures_remove_entries_but_io_errors_preserve_them() {
    let (directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
    let request = ByteRange::new(0, 1).unwrap();
    let key = ObjectKeyHash::from("corrupt");
    cache.insert_batch(vec![(key, download(4))]).await.unwrap();
    let address = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key][&0].payload_address;
    cache
        .disk
        .file
        .write_at(address, &Bytes::from(vec![0; 4096]))
        .await
        .unwrap();
    assert!(cache.get(&key, request).await.is_none());
    assert_eq!(
        value(&registry, "feuer_disk_lookup_total", &[("outcome", "checksum_failed")]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 0.0);
    // Only metadata remains reserved; payload reuse does not wait for invalidation writes.
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 1.0);
    cache.insert_batch(vec![(key, download(4))]).await.unwrap();
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
    assert!(cache.covers_range(&key, request));
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 0.0);
}

#[tokio::test]
async fn pressure_eviction_and_write_failure_record_each_entry_outcome() {
    let (_directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
    let outcome = |label| value(&registry, "feuer_disk_write_entries_total", &[("outcome", label)]);
    cache.insert_batch(vec![("first".into(), download(4))]).await.unwrap();
    cache.insert_batch(vec![("second".into(), download(4))]).await.unwrap();
    assert_eq!(
        value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    // As in the existing write-failure test, let allocation exceed the actual file.
    let mut disk = Arc::try_unwrap(cache.disk).ok().unwrap();
    let shard = &disk.shards[0];
    shard.remove_entry(&ObjectKeyHash::from("second"), 0);
    disk.shards[0].allocator = DiskChunkAllocator::with_metrics(CHUNK_BYTES..4 * CHUNK_BYTES, disk.metrics.clone());
    let cache = DiskCache {
        disk: Arc::new(disk),
        write_sender: cache.write_sender,
    };
    assert!(
        cache
            .insert_batch(vec![
                ("small".into(), download(1)),
                ("large".into(), download(2 * CHUNK_BYTES as usize))
            ])
            .await
            .is_err()
    );
    assert_eq!(outcome("failed"), 1.0);
    assert_eq!(value(&registry, "feuer_disk_written_entries_total", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    assert_eq!(outcome("published"), 2.0);
    assert!(cache.covers_range(&"small".into(), ByteRange::new(0, 1).unwrap()));
    cache.discard_pending().await;
    assert_eq!(outcome("canceled"), 1.0);
    // The small entry never reserved disk space; discard only releases its buffered bytes and accounting.
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 1.0);
}

#[tokio::test]
async fn buffered_entries_only_attempt_disk_admission_when_flushed() {
    let (_directory, cache, registry) = measured_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    let range = ByteRange::new(0, 1).unwrap();
    assert_eq!(cache.flush().await.unwrap(), 0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
    cache.write(key, download(1), ()).await.unwrap();
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
    assert_eq!(cache.get(&key, range).await.unwrap(), download(1).bytes());
    assert_eq!(cache.flush().await.unwrap(), 0); // Only metadata fits on disk.
    assert!(!cache.covers_range(&key, range));
    assert_eq!(
        value(
            &registry,
            "feuer_disk_write_entries_total",
            &[("outcome", "no_capacity")]
        ),
        1.0
    );
    cache.write(key, download(1), ()).await.unwrap();
    cache.discard_pending().await;
    assert!(!cache.covers_range(&key, range));
    assert_eq!(cache.flush().await.unwrap(), 0);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "canceled")]),
        1.0
    );
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
    finished.outcome = DiskWriteOutcome::Published;
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
