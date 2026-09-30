//! Bounded best-effort scheduling; one writer drains explicit immutable batches.

mod metrics;
#[cfg(test)]
mod tests;

use metrics::{DiskWriteQueueMetrics, DiskWriteQueueOutcome};
use mixtrics::metrics::BoxedRegistry;
use std::{sync::Arc, time::Instant};

use feuer_memory::MemoryCache;
use feuer_storage::DiskRangeCache;
use feuer_types::{ByteRange, Download, ObjectKey};
use tokio::sync::mpsc;

const MAX_QUEUED_ENTRIES: usize = 256;
const MAX_BATCH_ENTRIES: usize = 64;

/// A bounded queue scheduling best-effort disk writes.
pub(crate) struct DiskWriteQueue {
    sender: mpsc::Sender<PendingDiskWrite>,
    metrics: Arc<DiskWriteQueueMetrics>,
}

/// The exact memory admission authorizing a queued or active disk write.
struct DiskWriteSource {
    key: ObjectKey,
    range: ByteRange,
    entry_id: u64,
    queued_at: Option<Instant>,
    metrics: Arc<DiskWriteQueueMetrics>,
}

struct PendingDiskWrite {
    download: Download,
    source: DiskWriteSource,
}

impl DiskWriteSource {
    /// Records dequeue time and removes this write from the queued-entry accounting.
    fn record_dequeue(&mut self) {
        let queued_at = self.queued_at.take().expect("queued write dequeued once");
        self.metrics.queued_entries.decrease(1);
        self.metrics.queue_duration.record(queued_at.elapsed().as_secs_f64());
    }
}

impl Drop for DiskWriteSource {
    fn drop(&mut self) {
        self.metrics.pending_bytes.decrease(self.range.len());
        if self.queued_at.is_some() {
            self.metrics.queued_entries.decrease(1);
            self.metrics.record(DiskWriteQueueOutcome::Canceled);
        }
    }
}

impl DiskWriteQueue {
    pub(crate) fn with_metrics(memory: Arc<MemoryCache>, disk: DiskRangeCache, registry: &BoxedRegistry) -> Self {
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

    /// Records a download skipped because its memory admission was redundant.
    pub(crate) fn record_redundant_admission(&self) {
        self.metrics.record(DiskWriteQueueOutcome::Redundant);
    }

    /// Enqueues a disk write if the queue has capacity, without waiting for space.
    /// Full or closed queues discard the download and record the corresponding outcome.
    pub(crate) fn enqueue_if_capacity(&self, key: ObjectKey, download: Download, entry_id: u64) {
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
        let source = DiskWriteSource {
            key,
            range: download.downloaded_range(),
            entry_id,
            queued_at: Some(Instant::now()),
            metrics: self.metrics.clone(),
        };
        permit.send(PendingDiskWrite { download, source });
    }

    /// Writes queued downloads in batches, checking their memory identities before publication.
    async fn write_queued_batches(
        mut receiver: mpsc::Receiver<PendingDiskWrite>,
        memory: Arc<MemoryCache>,
        disk: DiskRangeCache,
    ) {
        while let Some(first_write) = receiver.recv().await {
            let mut pending_writes = vec![first_write];
            while pending_writes.len() < MAX_BATCH_ENTRIES {
                let Ok(next_write) = receiver.try_recv() else { break };
                pending_writes.push(next_write);
            }
            let downloads = pending_writes
                .into_iter()
                .filter_map(|PendingDiskWrite { download, mut source }| {
                    source.record_dequeue();
                    // This is the transition from queued to active. Later eviction may
                    // discard publication, but cannot cancel issued I/O or release its regions.
                    if memory
                        .with_current_entry(&source.key, source.range, source.entry_id, || ())
                        .is_none()
                    {
                        source.metrics.record(DiskWriteQueueOutcome::Stale);
                        return None;
                    }
                    Some((source.key.clone(), download, source))
                })
                .collect::<Vec<_>>();
            if downloads.is_empty() {
                continue;
            }
            let memory = memory.clone();
            if let Err(error) = disk
                .insert_batch_checked(downloads, move |source, publish| {
                    // Lock order is disk index -> memory shard. No memory operation
                    // acquires a disk lock; eviction cannot race this publication.
                    memory.with_current_entry(&source.key, source.range, source.entry_id, publish);
                })
                .await
            {
                tracing::warn!(target: "feuer::storage", %error, "disk write failed");
            }
        }
    }
}
