//! Writes payload bytes before publishing entry metadata, with one eviction budget per write.

use super::*;

/// Metadata and guards for an entry awaiting a payload write and publication.
pub(super) struct WriteEntry {
    pub(super) key: ObjectKeyHash,
    pub(super) payload_len: u64,
    pub(super) payload_checksum: u64,
    pub(super) buffer_offset: usize,
    pub(super) attempt: DiskWriteAttempt,
    /// Dropped when the entry is published or discarded.
    pub(super) _completion_guard: Box<dyn Send>,
}

impl DiskCacheShard {
    pub(super) async fn write_payload(
        &self,
        disk: &DiskCacheInner,
        bytes: Bytes,
        mut entries: Vec<WriteEntry>,
    ) -> Result<usize, DiskCacheError> {
        let result = async {
            let required_chunks = (bytes.len() as u64).div_ceil(CHUNK_BYTES);
            let mut remaining_eviction_attempts = MAX_EVICTION_ATTEMPTS;
            let mut remaining_eviction_chunks = MAX_EVICTION_CHUNKS;
            let chunks = loop {
                let mut pages = self.metadata_pages.lock().unwrap();
                // Establish the metadata chain before writing payloads. Store entry metadata after I/O.
                let chunks = pages
                    .ensure_free_slots(entries.len(), &self.allocator)
                    .and_then(|()| self.allocator.reserve_chunks(required_chunks));
                let payload_chunk_capacity = self.allocator.chunk_capacity - pages.chunks.len() as u64;
                drop(pages);
                if let Some(chunks) = chunks {
                    break chunks;
                }
                if required_chunks > payload_chunk_capacity
                    || !self.sample_and_evict_entry(
                        disk,
                        &mut remaining_eviction_attempts,
                        &mut remaining_eviction_chunks,
                    )
                {
                    return Ok(0);
                }
                entries[0].attempt.evicted = remaining_eviction_chunks != MAX_EVICTION_CHUNKS;
            };
            let range = chunks.disk_byte_range();
            disk.file
                .write_padded(range.start, (range.end - range.start) as usize, &bytes)
                .await?;
            disk.metrics.written_entries.increase(entries.len() as u64);
            disk.metrics
                .packed_payload_bytes
                .increase(entries.iter().map(|entry| entry.payload_len).sum());
            disk.metrics
                .packed_chunk_bytes
                .increase(chunks.chunk_count() * CHUNK_BYTES);
            let mut disk_index = self.entry_index.lock().unwrap();
            let mut pages = self.metadata_pages.lock().unwrap();
            // Other writers may consume metadata space during I/O. Ensure room for every entry before publishing any.
            if pages.ensure_free_slots(entries.len(), &self.allocator).is_none() {
                return Ok(0);
            }
            self.allocator
                .hold_chunks_until_payloads_released(chunks, entries.len());
            let mut published = 0;
            let mut published_payload_bytes = 0;
            // Publish newest entries first so the last buffered write wins for duplicate keys.
            for mut buffered in entries.drain(..).rev() {
                if disk_index.entries_by_key.contains_key(&buffered.key) {
                    buffered.attempt.outcome = DiskWriteOutcome::AlreadyCovered;
                    self.allocator.release_payload(range.start);
                    continue;
                }
                let entry_metadata = metadata::EntryMetadata {
                    key: buffered.key,
                    payload_len: buffered.payload_len,
                    payload_address: range.start + buffered.buffer_offset as u64,
                    payload_checksum: buffered.payload_checksum,
                };
                let entry = DiskEntry::new(&entry_metadata, pages.store_entry_metadata(entry_metadata));
                self.insert_entry(&mut disk_index, &mut pages, buffered.key, entry);
                published += 1;
                published_payload_bytes += buffered.payload_len;
                buffered.attempt.outcome = DiskWriteOutcome::Published;
            }
            if published != 0 {
                disk.metrics.entries.increase(published as u64);
                disk.metrics.payload_bytes.increase(published_payload_bytes);
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
