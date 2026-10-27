use super::*;
use crate::test_metrics::{registry, value};

fn read_byte_count(registry: &prometheus::Registry) -> f64 {
    value(registry, "feuer_disk_io_bytes_total", &[("operation", "read")])
}

fn download(length: usize) -> Download {
    Download::new(Bytes::from(vec![7; length]))
}

async fn measured_cache(capacity: u64) -> (tempfile::TempDir, DiskCache, prometheus::Registry) {
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let cache = DiskCache::open(
        directory.path(),
        capacity + CHUNK_BYTES,
        DiskCacheOptions {
            io_metrics: IoMetrics::new(&backend),
            metrics: DiskMetrics::new(&backend),
            ..Default::default()
        },
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
    let header_bytes = 4096;
    assert_eq!(written_bytes(), (header_bytes + CHUNK_BYTES + payload_bytes) as f64);
    assert_eq!(read_byte_count(&registry), (header_bytes + CHUNK_BYTES) as f64);
    cache.write_dirty_metadata_pages().await;
    assert_eq!(written_bytes(), (header_bytes + 2 * CHUNK_BYTES + payload_bytes) as f64);
    for key in [ObjectKeyHash(1), ObjectKeyHash(2)] {
        cache.disk.shards[0].remove_entry(&key);
    }
    let before = written_bytes();
    let weak = Arc::downgrade(&cache.disk);
    tokio::spawn(DiskCacheInner::write_metadata_periodically(weak.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        // Both removals share one page in an initialized chunk.
        while written_bytes() < before + METADATA_PAGE_BYTES as f64 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(written_bytes(), before + METADATA_PAGE_BYTES as f64);
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
        (101, 1024, 1),
        (102, 1024, 1),
        (202, 1024, 1),
        (203, 1024, 1),
        (303, 1024, 1),
        (1, CHUNK_BYTES as usize - METADATA_PAGE_BYTES, 1),
        (1, 2 * CHUNK_BYTES as usize, 1),
        (page_format::ENTRIES_PER_METADATA_CHUNK as u128 + 1, 1, 2),
    ] {
        let capacity = if chunks == 2 { 128 } else { 4 } * CHUNK_BYTES;
        let (directory, cache) = tests::open_test_cache(capacity).await;
        let source = download(payload_bytes);
        let inputs: Vec<_> = (0..entries).map(|key| (ObjectKeyHash(key), source.clone())).collect();
        assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), entries as usize);
        cache.write_dirty_metadata_pages().await;
        let capacity = cache.disk.file.capacity();
        drop(cache);

        let (registry, backend) = registry();
        let cache = DiskCache::open(
            directory.path(),
            capacity,
            DiskCacheOptions {
                io_metrics: IoMetrics::new(&backend),
                metrics: DiskMetrics::new(&backend),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(value(&registry, "feuer_disk_entries", &[]), entries as f64);
        assert_eq!(
            value(&registry, "feuer_disk_payload_bytes", &[]),
            (entries * payload_bytes as u128) as f64
        );
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "read"), ("outcome", "success")]
            ),
            (chunks + 1) as f64
        );
        assert_eq!(
            value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]),
            (4096 + chunks * CHUNK_BYTES) as f64
        );
        assert!(
            cache
                .disk
                .shards
                .iter()
                .all(|shard| shard.metadata_pages.lock().unwrap().dirty_pages.is_empty())
        );
        for (key, _) in inputs {
            assert!(cache.contains(&key));
        }
        drop(cache);
        assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 0.0);
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
        4.0 // Chain clearing, global header, one shared payload run, and one three-chunk payload run.
    );
    assert_eq!(
        value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]),
        (4096 + 5 * CHUNK_BYTES) as f64
    );
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), 0.0);
    let written_chunks = value(&registry, "feuer_disk_chunks", &[("state", "allocated")]);
    assert_eq!(written_chunks, 5.0); // Metadata, one shared payload chunk, and three large-entry chunks.
    let metrics = cache.disk.metrics.clone();
    cache.write_dirty_metadata_pages().await;
    drop(cache);

    let cache = DiskCache::open(
        directory.path(),
        capacity + CHUNK_BYTES,
        DiskCacheOptions {
            metrics,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), written_chunks);
    for (key, source) in inputs {
        assert_eq!(cache.get(&key).await.unwrap().into_bytes(), source.bytes());
    }
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), written_chunks);
    drop(cache);
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), 0.0);
}

#[tokio::test]
async fn records_write_outcomes_packing_and_index_usage() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");

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
    assert!(cache.get(&key).await.is_some());
    assert!(cache.get(&"absent".into()).await.is_none());
    for label in ["hit", "absent"] {
        assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", label)]), 1.0);
    }
    assert_eq!(value(&registry, "feuer_disk_lookup_duration_seconds", &[]), 1.0);
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
    assert!(!cache.contains(&"first".into()));
    assert!(!cache.contains(&"second".into()));
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
async fn io_errors_preserve_entries() {
    let (directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;

    let key = ObjectKeyHash::from("unreadable");
    cache.insert_batch(vec![(key, download(4))]).await.unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(cache.get(&key).await.is_none());
    assert_eq!(
        value(&registry, "feuer_disk_lookup_total", &[("outcome", "io_error")]),
        1.0
    );
    assert!(cache.contains(&key));
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
    shard.remove_entry(&ObjectKeyHash::from("second"));
    disk.shards[0].allocator = DiskChunkAllocator::with_metrics(2 * CHUNK_BYTES..5 * CHUNK_BYTES, disk.metrics.clone());
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
    assert!(cache.contains(&"small".into()));
    cache.discard_pending().await;
    assert_eq!(outcome("canceled"), 1.0);
    // The small entry never reserved disk space. Discarding it only releases its buffered bytes and accounting.
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 1.0);
}

#[tokio::test]
async fn buffered_entries_only_attempt_disk_admission_when_flushed() {
    let (_directory, cache, registry) = measured_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);

    assert_eq!(cache.flush().await.unwrap(), 0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
    cache.write(key, download(1), ()).await.unwrap();
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
    assert_eq!(cache.get(&key).await.unwrap().into_bytes(), download(1).bytes());
    assert_eq!(cache.flush().await.unwrap(), 0); // Only metadata fits on disk.
    assert!(!cache.contains(&key));
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
    assert!(!cache.contains(&key));
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
