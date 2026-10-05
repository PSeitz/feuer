//! Each shard owns an initialized small-entry buffer; disk space is reserved only when writing.

use super::*;
use feuer_memory::AlignedBuffer;
use std::time::Duration;

const SMALL_ENTRY_BYTES: usize = 128 * 1024;
const FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// One chunk of small-entry bytes and their descriptions, with an aligned append offset.
pub(super) struct BufferedSmallEntryChunk {
    buffer: AlignedBuffer,
    used_bytes: usize,
    entries: Vec<BufferedEntry>,
}

/// One entry's identity, checksum, offset in the write buffer, and completion accounting.
pub(super) struct BufferedEntry {
    key: ObjectKeyHash,
    object_range: ByteRange,
    payload_checksum: u64,
    offset: usize,
    attempt: DiskWriteAttempt,
    completion: Box<dyn Send>,
}

impl DiskCache {
    /// Flushes shared buffers. Returns the number of entries published by this call.
    pub async fn flush(&self) -> Result<usize, DiskCacheError> {
        self.disk.flush().await
    }

    /// Discards buffered entries without writing them; retains each shard's initialized buffer.
    pub async fn discard_pending(&self) {
        for shard in &self.disk.shards {
            let mut buffer = shard.buffered_small_entry_chunk.lock().await;
            buffer.entries.clear();
            buffer.used_bytes = 0;
        }
    }

    /// Inserts a batch and flushes shared buffers. Counts entries published by this call, including neighbors.
    /// Earlier publication survives later failure. Dropping this future does not abort its detached writer.
    pub async fn insert_batch(&self, mut downloads: Vec<(ObjectKeyHash, Download)>) -> Result<usize, DiskCacheError> {
        let cache = self.clone();
        downloads.sort_by_key(|(_, download)| download.bytes().len());
        tokio::spawn(async move {
            let mut published = 0;
            for (key, download) in downloads {
                published += cache.write(key, download, ()).await?;
            }
            Ok(published + cache.flush().await?)
        })
        .await
        .map_err(DiskCacheError::WriteTaskFailed)?
    }

    /// Buffers small entries; full/no-fit chunks and larger entries write immediately.
    /// Holds completion state until publication or discard. Cancellation may leave submitted I/O running.
    pub async fn write(
        &self,
        key: ObjectKeyHash,
        download: Download,
        completion: impl Send + 'static,
    ) -> Result<usize, DiskCacheError> {
        self.disk.write(key, download, completion).await
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

    async fn flush(&self) -> Result<usize, DiskCacheError> {
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
        let disk = self;
        let shard = &disk.shards[disk.shard_index_for_key(&key)];
        let (object_range, bytes) = download.into_parts();
        let length = payload_disk_bytes(object_range.len()) as usize;
        let mut attempt = DiskWriteAttempt::new(disk.metrics.clone());
        if self.covers_range(&key, object_range) {
            attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
            return Ok(0);
        }
        let mut entry = BufferedEntry {
            key,
            object_range,
            payload_checksum: XxHash64::oneshot(0, &bytes),
            offset: 0,
            attempt,
            completion: Box::new(completion),
        };
        if length >= SMALL_ENTRY_BYTES {
            return shard.write_payload(disk, bytes, vec![entry]).await;
        }
        let mut buffer = shard.buffered_small_entry_chunk.lock().await;
        let mut published = 0;
        if buffer.used_bytes + length > CHUNK_BYTES as usize {
            published += buffer.flush(shard, disk).await?;
        }
        entry.offset = buffer.used_bytes;
        buffer.buffer.as_mut_slice()[entry.offset..entry.offset + bytes.len()].copy_from_slice(&bytes);
        drop(bytes);
        buffer.used_bytes += length;
        buffer.entries.push(entry);
        if buffer.used_bytes == CHUNK_BYTES as usize {
            published += buffer.flush(shard, disk).await?;
        }
        Ok(published)
    }
}

impl BufferedSmallEntryChunk {
    pub(super) fn new() -> Result<Self, DataFileError> {
        let buffer = AlignedBuffer::allocate_zeroed(CHUNK_BYTES as usize).map_err(|source| DataFileError::Task {
            operation: crate::IoOperation::Write,
            source: Box::new(source),
        })?;
        Ok(Self {
            buffer,
            used_bytes: 0,
            entries: Vec::new(),
        })
    }

    pub(super) fn covering_entry(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<&BufferedEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| &entry.key == key && entry.object_range.contains(requested))
    }

    pub(super) fn get(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<Bytes> {
        let entry = self.covering_entry(key, requested)?;
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
        let buffered = std::mem::replace(self, Self::new()?);
        shard
            .write_payload(disk, buffered.buffer.into_bytes(), buffered.entries)
            .await
    }
}

impl DiskCacheShard {
    /// Reserves one payload run with one eviction budget. Only publication consumes metadata positions.
    async fn write_payload(
        &self,
        disk: &DiskCacheInner,
        bytes: Bytes,
        mut entries: Vec<BufferedEntry>,
    ) -> Result<usize, DiskCacheError> {
        let result = async {
            let count = (bytes.len() as u64).div_ceil(CHUNK_BYTES);
            let mut attempts = MAX_EVICTION_ATTEMPTS;
            let mut budget = MAX_EVICTION_CHUNKS;
            let chunks = loop {
                let mut pages = self.metadata_pages.lock().unwrap();
                // Establish the metadata chain before payloads; leave all entry positions free during I/O.
                let chunks = pages
                    .ensure_free_positions(entries.len(), &self.allocator)
                    .and_then(|()| self.allocator.reserve_chunks(count));
                let capacity = self.allocator.chunk_capacity - pages.chunks.len() as u64;
                drop(pages);
                if let Some(chunks) = chunks {
                    break chunks;
                }
                if count > capacity || !self.sample_and_evict_entry(&disk.access_histories, &mut attempts, &mut budget)
                {
                    return Ok(0);
                }
                entries[0].attempt.evicted = budget != MAX_EVICTION_CHUNKS;
            };
            let range = chunks.disk_byte_range();
            for address in range.clone().step_by(CHUNK_BYTES as usize) {
                let start = (address - range.start) as usize;
                let end = (start + CHUNK_BYTES as usize).min(bytes.len());
                disk.file
                    .write_padded(address..address + CHUNK_BYTES, &bytes.slice(start..end))
                    .await?;
            }
            disk.metrics.written_entries.increase(entries.len() as u64);
            disk.metrics
                .packed_payload_bytes
                .increase(entries.iter().map(|entry| entry.object_range.len()).sum());
            disk.metrics
                .packed_chunk_bytes
                .increase(chunks.chunk_count() * CHUNK_BYTES);
            let mut index = self.entry_index.lock().unwrap();
            let mut pages = self.metadata_pages.lock().unwrap();
            // Other writers may have consumed free positions during I/O. Publish all or discard this run.
            if pages.ensure_free_positions(entries.len(), &self.allocator).is_none() {
                return Ok(0);
            }
            self.allocator
                .hold_chunks_until_payloads_released(chunks, entries.len());
            let mut published = 0;
            for mut buffered in entries.drain(..).rev() {
                let _completion = buffered.completion;
                if index.covering_entry(&buffered.key, buffered.object_range).is_some() {
                    buffered.attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                    self.allocator.release_payload(range.start);
                    continue;
                }
                let start = range.start + buffered.offset as u64;
                let entry = DiskEntry {
                    in_flight_read: Weak::new(),
                    eviction_position: 0,
                    object_range: buffered.object_range,
                    payload_checksum: buffered.payload_checksum,
                    payload_range: start..start + payload_disk_bytes(buffered.object_range.len()),
                    metadata: pages.free_entry_positions.pop().unwrap(),
                };
                pages.set_entry_metadata(&buffered.key, &entry);
                for removed in index.insert(buffered.key, entry) {
                    self.allocator.release_payload(removed.payload_range.start);
                    pages.free_entry_positions.push(removed.metadata);
                }
                published += 1;
                buffered.attempt.set_outcome(DiskWriteOutcome::Published);
            }
            Ok(published)
        }
        .await;
        for mut entry in entries {
            entry.attempt.set_outcome(if result.is_err() {
                DiskWriteOutcome::Failed
            } else {
                DiskWriteOutcome::NoCapacity
            });
        }
        result
    }
}
