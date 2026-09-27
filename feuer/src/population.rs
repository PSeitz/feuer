//! Bounded best-effort scheduling; one writer drains explicit immutable batches.

#[cfg(test)]
mod tests;

use std::sync::Arc;

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
}

/// The exact memory admission authorizing a queued or active disk write.
struct DiskWriteSource {
    key: ObjectKey,
    range: ByteRange,
    entry_id: u64,
    _accesses: Arc<ObjectAccessHistory>,
    _bytes: OwnedSemaphorePermit,
}

struct PendingDiskWrite {
    download: Download,
    source: DiskWriteSource,
}

impl DiskPopulation {
    pub(crate) fn new(memory: Arc<MemoryCache>, disk: DiskRangeCache) -> Self {
        let (population, receiver) = Self::channel(MAX_QUEUED_ENTRIES, MAX_WRITE_BYTES);
        // The worker owns no sender. Dropping the last cache handle closes the queue;
        // submitted writes still finish under storage's detached reservation owner.
        tokio::spawn(Self::run(receiver, memory, disk));
        population
    }

    fn channel(entries: usize, bytes: u32) -> (Self, mpsc::Receiver<PendingDiskWrite>) {
        let (sender, receiver) = mpsc::channel(entries);
        (
            Self {
                sender,
                bytes: Arc::new(Semaphore::new(bytes as usize)),
            },
            receiver,
        )
    }

    pub(crate) fn schedule(
        &self,
        key: ObjectKey,
        download: Download,
        entry_id: u64,
        accesses: Arc<ObjectAccessHistory>,
    ) {
        let Ok(length) = u32::try_from(download.bytes().len()) else {
            return;
        };
        let Ok(bytes) = self.bytes.clone().try_acquire_many_owned(length) else {
            return;
        };
        let source = DiskWriteSource {
            key,
            range: download.downloaded_range(),
            entry_id,
            _accesses: accesses,
            _bytes: bytes,
        };
        // Full or closed queues drop the candidate and release its byte budget.
        let _ = self.sender.try_send(PendingDiskWrite { download, source });
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
                .filter_map(|PendingDiskWrite { download, source }| {
                    // This is the transition from queued to active. Later eviction may
                    // discard publication, but cannot cancel issued I/O or release its regions.
                    memory.with_current_entry(&source.key, source.range, source.entry_id, || ())?;
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
