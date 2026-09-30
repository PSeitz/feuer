use super::*;
use crate::test_metrics::{registry, value};
use bytes::Bytes;
use feuer_storage::IoMetrics;

#[test]
fn queue_metrics_cover_admission_pressure_dequeue_and_cancellation() {
    let (registry, backend) = registry();
    let metrics = DiskWriteQueueMetrics::new(&backend);
    let memory = MemoryCache::new(1024);
    let (disk_write_queue, mut receiver) = DiskWriteQueue::channel_with_metrics(1, metrics);
    enqueue(&disk_write_queue, &memory, "queued");
    enqueue(&disk_write_queue, &memory, "full");
    assert_eq!(
        value(&registry, "feuer_disk_write_queue_total", &[("outcome", "queue_full")]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 1.0);
    let mut active = receiver.try_recv().unwrap();
    active.source.record_dequeue();
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_write_queue_duration_seconds", &[]), 1.0);
    enqueue(&disk_write_queue, &memory, "queued-again");
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 8.0);
    drop(active);
    drop(receiver);
    assert_eq!(
        value(&registry, "feuer_disk_write_queue_total", &[("outcome", "canceled")]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
    enqueue(&disk_write_queue, &memory, "closed");
    assert_eq!(
        value(
            &registry,
            "feuer_disk_write_queue_total",
            &[("outcome", "queue_closed")]
        ),
        1.0
    );
}

fn enqueue(disk_write_queue: &DiskWriteQueue, memory: &MemoryCache, key: &str) {
    let key = key.to_owned();
    let accesses = memory.access_histories().for_key(&key);
    let download = Download::new(3, Bytes::from_static(b"abcd")).unwrap();
    let id = memory
        .insert_and_record(key.clone(), download.clone(), download.downloaded_range())
        .unwrap();
    disk_write_queue.enqueue_if_capacity(key, download, id, accesses);
}

#[test]
fn queue_saturation_is_nonblocking_and_bounded_by_entries() {
    let memory = MemoryCache::new(1024);
    let (disk_write_queue, mut receiver) = DiskWriteQueue::channel(1);
    enqueue(&disk_write_queue, &memory, "first");
    enqueue(&disk_write_queue, &memory, "skipped");
    assert_eq!(receiver.len(), 1);
    // Queue pressure did not reject memory admission or its successful access.
    let key = "skipped".to_owned();
    assert_eq!(memory.access_histories().for_key(&key).lock().generation(), 1);
    assert!(memory.get(&key, ByteRange::new(3, 7).unwrap()).is_some());
    let active = receiver.try_recv().unwrap();
    enqueue(&disk_write_queue, &memory, "next");
    assert_eq!(receiver.len(), 1);
    drop(active);
    drop(receiver);
    enqueue(&disk_write_queue, &memory, "closed");
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
    let (disk_write_queue, receiver) = DiskWriteQueue::channel_with_metrics(8, DiskWriteQueueMetrics::new(&backend));
    let range = ByteRange::new(3, 7).unwrap();
    for key in ["live-a", "evicted", "readmitted", "live-b"] {
        enqueue(&disk_write_queue, &memory, key);
    }
    assert!(memory.remove(&"evicted".to_owned(), range));
    assert!(memory.remove(&"readmitted".to_owned(), range));
    memory.insert(
        "readmitted".to_owned(),
        Download::new(3, Bytes::from_static(b"abcd")).unwrap(),
    );
    drop(disk_write_queue);
    DiskWriteQueue::write_queued_batches(receiver, memory.clone(), disk.clone()).await;
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
        value(&registry, "feuer_disk_write_queue_total", &[("outcome", "stale")]),
        2.0
    );
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
}
