//! Mutable metadata pages. The shard's async I/O lock serializes reads and writes.
//! Writes are not synced to stable storage; recovery is best-effort after a crash.

use std::collections::BTreeSet;

use super::{page_format::*, *};

/// Metadata pages in reserved chunks, with reusable record slots and dirty pages.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    free_slots: Vec<(usize, usize)>,
    dirty_pages: BTreeSet<(usize, usize)>,
}

/// One reserved metadata chunk and its independently checksummed pages.
pub(super) struct MetadataChunk {
    pub(super) region: DiskRegion,
    pub(super) bytes: Vec<u8>,
}

/// One entry's record position in the metadata chunks.
pub(super) struct MetadataSlot {
    pub(super) chunk: usize,
    pub(super) slot: usize,
}

fn record_offset(slot: usize) -> usize {
    slot / RECORDS_PER_PAGE * METADATA_PAGE_BYTES + PAGE_HEADER_BYTES + slot % RECORDS_PER_PAGE * ENTRY_METADATA_BYTES
}

impl MetadataChunk {
    pub(super) fn empty(region: DiskRegion) -> Self {
        let mut bytes = vec![0; CHUNK_BYTES as usize];
        for ordinal in 0..RECORD_PAGES {
            encode_page(
                &mut bytes[ordinal * METADATA_PAGE_BYTES..(ordinal + 1) * METADATA_PAGE_BYTES],
                ENTRY_METADATA_PAGE_TAG,
                0,
                region.range().start + (ordinal * METADATA_PAGE_BYTES) as u64,
                ordinal as u64,
                RECORDS_PER_PAGE as u64,
                &[],
            );
        }
        let mut chunk = Self { region, bytes };
        chunk.set_next_chunk_address(NO_CHUNK);
        chunk
    }

    /// Sets the next metadata chunk's disk address, or `NO_CHUNK` to end the chain.
    fn set_next_chunk_address(&mut self, address: u64) {
        let offset = RECORD_PAGES * METADATA_PAGE_BYTES;
        encode_page(
            &mut self.bytes[offset..],
            NEXT_CHUNK_PAGE_TAG,
            0,
            self.region.range().start + offset as u64,
            RECORD_PAGES as u64,
            1,
            &address.to_le_bytes(),
        );
    }

    /// Reads the next metadata chunk's disk address, or `NO_CHUNK` at the end of the chain.
    /// Returns `None` if the link page is invalid.
    pub(super) fn next_chunk_address(&self) -> Option<u64> {
        let offset = RECORD_PAGES * METADATA_PAGE_BYTES;
        let (count, contents) = validate_page(
            &self.bytes[offset..],
            NEXT_CHUNK_PAGE_TAG,
            0,
            self.region.range().start + offset as u64,
            RECORD_PAGES as u64,
        )?;
        if count != 1 || contents[8..].iter().any(|&byte| byte != 0) {
            return None;
        }
        Some(u64::from_le_bytes(contents[..8].try_into().ok()?))
    }

    pub(super) fn record(&self, slot: usize) -> &[u8] {
        let offset = record_offset(slot);
        &self.bytes[offset..offset + ENTRY_METADATA_BYTES]
    }
}

impl MetadataPages {
    pub(super) fn add_chunk(&mut self, chunk: MetadataChunk) -> usize {
        let index = self.chunks.len();
        self.free_slots.extend((0..RECORDS_PER_CHUNK).rev().filter_map(|slot| {
            chunk
                .record(slot)
                .iter()
                .all(|&byte| byte == 0)
                .then_some((index, slot))
        }));
        self.chunks.push(chunk);
        index
    }

    fn set_record(&mut self, chunk: usize, slot: usize, record: &[u8]) {
        let offset = record_offset(slot);
        let bytes = &mut self.chunks[chunk].bytes;
        bytes[offset..offset + ENTRY_METADATA_BYTES].copy_from_slice(record);
        let page = slot / RECORDS_PER_PAGE;
        let page_bytes = &mut bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES];
        let checksum = XxHash64::oneshot(0, &page_bytes[8..]);
        page_bytes[..8].copy_from_slice(&checksum.to_le_bytes());
        self.dirty_pages.insert((chunk, page));
    }

    /// Recycles a slot without invalidating its old record. Reads validate payload checksums.
    pub(super) fn release_slot(&mut self, chunk: usize, slot: usize) {
        self.free_slots.push((chunk, slot));
    }

    /// Resets a record page to empty records and makes all its slots reusable.
    pub(super) fn reset_record_page(&mut self, chunk_index: usize, page: usize) {
        self.free_slots
            .retain(|&(chunk, slot)| chunk != chunk_index || slot / RECORDS_PER_PAGE != page);
        let chunk = &mut self.chunks[chunk_index];
        encode_page(
            &mut chunk.bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES],
            ENTRY_METADATA_PAGE_TAG,
            0,
            chunk.region.range().start + (page * METADATA_PAGE_BYTES) as u64,
            page as u64,
            RECORDS_PER_PAGE as u64,
            &[],
        );
        self.dirty_pages.insert((chunk_index, page));
        self.free_slots
            .extend((page * RECORDS_PER_PAGE..(page + 1) * RECORDS_PER_PAGE).map(|slot| (chunk_index, slot)));
    }
}

impl MetadataPages {
    pub(super) fn end_chain(&mut self) {
        if let Some(last) = self.chunks.len().checked_sub(1) {
            self.chunks[last].set_next_chunk_address(NO_CHUNK);
            self.dirty_pages.insert((last, RECORD_PAGES));
        }
    }
}

impl DiskCacheShard {
    /// Called under metadata_io, before reserving payload chunks. The allocator returns the
    /// shard's first chunk for a new chain. New chunks are initialized before linking to them.
    pub(super) async fn ensure_metadata_slots(&self, file: &DataFile, count: usize) -> DataFileResult<bool> {
        if self.metadata.lock().unwrap().free_slots.len() >= count {
            return Ok(true);
        }
        let Some(region) = self.allocator.reserve_chunks(1) else {
            return Ok(false);
        };
        let chunk = MetadataChunk::empty(region);
        let address = chunk.region.range().start;
        file.write_parts(chunk.region.range(), &[(0, Bytes::copy_from_slice(&chunk.bytes))])
            .await?;
        {
            let mut pages = self.metadata.lock().unwrap();
            if let Some(last) = pages.chunks.len().checked_sub(1) {
                pages.chunks[last].set_next_chunk_address(address);
                pages.dirty_pages.insert((last, RECORD_PAGES));
            }
            pages.add_chunk(chunk);
        }
        self.flush_metadata(file).await?;
        Ok(true)
    }

    pub(super) fn record_entry(&self, key: &ObjectKeyHash, entry: &mut DiskEntry) {
        let record = encode_entry_metadata(key, entry.object_range, &entry.payload_range, entry.payload_checksum);
        let mut pages = self.metadata.lock().unwrap();
        let (chunk, slot) = pages
            .free_slots
            .pop()
            .expect("metadata slots reserved before packing payloads");
        pages.set_record(chunk, slot, &record);
        entry.metadata_slot = Some(MetadataSlot { chunk, slot });
    }

    /// Called under metadata_io. Slot recycling does not modify page contents.
    pub(super) async fn flush_metadata(&self, file: &DataFile) -> DataFileResult<()> {
        let writes: Vec<_> = {
            let pages = self.metadata.lock().unwrap();
            pages
                .dirty_pages
                .iter()
                .map(|&(chunk, page)| {
                    let chunk = &pages.chunks[chunk];
                    let offset = page * METADATA_PAGE_BYTES;
                    let start = chunk.region.range().start + offset as u64;
                    (
                        start..start + METADATA_PAGE_BYTES as u64,
                        Bytes::copy_from_slice(&chunk.bytes[offset..offset + METADATA_PAGE_BYTES]),
                    )
                })
                .collect()
        };
        for (region, bytes) in writes {
            file.write_parts(region, &[(0, bytes)]).await?;
        }
        self.metadata.lock().unwrap().dirty_pages.clear();
        Ok(())
    }
}
