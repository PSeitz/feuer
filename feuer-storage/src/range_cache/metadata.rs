//! Mutable metadata pages. The shard's async I/O lock serializes reads and writes.
//! Removed records retain their payload reservations until invalidation is durable.

use std::collections::BTreeSet;

use super::{page_format::*, *};

/// Metadata pages in reserved chunks, with reusable record slots and pending invalidations.
#[derive(Default)]
pub(super) struct MetadataPages {
    pub(super) chunks: Vec<MetadataChunk>,
    free_slots: Vec<(usize, usize)>,
    dirty_pages: BTreeSet<(usize, usize)>,
    pending_payload_releases: Vec<DiskRegion>,
    pub(super) closing: bool,
}

/// One reserved metadata chunk and its independently checksummed pages.
pub(super) struct MetadataChunk {
    pub(super) region: DiskRegion,
    pub(super) bytes: Vec<u8>,
}

/// One entry's metadata slot and the store containing it.
pub(super) struct MetadataSlot {
    pages: Weak<Mutex<MetadataPages>>,
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
        chunk.set_next(NO_CHUNK);
        chunk
    }

    fn set_next(&mut self, address: u64) {
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

    pub(super) fn next(&self) -> Option<u64> {
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

    pub(super) fn discard_record(&mut self, chunk: usize, slot: usize) {
        self.set_record(chunk, slot, &[0; ENTRY_METADATA_BYTES]);
        self.free_slots.push((chunk, slot));
    }

    pub(super) fn repair_page(&mut self, chunk: usize, page: usize) {
        self.free_slots
            .retain(|&(c, slot)| c != chunk || slot / RECORDS_PER_PAGE != page);
        let item = &mut self.chunks[chunk];
        encode_page(
            &mut item.bytes[page * METADATA_PAGE_BYTES..(page + 1) * METADATA_PAGE_BYTES],
            ENTRY_METADATA_PAGE_TAG,
            0,
            item.region.range().start + (page * METADATA_PAGE_BYTES) as u64,
            page as u64,
            RECORDS_PER_PAGE as u64,
            &[],
        );
        self.dirty_pages.insert((chunk, page));
        self.free_slots
            .extend((page * RECORDS_PER_PAGE..(page + 1) * RECORDS_PER_PAGE).map(|slot| (chunk, slot)));
    }
}

impl Drop for ObjectRangeDiskStorage {
    fn drop(&mut self) {
        if let Some(slot) = &self.metadata_slot
            && let Some(pages) = slot.pages.upgrade()
        {
            let mut pages = pages.lock().unwrap();
            if !pages.closing {
                pages.discard_record(slot.chunk, slot.slot);
                pages
                    .pending_payload_releases
                    .push(self.payload_region.slice(self.payload_region.range()));
            }
        }
    }
}

impl MetadataPages {
    pub(super) fn end_chain(&mut self) {
        if let Some(last) = self.chunks.len().checked_sub(1) {
            self.chunks[last].set_next(NO_CHUNK);
            self.dirty_pages.insert((last, RECORD_PAGES));
        }
    }
}

impl DiskCacheShard {
    /// Called under metadata_io; new chunks are initialized before the previous chunk links to them.
    pub(super) async fn ensure_metadata_slots(&self, file: &DataFile, count: usize) -> DataFileResult<bool> {
        if self.metadata.lock().unwrap().free_slots.len() >= count {
            return Ok(true);
        }
        let Some(region) = self.allocator.reserve_chunks(1) else {
            return Ok(false);
        };
        let chunk = MetadataChunk::empty(region);
        let address = chunk.region.range().start;
        file.write_parts(
            chunk.region.slice(chunk.region.range()),
            &[(0, Bytes::copy_from_slice(&chunk.bytes))],
        )
        .await?;
        // A durable link must never expose an uninitialized successor after a crash.
        file.sync_data().await?;
        {
            let mut pages = self.metadata.lock().unwrap();
            if let Some(last) = pages.chunks.len().checked_sub(1) {
                pages.chunks[last].set_next(address);
                pages.dirty_pages.insert((last, RECORD_PAGES));
            } else {
                self.metadata_head.store(address, Ordering::Relaxed);
            }
            pages.add_chunk(chunk);
        }
        self.flush_metadata(file).await?;
        Ok(true)
    }

    pub(super) fn record_entry(&self, key: &ObjectKeyHash, entry: &mut ObjectRangeDiskStorage) {
        let record = encode_entry_metadata(key, entry.object_range, &entry.payload_region, entry.payload_checksum);
        let mut pages = self.metadata.lock().unwrap();
        let (chunk, slot) = pages
            .free_slots
            .pop()
            .expect("metadata slots reserved before packing payloads");
        pages.set_record(chunk, slot, &record);
        entry.metadata_slot = Some(self.metadata_slot(chunk, slot));
    }

    pub(super) fn metadata_slot(&self, chunk: usize, slot: usize) -> MetadataSlot {
        MetadataSlot {
            pages: Arc::downgrade(&self.metadata),
            chunk,
            slot,
        }
    }

    /// Snapshot dirty pages without holding a synchronous lock over I/O. Concurrent removals
    /// create another dirty version and keep their reservations until that version is flushed.
    pub(super) async fn flush_metadata(&self, file: &DataFile) -> DataFileResult<()> {
        let (dirty, release_count, writes) = {
            let pages = self.metadata.lock().unwrap();
            let dirty = pages.dirty_pages.clone();
            let release_count = pages.pending_payload_releases.len();
            let writes: Vec<_> = dirty
                .iter()
                .map(|&(chunk, page)| {
                    let chunk = &pages.chunks[chunk];
                    let offset = page * METADATA_PAGE_BYTES;
                    let start = chunk.region.range().start + offset as u64;
                    (
                        chunk.region.slice(start..start + METADATA_PAGE_BYTES as u64),
                        Bytes::copy_from_slice(&chunk.bytes[offset..offset + METADATA_PAGE_BYTES]),
                    )
                })
                .collect();
            (dirty, release_count, writes)
        };
        for (region, bytes) in &writes {
            file.write_parts(region.slice(region.range()), &[(0, bytes.clone())])
                .await?;
        }
        if !dirty.is_empty() {
            file.sync_data().await?;
        }
        let mut pages = self.metadata.lock().unwrap();
        for ((chunk, page), (_, bytes)) in dirty.into_iter().zip(writes) {
            let offset = page * METADATA_PAGE_BYTES;
            if pages.chunks[chunk].bytes[offset..offset + METADATA_PAGE_BYTES] == bytes[..] {
                pages.dirty_pages.remove(&(chunk, page));
            }
        }
        // Errors and canceled flushes leave both dirty pages and payload ownership in the store.
        // Removals arriving during I/O belong to the next flush and retain their reservations.
        pages.pending_payload_releases.drain(..release_count);
        Ok(())
    }
}
