//! Inserts update pages under the metadata mutex; one periodic writer copies dirty pages,
//! releases the mutex, then checksums and writes them sequentially. Changes during I/O remain dirty.
//! Failed writes are not retried. No stable-storage sync or shutdown flush; recovery is best-effort.

use std::{collections::BTreeSet, time::Duration};

use super::{page_format::*, *};

/// The location of one entry's metadata, ordered by metadata chunk then entry within that chunk.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct EntryMetadataLocation {
    /// Index into `MetadataPages::chunks`, in metadata-chain order.
    pub(super) chunk_index: u32,
    /// Entry index within that metadata chunk, across its metadata pages.
    pub(super) entry_index: u16,
}

/// Metadata pages in reserved chunks, with free entry positions and dirty-page tracking.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    pub(super) dirty_pages: BTreeSet<(usize, usize)>,
    free_entry_positions: BTreeSet<EntryMetadataLocation>,
}

/// One reserved metadata chunk and its independently checksummed pages.
pub(super) struct MetadataChunk {
    pub(super) reserved_chunk: ReservedChunks,
    pub(super) bytes: Box<[u8]>,
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
            bytes: vec![0; CHUNK_BYTES as usize].into_boxed_slice(),
        };
        chunk.clear_entry_metadata_page(0);
        let bytes = &mut chunk.bytes;
        for page in 1..ENTRY_METADATA_PAGES_PER_CHUNK {
            bytes.copy_within(..METADATA_PAGE_BYTES, page * METADATA_PAGE_BYTES);
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
    /// Gets the first free metadata location in recovery scan order and removes it from the free locations.
    pub(super) fn get_free_metadata_location(&mut self) -> Option<EntryMetadataLocation> {
        // Earliest reuse lets live metadata claim reused payload chunks before stale metadata.
        self.free_entry_positions.pop_first()
    }

    /// Makes space for one entry's metadata available for reuse without changing its bytes.
    pub(super) fn free_entry_metadata(&mut self, location: EntryMetadataLocation) {
        self.free_entry_positions.insert(location);
    }

    #[cfg(test)]
    pub(super) fn free_position_count(&self) -> usize {
        self.free_entry_positions.len()
    }

    /// Grows metadata capacity without taking positions; publication consumes them under the entry-index lock.
    pub(super) fn ensure_free_positions(&mut self, count: usize, allocator: &DiskChunkAllocator) -> Option<()> {
        while self.free_entry_positions.len() < count {
            let chunk = MetadataChunk::empty(allocator.reserve_chunks(1)?);
            self.set_last_chunk_link(chunk.reserved_chunk.disk_byte_range().start);
            let chunk_index = u32::try_from(self.chunks.len()).unwrap();
            self.chunks.push(chunk);
            self.dirty_pages
                .insert((chunk_index as usize, ENTRY_METADATA_PAGES_PER_CHUNK));
            self.free_entry_positions
                .extend(
                    (0..ENTRIES_PER_METADATA_CHUNK).map(|entry_index| EntryMetadataLocation {
                        chunk_index,
                        entry_index: entry_index as u16,
                    }),
                );
        }
        Some(())
    }

    /// Updates an entry's reserved metadata bytes and marks its page for writing.
    pub(super) fn set_entry_metadata(&mut self, key: &ObjectKeyHash, entry: &DiskEntry) {
        let location = entry.metadata;
        let offset = entry_metadata_offset(location.entry_index as usize);
        let bytes = &mut self.chunks[location.chunk_index as usize].bytes[offset..offset + ENTRY_METADATA_BYTES];
        encode_entry_metadata(bytes, key, entry);
        let page = location.entry_index as usize / ENTRIES_PER_METADATA_PAGE;
        self.dirty_pages.insert((location.chunk_index as usize, page));
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
        let writes: Box<[(u64, Box<[u8]>)]> = {
            let mut pages = self.metadata_pages.lock().unwrap();
            std::mem::take(&mut pages.dirty_pages)
                .into_iter()
                .rev() // Write later chunks before the links pointing to them.
                .map(|(chunk, page)| {
                    let chunk = &pages.chunks[chunk];
                    let offset = page * METADATA_PAGE_BYTES;
                    let start = chunk.reserved_chunk.disk_byte_range().start + offset as u64;
                    (start, chunk.bytes[offset..offset + METADATA_PAGE_BYTES].into())
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
