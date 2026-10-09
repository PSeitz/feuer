//! Inserts and removals update entry metadata slots under the metadata mutex; one periodic writer
//! encodes dirty pages, releases the mutex, then writes them sequentially. Changes during I/O remain dirty.
//! Failed writes are not retried. No stable-storage sync or shutdown flush; recovery is best-effort.

use std::{collections::BTreeSet, time::Duration};

use super::{page_format::*, *};

/// The location of one entry's metadata, ordered by metadata chunk then entry within that chunk.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct EntryMetadataLocation {
    /// Index into `MetadataPages::chunks`, in metadata-chain order.
    pub(super) chunk_index: u32,
    /// Entry index within that metadata chunk, across its metadata pages.
    pub(super) entry_index: u16,
}

/// The location of one metadata page within the metadata chain.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct MetadataPageLocation {
    pub(super) chunk_index: u32,
    pub(super) page_index: u8,
}

/// One entry's key hash, object range, payload disk address, and payload checksum, as recorded on disk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct EntryMetadata {
    pub(super) key: ObjectKeyHash,
    pub(super) object_range: ByteRange,
    pub(super) payload_address: u64,
    pub(super) payload_checksum: u64,
}

/// Metadata chunks with free-slot accounting and dirty-page tracking.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    pub(super) dirty_pages: BTreeSet<MetadataPageLocation>,
    /// No slot before this location is free.
    free_slot_search_start: EntryMetadataLocation,
    free_slot_count: usize,
}

/// One reserved metadata chunk: its entry metadata slots in disk order, and the next chunk's address.
/// Free slots are `None` and written as zeros.
pub(super) struct MetadataChunk {
    pub(super) reserved_chunk: ReservedChunks,
    pub(super) slots: Box<[Option<EntryMetadata>]>,
    /// The next metadata chunk's disk address, or `NO_CHUNK` at the end of the chain.
    pub(super) next_chunk_address: u64,
}

impl MetadataChunk {
    /// Encodes one checksummed page: an entry metadata page, or the final next-chunk page.
    fn encode_page(&self, page_index: usize) -> Box<[u8]> {
        let mut page = vec![0; METADATA_PAGE_BYTES].into_boxed_slice();
        if page_index == ENTRY_METADATA_PAGES_PER_CHUNK {
            encode_page(
                &mut page,
                NEXT_CHUNK_PAGE_TAG,
                1,
                &self.next_chunk_address.to_le_bytes(),
            );
            return page;
        }
        let mut contents = [0; PAGE_CONTENT_BYTES];
        let slots = &self.slots[page_index * ENTRIES_PER_METADATA_PAGE..][..ENTRIES_PER_METADATA_PAGE];
        for (bytes, slot) in contents.as_chunks_mut().0.iter_mut().zip(slots) {
            if let Some(entry) = slot {
                encode_entry_metadata(bytes, entry);
            }
        }
        encode_page(
            &mut page,
            ENTRY_METADATA_PAGE_TAG,
            ENTRIES_PER_METADATA_PAGE as u64,
            &contents,
        );
        page
    }
}

impl MetadataPages {
    /// Stores one entry's metadata in the first free slot and marks its page for writing.
    /// Earliest reuse lets live metadata claim reused payload chunks before stale metadata.
    /// Panics without a free slot; callers reserve slots with `ensure_free_slots`.
    pub(super) fn store_entry_metadata(&mut self, entry: EntryMetadata) -> EntryMetadataLocation {
        let mut start = self.free_slot_search_start;
        let location = loop {
            let slots = &self.chunks[start.chunk_index as usize].slots[start.entry_index as usize..];
            if let Some(offset) = slots.iter().position(Option::is_none) {
                break EntryMetadataLocation {
                    entry_index: start.entry_index + offset as u16,
                    ..start
                };
            }
            start = EntryMetadataLocation {
                chunk_index: start.chunk_index + 1,
                entry_index: 0,
            };
        };
        *self.slot_mut(location) = Some(entry);
        self.free_slot_search_start = location;
        self.free_slot_count -= 1;
        self.mark_page_dirty(location);
        location
    }

    /// Frees one entry's slot for reuse and marks its page for rewriting without the entry.
    pub(super) fn free_entry_metadata(&mut self, location: EntryMetadataLocation) {
        assert!(self.slot_mut(location).take().is_some(), "metadata slot freed twice");
        self.free_slot_search_start = self.free_slot_search_start.min(location);
        self.free_slot_count += 1;
        self.mark_page_dirty(location);
    }

    #[cfg(test)]
    pub(super) fn free_slot_count(&self) -> usize {
        self.free_slot_count
    }

    /// Grows capacity for one payload write (at most 256 entries); publication consumes slots under the entry-index lock.
    pub(super) fn ensure_free_slots(&mut self, count: usize, allocator: &DiskChunkAllocator) -> Option<()> {
        if self.free_slot_count < count {
            let reserved_chunk = allocator.reserve_chunks(1)?;
            self.set_last_chunk_link(reserved_chunk.disk_byte_range().start);
            self.chunks.push(MetadataChunk {
                reserved_chunk,
                slots: vec![None; ENTRIES_PER_METADATA_CHUNK].into(),
                next_chunk_address: NO_CHUNK,
            });
            self.mark_link_page_dirty(self.chunks.len() - 1);
            self.free_slot_count += ENTRIES_PER_METADATA_CHUNK;
        }
        Some(())
    }

    /// Recounts free slots after recovery and searches for them from the first slot.
    pub(super) fn reset_free_slots(&mut self) {
        self.free_slot_search_start = EntryMetadataLocation::default();
        self.free_slot_count = self
            .chunks
            .iter()
            .flat_map(|chunk| &chunk.slots)
            .filter(|slot| slot.is_none())
            .count();
    }

    pub(super) fn set_last_chunk_link(&mut self, address: u64) {
        if let Some(last) = self.chunks.last_mut() {
            last.next_chunk_address = address;
            self.mark_link_page_dirty(self.chunks.len() - 1);
        }
    }

    fn slot_mut(&mut self, location: EntryMetadataLocation) -> &mut Option<EntryMetadata> {
        &mut self.chunks[location.chunk_index as usize].slots[location.entry_index as usize]
    }

    fn mark_page_dirty(&mut self, location: EntryMetadataLocation) {
        self.dirty_pages.insert(MetadataPageLocation {
            chunk_index: location.chunk_index,
            page_index: (location.entry_index as usize / ENTRIES_PER_METADATA_PAGE) as u8,
        });
    }

    fn mark_link_page_dirty(&mut self, chunk_index: usize) {
        self.dirty_pages.insert(MetadataPageLocation {
            chunk_index: chunk_index.try_into().unwrap(),
            page_index: ENTRY_METADATA_PAGES_PER_CHUNK as u8,
        });
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
                    tracing::error!(target: "feuer::storage", %error, "metadata write failed; entries remain usable");
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
                .map(|location| {
                    let chunk = &pages.chunks[location.chunk_index as usize];
                    let offset = location.page_index as usize * METADATA_PAGE_BYTES;
                    let start = chunk.reserved_chunk.disk_byte_range().start + offset as u64;
                    (start, chunk.encode_page(location.page_index as usize))
                })
                .collect()
        };
        for (address, bytes) in writes {
            file.write_at(address, &Bytes::from(bytes)).await?;
        }
        Ok(())
    }
}
