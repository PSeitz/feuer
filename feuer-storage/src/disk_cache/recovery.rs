//! Recover entries by following metadata-only chunks, never by scanning payload chunks.

use std::ops::Range;

use super::{metadata::EntryMetadataLocation, page_format::*, *};

/// The disk byte range assigned to one shard.
pub(super) fn shard_disk_range(capacity: u64, num_shards: usize, shard_index: usize) -> Range<u64> {
    let chunks = capacity / CHUNK_BYTES;
    chunks * shard_index as u64 / num_shards as u64 * CHUNK_BYTES
        ..chunks * (shard_index + 1) as u64 / num_shards as u64 * CHUNK_BYTES
}

impl DiskCacheInner {
    /// Recovers a shard's entries from metadata records before the cache becomes available.
    pub(super) async fn recover_shard(&self, shard_index: usize) {
        let shard = &self.shards[shard_index];
        let shard_disk_range = shard_disk_range(self.file.capacity(), self.shards.len(), shard_index);
        // Reserve every metadata chunk before accepting any payload addresses from the records.
        shard.load_metadata_chain(&self.file, shard_disk_range.start).await;
        let mut metadata = shard.metadata_pages.lock().unwrap();
        let mut disk_index = shard.entry_index.lock().unwrap();
        // Keep chunk bytes separate while recovery updates free metadata positions.
        let chunks = std::mem::take(&mut metadata.chunks);
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            for (entry_metadata_index, entry_metadata_bytes) in chunk.entry_metadata_records().enumerate() {
                let location = EntryMetadataLocation {
                    chunk_index: chunk_index.try_into().unwrap(),
                    entry_index: entry_metadata_index as u16,
                };
                let entry_metadata = decode_entry_metadata(entry_metadata_bytes, location).filter(|(key, entry)| {
                    self.shard_index_for_key(key) == shard_index
                        && disk_index.covering_entry(key, entry.object_range).is_none()
                        && shard
                            .allocator
                            .hold_chunks_for_recovered_payload(
                                &(entry.payload_address
                                    ..entry.payload_address + payload_disk_bytes(entry.object_range.len())),
                            )
                            .is_some()
                });
                let Some((key, entry)) = entry_metadata else {
                    metadata.free_entry_metadata(location);
                    continue;
                };
                shard.insert_entry(&mut disk_index, &mut metadata, key, entry);
            }
        }
        metadata.chunks = chunks;
    }
}

impl DiskCacheShard {
    /// Reads the metadata chain and resets invalid record pages before reserving payload chunks.
    async fn load_metadata_chain(&self, file: &DataFile, mut address: u64) {
        let mut metadata = metadata::MetadataPages::default();
        while address != NO_CHUNK {
            // The writer aligns links; reservations enforce current shard bounds and stop cycles.
            let Some(mut reserved_chunk) = self.allocator.reserve_chunks_at(address / CHUNK_BYTES, 1) else {
                break;
            };
            let Ok(bytes) = file.read_at(address, CHUNK_BYTES as usize).await else {
                break;
            };
            // An unwritten chunk ends the chain.
            if bytes.iter().all(|&byte| byte == 0) {
                break;
            }
            reserved_chunk.mark_recovered();
            let mut chunk = metadata::MetadataChunk {
                reserved_chunk,
                bytes: Box::from(bytes.as_ref()),
            };
            for page in 0..ENTRY_METADATA_PAGES_PER_CHUNK {
                let bytes = &chunk.bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES];
                if validate_page(bytes, ENTRY_METADATA_PAGE_TAG).is_none() {
                    // Clear invalid records before a later insertion rechecksums the page.
                    // Until then, the invalid page on disk is safe to leave untouched.
                    chunk.clear_entry_metadata_page(page);
                }
            }
            let next_chunk_address = chunk.next_chunk_address();
            metadata.chunks.push(chunk);
            match next_chunk_address {
                Some(next_chunk_address) => address = next_chunk_address,
                None => break,
            }
        }
        if address != NO_CHUNK {
            metadata.set_last_chunk_link(NO_CHUNK);
        }
        *self.metadata_pages.lock().unwrap() = metadata;
    }
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// Decodes an entry at its metadata position in a validated page. The writer guarantees
/// representable ranges and aligned payloads; all-zero records are unused.
fn decode_entry_metadata(
    bytes: &RawMetadataBytes,
    metadata: EntryMetadataLocation,
) -> Option<(ObjectKeyHash, DiskEntry)> {
    if bytes == &[0; ENTRY_METADATA_BYTES] {
        return None;
    }
    let key = ObjectKeyHash(u128::from_le_bytes(bytes[..16].try_into().unwrap()));
    let start = read_u64(bytes, 16);
    let object_range = ByteRange::new(start, start + read_u64(bytes, 24)).ok()?;
    let entry = DiskEntry {
        in_flight_read: Weak::new(),
        eviction_position: 0,
        object_range,
        payload_address: read_u64(bytes, 32),
        payload_checksum: read_u64(bytes, 40),
        metadata,
    };
    Some((key, entry))
}

#[cfg(test)]
mod benchmark;
#[cfg(test)]
pub(super) mod tests;
