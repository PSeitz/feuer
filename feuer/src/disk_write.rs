//! Best-effort disk writes: at most 256 queued entries, drained by one writer in batches of at most 64.

mod metrics;
#[cfg(test)]
mod tests;

use metrics::{DiskWriteQueueMetrics, DiskWriteQueueOutcome};
use mixtrics::metrics::BoxedRegistry;
use std::{sync::Arc, time::Instant};

use feuer_memory::MemoryCache;
use feuer_storage::DiskCache;
use feuer_types::{ByteRange, Download, ObjectKeyHash};
use tokio::sync::mpsc;

const MAX_QUEUED_ENTRIES: usize = 256;
const MAX_BATCH_ENTRIES: usize = 64;

/// A disk-write queue that skips downloads when its entry capacity is exhausted.
pub(crate) struct DiskWriteQueue {
    sender: mpsc::Sender<PendingDiskWrite>,
    metrics: Arc<DiskWriteQueueMetrics>,
}

/// Measures a disk-write queue entry's wait and keeps its payload bytes counted as pending until dropped.
/// The object key and range also identify the source entry for memory-presence checks.
struct DiskWriteQueueEntryMetrics {
    key: ObjectKeyHash,
    object_range: ByteRange,
    queued_at: Option<Instant>,
    metrics: Arc<DiskWriteQueueMetrics>,
}

struct PendingDiskWrite {
    download: Download,
    entry_metrics: DiskWriteQueueEntryMetrics,
}

impl DiskWriteQueueEntryMetrics {
    /// Finishes the queue wait, measuring its duration and decreasing the queued-entry count.
    fn finish_queue_wait(&mut self) {
        let queued_at = self.queued_at.take().expect("queued write dequeued once");
        self.metrics.queued_entries.decrease(1);
        self.metrics.queue_duration.record(queued_at.elapsed().as_secs_f64());
    }
}

impl Drop for DiskWriteQueueEntryMetrics {
    fn drop(&mut self) {
        self.metrics.pending_bytes.decrease(self.object_range.len());
        if self.queued_at.is_some() {
            self.metrics.queued_entries.decrease(1);
            self.metrics.record(DiskWriteQueueOutcome::Canceled);
        }
    }
}

impl DiskWriteQueue {
    pub(crate) fn with_metrics(memory: Arc<MemoryCache>, disk: DiskCache, registry: &BoxedRegistry) -> Self {
        let (disk_write_queue, receiver) =
            Self::channel_with_metrics(MAX_QUEUED_ENTRIES, DiskWriteQueueMetrics::new(registry));
        // The worker owns no sender. Dropping the last cache handle closes the queue;
        // submitted writes still finish under storage's detached reservation owner.
        tokio::spawn(Self::write_queued_batches(receiver, memory, disk));
        disk_write_queue
    }

    #[cfg(test)]
    fn channel(entry_capacity: usize) -> (Self, mpsc::Receiver<PendingDiskWrite>) {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::channel_with_metrics(entry_capacity, DiskWriteQueueMetrics::new(&registry))
    }

    fn channel_with_metrics(
        entry_capacity: usize,
        metrics: Arc<DiskWriteQueueMetrics>,
    ) -> (Self, mpsc::Receiver<PendingDiskWrite>) {
        let (sender, receiver) = mpsc::channel(entry_capacity);
        (Self { sender, metrics }, receiver)
    }

    /// Records a download skipped because the disk already covers its range.
    pub(crate) fn record_already_covered(&self) {
        self.metrics.record(DiskWriteQueueOutcome::AlreadyCovered);
    }

    /// Records a download skipped because memory already contains its range.
    pub(crate) fn record_already_in_memory(&self) {
        self.metrics.record(DiskWriteQueueOutcome::Redundant);
    }

    /// Enqueues a disk write if queue space is available, without waiting.
    /// Full or closed queues discard the download and record the corresponding outcome.
    pub(crate) fn enqueue_if_space_available(&self, key: ObjectKeyHash, download: Download) {
        let permit = match self.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.record(DiskWriteQueueOutcome::Full);
                return;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.record(DiskWriteQueueOutcome::Closed);
                return;
            }
        };
        self.metrics.record(DiskWriteQueueOutcome::Queued);
        self.metrics.queued_entries.increase(1);
        self.metrics.pending_bytes.increase(download.downloaded_range().len());
        let entry_metrics = DiskWriteQueueEntryMetrics {
            key,
            object_range: download.downloaded_range(),
            queued_at: Some(Instant::now()),
            metrics: self.metrics.clone(),
        };
        permit.send(PendingDiskWrite {
            download,
            entry_metrics,
        });
    }

    /// Skips evicted queued downloads, then writes admitted batches independently of memory retention.
    async fn write_queued_batches(
        mut receiver: mpsc::Receiver<PendingDiskWrite>,
        memory: Arc<MemoryCache>,
        disk: DiskCache,
    ) {
        while let Some(first_write) = receiver.recv().await {
            let mut pending_writes = vec![first_write];
            while pending_writes.len() < MAX_BATCH_ENTRIES {
                let Ok(next_write) = receiver.try_recv() else { break };
                pending_writes.push(next_write);
            }
            let (downloads, batch_entry_metrics): (Vec<_>, Vec<_>) = pending_writes
                .into_iter()
                .filter_map(
                    |PendingDiskWrite {
                         download,
                         mut entry_metrics,
                     }| {
                        entry_metrics.finish_queue_wait();
                        // Avoid starting work already evicted from memory. Once started,
                        // the owned immutable download remains valid after eviction.
                        if !memory.contains_entry(&entry_metrics.key, entry_metrics.object_range) {
                            entry_metrics.metrics.record(DiskWriteQueueOutcome::Stale);
                            return None;
                        }
                        Some(((entry_metrics.key, download), entry_metrics))
                    },
                )
                .unzip();
            if downloads.is_empty() {
                continue;
            }
            if let Err(error) = disk.insert_batch(downloads).await {
                tracing::warn!(target: "feuer::storage", %error, "disk write failed");
            }
            drop(batch_entry_metrics);
        }
    }
}
