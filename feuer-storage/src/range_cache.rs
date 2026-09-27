//! Experimental immutable batch writes, live range lookup and whole-entry validation.

mod page_format;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap, hash_map::DefaultHasher},
    fmt,
    hash::{Hash, Hasher},
    path::Path,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use feuer_types::{ByteRange, Download, ObjectKey};

use crate::{
    DataFile, DataFileError, DataFileResult, IoMetrics,
    allocation::{BLOCK_BYTES, CHUNK_BYTES, DiskAllocator, DiskRegion, DiskRegionReadGuard},
};
use page_format::{PAGE_BYTES, PAGE_CONTENT_BYTES};

/// A standalone experimental disk cache, not yet connected to the public tiered cache.
///
/// Explicit batches pack smaller entries together and write complete immutable 1-MiB chunks.
/// Entries have plain payload bytes, 4-KiB-aligned storage, and a checksum in their entry metadata.
/// Reads verify the whole covering entry, returning only requested bytes. Reuse requires a wholly free chunk.
/// Full-key hashing selects an independently allocated shard; admission may fail despite space elsewhere.
///
/// **Recovery and pressure eviction are not implemented.** Every open starts empty and logs that reset.
/// The experimental format is neither a persistence guarantee nor a stable on-disk interface.
#[derive(Clone)]
pub struct DiskRangeCache {
    disk: Arc<DiskRangeCacheInner>,
}

/// Shared inner state of the disk range cache: its file and independently allocated arenas.
struct DiskRangeCacheInner {
    file: DataFile,
    arenas: Box<[Shard]>,
}

/// An independently allocated disk-cache shard with live range lookup.
struct Shard {
    allocator: DiskAllocator,
    entry_index: Mutex<DiskEntryIndex>,
}

/// Disk entries indexed by object key and range start.
struct DiskEntryIndex {
    ranges_by_key: HashMap<ObjectKey, BTreeMap<u64, ObjectRangeDiskStorage>>,
}

/// One reserved chunk being assembled in memory before its only write.
struct UnwrittenChunk {
    region: DiskRegion,
    bytes: Vec<u8>,
    used_bytes: u64,
    metadata_starts: EntryMetadataStarts,
}

/// Entries and their complete chunk buffers prepared for one shard's batch write.
#[derive(Default)]
struct UnwrittenBatch {
    chunks: Vec<UnwrittenChunk>,
    entries: Vec<(ObjectKey, ObjectRangeDiskStorage)>,
}

/// Candidate entry metadata start positions within one chunk, encoded as a bitmap.
#[derive(Default)]
struct EntryMetadataStarts {
    bitmap: [u8; 32],
}

impl EntryMetadataStarts {
    fn insert(&mut self, offset_in_chunk: u64) {
        let slot = (offset_in_chunk / BLOCK_BYTES) as usize;
        self.bitmap[slot / 8] |= 1 << (slot % 8);
    }
}

/// Disk storage reserved for one object range, with its expected payload checksum.
struct ObjectRangeDiskStorage {
    object_range: ByteRange,
    payload_checksum: blake3::Hash,
    payload_regions: Vec<DiskRegion>,
    // Keep metadata-only chunks reserved for as long as the entry exists.
    _entry_metadata_regions: Vec<DiskRegion>,
}

/// Whole chunks retained through batch I/O; errors or unexpected task drops quarantine them.
struct UnfinishedChunkWrites(Vec<DiskRegion>);

impl Drop for UnfinishedChunkWrites {
    fn drop(&mut self) {
        for region in self.0.drain(..) {
            region.quarantine();
        }
    }
}

/// A guarded object-range read, with the expected checksum and all payload regions.
struct GuardedObjectRangeRead {
    object_range: ByteRange,
    payload_checksum: blake3::Hash,
    payload_regions: Vec<DiskRegionReadGuard>,
}

/// Failure opening or populating the experimental disk range cache. Read uncertainty becomes a miss.
#[derive(Debug, thiserror::Error)]
pub enum DiskRangeCacheError {
    /// The prototype requires a positive whole number of 1-MiB chunks within Linux's file-offset limit.
    #[error("disk range cache capacity must be a positive multiple of 1 MiB and at most i64::MAX")]
    InvalidCapacity,
    /// Raw storage failed.
    #[error(transparent)]
    DataFile(#[from] DataFileError),
    /// The task that owns a population failed; unfinished writes remain quarantined.
    #[error("disk population task failed: {0}")]
    PopulationTaskFailed(#[source] tokio::task::JoinError),
}

impl fmt::Debug for DiskRangeCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskRangeCache")
            .field("capacity", &self.disk.file.capacity())
            .field("arenas", &self.disk.arenas.len())
            .finish_non_exhaustive()
    }
}

impl DiskRangeCache {
    /// Opens an exclusively locked, fixed-capacity file and starts with an empty cache.
    /// Existing entries are neither recovered nor cleared. This is not a durable reset protocol.
    pub async fn open(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
    ) -> Result<Self, DiskRangeCacheError> {
        if capacity == 0 || capacity > i64::MAX as u64 || !capacity.is_multiple_of(CHUNK_BYTES) {
            return Err(DiskRangeCacheError::InvalidCapacity);
        }
        let file = DataFile::open(directory, capacity, metrics).await?;
        tracing::warn!(target: "feuer::storage", "disk range cache prototype starts empty; recovery is not implemented");
        let arena_count = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64);
        let chunk_count = capacity / CHUNK_BYTES;
        let arenas = (0..arena_count)
            .map(|arena_index| Shard {
                allocator: DiskAllocator::for_disk_range(
                    (chunk_count * arena_index / arena_count) * CHUNK_BYTES
                        ..(chunk_count * (arena_index + 1) / arena_count) * CHUNK_BYTES,
                )
                .unwrap(),
                entry_index: Mutex::new(DiskEntryIndex {
                    ranges_by_key: HashMap::new(),
                }),
            })
            .collect();
        Ok(Self {
            disk: Arc::new(DiskRangeCacheInner { file, arenas }),
        })
    }

    /// Packs an explicit batch into immutable chunks, grouping smaller payloads first within each shard.
    /// Returns the number of entries published; contained entries and entries that do not fit are skipped.
    /// Each shard's chunks finish writing before its entries publish, with containment revalidation.
    /// Publication is not transactional across shards. Does not record accesses.
    ///
    /// Partially filled final chunks are written too; later batches cannot fill their unused space.
    /// Callers bound batch size and concurrency: complete chunk buffers are assembled in memory.
    /// Dropping this future does not abort its detached writer or release storage needed by submitted I/O.
    pub async fn insert_batch(&self, downloads: Vec<(ObjectKey, Download)>) -> Result<usize, DiskRangeCacheError> {
        let disk = self.disk.clone();
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DataFileError::RuntimeUnavailable)?;
        runtime
            .spawn(async move {
                let mut by_arena: Vec<Vec<_>> = (0..disk.arenas.len()).map(|_| Vec::new()).collect();
                for (key, download) in downloads {
                    by_arena[disk.arena_index_for_key(&key)].push((key, download));
                }
                let mut published = 0;
                for (arena, mut downloads) in disk.arenas.iter().zip(by_arena) {
                    downloads.sort_by_key(|(_, download)| download.bytes().len());
                    let mut batch = UnwrittenBatch::default();
                    for (key, download) in downloads {
                        if arena
                            .entry_index
                            .lock()
                            .unwrap()
                            .covering_range(&key, download.downloaded_range())
                            .is_none()
                        {
                            batch.push(&arena.allocator, key, download);
                        }
                    }
                    let entries = batch.write(&disk.file).await?;
                    let mut index = arena.entry_index.lock().unwrap();
                    // Publish larger entries first so contained batch members need not publish at all.
                    for (key, storage) in entries.into_iter().rev() {
                        let object_range = storage.object_range;
                        // Another writer or an earlier entry in this batch may already cover this range.
                        if index.covering_range(&key, object_range).is_some() {
                            continue;
                        }
                        let entries = index.ranges_by_key.entry(key).or_default();
                        let replaced_starts: Vec<_> = entries
                            .range(object_range.start()..object_range.end())
                            .filter_map(|(&start, entry)| object_range.contains(entry.object_range).then_some(start))
                            .collect();
                        for start in replaced_starts {
                            entries.remove(&start);
                        }
                        entries.insert(object_range.start(), storage);
                        published += 1;
                    }
                }
                Ok(published)
            })
            .await
            .map_err(DiskRangeCacheError::PopulationTaskFailed)?
    }

    /// Returns exactly requested bytes from one covering entry, or a miss on any I/O/integrity uncertainty.
    /// Reads and hashes the whole entry, not neighboring entries or metadata. Results retain no disk ownership.
    pub async fn get(&self, key: &ObjectKey, requested: ByteRange) -> Option<Bytes> {
        let arena = &self.disk.arenas[self.disk.arena_index_for_key(key)];
        let guarded_read = {
            let index = arena.entry_index.lock().unwrap();
            let storage = index.covering_range(key, requested)?;
            GuardedObjectRangeRead {
                object_range: storage.object_range,
                payload_checksum: storage.payload_checksum,
                payload_regions: storage.payload_regions.iter().map(DiskRegion::read_guard).collect(),
            }
        };
        match guarded_read.read(&self.disk.file, requested).await {
            Ok(Some(bytes)) => Some(bytes),
            result => {
                match result {
                    Err(error) => {
                        tracing::warn!(target: "feuer::storage", %error, "disk read failed; entry invalidated")
                    }
                    _ => tracing::warn!(target: "feuer::storage", "disk integrity check failed; entry invalidated"),
                }
                arena.entry_index.lock().unwrap().invalidate(key, &guarded_read);
                None
            }
        }
    }
}

impl DiskEntryIndex {
    fn invalidate(&mut self, key: &str, read: &GuardedObjectRangeRead) {
        if let Some(entries) = self.ranges_by_key.get_mut(key) {
            let start = read.object_range.start();
            // Preserve different contents. Discarding a newer identical copy is an acceptable miss.
            if entries
                .get(&start)
                .is_some_and(|storage| storage.payload_checksum == read.payload_checksum)
            {
                entries.remove(&start);
            }
            if entries.is_empty() {
                self.ranges_by_key.remove(key);
            }
        }
    }

    fn covering_range(&self, key: &str, requested: ByteRange) -> Option<&ObjectRangeDiskStorage> {
        // Retained ranges never contain one another, so their ends increase with their starts.
        let (_, storage) = self.ranges_by_key.get(key)?.range(..=requested.start()).next_back()?;
        storage.object_range.contains(requested).then_some(storage)
    }
}

impl DiskRangeCacheInner {
    fn arena_index_for_key(&self, key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % self.arenas.len() as u64) as usize
    }
}

fn page_storage_bytes(content_bytes: usize) -> Option<u64> {
    (content_bytes as u64)
        .div_ceil(PAGE_CONTENT_BYTES as u64)
        .checked_mul(BLOCK_BYTES)
}

impl UnwrittenBatch {
    fn push(&mut self, allocator: &DiskAllocator, key: ObjectKey, download: Download) {
        let (object_range, bytes) = download.into_parts();
        let payload_bytes = (bytes.len() as u64).next_multiple_of(BLOCK_BYTES);
        let chunk_data_bytes = CHUNK_BYTES - BLOCK_BYTES;
        let tail_bytes = self.chunks.last().map_or(0, |chunk| CHUNK_BYTES - chunk.used_bytes);
        let payload_region_count =
            u64::from(tail_bytes > 0) + payload_bytes.saturating_sub(tail_bytes).div_ceil(chunk_data_bytes);
        let Some(metadata_bytes) = page_storage_bytes(72 + 16 * payload_region_count as usize + key.len()) else {
            return;
        };
        let new_chunk_count = (payload_bytes + metadata_bytes)
            .saturating_sub(tail_bytes)
            .div_ceil(chunk_data_bytes);
        let mut cursor = self.chunks.len().saturating_sub(1);
        if new_chunk_count > 0 {
            let Some(regions) = allocator.reserve_chunks(new_chunk_count) else {
                return;
            };
            self.chunks.extend(regions.into_iter().map(|region| UnwrittenChunk {
                region,
                bytes: vec![0; CHUNK_BYTES as usize],
                used_bytes: BLOCK_BYTES,
                metadata_starts: EntryMetadataStarts::default(),
            }));
        }
        let (payload_regions, first_chunk) = self.take_regions(&mut cursor, payload_bytes);
        let mut consumed = 0;
        for (chunk, region) in self.chunks[first_chunk..].iter_mut().zip(&payload_regions) {
            let range = region.range();
            let offset = (range.start % CHUNK_BYTES) as usize;
            let end = (consumed + (range.end - range.start) as usize).min(bytes.len());
            chunk.bytes[offset..offset + end - consumed].copy_from_slice(&bytes[consumed..end]);
            consumed = end;
        }
        let payload_checksum = blake3::hash(&bytes);
        let contents = page_format::encode_entry_metadata(&key, object_range, &payload_regions, &payload_checksum);
        let (entry_metadata_regions, first_chunk) = self.take_regions(&mut cursor, metadata_bytes);
        self.chunks[first_chunk]
            .metadata_starts
            .insert(entry_metadata_regions[0].range().start % CHUNK_BYTES);
        let content_checksum = blake3::hash(&contents);
        let mut page_ordinal = 0;
        let mut consumed = 0;
        for (region_index, (chunk, region)) in self.chunks[first_chunk..]
            .iter_mut()
            .zip(&entry_metadata_regions)
            .enumerate()
        {
            let range = region.range();
            for page_address in (range.start..range.end).step_by(PAGE_BYTES) {
                let next = if page_address + BLOCK_BYTES < range.end {
                    page_address + BLOCK_BYTES
                } else {
                    entry_metadata_regions
                        .get(region_index + 1)
                        .map_or(0, |region| region.range().start)
                };
                let offset = (page_address % CHUNK_BYTES) as usize;
                let end = (consumed + PAGE_CONTENT_BYTES).min(contents.len());
                page_format::encode_page(
                    &mut chunk.bytes[offset..offset + PAGE_BYTES],
                    page_format::ENTRY_METADATA_PAGE_TAG,
                    content_checksum.as_bytes(),
                    page_address,
                    page_ordinal,
                    next,
                    &contents[consumed..end],
                );
                consumed = end;
                page_ordinal += 1;
            }
        }
        self.entries.push((
            key,
            ObjectRangeDiskStorage {
                object_range,
                payload_checksum,
                payload_regions,
                _entry_metadata_regions: entry_metadata_regions,
            },
        ));
    }

    /// Carves aligned entry regions from reserved chunks, preserving shared whole-chunk ownership.
    fn take_regions(&mut self, cursor: &mut usize, mut bytes: u64) -> (Vec<DiskRegion>, usize) {
        while self.chunks[*cursor].used_bytes == CHUNK_BYTES {
            *cursor += 1;
        }
        let first_chunk = *cursor;
        let mut regions = Vec::new();
        while bytes > 0 {
            let chunk = &mut self.chunks[*cursor];
            let length = bytes.min(CHUNK_BYTES - chunk.used_bytes);
            let start = chunk.region.range().start + chunk.used_bytes;
            regions.push(chunk.region.slice(start..start + length));
            chunk.used_bytes += length;
            bytes -= length;
            if bytes > 0 {
                *cursor += 1;
            }
        }
        (regions, first_chunk)
    }

    async fn write(mut self, file: &DataFile) -> Result<Vec<(ObjectKey, ObjectRangeDiskStorage)>, DiskRangeCacheError> {
        let mut unfinished = UnfinishedChunkWrites(
            self.chunks
                .iter()
                .map(|chunk| chunk.region.slice(chunk.region.range()))
                .collect(),
        );
        for chunk in &mut self.chunks {
            let address = chunk.region.range().start;
            page_format::encode_page(
                &mut chunk.bytes[..PAGE_BYTES],
                page_format::ENTRY_METADATA_INDEX_PAGE_TAG,
                blake3::hash(&chunk.metadata_starts.bitmap).as_bytes(),
                address,
                address / CHUNK_BYTES,
                0,
                &chunk.metadata_starts.bitmap,
            );
            if let Err(error) = file
                .write_at(address, &Bytes::from(std::mem::take(&mut chunk.bytes)))
                .await
            {
                tracing::warn!(target: "feuer::storage", %error, "batch write failed; chunks quarantined");
                return Err(error.into());
            }
        }
        unfinished.0.clear();
        Ok(self.entries)
    }
}

impl GuardedObjectRangeRead {
    async fn read(&self, file: &DataFile, requested: ByteRange) -> DataFileResult<Option<Bytes>> {
        let start = requested.start() - self.object_range.start();
        let end = requested.end() - self.object_range.start();
        let length = requested.len() as usize;
        let mut output = Vec::new();
        output
            .try_reserve_exact(length)
            .map_err(|source| DataFileError::Allocation { length, source })?;
        let mut checksum = blake3::Hasher::new();
        let mut consumed = 0;
        for region in &self.payload_regions {
            let disk_range = region.range();
            let length = (disk_range.end - disk_range.start).min(self.object_range.len() - consumed);
            let bytes = file.read_at(disk_range.start, length as usize).await?;
            checksum.update(&bytes);
            let copy_start = start.saturating_sub(consumed).min(length) as usize;
            let copy_end = end.saturating_sub(consumed).min(length) as usize;
            output.extend_from_slice(&bytes[copy_start..copy_end]);
            consumed += length;
        }
        if consumed != self.object_range.len()
            || output.len() != requested.len() as usize
            || checksum.finalize() != self.payload_checksum
        {
            return Ok(None);
        }
        Ok(Some(Bytes::from(output)))
    }
}
