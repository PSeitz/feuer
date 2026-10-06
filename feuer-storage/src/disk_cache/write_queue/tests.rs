use super::*;
use crate::test_metrics::{registry, value};
use std::time::Duration;

async fn measured_disk() -> (tempfile::TempDir, DiskCache, prometheus::Registry) {
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let disk = DiskCache::open_with_metrics(
        directory.path(),
        4 * CHUNK_BYTES,
        IoMetrics::noop(),
        Arc::new(ObjectAccessHistories::new()),
        DiskMetrics::new(&backend),
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    (directory, disk, registry)
}

fn enqueue(disk: &DiskCache, key: &str) {
    disk.enqueue_if_space_available(
        ObjectKeyHash::from(key),
        Download::new(3, Bytes::from_static(b"abcd")).unwrap(),
    );
}

#[tokio::test]
async fn queue_metrics_cover_pressure_dequeue_and_cancellation() {
    let (_directory, mut disk, registry) = measured_disk().await;
    assert_eq!(disk.write_sender.max_capacity(), 512);
    let (sender, mut receiver) = mpsc::channel(1);
    disk.write_sender = sender;
    enqueue(&disk, "queued");
    enqueue(&disk, "full");
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
    enqueue(&disk, "queued-again");
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 8.0);
    drop(active);
    drop(receiver);
    assert_eq!(
        value(&registry, "feuer_disk_write_queue_total", &[("outcome", "canceled")]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
    enqueue(&disk, "closed");
    assert_eq!(
        value(
            &registry,
            "feuer_disk_write_queue_total",
            &[("outcome", "queue_closed")]
        ),
        1.0
    );
}

#[tokio::test]
async fn disk_timer_flushes_direct_and_queued_writes() {
    let (_directory, disk, registry) = measured_disk().await;
    let range = ByteRange::new(3, 7).unwrap();
    for (count, queued) in [(0, false), (1, true)] {
        tokio::time::pause();
        let key = ObjectKeyHash(count);
        let download = Download::new(3, Bytes::from_static(b"abcd")).unwrap();
        if queued {
            disk.enqueue_if_space_available(key, download);
        } else {
            disk.write(key, download, ()).await.unwrap();
        }
        tokio::task::yield_now().await;
        while value(&registry, "feuer_disk_write_queued_entries", &[]) != 0.0 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_secs(59)).await;
        assert_eq!(value(&registry, "feuer_disk_entries", &[]), count as f64);
        assert_eq!(disk.get(&key, range).await.unwrap(), b"abcd"[..]);
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::time::resume();
        tokio::time::timeout(Duration::from_secs(10), async {
            while value(&registry, "feuer_disk_entries", &[]) != (count + 1) as f64 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(disk.access_histories().request_count(), 0);
    let weak = Arc::downgrade(&disk.disk);
    drop(disk);
    tokio::time::timeout(Duration::from_secs(10), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
}

#[tokio::test]
async fn closing_disk_drains_writes_and_discards_partial_chunks() {
    let (_directory, mut disk, registry) = measured_disk().await;
    let (sender, receiver) = mpsc::channel(2);
    disk.write_sender = sender;
    disk.enqueue_if_space_available(
        ObjectKeyHash::from("large"),
        Download::new(0, Bytes::from(vec![7; 512 * 1024])).unwrap(),
    );
    enqueue(&disk, "small");
    let weak = Arc::downgrade(&disk.disk);
    drop(disk);
    assert!(
        weak.upgrade().is_some(),
        "queued writes retain the disk without retaining the sender"
    );
    DiskCache::write_queued(receiver).await;
    assert!(weak.upgrade().is_none());
    for outcome in ["published", "canceled"] {
        assert_eq!(
            value(&registry, "feuer_disk_write_entries_total", &[("outcome", outcome)]),
            1.0
        );
    }
    assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
}
