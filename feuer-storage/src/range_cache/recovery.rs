//! Recover entries by following metadata-only chunks, never by scanning payload chunks.

use std::ops::Range;

use super::{page_format::*, *};

pub(super) fn shard_range(capacity: u64, count: usize, index: usize) -> Range<u64> {
    let chunks = capacity / CHUNK_BYTES;
    chunks * index as u64 / count as u64 * CHUNK_BYTES..chunks * (index + 1) as u64 / count as u64 * CHUNK_BYTES
}

impl DiskRangeCacheState {
    /// Scan a shard's metadata on a blocking thread before the cache becomes available.
    pub(super) async fn recover_shard(&self, shard_index: usize) {
        let shard = &self.shards[shard_index];
        let bounds = shard_range(self.file.capacity(), self.shards.len(), shard_index);
        // Reserve every metadata chunk before accepting any payload addresses from the records.
        shard.load_metadata_chain(&self.file, bounds.clone()).await;
        let mut payload_chunks = BTreeMap::<u64, DiskRegion>::new();
        let mut payload_ranges = BTreeMap::<u64, u64>::new();
        let count = shard.metadata.lock().unwrap().chunks.len();
        let mut entries = Vec::new();
        for chunk in 0..count {
            let mut recovered_chunks = 0;
            {
                let mut pages = shard.metadata.lock().unwrap();
                for slot in 0..RECORDS_PER_CHUNK {
                    let record = pages.chunks[chunk].record(slot);
                    if record.iter().all(|&byte| byte == 0) {
                        continue;
                    }
                    let entry = decode_entry(record, bounds.clone()).filter(|(key, _, _, payload)| {
                        self.shard_index_for_key(key) == shard_index
                            && payload_ranges
                                .range(..payload.end)
                                .next_back()
                                .is_none_or(|(_, end)| *end <= payload.start)
                    });
                    let Some((key, object_range, checksum, payload)) = entry else {
                        pages.discard_record(chunk, slot);
                        continue;
                    };
                    let start = payload.start / CHUNK_BYTES * CHUNK_BYTES;
                    let end = payload.end.next_multiple_of(CHUNK_BYTES);
                    if let std::collections::btree_map::Entry::Vacant(entry) = payload_chunks.entry(start) {
                        let Some(region) = shard
                            .allocator
                            .recover_payload_chunks(start / CHUNK_BYTES, (end - start) / CHUNK_BYTES)
                        else {
                            pages.discard_record(chunk, slot);
                            continue;
                        };
                        recovered_chunks += region.chunk_count();
                        entry.insert(region);
                    }
                    let region = &payload_chunks[&start];
                    if payload.end > region.range().end {
                        pages.discard_record(chunk, slot);
                        continue;
                    }
                    let payload_region = region.slice(payload.clone());
                    let storage = DiskEntry {
                        in_flight_read: Weak::new(),
                        eviction_position: 0,
                        object_range,
                        payload_checksum: checksum,
                        metadata_slot: Some(shard.metadata_slot(chunk, slot)),
                        payload_region,
                    };
                    payload_ranges.insert(payload.start, payload.end);
                    entries.push((key, storage));
                }
            }
            // Account for retained payload chunks before publishing the batch.
            self.metrics.free_chunks.decrease(recovered_chunks);
            self.metrics.allocated_chunks.increase(recovered_chunks);
            self.metrics.recovered_chunks.increase(recovered_chunks);
            shard.entry_index.lock().unwrap().insert_batch(&mut entries);
        }
        shard.allocator.finish_recovery();
    }
}

impl DiskCacheShard {
    /// Read and repair the metadata chain before any entry can claim payload chunks.
    async fn load_metadata_chain(&self, file: &DataFile, bounds: Range<u64>) {
        let mut pages = metadata::MetadataPages::default();
        let mut address = bounds.start;
        while address != NO_CHUNK {
            if !address.is_multiple_of(CHUNK_BYTES) || !bounds.contains(&address) {
                break;
            }
            let Some(region) = self.allocator.reserve_for_recovery(address / CHUNK_BYTES, 1) else {
                break;
            };
            let Ok(bytes) = file.read_recovery_chunk(&region).await else {
                break;
            };
            // An unwritten chunk ends the chain; a new shard initializes its first chunk on insertion.
            if bytes.iter().all(|&byte| byte == 0) {
                break;
            }
            region.mark_recovered();
            let chunk = metadata::MetadataChunk {
                region,
                bytes: bytes.to_vec(),
            };
            let next = chunk.next();
            let chunk_index = pages.add_chunk(chunk);
            for page in 0..RECORD_PAGES {
                let bytes =
                    &pages.chunks[chunk_index].bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES];
                let valid = validate_page(
                    bytes,
                    ENTRY_METADATA_PAGE_TAG,
                    0,
                    address + (page * METADATA_PAGE_BYTES) as u64,
                    page as u64,
                )
                .is_some_and(|(count, contents)| {
                    count == RECORDS_PER_PAGE as u64 && contents[PAGE_CONTENT_BYTES..].iter().all(|&byte| byte == 0)
                });
                if !valid {
                    pages.repair_page(chunk_index, page);
                }
            }
            match next {
                Some(next) => address = next,
                None => break,
            }
        }
        if address != NO_CHUNK {
            pages.end_chain();
        }
        *self.metadata.lock().unwrap() = pages;
    }
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

fn valid_payload_range(range: &Range<u64>, bounds: Range<u64>) -> bool {
    range.start < range.end
        && range.start >= bounds.start
        && range.end <= bounds.end
        && range.start.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && range.end.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
        && (range.end <= (range.start / CHUNK_BYTES + 1) * CHUNK_BYTES || range.start.is_multiple_of(CHUNK_BYTES))
}

type DecodedEntry = (ObjectKeyHash, ByteRange, u64, Range<u64>);

fn decode_entry(bytes: &[u8], bounds: Range<u64>) -> Option<DecodedEntry> {
    if bytes.len() != ENTRY_METADATA_BYTES {
        return None;
    }
    let key = ObjectKeyHash(u128::from_le_bytes(bytes[..16].try_into().ok()?));
    let start = read_u64(bytes, 16)?;
    let range = ByteRange::new(start, start.checked_add(read_u64(bytes, 24)?)?).ok()?;
    let checksum = read_u64(bytes, 40)?;
    let start = read_u64(bytes, 32)?;
    let length =
        range.len().checked_add(PAYLOAD_ALIGNMENT_BYTES - 1)? / PAYLOAD_ALIGNMENT_BYTES * PAYLOAD_ALIGNMENT_BYTES;
    let payload = start..start.checked_add(length)?;
    valid_payload_range(&payload, bounds).then_some((key, range, checksum, payload))
}

#[cfg(test)]
pub(super) mod tests;
