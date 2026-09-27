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
use feuer_types::{
    ByteRange, Download, ObjectKey,
    retention::{ObjectAccessHistories, ObjectAccessHistory, compare_cost_per_byte, sample_candidates},
};

use crate::{
    DataFile, DataFileError, DataFileResult, IoMetrics,
    allocation::{CHUNK_BYTES, DiskAllocator, DiskRegion, DiskRegionReadGuard},
};
use page_format::{METADATA_PAGE_BYTES, PAGE_CONTENT_BYTES};

const PAYLOAD_ALIGNMENT_BYTES: u64 = crate::uring::DIRECT_IO_ALIGNMENT_BYTES as u64;

// Per shard batch: bound sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_REGIONS: usize = 4096;

/// A standalone experimental disk cache, not yet connected to the public tiered cache.
///
/// Explicit batches pack smaller entries together and write complete immutable 1-MiB chunks.
/// Entries have plain payload bytes, 4-KiB-aligned storage, and a checksum in their entry metadata.
/// Reads verify the whole covering entry, returning only requested bytes. Reuse requires a wholly free chunk.
/// Full-key hashing selects an independently allocated shard; admission may fail despite space elsewhere.
///
/// Pressure eviction samples individual entries by recent retrieval value per payload byte.
/// **Recovery is not implemented.** Every open starts empty and logs that reset.
/// The experimental format is neither a persistence guarantee nor a stable on-disk interface.
#[derive(Clone)]
pub struct DiskRangeCache {
    disk: Arc<DiskRangeCacheInner>,
}

/// Shared inner state of the disk range cache: its file and independently allocated arenas.
struct DiskRangeCacheInner {
    file: DataFile,
    arenas: Box<[Shard]>,
    access_histories: Arc<ObjectAccessHistories>,
}

/// An independently allocated disk-cache shard with live range lookup.
struct Shard {
    allocator: DiskAllocator,
    entry_index: Mutex<DiskEntryIndex>,
}

/// Disk entries indexed by object key and range start, with dense rotating eviction candidates.
#[derive(Default)]
struct DiskEntryIndex {
    ranges_by_key: HashMap<ObjectKey, BTreeMap<u64, ObjectRangeDiskStorage>>,
    eviction_candidates: Vec<(ObjectKey, u64)>,
    next_candidate: usize,
    next_publication_id: u64,
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
        let slot = (offset_in_chunk / METADATA_PAGE_BYTES as u64) as usize;
        self.bitmap[slot / 8] |= 1 << (slot % 8);
    }
}

/// Disk storage reserved for one object range, with its expected payload checksum.
struct ObjectRangeDiskStorage {
    eviction_position: usize,
    publication_id: u64,
    accesses: Arc<ObjectAccessHistory>,
    object_range: ByteRange,
    payload_checksum: blake3::Hash,
    payload_regions: Vec<DiskRegion>,
    // Keep metadata-only chunks reserved for as long as the entry exists.
    entry_metadata_regions: Vec<DiskRegion>,
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
        let shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        Self::open_with_access_histories(
            directory,
            capacity,
            metrics,
            Arc::new(ObjectAccessHistories::new(shards)),
        )
        .await
    }

    /// Opens a disk tier using the memory tier's shared access evidence.
    /// Population and `get` do not record accesses; the successful public lookup records exactly once.
    pub async fn open_with_access_histories(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
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
                entry_index: Mutex::new(DiskEntryIndex::default()),
            })
            .collect();
        Ok(Self {
            disk: Arc::new(DiskRangeCacheInner {
                file,
                arenas,
                access_histories,
            }),
        })
    }

    /// Shared evidence for recording successful lookups, once, outside raw storage operations.
    pub fn access_histories(&self) -> Arc<ObjectAccessHistories> {
        self.disk.access_histories.clone()
    }

    /// Packs an explicit batch into immutable chunks, grouping smaller payloads first within each shard.
    /// Returns the number of entries published; contained entries and entries that do not fit are skipped.
    /// Each shard's chunks finish writing before its entries publish, with containment revalidation.
    /// Publication is not transactional across shards. Does not record accesses.
    ///
    /// Entries share a chunk only when their complete payload and metadata fit inside it.
    /// Multi-chunk entries own their chunks exclusively. Partially filled chunks are finalized too.
    /// Pressure eviction is bounded; unavailable capacity causes admission to be skipped, not waited for.
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
                    let mut attempts_left = MAX_EVICTION_ATTEMPTS;
                    let mut regions_left = MAX_EVICTION_REGIONS;
                    for (key, download) in downloads {
                        if arena
                            .entry_index
                            .lock()
                            .unwrap()
                            .covering_range(&key, download.downloaded_range())
                            .is_none()
                        {
                            // Preserve the object's evidence even if pressure removes its last existing entry.
                            let accesses = disk.access_histories.for_key(&key);
                            while let Err(chunks_needed) = batch.push(&arena.allocator, &key, &download, &accesses) {
                                // Evicting cannot help an entry that exceeds the capacity left by this batch.
                                if chunks_needed > arena.allocator.chunk_capacity - batch.chunks.len() as u64
                                    || !arena.evict_candidate(&mut attempts_left, &mut regions_left)
                                {
                                    break;
                                }
                            }
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
                        index.insert(key, storage);
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

impl Shard {
    /// Evicts one sampled low-value entry, never its neighbors. Chunk release is left to ownership.
    fn evict_candidate(&self, attempts_left: &mut usize, regions_left: &mut usize) -> bool {
        if *attempts_left == 0 || *regions_left == 0 {
            return false;
        }
        let mut index = self.entry_index.lock().unwrap();
        let length = index.eviction_candidates.len();
        if length == 0 {
            return false;
        }
        *attempts_left -= 1;
        let (start, count) = sample_candidates(&mut index.next_candidate, length);
        let mut selected: Option<(usize, u64, u64, u64)> = None;
        for offset in 0..count {
            let position = (start + offset) % length;
            let (key, range_start) = &index.eviction_candidates[position];
            let entry = &index.ranges_by_key[key][range_start];
            if entry.region_count() > *regions_left {
                continue;
            }
            let value = entry.accesses.covered_retrieval_cost(entry.object_range);
            let payload_bytes = entry.object_range.len();
            if selected.is_none_or(|(_, current_value, current_bytes, current_id)| {
                compare_cost_per_byte(value, payload_bytes, current_value, current_bytes)
                    .then_with(|| entry.publication_id.cmp(&current_id))
                    .is_lt()
            }) {
                selected = Some((position, value, payload_bytes, entry.publication_id));
            }
        }
        if let Some((position, ..)) = selected {
            let (key, start) = index.eviction_candidates[position].clone();
            *regions_left -= index.ranges_by_key[&key][&start].region_count();
            index.remove(&key, start);
        }
        true
    }
}

impl ObjectRangeDiskStorage {
    fn region_count(&self) -> usize {
        self.payload_regions.len() + self.entry_metadata_regions.len()
    }

    #[cfg(test)]
    fn shared_chunk(&self) -> Option<u64> {
        if self.payload_regions.len() != 1 || self.entry_metadata_regions.len() != 1 {
            return None;
        }
        let chunk = self.payload_regions[0].range().start / CHUNK_BYTES;
        (self.entry_metadata_regions[0].range().start / CHUNK_BYTES == chunk).then_some(chunk * CHUNK_BYTES)
    }
}

impl DiskEntryIndex {
    fn insert(&mut self, key: ObjectKey, mut storage: ObjectRangeDiskStorage) {
        let object_range = storage.object_range;
        if let Some(entries) = self.ranges_by_key.get(&key) {
            let replaced: Vec<_> = entries
                .range(object_range.start()..object_range.end())
                .filter_map(|(&start, entry)| object_range.contains(entry.object_range).then_some(start))
                .collect();
            for start in replaced {
                self.remove(&key, start);
            }
        }
        self.next_publication_id = self
            .next_publication_id
            .checked_add(1)
            .expect("disk entry identities exhausted");
        storage.publication_id = self.next_publication_id;
        storage.eviction_position = self.eviction_candidates.len();
        self.eviction_candidates.push((key.clone(), object_range.start()));
        self.ranges_by_key
            .entry(key)
            .or_default()
            .insert(object_range.start(), storage);
    }

    fn remove(&mut self, key: &str, start: u64) -> Option<ObjectRangeDiskStorage> {
        let entries = self.ranges_by_key.get_mut(key)?;
        let storage = entries.remove(&start)?;
        if entries.is_empty() {
            self.ranges_by_key.remove(key);
        }
        self.eviction_candidates.swap_remove(storage.eviction_position);
        if let Some((moved_key, moved_start)) = self.eviction_candidates.get(storage.eviction_position) {
            self.ranges_by_key
                .get_mut(moved_key)
                .unwrap()
                .get_mut(moved_start)
                .unwrap()
                .eviction_position = storage.eviction_position;
        }
        Some(storage)
    }

    fn invalidate(&mut self, key: &str, read: &GuardedObjectRangeRead) {
        let start = read.object_range.start();
        // Preserve different contents. Discarding a newer identical copy is an acceptable miss.
        if self
            .ranges_by_key
            .get(key)
            .and_then(|entries| entries.get(&start))
            .is_some_and(|storage| storage.payload_checksum == read.payload_checksum)
        {
            self.remove(key, start);
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
        .checked_mul(METADATA_PAGE_BYTES as u64)
}

impl UnwrittenBatch {
    /// Allocation failure leaves the batch unchanged and reports the required number of new chunks.
    fn push(
        &mut self,
        allocator: &DiskAllocator,
        key: &ObjectKey,
        download: &Download,
        accesses: &Arc<ObjectAccessHistory>,
    ) -> Result<(), u64> {
        let object_range = download.downloaded_range();
        let bytes = download.bytes();
        let payload_bytes = (bytes.len() as u64).next_multiple_of(PAYLOAD_ALIGNMENT_BYTES);
        let chunk_data_bytes = CHUNK_BYTES - METADATA_PAGE_BYTES as u64;
        let payload_region_count = payload_bytes.div_ceil(chunk_data_bytes);
        let metadata_bytes = page_storage_bytes(72 + 16 * payload_region_count as usize + key.len()).ok_or(u64::MAX)?;
        let entry_bytes = payload_bytes + metadata_bytes;
        let tail_bytes = self.chunks.last().map_or(0, |chunk| CHUNK_BYTES - chunk.used_bytes);
        // Never split an entry across a shared chunk boundary, even if only metadata would spill.
        let shares_tail = entry_bytes <= tail_bytes;
        let new_chunk_count = if shares_tail {
            0
        } else {
            entry_bytes.div_ceil(chunk_data_bytes)
        };
        let mut cursor = if shares_tail {
            self.chunks.len() - 1
        } else {
            self.chunks.len()
        };
        if new_chunk_count > 0 {
            let regions = allocator.reserve_chunks(new_chunk_count).ok_or(new_chunk_count)?;
            self.chunks.extend(regions.into_iter().map(|region| UnwrittenChunk {
                region,
                bytes: vec![0; CHUNK_BYTES as usize],
                used_bytes: METADATA_PAGE_BYTES as u64,
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
        let payload_checksum = blake3::hash(bytes);
        let contents = page_format::encode_entry_metadata(key, object_range, &payload_regions, &payload_checksum);
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
            for page_address in (range.start..range.end).step_by(METADATA_PAGE_BYTES) {
                let next = if page_address + (METADATA_PAGE_BYTES as u64) < range.end {
                    page_address + METADATA_PAGE_BYTES as u64
                } else {
                    entry_metadata_regions
                        .get(region_index + 1)
                        .map_or(0, |region| region.range().start)
                };
                let offset = (page_address % CHUNK_BYTES) as usize;
                let end = (consumed + PAGE_CONTENT_BYTES).min(contents.len());
                page_format::encode_page(
                    &mut chunk.bytes[offset..offset + METADATA_PAGE_BYTES],
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
        // A multi-chunk entry's tail must never be offered to another entry.
        if new_chunk_count > 1 {
            self.chunks.last_mut().unwrap().used_bytes = CHUNK_BYTES;
        }
        self.entries.push((
            key.clone(),
            ObjectRangeDiskStorage {
                eviction_position: 0,
                publication_id: 0,
                accesses: accesses.clone(),
                object_range,
                payload_checksum,
                payload_regions,
                entry_metadata_regions,
            },
        ));
        Ok(())
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
                &mut chunk.bytes[..METADATA_PAGE_BYTES],
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
