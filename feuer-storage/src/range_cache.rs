//! Experimental immutable batch writes, live range lookup and whole-entry validation.

#[cfg(test)]
mod metrics_tests;
mod page_format;
mod recovery;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap, hash_map::DefaultHasher},
    fmt,
    hash::{Hash, Hasher},
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use bytes::Bytes;
use feuer_types::{
    ByteRange, Download, ObjectKey,
    retention::{ObjectAccessHistories, RECLAIM_SAMPLE_SIZE, compare_cost_per_byte, sample_candidates},
};
use tokio::sync::OnceCell;
use twox_hash::XxHash64;

use crate::{
    DataFile, DataFileError, DataFileResult, DiskMetrics, IoMetrics,
    allocation::{CHUNK_BYTES, DiskChunkAllocator, DiskRegion, DiskRegionReadGuard},
    disk_metrics::{DiskLookupOutcome, DiskWriteAttempt, DiskWriteOutcome},
};
use page_format::{METADATA_PAGE_BYTES, PAGE_CONTENT_BYTES};

const PAYLOAD_ALIGNMENT_BYTES: u64 = crate::uring::DIRECT_IO_ALIGNMENT_BYTES as u64;

// Per shard batch: bound sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_REGIONS: usize = 4096;

/// An experimental disk range cache used by the public tiered cache.
///
/// Explicit batches pack smaller entries together and write complete immutable 1-MiB chunks.
/// Entries have plain payload bytes, 4-KiB-aligned storage, and a checksum in their entry metadata.
/// Reads verify the whole covering entry, returning only requested bytes. Reuse requires a wholly free chunk.
/// Full-key hashing selects an independently allocated shard; admission may fail despite space elsewhere.
///
/// Pressure eviction uses sampled retrieval value per payload byte.
/// Recovery publishes old entries incrementally alongside foreground reads and writes.
/// The experimental format is neither a persistence guarantee nor a stable on-disk interface.
#[derive(Clone)]
pub struct DiskRangeCache {
    disk: Arc<DiskRangeCacheState>,
}

/// Shared state of the disk range cache: its file and independently allocated shards.
struct DiskRangeCacheState {
    file: DataFile,
    shards: Box<[DiskCacheShard]>,
    access_histories: Arc<ObjectAccessHistories>,
    metrics: Arc<DiskMetrics>,
    recovery: recovery::RecoveryState,
}

/// An independently allocated disk-cache shard with live range lookup.
struct DiskCacheShard {
    reclaim_sample_size: usize,
    allocator: DiskChunkAllocator,
    entry_index: Mutex<DiskEntryIndex>,
    written_end: AtomicU64,
}

/// Disk entries indexed by object key and range start, with dense rotating eviction candidates.
struct DiskEntryIndex {
    ranges_by_key: HashMap<ObjectKey, BTreeMap<u64, ObjectRangeDiskStorage>>,
    eviction_candidates: Vec<(ObjectKey, u64)>,
    next_candidate: usize,
    next_publication_id: u64,
    metrics: Arc<DiskMetrics>,
}

/// One reserved chunk's metadata and retained payload slices, before its only write.
struct UnwrittenChunk {
    region: DiskRegion,
    // Ordered 4-KiB-aligned byte positions and contents; gaps are zero-filled by the I/O layer.
    parts: Vec<(usize, Bytes)>,
    used_bytes: u64,
    metadata_starts: EntryMetadataStartBitmap,
}

/// Entries and their chunk contents prepared for one shard's batch write.
#[derive(Default)]
struct UnwrittenShardBatch {
    // Recovery may combine chunks only when they were written by the same batch.
    batch_id: [u8; 16],
    chunks: Vec<UnwrittenChunk>,
    entries: Vec<(ObjectKey, ObjectRangeDiskStorage)>,
}

/// Candidate entry metadata start positions within one chunk, encoded as a bitmap.
#[derive(Default)]
struct EntryMetadataStartBitmap {
    bitmap: [u8; 32],
}

impl EntryMetadataStartBitmap {
    fn insert(&mut self, offset_in_chunk: u64) {
        let bit_index = (offset_in_chunk / METADATA_PAGE_BYTES as u64) as usize;
        self.bitmap[bit_index / 8] |= 1 << (bit_index % 8);
    }
}

/// Verified bytes or failure shared by concurrent reads of one stored entry.
type EntryReadResult = OnceCell<Result<Bytes, DiskLookupOutcome>>;

/// Disk storage reserved for one object range, with its expected payload checksum.
struct ObjectRangeDiskStorage {
    // Only concurrent callers retain the result; the index must not cache payload bytes.
    read_result: Weak<EntryReadResult>,
    eviction_position: usize,
    publication_id: u64,
    object_range: ByteRange,
    payload_checksum: u64,
    payload_regions: Vec<DiskRegion>,
    // Keep metadata-only chunks reserved for as long as the entry exists.
    entry_metadata_regions: Vec<DiskRegion>,
}

/// A guarded object-range read, with the expected checksum and all payload regions.
struct GuardedObjectRangeRead {
    object_range: ByteRange,
    payload_checksum: u64,
    payload_regions: Vec<DiskRegionReadGuard>,
}

/// Failure opening or writing to the experimental disk range cache. Read uncertainty becomes a miss.
#[derive(Debug, thiserror::Error)]
pub enum DiskRangeCacheError {
    /// The prototype requires a positive whole number of 1-MiB chunks within Linux's file-offset limit.
    #[error("disk range cache capacity must be at least 1 MiB and at most i64::MAX")]
    InvalidCapacity,
    /// Raw storage failed.
    #[error(transparent)]
    DataFile(#[from] DataFileError),
    /// The task that owns a disk write failed; queued writes retain their chunks until completion.
    #[error("disk write task failed: {0}")]
    WriteTaskFailed(#[source] tokio::task::JoinError),
    /// The small recovery inventory could not be initialized or durably reset.
    #[error("opening disk recovery state failed: {0}")]
    RecoveryState(#[source] std::io::Error),
}

impl fmt::Debug for DiskRangeCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskRangeCache")
            .field("capacity", &self.disk.file.capacity())
            .field("arenas", &self.disk.shards.len())
            .finish_non_exhaustive()
    }
}

impl DiskRangeCache {
    /// Opens an exclusively locked, fixed-capacity file and starts background recovery.
    /// Reads and writes are immediately available; recovered entries appear incrementally.
    pub async fn open(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
    ) -> Result<Self, DiskRangeCacheError> {
        Self::open_with_access_histories(directory, capacity, metrics, Arc::new(ObjectAccessHistories::new())).await
    }

    /// Opens a disk tier that consults standalone shared request history for retention decisions.
    /// Insertion and `get` do not record accesses; public request handling records once before lookup.
    pub async fn open_with_access_histories(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
    ) -> Result<Self, DiskRangeCacheError> {
        Self::open_with_metrics(
            directory,
            capacity,
            metrics,
            access_histories,
            DiskMetrics::noop(),
            RECLAIM_SAMPLE_SIZE,
        )
        .await
    }

    /// Opens a disk tier with registered file-I/O and range-cache metrics.
    pub async fn open_with_metrics(
        directory: impl AsRef<Path>,
        capacity: u64,
        io_metrics: Arc<IoMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
        metrics: Arc<DiskMetrics>,
        reclaim_sample_size: usize,
    ) -> Result<Self, DiskRangeCacheError> {
        assert!(reclaim_sample_size > 0, "reclaim sample size must be greater than zero");
        if capacity < CHUNK_BYTES || capacity > i64::MAX as u64 {
            return Err(DiskRangeCacheError::InvalidCapacity);
        }
        let capacity = capacity / CHUNK_BYTES * CHUNK_BYTES;
        let directory = directory.as_ref().to_path_buf();
        let file = DataFile::open(&directory, capacity, io_metrics).await?;
        let num_shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        let lock_owner = file.clone();
        let (recovery, ends) = tokio::task::spawn_blocking(move || {
            let _lock_owner = lock_owner;
            recovery::RecoveryState::open(&directory, capacity, num_shards)
        })
        .await
        .map_err(|error| DiskRangeCacheError::RecoveryState(std::io::Error::other(error)))?
        .map_err(DiskRangeCacheError::RecoveryState)?;
        let shards = (0..num_shards)
            .map(|shard_index| {
                let range = recovery::shard_range(capacity, num_shards, shard_index);
                let allocator = DiskChunkAllocator::with_metrics(range.clone(), metrics.clone()).unwrap();
                if ends[shard_index] > range.start {
                    allocator.start_recovery(range.start, ends[shard_index]);
                }
                DiskCacheShard {
                    reclaim_sample_size,
                    allocator,
                    entry_index: Mutex::new(DiskEntryIndex::new(metrics.clone())),
                    written_end: AtomicU64::new(ends[shard_index]),
                }
            })
            .collect();
        let disk = Arc::new(DiskRangeCacheState {
            file,
            shards,
            access_histories,
            metrics,
            recovery,
        });
        disk.start_background_tasks(ends);
        Ok(Self { disk })
    }

    /// Shared history for recording requests once, before lookup and outside raw storage operations.
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
    /// Callers bound batch size and concurrency: payload slices are retained until writing completes.
    /// Dropping this future does not abort its detached writer or release storage needed by submitted I/O.
    pub async fn insert_batch(&self, downloads: Vec<(ObjectKey, Download)>) -> Result<usize, DiskRangeCacheError> {
        self.insert_batch_checked(
            downloads
                .into_iter()
                .map(|(key, download)| (key, download, ()))
                .collect(),
            |(), publish| publish(),
        )
        .await
    }

    /// Writes a batch with per-entry publication checks owned by the caller.
    /// `with_current` must synchronously invoke `publish` once if its token is still current,
    /// keeping it current throughout publication, or not invoke it to discard the completed write.
    /// It runs under the disk index lock and must not reenter disk storage or perform I/O.
    /// The detached writer retains tokens and reservations even if this future is canceled.
    pub async fn insert_batch_checked<T, F>(
        &self,
        downloads: Vec<(ObjectKey, Download, T)>,
        with_current: F,
    ) -> Result<usize, DiskRangeCacheError>
    where
        T: Send + 'static,
        F: Fn(&T, &mut dyn FnMut()) + Send + 'static,
    {
        let disk = self.disk.clone();
        let downloads: Vec<_> = downloads
            .into_iter()
            .map(|(key, download, token)| (key, download, token, DiskWriteAttempt::new(disk.metrics.clone())))
            .collect();
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DataFileError::RuntimeUnavailable)?;
        runtime
            .spawn(async move {
                let mut downloads_by_shard: Vec<Vec<_>> = (0..disk.shards.len()).map(|_| Vec::new()).collect();
                for (key, download, token, attempt) in downloads {
                    downloads_by_shard[disk.shard_index_for_key(&key)].push((key, download, token, attempt));
                }
                let mut published_entries = 0;
                for (shard, mut downloads) in disk.shards.iter().zip(downloads_by_shard) {
                    downloads.sort_by_key(|(_, download, _, _)| download.bytes().len());
                    let mut batch = UnwrittenShardBatch {
                        batch_id: disk.recovery.next_batch_id(),
                        ..Default::default()
                    };
                    let mut publication_tokens = Vec::new();
                    let mut attempts_left = MAX_EVICTION_ATTEMPTS;
                    let mut regions_left = MAX_EVICTION_REGIONS;
                    for (key, download, token, mut attempt) in downloads {
                        if shard
                            .entry_index
                            .lock()
                            .unwrap()
                            .covering_range(&key, download.downloaded_range())
                            .is_none()
                        {
                            let previous_entry_count = batch.entries.len();
                            let previous_regions_left = regions_left;
                            while let Err(chunks_needed) = batch.pack_download(&shard.allocator, &key, &download) {
                                // Evicting cannot help an entry that exceeds the capacity left by this batch.
                                if chunks_needed > shard.allocator.chunk_capacity - batch.chunks.len() as u64
                                    || !shard.evict_candidate(
                                        &disk.access_histories,
                                        &mut attempts_left,
                                        &mut regions_left,
                                    )
                                {
                                    break;
                                }
                            }
                            // Only actual removals consume the region budget; sampling alone does not.
                            attempt.evicted = regions_left != previous_regions_left;
                            if batch.entries.len() != previous_entry_count {
                                publication_tokens.push((token, attempt));
                            } else {
                                attempt.set_outcome(DiskWriteOutcome::NoCapacity);
                            }
                        } else {
                            attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                        }
                    }
                    let entries = match batch
                        .write_chunks(&disk.file, &disk.metrics, &disk.recovery.generation, &shard.written_end)
                        .await
                    {
                        Ok(entries) => entries,
                        Err(error) => {
                            for (_, attempt) in &mut publication_tokens {
                                attempt.set_outcome(DiskWriteOutcome::Failed);
                            }
                            return Err(error);
                        }
                    };
                    let mut index = shard.entry_index.lock().unwrap();
                    // Publish larger entries first so contained batch members need not publish at all.
                    for ((key, storage), (token, mut attempt)) in entries.into_iter().zip(publication_tokens).rev() {
                        let object_range = storage.object_range;
                        // Another writer or an earlier entry in this batch may already cover this range.
                        if index.covering_range(&key, object_range).is_some() {
                            attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                            continue;
                        }
                        attempt.set_outcome(DiskWriteOutcome::Stale);
                        let mut entry = Some((key, storage));
                        with_current(&token, &mut || {
                            if let Some((key, storage)) = entry.take() {
                                index.insert(key, storage);
                                published_entries += 1;
                                attempt.set_outcome(DiskWriteOutcome::Published);
                            }
                        });
                    }
                }
                Ok(published_entries)
            })
            .await
            .map_err(DiskRangeCacheError::WriteTaskFailed)?
    }

    /// Checks indexed coverage without reading payload or recording an access.
    pub fn contains(&self, key: &ObjectKey, range: ByteRange) -> bool {
        self.disk.shards[self.disk.shard_index_for_key(key)]
            .entry_index
            .lock()
            .unwrap()
            .covering_range(key, range)
            .is_some()
    }

    /// Returns exactly requested bytes from one covering entry, or a miss on any I/O/integrity uncertainty.
    /// Concurrent reads of one stored entry share whole-entry I/O and checksum verification.
    /// Reads no neighboring entries or metadata. Results retain no disk ownership.
    pub async fn get(&self, key: &ObjectKey, requested: ByteRange) -> Option<Bytes> {
        let started = Instant::now();
        let metrics = &self.disk.metrics;
        let shard = &self.disk.shards[self.disk.shard_index_for_key(key)];
        let (guarded_read, read_result) = {
            let mut index = shard.entry_index.lock().unwrap();
            let Some(storage) = index.covering_range(key, requested) else {
                metrics.record_lookup(DiskLookupOutcome::Absent, started.elapsed());
                return None;
            };
            let start = storage.object_range.start();
            let storage = index.ranges_by_key.get_mut(key).unwrap().get_mut(&start).unwrap();
            let read_result = storage.read_result.upgrade().unwrap_or_else(|| {
                let result = Arc::new(OnceCell::new());
                storage.read_result = Arc::downgrade(&result);
                result
            });
            (
                GuardedObjectRangeRead {
                    object_range: storage.object_range,
                    payload_checksum: storage.payload_checksum,
                    payload_regions: storage.payload_regions.iter().map(DiskRegion::read_guard).collect(),
                },
                read_result,
            )
        };
        // If the initializing caller is canceled, OnceCell lets a waiter take over.
        let result = read_result
            .get_or_init(|| async {
                match guarded_read
                    .read_verified_range(&self.disk.file, guarded_read.object_range)
                    .await
                {
                    Ok(Some(bytes)) => Ok(bytes),
                    result => {
                        let outcome = match result {
                            Err(error) => {
                                tracing::warn!(target: "feuer::storage", %error, "disk read failed; entry invalidated");
                                DiskLookupOutcome::IoError
                            }
                            _ => {
                                tracing::warn!(target: "feuer::storage", "disk checksum failed; entry invalidated");
                                DiskLookupOutcome::ChecksumFailed
                            }
                        };
                        shard
                            .entry_index
                            .lock()
                            .unwrap()
                            .remove_entry_matching_read(key, &guarded_read);
                        Err(outcome)
                    }
                }
            })
            .await;
        match result {
            Ok(bytes) => {
                metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
                let start = (requested.start() - guarded_read.object_range.start()) as usize;
                Some(bytes.slice(start..start + requested.len() as usize))
            }
            Err(outcome) => {
                metrics.record_lookup(*outcome, started.elapsed());
                None
            }
        }
    }
}

impl DiskCacheShard {
    /// Advances bounded policy work and evicts at most one entry, never its neighbors.
    /// Chunk release is left to ownership.
    fn evict_candidate(
        &self,
        access_histories: &ObjectAccessHistories,
        attempts_left: &mut usize,
        regions_left: &mut usize,
    ) -> bool {
        if *attempts_left == 0 || *regions_left == 0 {
            return false;
        }
        let mut index = self.entry_index.lock().unwrap();
        let candidate_count = index.eviction_candidates.len();
        if candidate_count == 0 {
            return false;
        }
        *attempts_left -= 1;
        let (sample_start, sample_count) =
            sample_candidates(&mut index.next_candidate, candidate_count, self.reclaim_sample_size);
        let mut selected_candidate: Option<(usize, f64, u64, u64)> = None;
        for sample_offset in 0..sample_count {
            let position = (sample_start + sample_offset) % candidate_count;
            let (key, range_start) = &index.eviction_candidates[position];
            let entry = &index.ranges_by_key[key][range_start];
            if entry.region_count() > *regions_left {
                continue;
            }
            let retrieval_cost = access_histories.retention_score(key, entry.object_range);
            let payload_bytes = entry.object_range.len();
            if selected_candidate.is_none_or(|(_, selected_cost, selected_bytes, selected_id)| {
                compare_cost_per_byte(retrieval_cost, payload_bytes, selected_cost, selected_bytes)
                    .then_with(|| entry.publication_id.cmp(&selected_id))
                    .is_lt()
            }) {
                selected_candidate = Some((position, retrieval_cost, payload_bytes, entry.publication_id));
            }
        }
        if let Some((position, ..)) = selected_candidate {
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
    fn single_chunk_start(&self) -> Option<u64> {
        if self.payload_regions.len() != 1 || self.entry_metadata_regions.len() != 1 {
            return None;
        }
        let chunk_number = self.payload_regions[0].range().start / CHUNK_BYTES;
        (self.entry_metadata_regions[0].range().start / CHUNK_BYTES == chunk_number)
            .then_some(chunk_number * CHUNK_BYTES)
    }
}

impl DiskEntryIndex {
    fn new(metrics: Arc<DiskMetrics>) -> Self {
        Self {
            ranges_by_key: HashMap::new(),
            eviction_candidates: Vec::new(),
            next_candidate: 0,
            next_publication_id: 0,
            metrics,
        }
    }

    fn insert(&mut self, key: ObjectKey, mut storage: ObjectRangeDiskStorage) {
        let object_range = storage.object_range;
        if let Some(entries) = self.ranges_by_key.get(&key) {
            let replaced_range_starts: Vec<_> = entries
                .range(object_range.start()..object_range.end())
                .filter_map(|(&start, entry)| object_range.contains(entry.object_range).then_some(start))
                .collect();
            for start in replaced_range_starts {
                self.remove(&key, start);
            }
        }
        self.next_publication_id = self
            .next_publication_id
            .checked_add(1)
            .expect("disk entry identities exhausted");
        storage.publication_id = self.next_publication_id;
        storage.eviction_position = self.eviction_candidates.len();
        self.metrics.entries.increase(1);
        self.metrics.payload_bytes.increase(object_range.len());
        self.eviction_candidates.push((key.clone(), object_range.start()));
        self.ranges_by_key
            .entry(key)
            .or_default()
            .insert(object_range.start(), storage);
    }

    fn remove(&mut self, key: &str, start: u64) -> Option<ObjectRangeDiskStorage> {
        let entries = self.ranges_by_key.get_mut(key)?;
        let storage = entries.remove(&start)?;
        self.metrics.entries.decrease(1);
        self.metrics.payload_bytes.decrease(storage.object_range.len());
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

    /// Removes the indexed entry matching the failed read's expected start and checksum.
    fn remove_entry_matching_read(&mut self, key: &str, read: &GuardedObjectRangeRead) {
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

impl Drop for DiskEntryIndex {
    fn drop(&mut self) {
        self.metrics.entries.decrease(self.eviction_candidates.len() as u64);
        self.metrics.payload_bytes.decrease(
            self.ranges_by_key
                .values()
                .flat_map(|entries| entries.values())
                .map(|entry| entry.object_range.len())
                .sum(),
        );
    }
}

impl DiskRangeCacheState {
    fn shard_index_for_key(&self, key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % self.shards.len() as u64) as usize
    }
}

/// Computes metadata storage bytes, including page headers and final padding.
fn metadata_storage_bytes(content_bytes: usize) -> Option<u64> {
    (content_bytes as u64)
        .div_ceil(PAGE_CONTENT_BYTES as u64)
        .checked_mul(METADATA_PAGE_BYTES as u64)
}

impl UnwrittenShardBatch {
    /// Packs one download's payload and metadata into reserved chunks without writing them.
    /// Allocation failure leaves the batch unchanged and reports the required number of new chunks.
    fn pack_download(
        &mut self,
        allocator: &DiskChunkAllocator,
        key: &ObjectKey,
        download: &Download,
    ) -> Result<(), u64> {
        let object_range = download.downloaded_range();
        let bytes = download.bytes();
        let aligned_payload_bytes = (bytes.len() as u64).next_multiple_of(PAYLOAD_ALIGNMENT_BYTES);
        let chunk_data_bytes = CHUNK_BYTES - METADATA_PAGE_BYTES as u64;
        let payload_region_count = aligned_payload_bytes.div_ceil(chunk_data_bytes);
        let metadata_bytes =
            metadata_storage_bytes(64 + 16 * payload_region_count as usize + key.len()).ok_or(u64::MAX)?;
        let entry_bytes = aligned_payload_bytes + metadata_bytes;
        let free_tail_bytes = self.chunks.last().map_or(0, |chunk| CHUNK_BYTES - chunk.used_bytes);
        // Never split an entry across a shared chunk boundary, even if only metadata would spill.
        let shares_tail = entry_bytes <= free_tail_bytes;
        let new_chunk_count = if shares_tail {
            0
        } else {
            entry_bytes.div_ceil(chunk_data_bytes)
        };
        let mut chunk_index = if shares_tail {
            self.chunks.len() - 1
        } else {
            self.chunks.len()
        };
        if new_chunk_count > 0 {
            let regions = allocator.reserve_chunks(new_chunk_count).ok_or(new_chunk_count)?;
            self.chunks.extend(regions.into_iter().map(|region| UnwrittenChunk {
                region,
                // The chunk metadata page is filled after all entry positions are known.
                parts: vec![(0, Bytes::new())],
                used_bytes: METADATA_PAGE_BYTES as u64,
                metadata_starts: EntryMetadataStartBitmap::default(),
            }));
        }
        let (payload_regions, first_payload_chunk) =
            self.reserve_entry_regions(&mut chunk_index, aligned_payload_bytes);
        let mut payload_offset = 0;
        for (chunk, region) in self.chunks[first_payload_chunk..].iter_mut().zip(&payload_regions) {
            let disk_range = region.range();
            let offset_in_chunk = (disk_range.start % CHUNK_BYTES) as usize;
            let payload_end = (payload_offset + (disk_range.end - disk_range.start) as usize).min(bytes.len());
            chunk
                .parts
                .push((offset_in_chunk, bytes.slice(payload_offset..payload_end)));
            payload_offset = payload_end;
        }
        let payload_checksum = XxHash64::oneshot(0, bytes);
        let contents =
            page_format::encode_entry_metadata(key, object_range, &payload_regions, payload_checksum, &self.batch_id);
        let (entry_metadata_regions, first_metadata_chunk) =
            self.reserve_entry_regions(&mut chunk_index, metadata_bytes);
        self.chunks[first_metadata_chunk]
            .metadata_starts
            .insert(entry_metadata_regions[0].range().start % CHUNK_BYTES);
        let content_checksum = XxHash64::oneshot(0, &contents);
        let mut page_ordinal = 0;
        let mut metadata_offset = 0;
        for (region_index, (chunk, region)) in self.chunks[first_metadata_chunk..]
            .iter_mut()
            .zip(&entry_metadata_regions)
            .enumerate()
        {
            let range = region.range();
            for page_address in (range.start..range.end).step_by(METADATA_PAGE_BYTES) {
                let next_page_address = if page_address + (METADATA_PAGE_BYTES as u64) < range.end {
                    page_address + METADATA_PAGE_BYTES as u64
                } else {
                    entry_metadata_regions
                        .get(region_index + 1)
                        .map_or(0, |region| region.range().start)
                };
                let offset_in_chunk = (page_address % CHUNK_BYTES) as usize;
                let metadata_end = (metadata_offset + PAGE_CONTENT_BYTES).min(contents.len());
                let mut page = vec![0; METADATA_PAGE_BYTES];
                page_format::encode_page(
                    &mut page,
                    page_format::ENTRY_METADATA_PAGE_TAG,
                    content_checksum,
                    page_address,
                    page_ordinal,
                    next_page_address,
                    &contents[metadata_offset..metadata_end],
                );
                chunk.parts.push((offset_in_chunk, Bytes::from(page)));
                metadata_offset = metadata_end;
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
                read_result: Weak::new(),
                eviction_position: 0,
                publication_id: 0,
                object_range,
                payload_checksum,
                payload_regions,
                entry_metadata_regions,
            },
        ));
        Ok(())
    }

    /// Reserves aligned entry regions within the batch's chunks, preserving shared whole-chunk ownership.
    fn reserve_entry_regions(&mut self, chunk_index: &mut usize, mut remaining_bytes: u64) -> (Vec<DiskRegion>, usize) {
        while self.chunks[*chunk_index].used_bytes == CHUNK_BYTES {
            *chunk_index += 1;
        }
        let first_chunk_index = *chunk_index;
        let mut regions = Vec::new();
        while remaining_bytes > 0 {
            let chunk = &mut self.chunks[*chunk_index];
            let region_bytes = remaining_bytes.min(CHUNK_BYTES - chunk.used_bytes);
            let disk_offset = chunk.region.range().start + chunk.used_bytes;
            regions.push(chunk.region.slice(disk_offset..disk_offset + region_bytes));
            chunk.used_bytes += region_bytes;
            remaining_bytes -= region_bytes;
            if remaining_bytes > 0 {
                *chunk_index += 1;
            }
        }
        (regions, first_chunk_index)
    }

    /// Finalizes chunk metadata and writes all chunks before returning entries for publication.
    async fn write_chunks(
        mut self,
        file: &DataFile,
        metrics: &DiskMetrics,
        generation: &[u8; 16],
        written_end: &AtomicU64,
    ) -> Result<Vec<(ObjectKey, ObjectRangeDiskStorage)>, DiskRangeCacheError> {
        for chunk in &mut self.chunks {
            let address = chunk.region.range().start;
            let mut page = vec![0; METADATA_PAGE_BYTES];
            let mut contents = [0; page_format::CHUNK_METADATA_CONTENT_BYTES];
            contents[..32].copy_from_slice(&chunk.metadata_starts.bitmap);
            contents[32..48].copy_from_slice(generation);
            contents[48..64].copy_from_slice(&self.batch_id);
            page_format::encode_page(
                &mut page,
                page_format::CHUNK_METADATA_PAGE_TAG,
                XxHash64::oneshot(0, &contents),
                address,
                address / CHUNK_BYTES,
                0,
                &contents,
            );
            chunk.parts[0].1 = Bytes::from(page);
            if let Err(error) = file
                .write_parts(chunk.region.slice(chunk.region.range()), &chunk.parts)
                .await
            {
                tracing::warn!(target: "feuer::storage", %error, "batch write failed");
                return Err(error.into());
            }
            written_end.fetch_max(address + CHUNK_BYTES, Ordering::Relaxed);
        }
        metrics.written_entries.increase(self.entries.len() as u64);
        metrics
            .packed_payload_bytes
            .increase(self.entries.iter().map(|(_, entry)| entry.object_range.len()).sum());
        metrics
            .packed_chunk_bytes
            .increase(self.chunks.len() as u64 * CHUNK_BYTES);
        Ok(self.entries)
    }
}

impl GuardedObjectRangeRead {
    /// Reads the whole entry, verifies its checksum, and returns only the requested byte range.
    async fn read_verified_range(&self, file: &DataFile, requested: ByteRange) -> DataFileResult<Option<Bytes>> {
        let start = requested.start() - self.object_range.start();
        let end = requested.end() - self.object_range.start();
        let bytes = file
            .read_regions(self.payload_regions.clone(), self.object_range.len() as usize)
            .await?;
        if XxHash64::oneshot(0, &bytes) != self.payload_checksum {
            return Ok(None);
        }
        Ok(Some(bytes.slice(start as usize..end as usize)))
    }
}
