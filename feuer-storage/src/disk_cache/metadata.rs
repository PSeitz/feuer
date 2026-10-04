//! Inserts update pages under the metadata mutex; one periodic writer copies dirty pages,
//! releases the mutex, then checksums and writes them sequentially. Changes during I/O remain dirty.
//! Failed writes are not retried. No stable-storage sync or shutdown flush; recovery is best-effort.

use std::{collections::BTreeSet, time::Duration};

use super::{page_format::*, *};

/// Metadata pages in reserved chunks, with free entry positions and dirty-page tracking.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    pub(super) dirty_pages: BTreeSet<(usize, usize)>,
    pub(super) free_entry_positions: Vec<(usize, usize)>,
}

/// One reserved metadata chunk and its independently checksummed pages.
pub(super) struct MetadataChunk {
    pub(super) reserved_chunk: ReservedChunks,
    pub(super) bytes: Vec<u8>,
}

/// Byte offset of one entry's metadata within a chunk, skipping metadata page headers.
fn entry_metadata_offset(entry_metadata_index: usize) -> usize {
    entry_metadata_index / ENTRIES_PER_METADATA_PAGE * METADATA_PAGE_BYTES
        + PAGE_HEADER_BYTES
        + entry_metadata_index % ENTRIES_PER_METADATA_PAGE * ENTRY_METADATA_BYTES
}

impl MetadataChunk {
    pub(super) fn empty(reserved_chunk: ReservedChunks) -> Self {
        let mut chunk = Self {
            reserved_chunk,
            bytes: vec![0; CHUNK_BYTES as usize],
        };
        for page in 0..ENTRY_METADATA_PAGES_PER_CHUNK {
            chunk.clear_entry_metadata_page(page);
        }
        chunk.set_next_chunk_address(NO_CHUNK);
        chunk
    }

    /// Clears a page's entry metadata.
    pub(super) fn clear_entry_metadata_page(&mut self, page: usize) {
        encode_page(
            &mut self.bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES],
            ENTRY_METADATA_PAGE_TAG,
            ENTRIES_PER_METADATA_PAGE as u64,
            &[],
        );
    }

    /// Sets the next metadata chunk's disk address, or `NO_CHUNK` to end the chain.
    fn set_next_chunk_address(&mut self, address: u64) {
        let offset = ENTRY_METADATA_PAGES_PER_CHUNK * METADATA_PAGE_BYTES;
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
        let offset = ENTRY_METADATA_PAGES_PER_CHUNK * METADATA_PAGE_BYTES;
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
    /// Grows metadata capacity without taking positions; publication consumes them under the entry-index lock.
    pub(super) fn ensure_free_positions(&mut self, count: usize, allocator: &DiskChunkAllocator) -> Option<()> {
        while self.free_entry_positions.len() < count {
            let chunk = MetadataChunk::empty(allocator.reserve_chunks(1)?);
            self.set_last_chunk_link(chunk.reserved_chunk.disk_byte_range().start);
            let chunk_index = self.chunks.len();
            self.chunks.push(chunk);
            self.dirty_pages.insert((chunk_index, ENTRY_METADATA_PAGES_PER_CHUNK));
            self.free_entry_positions
                .extend((0..ENTRIES_PER_METADATA_CHUNK).rev().map(|entry| (chunk_index, entry)));
        }
        Some(())
    }

    /// Updates an entry's reserved metadata bytes and marks its page for writing.
    pub(super) fn set_entry_metadata(&mut self, key: &ObjectKeyHash, entry: &DiskEntry) {
        let (chunk_index, entry_metadata_index) = entry.metadata;
        let entry_metadata = encode_entry_metadata(
            key,
            entry.object_range,
            entry.payload_range.start,
            entry.payload_checksum,
        );
        let offset = entry_metadata_offset(entry_metadata_index);
        let bytes = &mut self.chunks[chunk_index].bytes;
        bytes[offset..offset + ENTRY_METADATA_BYTES].copy_from_slice(&entry_metadata);
        let page = entry_metadata_index / ENTRIES_PER_METADATA_PAGE;
        self.dirty_pages.insert((chunk_index, page));
    }

    pub(super) fn set_last_chunk_link(&mut self, address: u64) {
        if let Some(last) = self.chunks.len().checked_sub(1) {
            self.chunks[last].set_next_chunk_address(address);
            self.dirty_pages.insert((last, ENTRY_METADATA_PAGES_PER_CHUNK));
        }
    }
}

impl DiskCacheInner {
    /// Sole metadata writer. Keeps the cache alive only while a write round is running.
    pub(super) async fn write_metadata_periodically(disk: Weak<Self>) {
        const INTERVAL: Duration = Duration::from_secs(1);
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + INTERVAL, INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(disk) = disk.upgrade() else { return };
            for shard in &disk.shards {
                if let Err(error) = shard.write_dirty_metadata_pages(&disk.file).await {
                    tracing::warn!(target: "feuer::storage", %error, "metadata write failed; entries remain usable");
                }
            }
        }
    }
}

impl DiskCacheShard {
    /// Writes dirty metadata pages without syncing them to stable storage.
    /// Called only by the periodic writer. Never holds the metadata mutex while waiting for I/O.
    pub(super) async fn write_dirty_metadata_pages(&self, file: &DataFile) -> DataFileResult<()> {
        let writes: Vec<_> = {
            let mut pages = self.metadata_pages.lock().unwrap();
            std::mem::take(&mut pages.dirty_pages)
                .into_iter()
                .rev() // Write later chunks before the links pointing to them.
                .map(|(chunk, page)| {
                    let chunk = &pages.chunks[chunk];
                    let offset = page * METADATA_PAGE_BYTES;
                    let start = chunk.reserved_chunk.disk_byte_range().start + offset as u64;
                    (start, chunk.bytes[offset..offset + METADATA_PAGE_BYTES].to_vec())
                })
                .collect()
        };
        for (address, mut bytes) in writes {
            let checksum = XxHash64::oneshot(0, &bytes[8..]);
            bytes[..8].copy_from_slice(&checksum.to_le_bytes());
            file.write_at(address, &Bytes::from(bytes)).await?;
        }
        Ok(())
    }
}
