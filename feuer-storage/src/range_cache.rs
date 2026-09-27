//! Experimental allocation → payload/entry metadata write → publication → checked subrange read path.

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
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    DataFile, DataFileError, DataFileResult, IoMetrics,
    allocation::{BLOCK_BYTES, CHUNK_BYTES, DiskAllocator, DiskRegion, DiskRegionReadGuard},
};
use page_format::{PAGE_BYTES, PAGE_CONTENT_BYTES};

/// A standalone, experimental disk range cache, not yet connected to the public tiered cache.
///
/// Insertions write payload, entry metadata and an embedded entry metadata index before publication.
/// Entries have plain payload bytes, 4-KiB-aligned allocations, and one checksum stored in their entry metadata.
/// Reads verify the entire covering entry, returning only the requested bytes. Entries share 1-MiB chunks.
/// Each object belongs to one independently locked arena; an insertion that does not fit is skipped.
///
/// **Recovery and pressure eviction are not implemented.** Every open starts empty and logs
/// that reset. The experimental format is not a persistence guarantee or a stable on-disk interface.
/// Payload I/O uses the real Linux direct-I/O backend; no buffered fallback exists.
#[derive(Clone)]
pub struct DiskRangeCache {
    disk: Arc<DiskRangeCacheInner>,
}

/// Shared inner state of the disk range cache: its file and independently allocated arenas.
struct DiskRangeCacheInner {
    file: DataFile,
    arenas: Box<[Shard]>,
}

/// An independently allocated disk-cache shard with range lookup and entry metadata index writes.
struct Shard {
    allocator: DiskAllocator,
    entry_index: Mutex<DiskEntryIndex>,
    // Index pages are shared by entries. Serialize their writes, not payload writes or reads.
    entry_metadata_index: AsyncMutex<EntryMetadataIndex>,
}

/// Disk entries indexed by object key and range start.
struct DiskEntryIndex {
    ranges_by_key: HashMap<ObjectKey, BTreeMap<u64, ObjectRangeDiskStorage>>,
}

/// An index of where entry metadata starts on disk, with cached bitmaps and write availability.
struct EntryMetadataIndex {
    starts_by_chunk_address: BTreeMap<u64, EntryMetadataStarts>,
    // Cleared before issuing an index write, restored only on success. Failure/abandonment stops
    // further index writes, including after an unknown completion, without reusing that page.
    writes_enabled: bool,
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
    entry_metadata_regions: Vec<DiskRegion>,
}

/// Reserved storage for unfinished writes. A write error or unexpected task drop quarantines it;
/// successful completion transfers ordinary ownership.
struct UnfinishedWriteStorage(Option<ObjectRangeDiskStorage>);

impl Drop for UnfinishedWriteStorage {
    fn drop(&mut self) {
        if let Some(storage) = self.0.take() {
            for region in storage
                .payload_regions
                .into_iter()
                .chain(storage.entry_metadata_regions)
            {
                region.quarantine();
            }
        }
    }
}

/// A guarded object-range read, with the expected checksum and all of the entry's payload regions.
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
    /// A previous failed or abandoned index write prevents safe further index updates in this arena.
    #[error("entry metadata index writes are disabled in this arena")]
    EntryMetadataIndexWritesDisabled,
    /// The task that owns a population failed; its unfinished reservations remain quarantined.
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
    ///
    /// Existing entries are deliberately not recovered yet. Reopening resets the first embedded index;
    /// this is not a persistent reset of every entry metadata index in the file.
    pub async fn open(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
    ) -> Result<Self, DiskRangeCacheError> {
        if capacity == 0 || capacity > i64::MAX as u64 || !capacity.is_multiple_of(CHUNK_BYTES) {
            return Err(DiskRangeCacheError::InvalidCapacity);
        }
        let file = DataFile::open(directory, capacity, metrics).await?;
        let mut first_index_page = vec![0; PAGE_BYTES];
        page_format::encode_page(
            &mut first_index_page,
            page_format::ENTRY_METADATA_INDEX_PAGE_TAG,
            blake3::hash(&[0; 32]).as_bytes(),
            0,
            0,
            0,
            &[0; 32],
        );
        file.write_at(0, &Bytes::from(first_index_page)).await?;
        tracing::warn!(target: "feuer::storage", "disk range cache prototype starts empty; recovery is not implemented");

        // Keep small test/deployment capacities useful for ~100-MiB entries. This is an experimental
        // arena split, not a public tuning knob or a guarantee that every entry fits every arena.
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
                entry_metadata_index: AsyncMutex::new(EntryMetadataIndex {
                    starts_by_chunk_address: BTreeMap::new(),
                    writes_enabled: true,
                }),
            })
            .collect();
        Ok(Self {
            disk: Arc::new(DiskRangeCacheInner { file, arenas }),
        })
    }

    /// Writes one download and publishes it only after successful I/O and containment revalidation.
    /// Returns false when contained by existing data or when its arena has insufficient free space.
    /// Does not record an access. Partially overlapping downloads may coexist.
    ///
    /// Dropping this future does not abort a started writer. Its task owns storage through completion;
    /// a successfully completed population may still publish after its requester has gone away.
    pub async fn insert(&self, key: ObjectKey, download: Download) -> Result<bool, DiskRangeCacheError> {
        let arena_index = self.disk.arena_index_for_key(&key);
        let arena = &self.disk.arenas[arena_index];
        let (object_range, bytes) = download.into_parts();
        if arena
            .entry_index
            .lock()
            .unwrap()
            .covering_range(&key, object_range)
            .is_some()
        {
            return Ok(false);
        }
        let Some(payload_regions) = arena.allocator.reserve(bytes.len() as u64) else {
            return Ok(false);
        };
        let payload_checksum = blake3::hash(&bytes);
        let entry_metadata_bytes =
            page_format::encode_entry_metadata(&key, object_range, &payload_regions, &payload_checksum);
        let Some(entry_metadata_storage_bytes) = page_storage_bytes(entry_metadata_bytes.len()) else {
            return Ok(false);
        };
        let Some(entry_metadata_regions) = arena.allocator.reserve(entry_metadata_storage_bytes) else {
            return Ok(false);
        };
        let entry_metadata_start_address = entry_metadata_regions[0].range().start;
        let storage = ObjectRangeDiskStorage {
            object_range,
            payload_checksum,
            payload_regions,
            entry_metadata_regions,
        };
        let disk = self.disk.clone();
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DataFileError::RuntimeUnavailable)?;
        // The caller never receives the task handle, so cancellation cannot abort submitted writes.
        runtime
            .spawn(async move {
                let mut unfinished = UnfinishedWriteStorage(Some(storage));
                let storage = unfinished.0.as_ref().unwrap();
                let write_result = async {
                    write_payload(&disk.file, &storage.payload_regions, &bytes).await?;
                    write_entry_metadata_pages(&disk.file, &storage.entry_metadata_regions, &entry_metadata_bytes).await?;
                    disk.write_entry_metadata_index_page(arena_index, entry_metadata_start_address).await
                }
                .await;
                if let Err(error) = write_result {
                    tracing::warn!(target: "feuer::storage", %error, "disk population failed; reservations quarantined");
                    return Err(error);
                }
                let storage = unfinished.0.take().unwrap();
                let mut index = disk.arenas[arena_index].entry_index.lock().unwrap();
                // Another writer may have published an equal or broader interval while we wrote.
                if index.covering_range(&key, object_range).is_some() {
                    return Ok(false);
                }
                let entries = index.ranges_by_key.entry(key).or_default();
                let replaced_starts: Vec<_> = entries
                    .range(object_range.start()..object_range.end())
                    .filter_map(|(&start, storage)| object_range.contains(storage.object_range).then_some(start))
                    .collect();
                for start in replaced_starts {
                    entries.remove(&start);
                }
                entries.insert(object_range.start(), storage);
                Ok(true)
            })
            .await
            .map_err(DiskRangeCacheError::PopulationTaskFailed)?
    }

    /// Returns exactly the requested bytes from one covering entry, or a miss on any I/O/integrity
    /// uncertainty. Reads and hashes the whole covering entry, but copies only the requested bytes.
    /// Does not read neighboring entries or the entry metadata chain.
    /// Returned bytes retain neither disk allocations nor read guards. Does not record an access.
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

    async fn write_entry_metadata_index_page(
        &self,
        arena_index: usize,
        entry_metadata_start_address: u64,
    ) -> Result<(), DiskRangeCacheError> {
        let mut index_pages = self.arenas[arena_index].entry_metadata_index.lock().await;
        if !index_pages.writes_enabled {
            return Err(DiskRangeCacheError::EntryMetadataIndexWritesDisabled);
        }
        let chunk_address = entry_metadata_start_address / CHUNK_BYTES * CHUNK_BYTES;
        let entry_metadata_starts = index_pages.starts_by_chunk_address.entry(chunk_address).or_default();
        entry_metadata_starts.insert(entry_metadata_start_address - chunk_address);
        let mut page = vec![0; PAGE_BYTES];
        page_format::encode_page(
            &mut page,
            page_format::ENTRY_METADATA_INDEX_PAGE_TAG,
            blake3::hash(&entry_metadata_starts.bitmap).as_bytes(),
            chunk_address,
            chunk_address / CHUNK_BYTES,
            0,
            &entry_metadata_starts.bitmap,
        );
        index_pages.writes_enabled = false;
        self.file.write_at(chunk_address, &Bytes::from(page)).await?;
        index_pages.writes_enabled = true;
        Ok(())
    }
}

fn page_storage_bytes(content_bytes: usize) -> Option<u64> {
    (content_bytes as u64)
        .div_ceil(PAGE_CONTENT_BYTES as u64)
        .checked_mul(BLOCK_BYTES)
}

async fn write_payload(file: &DataFile, regions: &[DiskRegion], bytes: &Bytes) -> DataFileResult<()> {
    let mut consumed = 0;
    for region in regions {
        let disk_range = region.range();
        let length = (disk_range.end - disk_range.start) as usize;
        let end = (consumed + length).min(bytes.len());
        let contents = if end - consumed == length {
            bytes.slice(consumed..end)
        } else {
            // Only the final region needs padding; complete regions use the source bytes directly.
            let mut padded = vec![0; length];
            padded[..end - consumed].copy_from_slice(&bytes[consumed..end]);
            Bytes::from(padded)
        };
        file.write_at(disk_range.start, &contents).await?;
        consumed = end;
    }
    debug_assert_eq!(consumed, bytes.len());
    Ok(())
}

async fn write_entry_metadata_pages(file: &DataFile, regions: &[DiskRegion], contents: &[u8]) -> DataFileResult<()> {
    let content_checksum = blake3::hash(contents);
    let mut page_ordinal = 0;
    let mut consumed = 0;
    for (region_index, region) in regions.iter().enumerate() {
        let disk_range = region.range();
        // Encode at most one chunk of metadata at a time.
        let mut buffer = vec![0; (disk_range.end - disk_range.start) as usize];
        for (page_index, page) in buffer.as_chunks_mut::<PAGE_BYTES>().0.iter_mut().enumerate() {
            let page_address = disk_range.start + page_index as u64 * BLOCK_BYTES;
            let next_entry_metadata_page_address = if page_address + BLOCK_BYTES < disk_range.end {
                page_address + BLOCK_BYTES
            } else {
                regions.get(region_index + 1).map_or(0, |region| region.range().start)
            };
            let end = (consumed + PAGE_CONTENT_BYTES).min(contents.len());
            page_format::encode_page(
                page,
                page_format::ENTRY_METADATA_PAGE_TAG,
                content_checksum.as_bytes(),
                page_address,
                page_ordinal,
                next_entry_metadata_page_address,
                &contents[consumed..end],
            );
            consumed = end;
            page_ordinal += 1;
        }
        file.write_at(disk_range.start, &Bytes::from(buffer)).await?;
    }
    debug_assert_eq!(consumed, contents.len());
    Ok(())
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
