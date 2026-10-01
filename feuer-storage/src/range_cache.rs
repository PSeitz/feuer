//! Immutable payload chunks, mutable metadata chunks, and whole-entry validation.

mod metadata;
#[cfg(test)]
mod metrics_tests;
mod page_format;
mod recovery;
#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use bytes::Bytes;
use feuer_memory::BufferPool;
use feuer_types::{
    ByteRange, Download, ObjectKeyHash,
    retention::{ObjectAccessHistories, RECLAIM_SAMPLE_SIZE, compare_cost_per_byte, sample_candidates},
};
use tokio::sync::OnceCell;
use twox_hash::XxHash64;

use crate::{
    DataFile, DataFileError, DataFileResult, DiskMetrics, IoMetrics,
    allocation::{CHUNK_BYTES, DiskChunkAllocator, DiskRegion, DiskRegionReadGuard},
    disk_metrics::{DiskLookupOutcome, DiskWriteAttempt, DiskWriteOutcome},
};
#[cfg(test)]
use page_format::{METADATA_PAGE_BYTES, PAGE_CONTENT_BYTES};

const PAYLOAD_ALIGNMENT_BYTES: u64 = crate::uring::DIRECT_IO_ALIGNMENT_BYTES as u64;

// Per shard batch: bound sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_CHUNKS: usize = 4096;

/// An experimental disk range cache used by the public tiered cache.
///
/// Explicit batches pack payloads into immutable 1-MiB chunks. Separate mutable metadata chunks
/// hold entry records and links between metadata chunks.
/// Entries have plain payload bytes, 4-KiB-aligned storage, and a checksum in their entry metadata.
/// Reads verify the whole covering entry, returning only requested bytes. Reuse requires a wholly free chunk.
/// The key hash selects an independently allocated shard; admission may fail despite space elsewhere.
///
/// Pressure eviction uses sampled retrieval value per payload byte.
/// Opening waits for all shards to recover before making the cache available.
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
    metadata: Arc<Mutex<metadata::MetadataPages>>,
    metadata_io: tokio::sync::Mutex<()>,
    metadata_head: AtomicU64,
}

/// Disk entries indexed by object key and range start, with dense rotating eviction candidates.
struct DiskEntryIndex {
    ranges_by_key: HashMap<ObjectKeyHash, BTreeMap<u64, ObjectRangeDiskStorage>>,
    eviction_candidates: Vec<(ObjectKeyHash, u64)>,
    next_candidate: usize,
    metrics: Arc<DiskMetrics>,
}

/// One contiguous reserved disk region and retained payload slices, before writing.
struct UnwrittenRegion {
    region: DiskRegion,
    // Payload slices at 4-KiB-aligned offsets; metadata lives in separate chunks.
    parts: Vec<(usize, Bytes)>,
    used_bytes: u64,
}

/// Entries and their chunk contents prepared for one shard's batch write.
#[derive(Default)]
struct UnwrittenShardBatch {
    regions: Vec<UnwrittenRegion>,
    entries: Vec<(ObjectKeyHash, ObjectRangeDiskStorage)>,
}

/// Verified bytes or failure shared by concurrent reads of one stored entry.
type EntryReadResult = OnceCell<Result<(Bytes, usize), DiskLookupOutcome>>;

/// Disk storage reserved for one object range, with its expected payload checksum.
struct ObjectRangeDiskStorage {
    // Only concurrent callers retain the result; the index must not cache payload bytes.
    read_result: Weak<EntryReadResult>,
    eviction_position: usize,
    object_range: ByteRange,
    payload_checksum: u64,
    payload_region: DiskRegion,
    metadata_slot: Option<metadata::MetadataSlot>,
}

impl Drop for DiskRangeCacheState {
    fn drop(&mut self) {
        // Closing the in-memory index must not erase records needed by the next open.
        for shard in &self.shards {
            shard.metadata.lock().unwrap().closing = true;
        }
    }
}

/// A guarded object-range read, with the expected checksum and contiguous payload region.
struct GuardedObjectRangeRead {
    object_range: ByteRange,
    payload_checksum: u64,
    payload_region: DiskRegionReadGuard,
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
    /// Opens an exclusively locked, fixed-capacity file after scanning every shard's metadata.
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
        Self::open_with_buffer_pool(
            directory,
            capacity,
            io_metrics,
            access_histories,
            metrics,
            reclaim_sample_size,
            BufferPool::unpooled(),
        )
        .await
    }

    /// Opens a disk tier sharing its memory cache's aligned buffer pool.
    pub async fn open_with_buffer_pool(
        directory: impl AsRef<Path>,
        capacity: u64,
        io_metrics: Arc<IoMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
        metrics: Arc<DiskMetrics>,
        reclaim_sample_size: usize,
        buffer_pool: Arc<BufferPool>,
    ) -> Result<Self, DiskRangeCacheError> {
        assert!(reclaim_sample_size > 0, "reclaim sample size must be greater than zero");
        if capacity < CHUNK_BYTES || capacity > i64::MAX as u64 {
            return Err(DiskRangeCacheError::InvalidCapacity);
        }
        let capacity = capacity / CHUNK_BYTES * CHUNK_BYTES;
        let directory = directory.as_ref().to_path_buf();
        let file = DataFile::open_with_buffer_pool(&directory, capacity, io_metrics, buffer_pool).await?;
        let num_shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        let lock_owner = file.clone();
        let (recovery, metadata_heads) = tokio::task::spawn_blocking(move || {
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
                if metadata_heads[shard_index] != page_format::NO_CHUNK {
                    allocator.start_recovery(range.start, range.end);
                }
                DiskCacheShard {
                    reclaim_sample_size,
                    allocator,
                    entry_index: Mutex::new(DiskEntryIndex::new(metrics.clone())),
                    metadata: Arc::new(Mutex::new(metadata::MetadataPages::default())),
                    metadata_io: tokio::sync::Mutex::new(()),
                    metadata_head: AtomicU64::new(metadata_heads[shard_index]),
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
        let started = Instant::now();
        tracing::info!(target: "feuer::storage", "starting disk cache recovery");
        for index in 0..disk.shards.len() {
            disk.recover_shard(index).await;
        }
        tracing::info!(target: "feuer::storage", elapsed_seconds = started.elapsed().as_secs_f64(), "disk cache recovery finished");
        disk.start_checkpoint_task();
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
    /// Entries share a payload chunk only when their complete aligned payloads fit inside it.
    /// Multi-chunk entries own their chunks exclusively. Partially filled chunks are finalized too.
    /// Pressure eviction is bounded; unavailable capacity causes admission to be skipped, not waited for.
    /// Callers bound batch size and concurrency: payload slices are retained until writing completes.
    /// Dropping this future does not abort its detached writer or release storage needed by submitted I/O.
    pub async fn insert_batch(&self, downloads: Vec<(ObjectKeyHash, Download)>) -> Result<usize, DiskRangeCacheError> {
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
        downloads: Vec<(ObjectKeyHash, Download, T)>,
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
                    if downloads.is_empty() {
                        continue;
                    }
                    let _metadata_io = shard.metadata_io.lock().await;
                    shard.flush_metadata(&disk.file).await?;
                    downloads.sort_by_key(|(_, download, _, _)| download.bytes().len());
                    let mut batch = UnwrittenShardBatch::default();
                    let mut publication_tokens: Vec<(T, DiskWriteAttempt)> = Vec::new();
                    let mut attempts_left = MAX_EVICTION_ATTEMPTS;
                    let mut chunks_left = MAX_EVICTION_CHUNKS;
                    for (key, download, token, mut attempt) in downloads {
                        if shard
                            .entry_index
                            .lock()
                            .unwrap()
                            .covering_range(&key, download.downloaded_range())
                            .is_none()
                        {
                            let previous_entry_count = batch.entries.len();
                            let previous_chunks_left = chunks_left;
                            loop {
                                let ready = match shard.ensure_metadata_slots(&disk.file, batch.entries.len() + 1).await
                                {
                                    Ok(ready) => ready,
                                    Err(error) => {
                                        attempt.set_outcome(DiskWriteOutcome::Failed);
                                        for (_, pending) in &mut publication_tokens {
                                            pending.set_outcome(DiskWriteOutcome::Failed);
                                        }
                                        return Err(error.into());
                                    }
                                };
                                let chunks_needed = if ready {
                                    match batch.pack_download(&shard.allocator, &key, &download) {
                                        Ok(()) => break,
                                        Err(count) => count,
                                    }
                                } else {
                                    1
                                };
                                let metadata_chunks = shard.metadata.lock().unwrap().chunks.len() as u64;
                                // Evicting cannot help an entry that exceeds the capacity left by this batch.
                                if chunks_needed
                                    > shard.allocator.chunk_capacity - batch.chunk_count() - metadata_chunks
                                    || !shard.evict_candidate(
                                        &disk.access_histories,
                                        &mut attempts_left,
                                        &mut chunks_left,
                                    )
                                {
                                    break;
                                }
                                shard.flush_metadata(&disk.file).await?;
                            }
                            // Only actual removals consume the chunk budget; sampling alone does not.
                            attempt.evicted = chunks_left != previous_chunks_left;
                            if batch.entries.len() != previous_entry_count {
                                publication_tokens.push((token, attempt));
                            } else {
                                attempt.set_outcome(DiskWriteOutcome::NoCapacity);
                            }
                        } else {
                            attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                        }
                    }
                    let entries = match batch.write_chunks(&disk.file, &disk.metrics, shard).await {
                        Ok(entries) => entries,
                        Err(error) => {
                            for (_, attempt) in &mut publication_tokens {
                                attempt.set_outcome(DiskWriteOutcome::Failed);
                            }
                            return Err(error);
                        }
                    };
                    {
                        let mut index = shard.entry_index.lock().unwrap();
                        // Publish larger entries first so contained batch members need not publish at all.
                        for ((key, storage), (token, mut attempt)) in entries.into_iter().zip(publication_tokens).rev()
                        {
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
                                    index.remove_covered_ranges(&key, storage.object_range);
                                    index.insert(key, storage);
                                    published_entries += 1;
                                    attempt.set_outcome(DiskWriteOutcome::Published);
                                }
                            });
                        }
                    }
                    shard.flush_metadata(&disk.file).await?;
                }
                Ok(published_entries)
            })
            .await
            .map_err(DiskRangeCacheError::WriteTaskFailed)?
    }

    /// Checks indexed coverage without reading payload or recording an access.
    pub fn contains(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
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
    pub async fn get(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<Bytes> {
        self.get_with_capacity(key, requested).await.map(|(bytes, _)| bytes)
    }

    /// Returns the requested slice and its whole backing allocation capacity for memory admission.
    pub async fn get_with_capacity(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<(Bytes, usize)> {
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
                    payload_region: storage.payload_region.read_guard(),
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
            Ok((bytes, capacity)) => {
                metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
                let start = (requested.start() - guarded_read.object_range.start()) as usize;
                Some((bytes.slice(start..start + requested.len() as usize), *capacity))
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
        chunks_left: &mut usize,
    ) -> bool {
        if *attempts_left == 0 || *chunks_left == 0 {
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
        let mut selected_candidate: Option<(usize, f64, u64)> = None;
        for sample_offset in 0..sample_count {
            let position = (sample_start + sample_offset) % candidate_count;
            let (key, range_start) = &index.eviction_candidates[position];
            let entry = &index.ranges_by_key[key][range_start];
            if entry.payload_region.chunk_count() as usize > *chunks_left {
                continue;
            }
            let retrieval_cost = access_histories.retention_score(key, entry.object_range);
            let payload_bytes = entry.object_range.len();
            if selected_candidate.is_none_or(|(_, selected_cost, selected_bytes)| {
                compare_cost_per_byte(retrieval_cost, payload_bytes, selected_cost, selected_bytes).is_lt()
            }) {
                selected_candidate = Some((position, retrieval_cost, payload_bytes));
            }
        }
        if let Some((position, ..)) = selected_candidate {
            let (key, start) = index.eviction_candidates[position];
            *chunks_left -= index.ranges_by_key[&key][&start].payload_region.chunk_count() as usize;
            index.remove(&key, start);
        }
        true
    }
}

impl DiskEntryIndex {
    fn new(metrics: Arc<DiskMetrics>) -> Self {
        Self {
            ranges_by_key: HashMap::new(),
            eviction_candidates: Vec::new(),
            next_candidate: 0,
            metrics,
        }
    }

    // Remove entries fully covered by a write; recovery skips this cleanup.
    fn remove_covered_ranges(&mut self, key: &ObjectKeyHash, object_range: ByteRange) {
        if let Some(entries) = self.ranges_by_key.get(key) {
            let replaced_range_starts: Vec<_> = entries
                .range(object_range.start()..object_range.end())
                .filter_map(|(&start, entry)| object_range.contains(entry.object_range).then_some(start))
                .collect();
            for start in replaced_range_starts {
                self.remove(key, start);
            }
        }
    }

    fn insert(&mut self, key: ObjectKeyHash, storage: ObjectRangeDiskStorage) {
        self.insert_entries(std::iter::once((key, storage)));
    }

    fn insert_batch(&mut self, entries: &mut Vec<(ObjectKeyHash, ObjectRangeDiskStorage)>) {
        self.ranges_by_key.reserve(entries.len());
        self.eviction_candidates.reserve(entries.len());
        self.insert_entries(entries.drain(..));
    }

    fn insert_entries(&mut self, entries: impl IntoIterator<Item = (ObjectKeyHash, ObjectRangeDiskStorage)>) {
        let mut entry_count = 0;
        let mut payload_bytes = 0;
        for (key, mut storage) in entries {
            let object_range = storage.object_range;
            storage.eviction_position = self.eviction_candidates.len();
            entry_count += 1;
            payload_bytes += object_range.len();
            self.eviction_candidates.push((key, object_range.start()));
            self.ranges_by_key
                .entry(key)
                .or_default()
                .insert(object_range.start(), storage);
        }
        self.metrics.entries.increase(entry_count);
        self.metrics.payload_bytes.increase(payload_bytes);
    }

    fn remove(&mut self, key: &ObjectKeyHash, start: u64) -> Option<ObjectRangeDiskStorage> {
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
    fn remove_entry_matching_read(&mut self, key: &ObjectKeyHash, read: &GuardedObjectRangeRead) {
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

    fn covering_range(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<&ObjectRangeDiskStorage> {
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
    fn shard_index_for_key(&self, key: &ObjectKeyHash) -> usize {
        (key.0 % self.shards.len() as u128) as usize
    }
}

/// Computes metadata storage bytes, including page headers and final padding.
impl UnwrittenShardBatch {
    /// Packs one download's payload into reserved chunks without writing it.
    /// Allocation failure leaves the batch unchanged and reports the required number of new chunks.
    fn pack_download(
        &mut self,
        allocator: &DiskChunkAllocator,
        key: &ObjectKeyHash,
        download: &Download,
    ) -> Result<(), u64> {
        let object_range = download.downloaded_range();
        let bytes = download.bytes();
        let aligned_payload_bytes = (bytes.len() as u64).next_multiple_of(PAYLOAD_ALIGNMENT_BYTES);
        let shares_chunk = self.regions.last().is_some_and(|region| {
            region.region.chunk_count() == 1 && region.used_bytes + aligned_payload_bytes <= CHUNK_BYTES
        });
        if !shares_chunk {
            let count = aligned_payload_bytes.div_ceil(CHUNK_BYTES);
            let region = allocator.reserve_chunks(count).ok_or(count)?;
            self.regions.push(UnwrittenRegion {
                region,
                parts: Vec::new(),
                used_bytes: 0,
            });
        }
        let prepared = self.regions.last_mut().unwrap();
        let base = prepared.region.range().start;
        let payload_start = base + prepared.used_bytes;
        let payload_region = prepared
            .region
            .slice(payload_start..payload_start + aligned_payload_bytes);
        let payload_checksum = XxHash64::oneshot(0, bytes);
        prepared.parts.push(((payload_start - base) as usize, bytes.clone()));
        prepared.used_bytes += aligned_payload_bytes;
        self.entries.push((
            *key,
            ObjectRangeDiskStorage {
                read_result: Weak::new(),
                eviction_position: 0,
                object_range,
                payload_checksum,
                payload_region,
                metadata_slot: None,
            },
        ));
        Ok(())
    }

    fn chunk_count(&self) -> u64 {
        self.regions.iter().map(|prepared| prepared.region.chunk_count()).sum()
    }

    /// Writes payload chunks, then updates metadata pages before returning entries for publication.
    async fn write_chunks(
        mut self,
        file: &DataFile,
        metrics: &DiskMetrics,
        shard: &DiskCacheShard,
    ) -> Result<Vec<(ObjectKeyHash, ObjectRangeDiskStorage)>, DiskRangeCacheError> {
        for prepared in &mut self.regions {
            let address = prepared.region.range().start;
            for offset in (0..prepared.region.chunk_count() * CHUNK_BYTES).step_by(CHUNK_BYTES as usize) {
                let end = offset as usize + CHUNK_BYTES as usize;
                let mut parts = Vec::new();
                for (start, bytes) in &prepared.parts {
                    let from = (*start).max(offset as usize);
                    let to = (*start + bytes.len()).min(end);
                    if from < to {
                        parts.push((from - offset as usize, bytes.slice(from - start..to - start)));
                    }
                }
                if parts.first().is_none_or(|(start, _)| *start != 0) {
                    parts.insert(0, (0, Bytes::new()));
                }
                let chunk = prepared.region.slice(address + offset..address + offset + CHUNK_BYTES);
                if let Err(error) = file.write_parts(chunk, &parts).await {
                    tracing::warn!(target: "feuer::storage", %error, "batch write failed");
                    return Err(error.into());
                }
            }
        }
        for (key, entry) in &mut self.entries {
            shard.record_entry(key, entry);
        }
        shard.flush_metadata(file).await?;
        metrics.written_entries.increase(self.entries.len() as u64);
        metrics
            .packed_payload_bytes
            .increase(self.entries.iter().map(|(_, entry)| entry.object_range.len()).sum());
        metrics.packed_chunk_bytes.increase(self.chunk_count() * CHUNK_BYTES);
        Ok(self.entries)
    }
}

impl GuardedObjectRangeRead {
    /// Reads the whole entry, verifies its checksum, and returns only the requested byte range.
    async fn read_verified_range(
        &self,
        file: &DataFile,
        requested: ByteRange,
    ) -> DataFileResult<Option<(Bytes, usize)>> {
        let start = requested.start() - self.object_range.start();
        let end = requested.end() - self.object_range.start();
        let (bytes, capacity) = file
            .read_region(self.payload_region.clone(), self.object_range.len() as usize)
            .await?;
        if XxHash64::oneshot(0, &bytes) != self.payload_checksum {
            return Ok(None);
        }
        Ok(Some((bytes.slice(start as usize..end as usize), capacity)))
    }
}
