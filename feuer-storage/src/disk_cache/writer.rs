//! Writes and publishes payload runs with one eviction budget per run; only publication consumes metadata positions.

use super::*;

/// One entry's identity, checksum, offset in the write buffer, and completion accounting.
pub(super) struct BufferedEntry {
    pub(super) key: ObjectKeyHash,
    pub(super) object_range: ByteRange,
    pub(super) payload_checksum: u64,
    pub(super) offset: usize,
    pub(super) attempt: DiskWriteAttempt,
    pub(super) _completion: Box<dyn Send>,
}

impl DiskCacheShard {
    pub(super) async fn write_payload(
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
                if count > capacity || !self.sample_and_evict_entry(disk, &mut attempts, &mut budget) {
                    return Ok(0);
                }
                entries[0].attempt.evicted = budget != MAX_EVICTION_CHUNKS;
            };
            let range = chunks.disk_byte_range();
            disk.file
                .write_padded(range.start, (range.end - range.start) as usize, &bytes)
                .await?;
            disk.metrics.written_entries.increase(entries.len() as u64);
            disk.metrics
                .packed_payload_bytes
                .increase(entries.iter().map(|entry| entry.object_range.len()).sum());
            disk.metrics
                .packed_chunk_bytes
                .increase(chunks.chunk_count() * CHUNK_BYTES);
            let mut disk_index = self.entry_index.lock().unwrap();
            let mut pages = self.metadata_pages.lock().unwrap();
            // Other writers may have consumed free positions during I/O. Publish all or discard this run.
            if pages.ensure_free_positions(entries.len(), &self.allocator).is_none() {
                return Ok(0);
            }
            self.allocator
                .hold_chunks_until_payloads_released(chunks, entries.len());
            let mut published = 0;
            for mut buffered in entries.drain(..).rev() {
                if disk_index
                    .covering_entry(&buffered.key, buffered.object_range)
                    .is_some()
                {
                    buffered.attempt.outcome = DiskWriteOutcome::AlreadyCovered;
                    self.allocator.release_payload(range.start);
                    continue;
                }
                let entry = DiskEntry {
                    in_flight_read: Weak::new(),
                    eviction_position: 0,
                    object_range: buffered.object_range,
                    payload_checksum: buffered.payload_checksum,
                    payload_address: range.start + buffered.offset as u64,
                    metadata: pages.free_entry_positions.pop().unwrap(),
                };
                pages.set_entry_metadata(&buffered.key, &entry);
                self.insert_entry(&mut disk_index, &mut pages, buffered.key, entry);
                published += 1;
                buffered.attempt.outcome = DiskWriteOutcome::Published;
            }
            Ok(published)
        }
        .await;
        for entry in &mut entries {
            entry.attempt.outcome = if result.is_err() {
                DiskWriteOutcome::Failed
            } else {
                DiskWriteOutcome::NoCapacity
            };
        }
        result
    }
}
