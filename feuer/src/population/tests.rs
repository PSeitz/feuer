use super::*;
use crate::test_metrics::{registry, value};
use bytes::Bytes;
use feuer_storage::IoMetrics;

#[test]
fn queue_metrics_cover_admission_pressure_dequeue_and_cancellation() {
    let (registry, backend) = registry();
    let metrics = PopulationQueueMetrics::new(&backend);
    let memory = MemoryCache::new(1024);
    let (population, mut receiver) = DiskPopulationQueue::channel_with_metrics(1, metrics);
    enqueue(&population, &memory, "queued");
    enqueue(&population, &memory, "full");
    assert_eq!(
        value(
            &registry,
            "feuer_disk_population_queue_total",
            &[("outcome", "queue_full")]
        ),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_population_pending_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_population_queued_entries", &[]), 1.0);
    let mut active = receiver.try_recv().unwrap();
    active.source.record_dequeue();
    assert_eq!(value(&registry, "feuer_disk_population_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_population_pending_bytes", &[]), 4.0);
    assert_eq!(
        value(&registry, "feuer_disk_population_queue_duration_seconds", &[]),
        1.0
    );
    enqueue(&population, &memory, "queued-again");
    assert_eq!(value(&registry, "feuer_disk_population_pending_bytes", &[]), 8.0);
    drop(active);
    drop(receiver);
    assert_eq!(
        value(
            &registry,
            "feuer_disk_population_queue_total",
            &[("outcome", "canceled")]
        ),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_population_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_population_pending_bytes", &[]), 0.0);
    enqueue(&population, &memory, "closed");
    assert_eq!(
        value(
            &registry,
            "feuer_disk_population_queue_total",
            &[("outcome", "queue_closed")]
        ),
        1.0
    );
}

fn enqueue(population: &DiskPopulationQueue, memory: &MemoryCache, key: &str) {
    let key = key.to_owned();
    let accesses = memory.access_histories().for_key(&key);
    let download = Download::new(3, Bytes::from_static(b"abcd")).unwrap();
    let id = memory
        .insert_and_record(key.clone(), download.clone(), download.downloaded_range())
        .unwrap();
    population.enqueue_if_capacity(key, download, id, accesses);
}

#[test]
fn queue_saturation_is_nonblocking_and_bounded_by_entries() {
    let memory = MemoryCache::new(1024);
    let (population, mut receiver) = DiskPopulationQueue::channel(1);
    enqueue(&population, &memory, "first");
    enqueue(&population, &memory, "skipped");
    assert_eq!(receiver.len(), 1);
    // Queue pressure did not reject memory admission or its successful access.
    let key = "skipped".to_owned();
    assert_eq!(memory.access_histories().for_key(&key).lock().generation(), 1);
    assert!(memory.get(&key, ByteRange::new(3, 7).unwrap()).is_some());
    let active = receiver.try_recv().unwrap();
    enqueue(&population, &memory, "next");
    assert_eq!(receiver.len(), 1);
    drop(active);
    drop(receiver);
    enqueue(&population, &memory, "closed");
}

#[tokio::test]
async fn queued_eviction_and_readmission_cancel_old_writes_while_live_entries_batch_together() {
    let directory = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryCache::new(4096));
    let disk = DiskRangeCache::open_with_access_histories(
        directory.path(),
        1 << 20,
        IoMetrics::noop(),
        memory.access_histories(),
    )
    .await
    .unwrap();
    let (registry, backend) = registry();
    let (population, receiver) = DiskPopulationQueue::channel_with_metrics(8, PopulationQueueMetrics::new(&backend));
    let range = ByteRange::new(3, 7).unwrap();
    for key in ["live-a", "evicted", "readmitted", "live-b"] {
        enqueue(&population, &memory, key);
    }
    assert!(memory.remove(&"evicted".to_owned(), range));
    assert!(memory.remove(&"readmitted".to_owned(), range));
    memory.insert(
        "readmitted".to_owned(),
        Download::new(3, Bytes::from_static(b"abcd")).unwrap(),
    );
    drop(population);
    DiskPopulationQueue::write_queued_batches(receiver, memory.clone(), disk.clone()).await;
    assert!(!disk.contains(&"evicted".to_owned(), range));
    assert!(!disk.contains(&"readmitted".to_owned(), range));
    // Both fit on disk only if the drained batch packs them into the same chunk.
    for key in ["live-a", "live-b"] {
        assert_eq!(
            disk.get(&key.to_owned(), range).await.unwrap(),
            Bytes::from_static(b"abcd")
        );
        assert_eq!(
            memory.access_histories().for_key(&key.to_owned()).lock().generation(),
            1
        );
    }
    assert_eq!(
        value(&registry, "feuer_disk_population_queue_total", &[("outcome", "stale")]),
        2.0
    );
    assert_eq!(value(&registry, "feuer_disk_population_pending_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_population_queued_entries", &[]), 0.0);
}
