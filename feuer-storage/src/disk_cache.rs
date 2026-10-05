//! Cache object bytes on disk, looked up by object key and byte range.

mod metadata;
mod small_entry_buffer;
mod write_queue;
mod writer;
use small_entry_buffer::BufferedSmallEntryChunk;
use write_queue::PendingDiskWrite;
#[cfg(test)]
mod metrics_tests;
mod page_format;
mod recovery;
#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    fmt,
    ops::Range,
    path::Path,
    sync::{Arc, Mutex, Weak},
    time::Instant,
};

use bytes::Bytes;
use feuer_memory::{BUFFER_ALIGNMENT, BufferPool};
use feuer_types::{
    ByteRange, Download, ObjectKeyHash,
    retention::{ObjectAccessHistories, RECLAIM_SAMPLE_SIZE, compare_cost_per_byte, sample_candidates},
};
use rustc_hash::FxHashMap;
use tokio::sync::{OnceCell, mpsc};
use twox_hash::XxHash64;

use crate::{
    DataFile, DataFileError, DataFileResult, DiskMetrics, IoMetrics,
    allocation::{CHUNK_BYTES, DiskChunkAllocator, ReservedChunks},
    disk_metrics::{DiskLookupOutcome, DiskWriteAttempt, DiskWriteOutcome},
};
#[cfg(test)]
use page_format::METADATA_PAGE_BYTES;

// Per admission: limit sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_CHUNKS: usize = 4096;

/// Disk byte count for a payload, including alignment padding and a 4-KiB minimum.
fn payload_disk_bytes(length: u64) -> u64 {
    length
        .max(BUFFER_ALIGNMENT as u64)
        .next_multiple_of(BUFFER_ALIGNMENT as u64)
}

/// Caches object bytes on disk, looked up by object key and byte range.
///
/// Background writes use a nonblocking 256-entry queue. Partial small-entry buffers flush every 60 seconds.
/// Closing finishes queued writes and discards remaining partial chunks.
/// Small entries share immutable 1-MiB payload chunks. Separate mutable metadata chunks
/// hold entry records and links between metadata chunks. Each shard's chain starts at its first chunk.
/// Entries have plain payload bytes, 4-KiB-aligned storage, and a checksum in their entry metadata.
/// Reads verify the whole covering entry, returning only requested bytes. Reuse requires a wholly free chunk.
/// The key hash selects an independently allocated shard; admission may fail despite space elsewhere.
///
/// Pressure eviction uses sampled retrieval value per payload byte.
/// Opening waits for all shards to recover before making the cache available.
/// The experimental format is neither a persistence guarantee nor a stable on-disk interface.
#[derive(Clone)]
pub struct DiskCache {
    disk: Arc<DiskCacheInner>,
    write_sender: mpsc::Sender<PendingDiskWrite>,
}

/// Shared disk-cache internals: file, buffer pool, shards, access histories, and metrics.
struct DiskCacheInner {
    file: DataFile,
    buffer_pool: Arc<BufferPool>,
    shards: Box<[DiskCacheShard]>,
    access_histories: Arc<ObjectAccessHistories>,
    metrics: Arc<DiskMetrics>,
}

/// One shard's small-entry buffer, allocator, published entry index, and metadata pages.
struct DiskCacheShard {
    reclaim_sample_size: usize,
    allocator: DiskChunkAllocator,
    entry_index: Mutex<DiskEntryIndex>,
    metadata_pages: Mutex<metadata::MetadataPages>,
    buffered_small_entry_chunk: tokio::sync::Mutex<BufferedSmallEntryChunk>,
}

/// Disk entries indexed by object key and range start, with a list sampled in rotation for eviction.
struct DiskEntryIndex {
    entries_by_key: FxHashMap<ObjectKeyHash, BTreeMap<u64, DiskEntry>>,
    eviction_candidates: Vec<(ObjectKeyHash, u64)>,
    /// Index where the next eviction sample starts.
    next_sample_start: usize,
    metrics: Arc<DiskMetrics>,
}

/// Verified bytes or failure shared by concurrent reads of one stored entry.
type EntryReadResult = OnceCell<Result<(Bytes, usize), DiskLookupOutcome>>;

/// One entry's object range, payload disk range and checksum, metadata, eviction position, and shared read.
struct DiskEntry {
    // In-flight deduplication: concurrent readers share one disk read and checksum verification.
    // Only concurrent callers retain the result; the index must not cache payload bytes.
    in_flight_read: Weak<EntryReadResult>,
    eviction_position: usize,
    object_range: ByteRange,
    payload_checksum: u64,
    payload_range: Range<u64>,
    /// Metadata chunk and entry metadata indexes identifying this entry's 48-byte disk metadata.
    metadata: (usize, usize),
}

/// The disk address, object range, and expected checksum for one payload read.
struct PayloadRead {
    object_range: ByteRange,
    payload_checksum: u64,
    payload_range: Range<u64>,
}

/// An error opening or writing the disk cache. Read uncertainty becomes a miss.
#[derive(Debug, thiserror::Error)]
pub enum DiskCacheError {
    /// The requested disk capacity is unsupported.
    #[error("disk cache capacity must be at least 1 MiB and at most i64::MAX")]
    InvalidCapacity,
    /// Raw storage failed.
    #[error(transparent)]
    DataFile(#[from] DataFileError),
    /// The task performing a disk write failed; submitted I/O may still finish.
    #[error("disk write task failed: {0}")]
    WriteTaskFailed(#[source] tokio::task::JoinError),
}

impl fmt::Debug for DiskCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskCache")
            .field("capacity", &self.disk.file.capacity())
            .field("shards", &self.disk.shards.len())
            .finish_non_exhaustive()
    }
}

impl DiskCache {
    /// Opens an exclusively locked, fixed-capacity file after scanning every shard's metadata.
    pub async fn open(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
    ) -> Result<Self, DiskCacheError> {
        Self::open_with_access_histories(directory, capacity, metrics, Arc::new(ObjectAccessHistories::new())).await
    }

    /// Opens a disk tier that consults standalone shared request history for retention decisions.
    /// Insertion and `get` do not record accesses; public request handling records once before lookup.
    pub async fn open_with_access_histories(
        directory: impl AsRef<Path>,
        capacity: u64,
        metrics: Arc<IoMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
    ) -> Result<Self, DiskCacheError> {
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

    /// Opens a disk tier with registered file-I/O and disk-cache metrics.
    pub async fn open_with_metrics(
        directory: impl AsRef<Path>,
        capacity: u64,
        io_metrics: Arc<IoMetrics>,
        access_histories: Arc<ObjectAccessHistories>,
        metrics: Arc<DiskMetrics>,
        reclaim_sample_size: usize,
    ) -> Result<Self, DiskCacheError> {
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
    ) -> Result<Self, DiskCacheError> {
        assert!(reclaim_sample_size > 0, "reclaim sample size must be greater than zero");
        if capacity < CHUNK_BYTES || capacity > i64::MAX as u64 {
            return Err(DiskCacheError::InvalidCapacity);
        }
        let capacity = capacity / CHUNK_BYTES * CHUNK_BYTES;
        let file = DataFile::open_with_buffer_pool(directory, capacity, io_metrics, buffer_pool.clone()).await?;
        let num_shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        let shards = (0..num_shards)
            .map(|shard_index| {
                let range = recovery::shard_disk_range(capacity, num_shards, shard_index);
                let allocator = DiskChunkAllocator::with_metrics(range, metrics.clone()).unwrap();
                Ok(DiskCacheShard {
                    reclaim_sample_size,
                    allocator,
                    entry_index: Mutex::new(DiskEntryIndex::new(metrics.clone())),
                    buffered_small_entry_chunk: tokio::sync::Mutex::new(BufferedSmallEntryChunk::new()?),
                    metadata_pages: Mutex::new(metadata::MetadataPages::default()),
                })
            })
            .collect::<Result<_, DataFileError>>()?;
        let disk = Arc::new(DiskCacheInner {
            file,
            buffer_pool,
            shards,
            access_histories,
            metrics,
        });
        let started = Instant::now();
        tracing::info!(target: "feuer::storage", "starting disk cache recovery");
        let mut recovery_tasks = tokio::task::JoinSet::new();
        for index in 0..disk.shards.len() {
            let disk = disk.clone();
            // Poll recovery on a blocking thread, including metadata parsing and index rebuilding.
            // Its io_uring reads still use the runtime for asynchronous completion.
            recovery_tasks
                .spawn_blocking(move || tokio::runtime::Handle::current().block_on(disk.recover_shard(index)));
        }
        recovery_tasks.join_all().await;
        tracing::info!(target: "feuer::storage", elapsed_seconds = started.elapsed().as_secs_f64(), "disk cache recovery finished");
        tokio::spawn(DiskCacheInner::write_metadata_periodically(Arc::downgrade(&disk)));
        tokio::spawn(DiskCacheInner::flush_payload_periodically(Arc::downgrade(&disk)));
        let (write_sender, receiver) = mpsc::channel(write_queue::MAX_QUEUED_ENTRIES);
        tokio::spawn(Self::write_queued(receiver));
        Ok(Self { disk, write_sender })
    }

    /// Inserts a batch and flushes shared buffers. Counts entries published by this call, including neighbors.
    /// Earlier publication survives later failure. Dropping this future does not abort its detached writer.
    pub async fn insert_batch(&self, mut downloads: Vec<(ObjectKeyHash, Download)>) -> Result<usize, DiskCacheError> {
        let cache = self.clone();
        downloads.sort_by_key(|(_, download)| download.bytes().len());
        tokio::spawn(async move {
            let mut published = 0;
            for (key, download) in downloads {
                published += cache.write(key, download, ()).await?;
            }
            Ok(published + cache.flush().await?)
        })
        .await
        .map_err(DiskCacheError::WriteTaskFailed)?
    }

    /// Buffers small entries; full/no-fit chunks and larger entries write immediately.
    /// Holds completion state until publication or discard. Cancellation may leave submitted I/O running.
    pub async fn write(
        &self,
        key: ObjectKeyHash,
        download: Download,
        completion: impl Send + 'static,
    ) -> Result<usize, DiskCacheError> {
        self.disk.write(key, download, completion).await
    }

    /// Shared history for recording requests once, before lookup and outside raw storage operations.
    pub fn access_histories(&self) -> Arc<ObjectAccessHistories> {
        self.disk.access_histories.clone()
    }

    /// Checks for one covering buffered or disk entry, without disk I/O or recording an access.
    pub fn covers_range(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
        self.disk.covers_range(key, range)
    }

    /// Returns exactly requested bytes from one covering entry, or a miss on any I/O/integrity uncertainty.
    /// Concurrent reads of one stored entry share whole-entry I/O and checksum verification.
    /// Reads no neighboring entries or metadata. Results retain no disk ownership.
    pub async fn get(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<Bytes> {
        self.fetch_from_disk(key, requested).await.map(|(bytes, _)| bytes)
    }

    /// Copies buffered bytes, or reads disk after verifying the whole covering entry's checksum.
    /// Copies the requested slice if the pool offers a smaller backing allocation; allocation failure keeps the slice.
    /// Returns the final backing buffer's capacity too. Concurrent callers share the entry read.
    /// Missing entries and read or checksum failures are misses.
    pub async fn fetch_from_disk(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<(Bytes, usize)> {
        let started = Instant::now();
        let metrics = &self.disk.metrics;
        let shard = &self.disk.shards[self.disk.shard_index_for_key(key)];
        // Do not wait on a busy buffer; flushing chunks may miss until disk publication.
        if let Ok(pending) = shard.buffered_small_entry_chunk.try_lock()
            && let Some(bytes) = pending.get(key, requested)
        {
            metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
            return Some((bytes, requested.len() as usize));
        }
        let (read, in_flight_read) = {
            let mut index = shard.entry_index.lock().unwrap();
            let Some(entry) = index.covering_entry(key, requested) else {
                metrics.record_lookup(DiskLookupOutcome::Absent, started.elapsed());
                return None;
            };
            let in_flight_read = entry.in_flight_read.upgrade().unwrap_or_else(|| {
                let result = Arc::new(OnceCell::new());
                entry.in_flight_read = Arc::downgrade(&result);
                result
            });
            (
                PayloadRead {
                    object_range: entry.object_range,
                    payload_checksum: entry.payload_checksum,
                    payload_range: entry.payload_range.clone(),
                },
                in_flight_read,
            )
        };
        // If the initializing caller is canceled, OnceCell lets a waiter take over.
        let result = in_flight_read
            .get_or_init(|| async {
                let result = read.read_and_verify_payload(&self.disk.file).await;
                if result.is_err() {
                    let mut index = shard.entry_index.lock().unwrap();
                    if let Some(entry) = index.remove_entry_matching_read(key, &read) {
                        shard.release_payload_and_allow_metadata_overwrite(entry);
                    }
                }
                result
            })
            .await;
        match result {
            Ok((bytes, buffer_capacity)) => {
                let start = (requested.start() - read.object_range.start()) as usize;
                let bytes = bytes.slice(start..start + requested.len() as usize);
                let (bytes, buffer_capacity) = shrink_disk_result(&self.disk.buffer_pool, bytes, *buffer_capacity);
                metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
                Some((bytes, buffer_capacity))
            }
            Err(outcome) => {
                metrics.record_lookup(*outcome, started.elapsed());
                None
            }
        }
    }
}

/// Copies a disk result only if the destination has smaller backing capacity.
/// Allocation failure leaves the verified result and its charge unchanged.
fn shrink_disk_result(pool: &Arc<BufferPool>, bytes: Bytes, capacity: usize) -> (Bytes, usize) {
    if BufferPool::allocation_capacity(bytes.len()) >= capacity {
        return (bytes, capacity);
    }
    let Ok(mut buffer) = pool.allocate(bytes.len()) else {
        return (bytes, capacity);
    };
    buffer.as_mut_slice().copy_from_slice(&bytes);
    let capacity = buffer.capacity();
    (buffer.into_bytes(), capacity)
}

impl DiskCacheShard {
    /// Samples up to `reclaim_sample_size` entries and evicts at most one that fits the remaining chunk budget.
    /// Returns whether sampling occurred, even if no entry was evicted.
    /// Consumes one attempt when sampling; removed entries consume their chunk count. Neighbors are not evicted.
    /// The allocator releases removed payloads without waiting for metadata writes or readers.
    fn sample_and_evict_entry(
        &self,
        access_histories: &ObjectAccessHistories,
        remaining_eviction_attempts: &mut usize,
        remaining_eviction_chunk_budget: &mut usize,
    ) -> bool {
        if *remaining_eviction_attempts == 0 || *remaining_eviction_chunk_budget == 0 {
            return false;
        }
        let mut index = self.entry_index.lock().unwrap();
        let candidate_count = index.eviction_candidates.len();
        if candidate_count == 0 {
            return false;
        }
        *remaining_eviction_attempts -= 1;
        let (sample_start, sample_count) =
            sample_candidates(&mut index.next_sample_start, candidate_count, self.reclaim_sample_size);
        let selected_candidate = (0..sample_count)
            .filter_map(|sample_offset| {
                let position = (sample_start + sample_offset) % candidate_count;
                let (key, range_start) = &index.eviction_candidates[position];
                let entry = &index.entries_by_key[key][range_start];
                (entry.chunk_count() as usize <= *remaining_eviction_chunk_budget).then(|| {
                    let cost = access_histories.decayed_retrieval_cost(key, entry.object_range);
                    (position, cost, entry.object_range.len())
                })
            })
            .min_by(|(_, left_cost, left_bytes), (_, right_cost, right_bytes)| {
                compare_cost_per_byte(*left_cost, *left_bytes, *right_cost, *right_bytes)
            });
        if let Some((position, ..)) = selected_candidate {
            let (key, start) = index.eviction_candidates[position];
            *remaining_eviction_chunk_budget -= index.entries_by_key[&key][&start].chunk_count() as usize;
            self.release_payload_and_allow_metadata_overwrite(index.remove_entry(&key, start).unwrap());
        }
        true
    }
}

impl DiskEntry {
    fn chunk_count(&self) -> u64 {
        self.payload_range.end.div_ceil(CHUNK_BYTES) - self.payload_range.start / CHUNK_BYTES
    }
}

impl DiskCacheShard {
    /// Releases the payload's use of its chunks and allows its metadata to be overwritten.
    /// Neither index removal nor entry destruction has side effects on disk ownership or metadata.
    fn release_payload_and_allow_metadata_overwrite(&self, entry: DiskEntry) {
        self.allocator.release_payload(entry.payload_range.start);
        self.metadata_pages
            .lock()
            .unwrap()
            .free_entry_positions
            .push(entry.metadata);
    }
}

impl DiskEntryIndex {
    fn new(metrics: Arc<DiskMetrics>) -> Self {
        Self {
            entries_by_key: FxHashMap::default(),
            eviction_candidates: Vec::new(),
            next_sample_start: 0,
            metrics,
        }
    }

    /// Inserts one range and returns entries displaced or rejected by containment.
    fn insert(&mut self, key: ObjectKeyHash, mut entry: DiskEntry) -> Vec<DiskEntry> {
        let object_range = entry.object_range;
        if self.covering_entry(&key, object_range).is_some() {
            return vec![entry];
        }
        let mut removed = Vec::new();
        // Entry ranges have increasing ends, so stop at the first range not contained.
        while let Some((&start, existing)) = self
            .entries_by_key
            .get(&key)
            .and_then(|entries| entries.range(object_range.start()..).next())
            && object_range.contains(existing.object_range)
        {
            removed.push(self.remove_entry(&key, start).unwrap());
        }
        entry.eviction_position = self.eviction_candidates.len();
        self.metrics.entries.increase(1);
        self.metrics.payload_bytes.increase(object_range.len());
        self.eviction_candidates.push((key, object_range.start()));
        self.entries_by_key
            .entry(key)
            .or_default()
            .insert(object_range.start(), entry);
        removed
    }

    /// Removes one entry from the index and eviction candidates without releasing its disk space.
    fn remove_entry(&mut self, key: &ObjectKeyHash, start: u64) -> Option<DiskEntry> {
        let entries = self.entries_by_key.get_mut(key)?;
        let entry = entries.remove(&start)?;
        self.metrics.entries.decrease(1);
        self.metrics.payload_bytes.decrease(entry.object_range.len());
        if entries.is_empty() {
            self.entries_by_key.remove(key);
        }
        self.eviction_candidates.swap_remove(entry.eviction_position);
        if let Some((moved_key, moved_start)) = self.eviction_candidates.get(entry.eviction_position) {
            self.entries_by_key
                .get_mut(moved_key)
                .unwrap()
                .get_mut(moved_start)
                .unwrap()
                .eviction_position = entry.eviction_position;
        }
        Some(entry)
    }

    /// Removes the indexed entry matching the failed read's expected start and checksum.
    fn remove_entry_matching_read(&mut self, key: &ObjectKeyHash, read: &PayloadRead) -> Option<DiskEntry> {
        let start = read.object_range.start();
        // Preserve different contents. Discarding a newer identical copy is an acceptable miss.
        let entry = self.entries_by_key.get(key)?.get(&start)?;
        if entry.payload_checksum != read.payload_checksum {
            return None;
        }
        self.remove_entry(key, start)
    }

    /// Finds the entry whose byte range covers the entire request.
    fn covering_entry(&mut self, key: &ObjectKeyHash, requested: ByteRange) -> Option<&mut DiskEntry> {
        // Entry ranges never contain one another, so their ends increase with their starts.
        let entries = self.entries_by_key.get_mut(key)?;
        let (_, entry) = entries.range_mut(..=requested.start()).next_back()?;
        entry.object_range.contains(requested).then_some(entry)
    }
}

impl Drop for DiskEntryIndex {
    fn drop(&mut self) {
        self.metrics.entries.decrease(self.eviction_candidates.len() as u64);
        self.metrics.payload_bytes.decrease(
            self.entries_by_key
                .values()
                .flat_map(|entries| entries.values())
                .map(|entry| entry.object_range.len())
                .sum(),
        );
    }
}

impl DiskCacheInner {
    fn covers_range(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
        let shard = &self.shards[self.shard_index_for_key(key)];
        shard
            .buffered_small_entry_chunk
            .try_lock()
            .is_ok_and(|pending| pending.get(key, range).is_some())
            || shard.entry_index.lock().unwrap().covering_entry(key, range).is_some()
    }

    fn shard_index_for_key(&self, key: &ObjectKeyHash) -> usize {
        (key.0 % self.shards.len() as u128) as usize
    }
}

impl PayloadRead {
    /// Reads the whole entry payload and verifies its checksum.
    async fn read_and_verify_payload(&self, file: &DataFile) -> Result<(Bytes, usize), DiskLookupOutcome> {
        let (bytes, buffer_capacity) = file
            .read_payload(self.payload_range.clone(), self.object_range.len() as usize)
            .await
            .map_err(|error| {
                tracing::warn!(target: "feuer::storage", %error, "disk read failed; entry invalidated");
                DiskLookupOutcome::IoError
            })?;
        if XxHash64::oneshot(0, &bytes) != self.payload_checksum {
            tracing::warn!(target: "feuer::storage", "disk checksum failed; entry invalidated");
            return Err(DiskLookupOutcome::ChecksumFailed);
        }
        Ok((bytes, buffer_capacity))
    }
}
