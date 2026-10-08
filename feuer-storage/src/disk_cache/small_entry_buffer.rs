//! Each shard owns an initialized small-entry buffer; disk space is reserved only when writing.

use super::{writer::BufferedEntry, *};
use feuer_memory::AlignedBuffer;
use std::time::Duration;

// Pack entries below this aligned size so they share a payload chunk.
const SMALL_ENTRY_BYTES: usize = 512 * 1024;
// Flush partial chunks even when no later writes arrive.
const FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// One chunk of small-entry bytes and their descriptions, including aligned payload offsets.
pub(super) struct BufferedSmallEntryChunk {
    /// Aligned 1-MiB payload storage for packed writes and pre-flush reads.
    buffer: AlignedBuffer,
    /// Entry descriptions and completion state for buffered lookups, publication, and discard.
    entries: Vec<BufferedEntry>,
}

impl DiskCache {
    /// Flushes shared buffers. Returns the number of entries published by this call.
    pub async fn flush(&self) -> Result<usize, DiskCacheError> {
        self.disk.flush().await
    }

    /// Discards buffered entries without writing them; retains each shard's initialized buffer.
    pub async fn discard_pending(&self) {
        for shard in &self.disk.shards {
            shard.buffered_small_entry_chunk.lock().await.entries.clear();
        }
    }
}

impl DiskCacheInner {
    /// Keeps the disk alive only while flushing; direct and queued writes share this timer.
    pub(super) async fn flush_payload_periodically(disk: Weak<Self>) {
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + FLUSH_INTERVAL, FLUSH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(disk) = disk.upgrade() else { return };
            if let Err(error) = disk.flush().await {
                tracing::warn!(target: "feuer::storage", %error, "payload flush failed");
            }
        }
    }

    pub(super) async fn flush(&self) -> Result<usize, DiskCacheError> {
        let mut published = 0;
        for shard in &self.shards {
            published += shard.buffered_small_entry_chunk.lock().await.flush(shard, self).await?;
        }
        Ok(published)
    }

    pub(super) async fn write(
        &self,
        key: ObjectKeyHash,
        download: Download,
        completion: impl Send + 'static,
    ) -> Result<usize, DiskCacheError> {
        let shard = &self.shards[self.shard_index_for_key(&key)];
        let (object_range, bytes) = download.into_parts();
        let length = payload_disk_bytes(object_range.len()) as usize;
        let mut attempt = DiskWriteAttempt::new(self.metrics.clone());
        if shard.covers_range(&key, object_range) {
            attempt.outcome = DiskWriteOutcome::AlreadyCovered;
            return Ok(0);
        }
        let mut entry = BufferedEntry {
            key,
            object_range,
            payload_checksum: XxHash64::oneshot(0, &bytes),
            offset: 0,
            attempt,
            _completion: Box::new(completion),
        };
        if length >= SMALL_ENTRY_BYTES {
            return shard.write_payload(self, bytes, vec![entry]).await;
        }
        let mut buffer = shard.buffered_small_entry_chunk.lock().await;
        let mut published = 0;
        let mut offset = buffer.entries.last().map_or(0, |entry| {
            entry.offset + payload_disk_bytes(entry.object_range.len()) as usize
        });
        if offset + length > CHUNK_BYTES as usize {
            published += buffer.flush(shard, self).await?;
            offset = 0;
        }
        entry.offset = offset;
        buffer.buffer.as_mut_slice()[offset..offset + bytes.len()].copy_from_slice(&bytes);
        drop(bytes);
        buffer.entries.push(entry);
        if offset + length == CHUNK_BYTES as usize {
            published += buffer.flush(shard, self).await?;
        }
        Ok(published)
    }
}

impl BufferedSmallEntryChunk {
    pub(super) fn new() -> Result<Self, DataFileError> {
        Ok(Self {
            buffer: AlignedBuffer::allocate_zeroed(CHUNK_BYTES as usize).map_err(|source| DataFileError::Task {
                operation: crate::IoOperation::Write,
                source: Box::new(source),
            })?,
            entries: Vec::new(),
        })
    }

    /// Finds the newest buffered entry covering the requested range without copying its payload.
    pub(super) fn covering_entry(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<&BufferedEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| &entry.key == key && entry.object_range.contains(requested))
    }

    /// Checks if a buffered entry covers the requested range, returning its bytes if so.
    /// We buffer for 60 seconds, so reads may miss during payload I/O.
    pub(super) fn get(&self, key: &ObjectKeyHash, requested: Option<ByteRange>) -> Option<Bytes> {
        let entry = self.covering_entry(key, requested.unwrap_or_else(|| ByteRange::new(0, 0).unwrap()))?;
        let requested = requested.unwrap_or(entry.object_range);
        let start = entry.offset + (requested.start() - entry.object_range.start()) as usize;
        Some(Bytes::copy_from_slice(
            &self.buffer.as_ref()[start..start + requested.len() as usize],
        ))
    }

    async fn flush(&mut self, shard: &DiskCacheShard, disk: &DiskCacheInner) -> Result<usize, DiskCacheError> {
        if self.entries.is_empty() {
            return Ok(0);
        }
        // Detached chunks are not lookup-visible until publication. Reads may miss during payload I/O.
        let Self { buffer, entries, .. } = std::mem::replace(self, Self::new()?);
        shard.write_payload(disk, buffer.into_bytes(), entries).await
    }
}
