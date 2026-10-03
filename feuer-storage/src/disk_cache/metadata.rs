//! Mutable metadata pages and entry positions. The async I/O lock serializes metadata writes only.
//! Writes are not synced to stable storage; recovery is best-effort after a crash.

use std::collections::BTreeSet;

use super::{page_format::*, *};

/// Metadata pages in reserved chunks, with free entry positions and dirty-page tracking.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    pub(super) dirty_pages: BTreeSet<(usize, usize)>,
    pub(super) free_entry_positions: Vec<(usize, usize)>,
    /// Number of metadata chunks already initialized on disk.
    pub(super) initialized_chunks: usize,
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
        let contents = validate_page(&self.bytes[offset..], NEXT_CHUNK_PAGE_TAG)?;
        Some(u64::from_le_bytes(contents[..8].try_into().unwrap()))
    }

    /// The 48 bytes describing one entry's key, object range, payload address, and checksum.
    pub(super) fn entry_metadata_bytes(&self, entry_metadata_index: usize) -> &[u8] {
        let offset = entry_metadata_offset(entry_metadata_index);
        &self.bytes[offset..offset + ENTRY_METADATA_BYTES]
    }
}

impl MetadataPages {
    /// Reserves an entry's metadata position, adding a metadata chunk when needed.
    pub(super) fn reserve_metadata(&mut self, allocator: &DiskChunkAllocator) -> Option<(usize, usize)> {
        if self.free_entry_positions.is_empty() {
            let chunk = MetadataChunk::empty(allocator.reserve_chunks(1)?);
            self.set_last_chunk_link(chunk.region.range().start);
            let chunk_index = self.chunks.len();
            self.chunks.push(chunk);
            self.free_entry_positions
                .extend((0..RECORDS_PER_CHUNK).rev().map(|entry| (chunk_index, entry)));
        }
        self.free_entry_positions.pop()
    }

    /// Updates an entry's reserved metadata bytes and marks its page for writing.
    pub(super) fn set_entry_metadata(&mut self, key: &ObjectKeyHash, entry: &DiskEntry) {
        let (chunk_index, entry_metadata_index) = entry.metadata.unwrap();
        let entry_metadata =
            encode_entry_metadata(key, entry.object_range, &entry.payload_range, entry.payload_checksum);
        let offset = entry_metadata_offset(entry_metadata_index);
        let bytes = &mut self.chunks[chunk_index].bytes;
        bytes[offset..offset + ENTRY_METADATA_BYTES].copy_from_slice(&entry_metadata);
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
    /// Writes new metadata chunks before changed pages. Concurrent changes remain dirty for the next flush.
    pub(super) async fn flush_metadata(&self, file: &DataFile) -> DataFileResult<()> {
        let _io = self.metadata_io.lock().await;
        let (dirty_pages, chunk_count, writes) = {
            let mut pages = self.metadata_pages.lock().unwrap();
            let dirty_pages = std::mem::take(&mut pages.dirty_pages);
            let mut writes: Vec<_> = pages.chunks[pages.initialized_chunks..]
                .iter()
                .rev() // Initialize linked chunks before the chunks pointing to them.
                .map(|chunk| (chunk.region.range(), Bytes::copy_from_slice(&chunk.bytes)))
                .collect();
            for &(chunk, page) in &dirty_pages {
                if chunk < pages.initialized_chunks {
                    let chunk = &pages.chunks[chunk];
                    let offset = page * METADATA_PAGE_BYTES;
                    let start = chunk.region.range().start + offset as u64;
                    writes.push((
                        start..start + METADATA_PAGE_BYTES as u64,
                        Bytes::copy_from_slice(&chunk.bytes[offset..offset + METADATA_PAGE_BYTES]),
                    ));
                }
            }
            (dirty_pages, pages.chunks.len(), writes)
        };
        for (region, bytes) in writes {
            if let Err(error) = file.write_parts(region, &[(0, bytes)]).await {
                self.metadata_pages.lock().unwrap().dirty_pages.extend(dirty_pages);
                return Err(error);
            }
        }
        self.metadata_pages.lock().unwrap().initialized_chunks = chunk_count;
        Ok(())
    }
}
