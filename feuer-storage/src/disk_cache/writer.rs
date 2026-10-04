//! Packs small entries into one unfinished payload chunk per shard.

use super::*;
use feuer_memory::AlignedBuffer;

const SMALL_ENTRY_BYTES: u64 = 128 * 1024;

/// One writer's unfinished small-entry chunks. `T` is held until each entry finishes or is discarded.
/// Drop discards unfinished chunks; it does not flush. Callers serialize writes and drive flushing.
pub struct DiskWriter<'a, T> {
    disk: &'a DiskCacheInner,
    pending: Vec<PayloadWrite<'a, T>>,
}

/// Reserved payload chunks, entry metadata, and an optional prepared small-entry buffer.
struct PayloadWrite<'a, T> {
    shard: &'a DiskCacheShard,
    chunks: Option<ReservedChunks>,
    /// Prepared payload bytes, including alignment padding.
    used_bytes: u64,
    entries: Vec<(ObjectKeyHash, DiskEntry, DiskWriteAttempt, T)>,
    buffer: Option<AlignedBuffer>,
}

impl DiskCache {
    /// Starts a writer with one unfinished small-entry chunk per shard.
    pub fn writer<T>(&self) -> DiskWriter<'_, T> {
        DiskWriter {
            disk: &self.disk,
            pending: self.disk.shards.iter().map(PayloadWrite::new).collect(),
        }
    }

    /// Writes an explicit batch and flushes its partial chunks. Returns published entry count.
    /// Payload completion precedes publication; metadata is written separately every second.
    /// Contained entries and unavailable capacity are skipped. Earlier publication survives later failure.
    /// Dropping this future does not abort its detached writer.
    pub async fn insert_batch(&self, mut downloads: Vec<(ObjectKeyHash, Download)>) -> Result<usize, DiskCacheError> {
        let cache = self.clone();
        downloads.sort_by_key(|(_, download)| download.bytes().len());
        tokio::spawn(async move {
            let mut writer = cache.writer();
            let mut published = 0;
            for (key, download) in downloads {
                published += writer.write(key, download, ()).await?;
            }
            Ok(published + writer.flush().await?)
        })
        .await
        .map_err(DiskCacheError::WriteTaskFailed)?
    }
}

impl<T> DiskWriter<'_, T> {
    /// Copies entries with aligned size <128 KiB into a shared chunk, releasing incoming bytes.
    /// Full/no-fit chunks flush immediately. Larger entries write separately without flushing small entries.
    /// Each admission allows 64 eviction attempts and 4,096 chunks charged to removed entries.
    /// The caller must await completion; cancellation may leave submitted I/O running.
    pub async fn write(
        &mut self,
        key: ObjectKeyHash,
        download: Download,
        completion: T,
    ) -> Result<usize, DiskCacheError> {
        let disk = self.disk;
        let index = disk.shard_index_for_key(&key);
        let shard = &disk.shards[index];
        let mut attempt = DiskWriteAttempt::new(disk.metrics.clone());
        if shard
            .entry_index
            .lock()
            .unwrap()
            .covering_entry(&key, download.downloaded_range())
            .is_some()
        {
            attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
            return Ok(0);
        }
        let length = download
            .downloaded_range()
            .len()
            .next_multiple_of(BUFFER_ALIGNMENT as u64);
        let mut published = 0;
        let mut exclusive = PayloadWrite::new(shard);
        let pending = if length < SMALL_ENTRY_BYTES {
            &mut self.pending[index]
        } else {
            &mut exclusive
        };
        if pending.used_bytes > 0 && pending.used_bytes + length > CHUNK_BYTES {
            published += pending.take().write(disk, Bytes::new()).await?;
        }
        if length < SMALL_ENTRY_BYTES && pending.buffer.is_none() {
            pending.buffer =
                Some(
                    AlignedBuffer::allocate_zeroed(CHUNK_BYTES as usize).map_err(|source| DataFileError::Task {
                        operation: crate::IoOperation::Write,
                        source: Box::new(source),
                    })?,
                );
        }
        let mut attempts = MAX_EVICTION_ATTEMPTS;
        let mut chunks = MAX_EVICTION_CHUNKS;
        let count = length.div_ceil(CHUNK_BYTES);
        let (metadata, start) = loop {
            let metadata = shard.metadata_pages.lock().unwrap().reserve_metadata(&shard.allocator);
            let needed = if let Some(metadata) = metadata {
                if pending.chunks.is_none() {
                    pending.chunks = shard.allocator.reserve_chunks(count);
                }
                if let Some(reserved) = &pending.chunks {
                    break (metadata, reserved.disk_byte_range().start + pending.used_bytes);
                }
                shard.metadata_pages.lock().unwrap().free_entry_positions.push(metadata);
                count
            } else {
                1
            };
            let metadata_chunks = shard.metadata_pages.lock().unwrap().chunks.len() as u64;
            let reserved = pending.chunks.as_ref().map_or(0, ReservedChunks::chunk_count);
            if needed > shard.allocator.chunk_capacity - metadata_chunks - reserved
                || !shard.sample_and_evict_entry(&disk.access_histories, &mut attempts, &mut chunks)
            {
                attempt.set_outcome(DiskWriteOutcome::NoCapacity);
                attempt.evicted = chunks != MAX_EVICTION_CHUNKS;
                return Ok(published);
            }
        };
        attempt.evicted = chunks != MAX_EVICTION_CHUNKS;
        let entry = DiskEntry {
            in_flight_read: Weak::new(),
            eviction_position: 0,
            object_range: download.downloaded_range(),
            payload_checksum: XxHash64::oneshot(0, download.bytes()),
            payload_range: start..start + length,
            metadata,
        };
        if let Some(buffer) = &mut pending.buffer {
            let offset = pending.used_bytes as usize;
            buffer.as_mut_slice()[offset..offset + download.bytes().len()].copy_from_slice(download.bytes());
        }
        pending.used_bytes += length;
        pending.entries.push((key, entry, attempt, completion));
        if length >= SMALL_ENTRY_BYTES || pending.used_bytes == CHUNK_BYTES {
            published += pending.take().write(disk, download.into_parts().1).await?;
        }
        Ok(published)
    }

    /// Writes all unfinished chunks. This is payload publication, not a durability barrier.
    pub async fn flush(&mut self) -> Result<usize, DiskCacheError> {
        let mut published = 0;
        for pending in &mut self.pending {
            published += pending.take().write(self.disk, Bytes::new()).await?;
        }
        Ok(published)
    }
}

impl<T> Drop for PayloadWrite<'_, T> {
    fn drop(&mut self) {
        self.shard
            .metadata_pages
            .lock()
            .unwrap()
            .free_entry_positions
            .extend(self.entries.drain(..).map(|(_, entry, _, _)| entry.metadata));
    }
}

impl<'a, T> PayloadWrite<'a, T> {
    fn new(shard: &'a DiskCacheShard) -> Self {
        Self {
            shard,
            chunks: None,
            used_bytes: 0,
            entries: Vec::new(),
            buffer: None,
        }
    }

    fn take(&mut self) -> Self {
        std::mem::replace(self, Self::new(self.shard))
    }

    async fn write(mut self, disk: &DiskCacheInner, bytes: Bytes) -> Result<usize, DiskCacheError> {
        let shard = self.shard;
        let Some(chunks) = self.chunks.take() else { return Ok(0) };
        let bytes = self.buffer.take().map_or(bytes, AlignedBuffer::into_bytes);
        let range = chunks.disk_byte_range();
        for address in range.clone().step_by(CHUNK_BYTES as usize) {
            let start = (address - range.start) as usize;
            let end = (start + CHUNK_BYTES as usize).min(bytes.len());
            if let Err(error) = disk
                .file
                .write_parts(address..address + CHUNK_BYTES, &[(0, bytes.slice(start..end))])
                .await
            {
                for (_, _, attempt, _) in &mut self.entries {
                    attempt.set_outcome(DiskWriteOutcome::Failed);
                }
                return Err(error.into());
            }
        }
        let entries = std::mem::take(&mut self.entries);
        disk.metrics.written_entries.increase(entries.len() as u64);
        disk.metrics
            .packed_payload_bytes
            .increase(entries.iter().map(|(_, entry, _, _)| entry.object_range.len()).sum());
        disk.metrics
            .packed_chunk_bytes
            .increase(chunks.chunk_count() * CHUNK_BYTES);
        shard
            .allocator
            .hold_chunks_until_payloads_released(chunks, entries.len());
        let mut index = shard.entry_index.lock().unwrap();
        let mut published = 0;
        for (key, entry, mut attempt, _completion) in entries.into_iter().rev() {
            if index.covering_entry(&key, entry.object_range).is_some() {
                attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                shard.release_payload_and_allow_metadata_overwrite(entry);
                continue;
            }
            shard.metadata_pages.lock().unwrap().set_entry_metadata(&key, &entry);
            for removed in index.insert(key, entry) {
                shard.release_payload_and_allow_metadata_overwrite(removed);
            }
            published += 1;
            attempt.set_outcome(DiskWriteOutcome::Published);
        }
        Ok(published)
    }
}
