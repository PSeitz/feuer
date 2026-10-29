//! Each shard owns an initialized small-entry buffer. Disk space is reserved only when writing.

use super::{writer::WriteEntry, *};
use feuer_memory::AlignedBuffer;
use std::time::Duration;

// Pack entries below this aligned size so they share a payload chunk.
const SMALL_ENTRY_BYTES: usize = 512 * 1024;
// Flush partial chunks even when no later writes arrive.
const FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// A buffer that accumulates small entries in one payload chunk, with aligned offsets.
pub(super) struct SmallEntryBuffer {
    /// Aligned 1-MiB payload storage for packed writes and pre-flush reads.
    buffer: AlignedBuffer,
    /// Entry metadata and guards retained until publication or discard.
    entries: Vec<WriteEntry>,
}

impl DiskCache {
    /// Flushes shared buffers. Returns the number of entries published by this call.
    pub async fn flush(&self) -> Result<usize, DiskCacheError> {
        self.disk.flush().await
    }

    /// Discards buffered entries without writing them and retains each shard's initialized buffer.
    pub async fn discard_pending(&self) {
        for shard in &self.disk.shards {
            shard.small_entry_buffer.lock().await.entries.clear();
        }
    }
}

impl DiskCacheInner {
    /// Keeps the disk alive only while flushing. Direct and queued writes share this timer.
    pub(super) async fn flush_payload_periodically(disk: Weak<Self>) {
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + FLUSH_INTERVAL, FLUSH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(disk) = disk.upgrade() else { return };
            if let Err(error) = disk.flush().await {
                tracing::error!(target: "feuer::storage", %error, "payload flush failed");
            }
        }
    }

    pub(super) async fn flush(&self) -> Result<usize, DiskCacheError> {
        let mut published = 0;
        for shard in &self.shards {
            published += shard.small_entry_buffer.lock().await.flush(shard, self).await?;
        }
        Ok(published)
    }

    pub(super) async fn write(
        &self,
        key: ObjectKeyHash,
        bytes: Bytes,
        completion: impl Send + 'static,
    ) -> Result<usize, DiskCacheError> {
        let shard = &self.shards[self.shard_index_for_key(&key)];
        let payload_len = bytes.len() as u64;
        let aligned_payload_len = payload_disk_bytes(payload_len) as usize;
        let mut attempt = DiskWriteAttempt::new(self.metrics.clone());
        if shard.contains(&key) {
            attempt.outcome = DiskWriteOutcome::AlreadyCovered;
            return Ok(0);
        }
        let mut entry = WriteEntry {
            key,
            payload_len,
            payload_checksum: XxHash64::oneshot(0, &bytes),
            buffer_offset: 0,
            attempt,
            _completion_guard: Box::new(completion),
        };
        if aligned_payload_len >= SMALL_ENTRY_BYTES {
            return shard.write_payload(self, bytes, vec![entry]).await;
        }
        let mut buffer = shard.small_entry_buffer.lock().await;
        let mut published = 0;
        let mut offset = buffer.entries.last().map_or(0, |entry| {
            entry.buffer_offset + payload_disk_bytes(entry.payload_len) as usize
        });
        if offset + aligned_payload_len > CHUNK_BYTES as usize {
            published += buffer.flush(shard, self).await?;
            offset = 0;
        }
        entry.buffer_offset = offset;
        buffer.buffer.as_mut_slice()[offset..offset + bytes.len()].copy_from_slice(&bytes);
        drop(bytes);
        buffer.entries.push(entry);
        if offset + aligned_payload_len == CHUNK_BYTES as usize {
            published += buffer.flush(shard, self).await?;
        }
        Ok(published)
    }
}

impl SmallEntryBuffer {
    pub(super) fn new() -> Result<Self, DataFileError> {
        Ok(Self {
            buffer: AlignedBuffer::allocate_zeroed(CHUNK_BYTES as usize).map_err(|source| DataFileError::Task {
                operation: crate::IoOperation::Write,
                source: Box::new(source),
            })?,
            entries: Vec::new(),
        })
    }

    /// Borrows the newest buffered entry's metadata and guards.
    pub(super) fn entry(&self, key: &ObjectKeyHash) -> Option<&WriteEntry> {
        self.entries.iter().rev().find(|entry| &entry.key == key)
    }

    /// Copies a buffered object's bytes. Reads may miss during payload I/O.
    pub(super) fn get(&self, key: &ObjectKeyHash) -> Option<Bytes> {
        let entry = self.entry(key)?;
        Some(Bytes::copy_from_slice(
            &self.buffer.as_ref()[entry.buffer_offset..entry.buffer_offset + entry.payload_len as usize],
        ))
    }

    async fn flush(&mut self, shard: &DiskCacheShard, disk: &DiskCacheInner) -> Result<usize, DiskCacheError> {
        if self.entries.is_empty() {
            return Ok(0);
        }
        // Flushing entries become visible to lookups after publication.
        let Self { buffer, entries } = std::mem::replace(self, Self::new()?);
        shard.write_payload(disk, buffer.into_bytes(), entries).await
    }
}
