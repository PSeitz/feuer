//! Best-effort background writes. The disk writer handles buffer flushing.

#[cfg(test)]
mod tests;

use super::*;
use crate::disk_metrics::DiskWriteQueueOutcome;

pub(super) const MAX_QUEUED_ENTRIES: usize = 512;

/// A queued download, retaining the disk internals until its write finishes.
pub(super) struct PendingDiskWrite {
    disk: Arc<DiskCacheInner>,
    key: ObjectKeyHash,
    download: Download,
    entry_metrics: QueuedWriteMetrics,
}

/// Measures queue wait and keeps payload bytes counted as pending until publication or discard.
struct QueuedWriteMetrics {
    payload_bytes: u64,
    queued_at: Option<Instant>,
    metrics: Arc<DiskMetrics>,
}

impl QueuedWriteMetrics {
    fn finish_queue_wait(&mut self) {
        let queued_at = self.queued_at.take().expect("queued write dequeued once");
        self.metrics.queued_entries.decrease(1);
        self.metrics.queue_duration.record(queued_at.elapsed().as_secs_f64());
    }
}

impl Drop for QueuedWriteMetrics {
    fn drop(&mut self) {
        self.metrics.pending_bytes.decrease(self.payload_bytes);
        if self.queued_at.is_some() {
            self.metrics.queued_entries.decrease(1);
            self.metrics.record_queue(DiskWriteQueueOutcome::Canceled);
        }
    }
}

impl DiskCache {
    /// Queues a background write without waiting. A full or closed queue skips the write.
    pub fn enqueue_if_space_available(&self, key: ObjectKeyHash, download: Download) {
        let metrics = &self.disk.metrics;
        let permit = match self.write_sender.try_reserve() {
            Ok(permit) => permit,
            Err(error) => {
                metrics.record_queue(match error {
                    mpsc::error::TrySendError::Full(_) => DiskWriteQueueOutcome::Full,
                    mpsc::error::TrySendError::Closed(_) => DiskWriteQueueOutcome::Closed,
                });
                return;
            }
        };
        let payload_bytes = download.bytes().len() as u64;
        metrics.record_queue(DiskWriteQueueOutcome::Queued);
        metrics.queued_entries.increase(1);
        metrics.pending_bytes.increase(payload_bytes);
        let entry_metrics = QueuedWriteMetrics {
            payload_bytes,
            queued_at: Some(Instant::now()),
            metrics: metrics.clone(),
        };
        permit.send(PendingDiskWrite {
            disk: self.disk.clone(),
            key,
            download,
            entry_metrics,
        });
    }

    // Closing the last sender drains queued writes. Each write retains the disk until
    // it finishes. Dropping the last disk reference discards remaining buffered entries.
    pub(super) async fn write_queued(mut receiver: mpsc::Receiver<PendingDiskWrite>) {
        while let Some(PendingDiskWrite {
            disk,
            key,
            download,
            mut entry_metrics,
        }) = receiver.recv().await
        {
            entry_metrics.finish_queue_wait();
            if let Err(error) = disk.write(key, download, entry_metrics).await {
                tracing::error!(target: "feuer::storage", %error, "disk write failed");
            }
        }
    }
}
