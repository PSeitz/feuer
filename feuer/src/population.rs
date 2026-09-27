//! Bounded best-effort scheduling; one writer drains explicit immutable batches.

mod metrics;
#[cfg(test)]
mod tests;

use metrics::{PopulationMetrics, QueueOutcome};
use mixtrics::metrics::BoxedRegistry;
use std::{sync::Arc, time::Instant};

use feuer_memory::MemoryCache;
use feuer_storage::DiskRangeCache;
use feuer_types::{ByteRange, Download, ObjectKey, retention::ObjectAccessHistory};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

const MAX_QUEUED_ENTRIES: usize = 256;
// Covers queued and active payload together; oversized downloads remain memory-only.
const MAX_WRITE_BYTES: u32 = 64 << 20;
const MAX_BATCH_ENTRIES: usize = 64;

pub(crate) struct DiskPopulation {
    sender: mpsc::Sender<PendingDiskWrite>,
    bytes: Arc<Semaphore>,
    byte_capacity: u32,
    metrics: Arc<PopulationMetrics>,
}

/// The exact memory admission authorizing a queued or active disk write.
struct DiskWriteSource {
    key: ObjectKey,
    range: ByteRange,
    entry_id: u64,
    _accesses: Arc<ObjectAccessHistory>,
    bytes: OwnedSemaphorePermit,
    queued_at: Option<Instant>,
    metrics: Arc<PopulationMetrics>,
}

struct PendingDiskWrite {
    download: Download,
    source: DiskWriteSource,
}

impl DiskWriteSource {
    fn dequeue(&mut self) {
        let queued_at = self.queued_at.take().expect("queued write dequeued once");
        self.metrics.queued_entries.decrease(1);
        self.metrics.queue_duration.record(queued_at.elapsed().as_secs_f64());
    }
}

impl Drop for DiskWriteSource {
    fn drop(&mut self) {
        self.metrics.pending_bytes.decrease(self.bytes.num_permits() as u64);
        if self.queued_at.is_some() {
            self.metrics.queued_entries.decrease(1);
            self.metrics.record(QueueOutcome::Canceled);
        }
    }
}

impl DiskPopulation {
    pub(crate) fn with_metrics(memory: Arc<MemoryCache>, disk: DiskRangeCache, registry: &BoxedRegistry) -> Self {
        let (population, receiver) =
            Self::channel_with_metrics(MAX_QUEUED_ENTRIES, MAX_WRITE_BYTES, PopulationMetrics::new(registry));
        // The worker owns no sender. Dropping the last cache handle closes the queue;
        // submitted writes still finish under storage's detached reservation owner.
        tokio::spawn(Self::run(receiver, memory, disk));
        population
    }

    #[cfg(test)]
    fn channel(entries: usize, bytes: u32) -> (Self, mpsc::Receiver<PendingDiskWrite>) {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::channel_with_metrics(entries, bytes, PopulationMetrics::new(&registry))
    }

    fn channel_with_metrics(
        entries: usize,
        bytes: u32,
        metrics: Arc<PopulationMetrics>,
    ) -> (Self, mpsc::Receiver<PendingDiskWrite>) {
        let (sender, receiver) = mpsc::channel(entries);
        (
            Self {
                sender,
                bytes: Arc::new(Semaphore::new(bytes as usize)),
                byte_capacity: bytes,
                metrics,
            },
            receiver,
        )
    }

    pub(crate) fn skip_covered(&self) {
        self.metrics.record(QueueOutcome::AlreadyCovered);
    }

    pub(crate) fn skip_redundant(&self) {
        self.metrics.record(QueueOutcome::Redundant);
    }

    pub(crate) fn schedule(
        &self,
        key: ObjectKey,
        download: Download,
        entry_id: u64,
        accesses: Arc<ObjectAccessHistory>,
    ) {
        if download.bytes().len() as u64 > self.byte_capacity as u64 {
            self.metrics.record(QueueOutcome::Oversized);
            return;
        }
        let length = download.bytes().len() as u32;
        let Ok(bytes) = self.bytes.clone().try_acquire_many_owned(length) else {
            self.metrics.record(QueueOutcome::ByteBudget);
            return;
        };
        let permit = match self.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.record(QueueOutcome::Full);
                return;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.record(QueueOutcome::Closed);
                return;
            }
        };
        self.metrics.record(QueueOutcome::Queued);
        self.metrics.queued_entries.increase(1);
        self.metrics.pending_bytes.increase(length as u64);
        let source = DiskWriteSource {
            key,
            range: download.downloaded_range(),
            entry_id,
            _accesses: accesses,
            bytes,
            queued_at: Some(Instant::now()),
            metrics: self.metrics.clone(),
        };
        permit.send(PendingDiskWrite { download, source });
    }

    async fn run(mut receiver: mpsc::Receiver<PendingDiskWrite>, memory: Arc<MemoryCache>, disk: DiskRangeCache) {
        while let Some(first) = receiver.recv().await {
            let mut pending = vec![first];
            while pending.len() < MAX_BATCH_ENTRIES {
                let Ok(next) = receiver.try_recv() else { break };
                pending.push(next);
            }
            let downloads = pending
                .into_iter()
                .filter_map(|PendingDiskWrite { download, mut source }| {
                    source.dequeue();
                    // This is the transition from queued to active. Later eviction may
                    // discard publication, but cannot cancel issued I/O or release its regions.
                    if memory
                        .with_current_entry(&source.key, source.range, source.entry_id, || ())
                        .is_none()
                    {
                        source.metrics.record(QueueOutcome::Stale);
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
                tracing::warn!(target: "feuer::storage", %error, "disk population failed");
            }
        }
    }
}
