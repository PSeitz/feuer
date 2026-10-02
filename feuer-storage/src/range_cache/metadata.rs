//! Mutable metadata pages. The shard's async I/O lock serializes reads and writes.
//! Writes are not synced to stable storage; recovery is best-effort after a crash.

use std::collections::BTreeSet;

use super::{page_format::*, *};

/// Metadata pages in reserved chunks, with dirty-page tracking.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    pub(super) dirty_pages: BTreeSet<(usize, usize)>,
}

/// One reserved metadata chunk and its independently checksummed pages.
pub(super) struct MetadataChunk {
    pub(super) region: DiskRegion,
    pub(super) bytes: Vec<u8>,
}

/// Byte offset of one entry's metadata within a chunk, skipping metadata page headers.
fn entry_metadata_offset(entry_metadata_index: usize) -> usize {
    entry_metadata_index / RECORDS_PER_PAGE * METADATA_PAGE_BYTES
        + PAGE_HEADER_BYTES
        + entry_metadata_index % RECORDS_PER_PAGE * ENTRY_METADATA_BYTES
}

impl MetadataChunk {
    pub(super) fn empty(region: DiskRegion) -> Self {
        let mut chunk = Self {
            region,
            bytes: vec![0; CHUNK_BYTES as usize],
        };
        for page in 0..RECORD_PAGES {
            chunk.reset_record_page(page);
        }
        chunk.set_next_chunk_address(NO_CHUNK);
        chunk
    }

    /// Resets a record page to empty records.
    pub(super) fn reset_record_page(&mut self, page: usize) {
        encode_page(
            &mut self.bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES],
            ENTRY_METADATA_PAGE_TAG,
            RECORDS_PER_PAGE as u64,
            &[],
        );
    }

    /// Sets the next metadata chunk's disk address, or `NO_CHUNK` to end the chain.
    fn set_next_chunk_address(&mut self, address: u64) {
        let offset = RECORD_PAGES * METADATA_PAGE_BYTES;
        encode_page(
            &mut self.bytes[offset..],
            NEXT_CHUNK_PAGE_TAG,
            1,
            &address.to_le_bytes(),
        );
    }

    /// Reads the next metadata chunk's disk address, or `NO_CHUNK` at the end of the chain.
    /// Returns `None` if the link page is invalid.
    pub(super) fn next_chunk_address(&self) -> Option<u64> {
        let offset = RECORD_PAGES * METADATA_PAGE_BYTES;
        let contents = validate_page(&self.bytes[offset..], NEXT_CHUNK_PAGE_TAG, 1)?;
        if contents[8..].iter().any(|&byte| byte != 0) {
            return None;
        }
        Some(u64::from_le_bytes(contents[..8].try_into().ok()?))
    }

    /// The 48 bytes describing one entry's key, object range, payload address, and checksum.
    pub(super) fn entry_metadata_bytes(&self, entry_metadata_index: usize) -> &[u8] {
        let offset = entry_metadata_offset(entry_metadata_index);
        &self.bytes[offset..offset + ENTRY_METADATA_BYTES]
    }
}

impl MetadataPages {
    fn set_entry_metadata(&mut self, chunk_index: usize, entry_metadata_index: usize, entry_metadata: &[u8]) {
        let offset = entry_metadata_offset(entry_metadata_index);
        let bytes = &mut self.chunks[chunk_index].bytes;
        bytes[offset..offset + ENTRY_METADATA_BYTES].copy_from_slice(entry_metadata);
        let page = entry_metadata_index / RECORDS_PER_PAGE;
        let page_bytes = &mut bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES];
        let checksum = XxHash64::oneshot(0, &page_bytes[8..]);
        page_bytes[..8].copy_from_slice(&checksum.to_le_bytes());
        self.dirty_pages.insert((chunk_index, page));
    }

    pub(super) fn set_last_chunk_link(&mut self, address: u64) {
        if let Some(last) = self.chunks.len().checked_sub(1) {
            self.chunks[last].set_next_chunk_address(address);
            self.dirty_pages.insert((last, RECORD_PAGES));
        }
    }
}

impl DiskCacheShard {
    /// Ensures metadata capacity for new entries, adding a chunk if needed.
    /// Called under metadata_io, before reserving payload chunks. The allocator returns the
    /// shard's first chunk for a new chain. New chunks are initialized before linking to them.
    pub(super) async fn ensure_metadata_capacity(&self, file: &DataFile, count: usize) -> DataFileResult<bool> {
        if self.allocator.available_metadata_count() >= count {
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
            pages.set_last_chunk_link(address);
            self.allocator.add_metadata_chunk(pages.chunks.len(), RECORDS_PER_CHUNK);
            pages.chunks.push(chunk);
        }
        self.flush_metadata(file).await?;
        Ok(true)
    }

    pub(super) fn record_entry(&self, key: &ObjectKeyHash, entry: &mut DiskEntry) {
        let entry_metadata =
            encode_entry_metadata(key, entry.object_range, &entry.payload_range, entry.payload_checksum);
        let (chunk_index, entry_metadata_index) = self
            .allocator
            .reserve_metadata()
            .expect("metadata capacity ensured before packing payloads");
        let mut pages = self.metadata.lock().unwrap();
        pages.set_entry_metadata(chunk_index, entry_metadata_index, &entry_metadata);
        entry.metadata = Some((chunk_index, entry_metadata_index));
    }

    /// Called under metadata_io.
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
