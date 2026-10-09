//! Recover entries by following metadata-only chunks, never by scanning payload chunks.

use std::ops::Range;

use super::{
    metadata::{EntryMetadata, EntryMetadataLocation, FREE_SLOT, MetadataChunk, MetadataSlot},
    page_format::*,
    *,
};

/// Records of a page that failed validation: all unused.
const UNUSED_RECORDS: &[RawMetadataBytes] = &[[0; ENTRY_METADATA_BYTES]; ENTRIES_PER_METADATA_PAGE];

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
        for chunk_index in 0..metadata.chunks.len() {
            for entry_index in 0..ENTRIES_PER_METADATA_CHUNK {
                let slot = &mut metadata.chunks[chunk_index].slots[entry_index];
                let MetadataSlot::Entry(entry_metadata) = *slot else {
                    continue;
                };
                let payload = entry_metadata.payload_address
                    ..entry_metadata.payload_address + payload_disk_bytes(entry_metadata.object_range.len());
                if self.shard_index_for_key(&entry_metadata.key) != shard_index
                    || disk_index
                        .covering_entry(&entry_metadata.key, entry_metadata.object_range)
                        .is_some()
                    || shard.allocator.hold_chunks_for_recovered_payload(&payload).is_none()
                {
                    // Rejected records stay on disk until a later change rewrites their page.
                    *slot = FREE_SLOT;
                    continue;
                }
                let location = EntryMetadataLocation {
                    chunk_index: chunk_index.try_into().unwrap(),
                    entry_index: entry_index as u16,
                };
                let entry = DiskEntry::new(&entry_metadata, location);
                shard.insert_entry(&mut disk_index, &mut metadata, entry_metadata.key, entry);
            }
        }
        metadata.relink_free_slots();
    }
}

impl DiskCacheShard {
    /// Reads the metadata chain and decodes its records before reserving payload chunks.
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
            let (entry_pages, link_page) = bytes.split_at(ENTRY_METADATA_PAGES_PER_CHUNK * METADATA_PAGE_BYTES);
            let slots = entry_pages
                .as_chunks::<METADATA_PAGE_BYTES>()
                .0
                .iter()
                .flat_map(|page| {
                    // An invalid page holds no entries. It is safe to leave on disk until a later insertion rewrites
                    // it.
                    validate_page(page, ENTRY_METADATA_PAGE_TAG).map_or(UNUSED_RECORDS, |contents| {
                        contents[..PAGE_CONTENT_BYTES].as_chunks::<ENTRY_METADATA_BYTES>().0
                    })
                })
                .map(|bytes| decode_entry_metadata(bytes).map_or(FREE_SLOT, MetadataSlot::Entry))
                .collect();
            let next_chunk_address =
                validate_page(link_page, NEXT_CHUNK_PAGE_TAG).map(|contents| read_u64(contents, 0));
            metadata.chunks.push(MetadataChunk {
                reserved_chunk,
                slots,
                next_chunk_address: next_chunk_address.unwrap_or(NO_CHUNK),
            });
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

/// Decodes one record from a validated page. The writer guarantees
/// representable ranges and aligned payloads; all-zero records are unused.
fn decode_entry_metadata(bytes: &RawMetadataBytes) -> Option<EntryMetadata> {
    if bytes == &[0; ENTRY_METADATA_BYTES] {
        return None;
    }
    let start = read_u64(bytes, 16);
    Some(EntryMetadata {
        key: ObjectKeyHash(u128::from_le_bytes(bytes[..16].try_into().unwrap())),
        object_range: ByteRange::new(start, start + read_u64(bytes, 24)).ok()?,
        payload_address: read_u64(bytes, 32),
        payload_checksum: read_u64(bytes, 40),
    })
}

#[cfg(test)]
mod benchmark;
#[cfg(test)]
pub(super) mod tests;
