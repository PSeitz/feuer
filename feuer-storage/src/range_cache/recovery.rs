//! Recover entries by following metadata-only chunks, never by scanning payload chunks.

use std::ops::Range;

use super::{page_format::*, *};

/// The disk byte range assigned to one shard.
pub(super) fn shard_disk_range(capacity: u64, count: usize, index: usize) -> Range<u64> {
    let chunks = capacity / CHUNK_BYTES;
    chunks * index as u64 / count as u64 * CHUNK_BYTES..chunks * (index + 1) as u64 / count as u64 * CHUNK_BYTES
}

impl DiskRangeCacheState {
    /// Recovers a shard's entries from metadata records before the cache becomes available.
    pub(super) async fn recover_shard(&self, shard_index: usize) {
        let shard = &self.shards[shard_index];
        let shard_disk_range = shard_disk_range(self.file.capacity(), self.shards.len(), shard_index);
        // Reserve every metadata chunk before accepting any payload addresses from the records.
        shard.load_metadata_chain(&self.file, shard_disk_range.start).await;
        let metadata = shard.metadata.lock().unwrap();
        let mut index = shard.entry_index.lock().unwrap();
        for chunk_index in 0..metadata.chunks.len() {
            for entry_metadata_index in 0..RECORDS_PER_CHUNK {
                let entry_metadata_bytes = metadata.chunks[chunk_index].entry_metadata_bytes(entry_metadata_index);
                let entry_metadata = decode_entry_metadata(entry_metadata_bytes, shard_disk_range.clone()).filter(
                    |(key, _, _, payload_range)| {
                        self.shard_index_for_key(key) == shard_index
                            && shard.allocator.reserve_payload_chunks(payload_range).is_some()
                    },
                );
                let Some((key, object_range, payload_checksum, payload_range)) = entry_metadata else {
                    shard
                        .allocator
                        .allow_metadata_overwrite((chunk_index, entry_metadata_index));
                    continue;
                };
                let entry = DiskEntry {
                    in_flight_read: Weak::new(),
                    eviction_position: 0,
                    object_range,
                    payload_checksum,
                    metadata: Some((chunk_index, entry_metadata_index)),
                    payload_range,
                };
                for entry in index.insert(key, entry) {
                    shard.remove_payload(entry);
                }
            }
        }
    }
}

impl DiskCacheShard {
    /// Reads the metadata chain and resets invalid record pages before reserving payload chunks.
    async fn load_metadata_chain(&self, file: &DataFile, mut address: u64) {
        let mut metadata = metadata::MetadataPages::default();
        while address != NO_CHUNK {
            if !address.is_multiple_of(CHUNK_BYTES) {
                break;
            }
            let Some(mut region) = self.allocator.reserve_for_recovery(address / CHUNK_BYTES, 1) else {
                break;
            };
            let Ok(bytes) = file.read_recovery_chunk(address).await else {
                break;
            };
            // An unwritten chunk ends the chain; a new shard initializes its first chunk on insertion.
            if bytes.iter().all(|&byte| byte == 0) {
                break;
            }
            region.mark_recovered();
            let mut chunk = metadata::MetadataChunk {
                region,
                bytes: bytes.to_vec(),
            };
            for page in 0..RECORD_PAGES {
                let bytes = &chunk.bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES];
                if validate_page(bytes, ENTRY_METADATA_PAGE_TAG, RECORDS_PER_PAGE as u64).is_none() {
                    // Clear invalid records before a later insertion rechecksums the page.
                    // Until then, the invalid page on disk is safe to leave untouched.
                    chunk.reset_record_page(page);
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
        *self.metadata.lock().unwrap() = metadata;
    }
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn valid_payload_range(range: &Range<u64>, shard_disk_range: Range<u64>) -> bool {
    range.start >= shard_disk_range.start
        && range.end <= shard_disk_range.end
        && range.start.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && (range.end <= (range.start / CHUNK_BYTES + 1) * CHUNK_BYTES || range.start.is_multiple_of(CHUNK_BYTES))
}

/// An entry's object key, object range, payload checksum, and payload disk range.
type EntryMetadata = (ObjectKeyHash, ByteRange, u64, Range<u64>);

/// Decodes an entry's metadata, rejecting payload ranges outside the shard or with invalid alignment.
fn decode_entry_metadata(bytes: &[u8], shard_disk_range: Range<u64>) -> Option<EntryMetadata> {
    if bytes.len() != ENTRY_METADATA_BYTES {
        return None;
    }
    let key = ObjectKeyHash(u128::from_le_bytes(bytes[..16].try_into().unwrap()));
    let start = read_u64(bytes, 16);
    let range = ByteRange::new(start, start.checked_add(read_u64(bytes, 24))?).ok()?;
    let checksum = read_u64(bytes, 40);
    let start = read_u64(bytes, 32);
    let length = range.len().checked_next_multiple_of(PAYLOAD_ALIGNMENT_BYTES)?;
    let payload = start..start.checked_add(length)?;
    valid_payload_range(&payload, shard_disk_range).then_some((key, range, checksum, payload))
}

#[cfg(test)]
pub(super) mod tests;
