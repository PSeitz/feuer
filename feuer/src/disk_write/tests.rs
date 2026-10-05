use super::*;
use crate::test_metrics::{registry, value};
use bytes::Bytes;
use feuer_memory::MemoryCache;
use feuer_storage::IoMetrics;
use feuer_types::ByteRange;

#[test]
fn queue_metrics_cover_enqueue_pressure_dequeue_and_cancellation() {
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
    active.entry_metrics.finish_queue_wait();
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
    let key = ObjectKeyHash::from(key);
    let download = Download::new(3, Bytes::from_static(b"abcd")).unwrap();
    assert!(memory.insert(key, download.clone()));
    disk_write_queue.enqueue_if_space_available(key, download);
}

#[test]
fn full_queue_skips_writes_without_waiting_or_rejecting_memory_admission() {
    let memory = MemoryCache::new(1024);
    let (disk_write_queue, mut receiver) = DiskWriteQueue::channel(1);
    enqueue(&disk_write_queue, &memory, "first");
    enqueue(&disk_write_queue, &memory, "skipped");
    assert_eq!(receiver.len(), 1);
    // Queue pressure did not reject memory admission.
    let key = ObjectKeyHash::from("skipped");
    assert!(memory.get(&key, ByteRange::new(3, 7).unwrap()).is_some());
    let active = receiver.try_recv().unwrap();
    enqueue(&disk_write_queue, &memory, "next");
    assert_eq!(receiver.len(), 1);
    drop(active);
    drop(receiver);
    enqueue(&disk_write_queue, &memory, "closed");
}

#[tokio::test]
async fn queued_writes_persist_after_memory_eviction() {
    let directory = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryCache::new(4096));
    let disk = DiskCache::open(directory.path(), 256 << 20, IoMetrics::noop())
        .await
        .unwrap();
    let (registry, backend) = registry();
    let (disk_write_queue, receiver) = DiskWriteQueue::channel_with_metrics(8, DiskWriteQueueMetrics::new(&backend));
    let range = ByteRange::new(3, 7).unwrap();
    for key in ["live", "evicted"] {
        enqueue(&disk_write_queue, &memory, key);
    }
    assert!(memory.remove(&ObjectKeyHash::from("evicted"), range));
    drop(memory);
    tokio::time::pause();
    let writer = tokio::spawn(DiskWriteQueue::write_queued(receiver, disk.clone()));
    tokio::task::yield_now().await;
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 8.0);
    tokio::time::advance(Duration::from_secs(59)).await;
    for key in ["live", "evicted"] {
        assert_eq!(disk.get(&ObjectKeyHash::from(key), range).await.unwrap(), b"abcd"[..]);
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    tokio::time::timeout(Duration::from_secs(10), async {
        while value(&registry, "feuer_disk_write_pending_bytes", &[]) != 0.0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(disk_write_queue);
    writer.await.unwrap();
    // The timer flushes the buffered entries without retaining the memory cache.
    for key in ["live", "evicted"] {
        assert_eq!(
            disk.get(&ObjectKeyHash::from(key), range).await.unwrap(),
            Bytes::from_static(b"abcd")
        );
        assert_eq!(
            disk.access_histories().request_count(),
            0,
            "queue operations do not record requests"
        );
    }
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
}

#[tokio::test]
async fn closing_queue_discards_partial_chunks() {
    let directory = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryCache::new(4096));
    let disk = DiskCache::open(directory.path(), 2 << 20, IoMetrics::noop())
        .await
        .unwrap();
    let (registry, backend) = registry();
    let (queue, receiver) = DiskWriteQueue::channel_with_metrics(1, DiskWriteQueueMetrics::new(&backend));
    enqueue(&queue, &memory, "object");
    drop(queue);
    DiskWriteQueue::write_queued(receiver, disk.clone()).await;
    assert!(!disk.covers_range(&ObjectKeyHash::from("object"), ByteRange::new(3, 7).unwrap()));
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
}
