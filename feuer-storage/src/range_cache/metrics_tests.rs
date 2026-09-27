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
        Arc::new(ObjectAccessHistories::new(1)),
        DiskMetrics::new(&backend),
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    (directory, cache, registry)
}

#[tokio::test]
async fn records_population_outcomes_packing_and_index_usage() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = "object".to_owned();
    let request = ByteRange::new(0, 1).unwrap();
    let outcome = |label| value(&registry, "feuer_disk_population_total", &[("outcome", label)]);
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
    assert_eq!(
        value(&registry, "feuer_disk_population_written_entries_total", &[]),
        3.0
    );
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "reserved")]), 1.0);
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
    for state in ["free", "reserved", "quarantined"] {
        assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", state)]), 0.0);
    }
}

#[tokio::test]
async fn distinguishes_integrity_failures_from_io_errors_and_removes_index_usage() {
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
            &[("outcome", "integrity_failure")]
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
    let (_directory, mut cache, registry) = measured_cache(CHUNK_BYTES).await;
    cache.insert_batch(vec![("first".into(), download(4))]).await.unwrap();
    cache.insert_batch(vec![("second".into(), download(4))]).await.unwrap();
    assert_eq!(value(&registry, "feuer_disk_evictions_total", &[]), 1.0);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    // As in the existing write-failure test, let allocation exceed the actual file.
    let disk = Arc::get_mut(&mut cache.disk).unwrap();
    disk.shards[0].entry_index.lock().unwrap().remove("second", 0);
    disk.shards[0].allocator = DiskChunkAllocator::with_metrics(0..3 * CHUNK_BYTES, disk.metrics.clone()).unwrap();
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
        value(&registry, "feuer_disk_population_total", &[("outcome", "failed")]),
        2.0
    );
    assert_eq!(
        value(&registry, "feuer_disk_population_written_entries_total", &[]),
        2.0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "quarantined")]), 3.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "reserved")]), 0.0);
}

#[test]
fn abandoned_population_attempts_count_once_as_canceled() {
    let (registry, backend) = registry();
    let metrics = DiskMetrics::new(&backend);
    drop(PopulationAttempt::new(metrics.clone()));
    let mut finished = PopulationAttempt::new(metrics);
    finished.set_outcome(PopulationOutcome::Published);
    drop(finished);
    assert_eq!(
        value(&registry, "feuer_disk_population_total", &[("outcome", "canceled")]),
        1.0
    );
    assert_eq!(
        value(&registry, "feuer_disk_population_total", &[("outcome", "published")]),
        1.0
    );
}
