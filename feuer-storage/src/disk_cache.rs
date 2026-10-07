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
    path::Path,
    sync::{Arc, Mutex, Weak},
    time::Instant,
};

use bytes::Bytes;
use feuer_memory::BufferPool;
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
    file::payload_disk_bytes,
};
#[cfg(test)]
use page_format::METADATA_PAGE_BYTES;

// Per admission: limit sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_CHUNKS: usize = 4096;

/// Caches object bytes on disk, looked up by object key and byte range.
///
/// Background writes use a nonblocking 512-entry queue. Partial small-entry buffers flush every 60 seconds.
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

/// Shared disk-cache internals: file, shards, access histories, and metrics.
struct DiskCacheInner {
    file: DataFile,
    shards: Box<[DiskCacheShard]>,
    access_histories: Arc<ObjectAccessHistories>,
    metrics: Arc<DiskMetrics>,
    reclaim_sample_size: usize,
}

/// One shard's small-entry buffer, allocator, published entry index, and metadata pages.
struct DiskCacheShard {
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

/// One entry's object range, payload disk address and checksum, metadata, eviction position, and shared read.
struct DiskEntry {
    // In-flight deduplication: concurrent readers share one disk read and checksum verification.
    // Only concurrent callers retain the result; the index must not cache payload bytes.
    in_flight_read: Weak<PayloadRead>,
    eviction_position: usize,
    object_range: ByteRange,
    payload_checksum: u64,
    payload_address: u64,
    /// Metadata chunk and entry metadata indexes identifying this entry's 48-byte disk metadata.
    metadata: (usize, usize),
}

/// One payload read's disk address, object range, checksum, and result shared by concurrent readers.
struct PayloadRead {
    object_range: ByteRange,
    payload_checksum: u64,
    payload_address: u64,
    result: OnceCell<Result<(Bytes, usize), DiskLookupOutcome>>,
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
        let file = DataFile::open_with_buffer_pool(directory, capacity, io_metrics, buffer_pool).await?;
        let num_shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        let shards = (0..num_shards)
            .map(|shard_index| {
                let range = recovery::shard_disk_range(capacity, num_shards, shard_index);
                let allocator = DiskChunkAllocator::with_metrics(range, metrics.clone());
                Ok(DiskCacheShard {
                    allocator,
                    entry_index: Mutex::new(DiskEntryIndex::new(metrics.clone())),
                    buffered_small_entry_chunk: tokio::sync::Mutex::new(BufferedSmallEntryChunk::new()?),
                    metadata_pages: Mutex::new(metadata::MetadataPages::default()),
                })
            })
            .collect::<Result<_, DataFileError>>()?;
        let disk = Arc::new(DiskCacheInner {
            file,
            shards,
            access_histories,
            metrics,
            reclaim_sample_size,
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
        let disk = self.disk.clone();
        downloads.sort_unstable_by_key(|(_, download)| download.bytes().len());
        tokio::spawn(async move {
            let mut published = 0;
            for (key, download) in downloads {
                published += disk.write(key, download, ()).await?;
            }
            Ok(published + disk.flush().await?)
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
        self.disk.shards[self.disk.shard_index_for_key(key)].covers_range(key, range)
    }

    /// Returns exactly requested bytes from one covering entry, or a miss on any I/O/integrity uncertainty.
    /// Concurrent reads of one stored entry share whole-entry I/O and checksum verification.
    /// Reads no neighboring entries or metadata. Results retain no disk ownership.
    pub async fn get(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<Bytes> {
        self.fetch_from_disk(key, requested).await.map(|(bytes, _)| bytes)
    }

    /// Copies buffered bytes, or reads disk after verifying the whole covering entry's checksum.
    /// Copies a disk-read slice only if it saves at least 25% of backing capacity.
    /// Allocation failure keeps the original slice.
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
        let read = {
            let mut disk_index = shard.entry_index.lock().unwrap();
            let Some(entry) = disk_index.covering_entry(key, requested) else {
                drop(disk_index);
                metrics.record_lookup(DiskLookupOutcome::Absent, started.elapsed());
                return None;
            };
            entry.share_payload_read()
        };
        // If the initializing caller is canceled, OnceCell lets a waiter take over.
        let result = read
            .result
            .get_or_init(|| async {
                let result = read.read_and_verify_payload(&self.disk.file).await;
                if matches!(result, Err(DiskLookupOutcome::ChecksumFailed)) {
                    shard.remove_entry(key, read.object_range.start());
                }
                result
            })
            .await;
        match result {
            Ok((bytes, capacity)) => {
                let start = (requested.start() - read.object_range.start()) as usize;
                let bytes = bytes.slice(start..start + requested.len() as usize);
                let (bytes, buffer_capacity) = self.disk.file.shrink_read_buffer(bytes, *capacity);
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

impl DiskCacheShard {
    /// Checks for a covering buffered or disk entry without copying payload bytes.
    fn covers_range(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
        self.buffered_small_entry_chunk
            .try_lock()
            .is_ok_and(|pending| pending.covering_entry(key, range).is_some())
            || self.entry_index.lock().unwrap().covering_entry(key, range).is_some()
    }

    /// Samples up to `reclaim_sample_size` entries and evicts at most one that fits the remaining chunk budget.
    /// Returns whether sampling occurred, even if no entry was evicted.
    /// Consumes one attempt when sampling; removed entries consume their chunk count. Neighbors are not evicted.
    /// The allocator releases removed payloads without waiting for metadata writes or readers.
    fn sample_and_evict_entry(
        &self,
        disk: &DiskCacheInner,
        remaining_eviction_attempts: &mut usize,
        remaining_eviction_chunk_budget: &mut usize,
    ) -> bool {
        if *remaining_eviction_attempts == 0 || *remaining_eviction_chunk_budget == 0 {
            return false;
        }
        let mut disk_index = self.entry_index.lock().unwrap();
        if disk_index.eviction_candidates.is_empty() {
            return false;
        }
        *remaining_eviction_attempts -= 1;
        let disk_index = &mut *disk_index;
        let selected_candidate = sample_candidates(
            &mut disk_index.next_sample_start,
            &disk_index.eviction_candidates,
            disk.reclaim_sample_size,
        )
        .filter_map(|candidate @ (key, range_start)| {
            let entry = &disk_index.entries_by_key[key][range_start];
            let chunk_count = entry.chunk_count() as usize;
            (chunk_count <= *remaining_eviction_chunk_budget).then(|| {
                let cost = disk.access_histories.decayed_retrieval_cost(key, entry.object_range);
                (candidate, cost, entry.object_range.len(), chunk_count)
            })
        })
        .min_by(|(_, left_cost, left_bytes, _), (_, right_cost, right_bytes, _)| {
            compare_cost_per_byte(*left_cost, *left_bytes, *right_cost, *right_bytes)
        });
        if let Some((&(key, start), _, _, chunk_count)) = selected_candidate {
            *remaining_eviction_chunk_budget -= chunk_count;
            self.remove_entry_from_index(disk_index, &key, start);
        }
        true
    }
}

impl DiskEntry {
    /// Shares the payload read, creating one from this entry's metadata when no readers remain.
    fn share_payload_read(&mut self) -> Arc<PayloadRead> {
        self.in_flight_read.upgrade().unwrap_or_else(|| {
            let read = Arc::new(PayloadRead {
                object_range: self.object_range,
                payload_checksum: self.payload_checksum,
                payload_address: self.payload_address,
                result: OnceCell::new(),
            });
            self.in_flight_read = Arc::downgrade(&read);
            read
        })
    }

    fn chunk_count(&self) -> u64 {
        // Small payloads stay within one chunk; larger payloads start at a chunk boundary.
        self.object_range.len().max(1).div_ceil(CHUNK_BYTES)
    }
}

impl DiskCacheShard {
    /// Locks the disk index, removes one entry, releases its payload, and allows its metadata to be overwritten.
    fn remove_entry(&self, key: &ObjectKeyHash, start: u64) {
        self.remove_entry_from_index(&mut self.entry_index.lock().unwrap(), key, start);
    }

    /// Removes and releases one entry while the caller holds the disk-index lock.
    fn remove_entry_from_index(&self, disk_index: &mut DiskEntryIndex, key: &ObjectKeyHash, start: u64) {
        let Some(entry) = disk_index.take_entry(key, start) else {
            return;
        };
        self.release_entry(entry, &mut self.metadata_pages.lock().unwrap());
    }

    /// Releases one held payload and allows its metadata slot to be overwritten.
    fn release_entry(&self, entry: DiskEntry, metadata: &mut metadata::MetadataPages) {
        self.allocator.release_payload(entry.payload_address);
        metadata.free_entry_positions.push(entry.metadata);
    }

    /// Inserts an entry not already covered and releases entries it contains.
    /// The caller holds both locks and has reserved the payload and metadata slot.
    fn insert_entry(
        &self,
        disk_index: &mut DiskEntryIndex,
        metadata: &mut metadata::MetadataPages,
        key: ObjectKeyHash,
        mut entry: DiskEntry,
    ) {
        let object_range = entry.object_range;
        // Entry ranges have increasing ends, so stop at the first range not contained.
        while let Some((&start, existing)) = disk_index
            .entries_by_key
            .get(&key)
            .and_then(|entries| entries.range(object_range.start()..).next())
            && object_range.contains(existing.object_range)
        {
            self.release_entry(disk_index.take_entry(&key, start).unwrap(), metadata);
        }
        entry.eviction_position = disk_index.eviction_candidates.len();
        disk_index.metrics.entries.increase(1);
        disk_index.metrics.payload_bytes.increase(object_range.len());
        disk_index.eviction_candidates.push((key, object_range.start()));
        disk_index
            .entries_by_key
            .entry(key)
            .or_default()
            .insert(object_range.start(), entry);
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

    /// Takes one entry out of the index and eviction candidates without releasing its payload or metadata slot.
    fn take_entry(&mut self, key: &ObjectKeyHash, start: u64) -> Option<DiskEntry> {
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
    fn shard_index_for_key(&self, key: &ObjectKeyHash) -> usize {
        (key.0 % self.shards.len() as u128) as usize
    }
}

impl PayloadRead {
    /// Reads the whole entry payload and verifies its checksum.
    async fn read_and_verify_payload(&self, file: &DataFile) -> Result<(Bytes, usize), DiskLookupOutcome> {
        let (bytes, capacity) = file
            .read_payload(self.payload_address, self.object_range.len() as usize)
            .await
            .map_err(|error| {
                tracing::warn!(target: "feuer::storage", %error, "disk read failed");
                DiskLookupOutcome::IoError
            })?;
        if XxHash64::oneshot(0, &bytes) != self.payload_checksum {
            tracing::warn!(target: "feuer::storage", "disk checksum failed; entry invalidated");
            return Err(DiskLookupOutcome::ChecksumFailed);
        }
        Ok((bytes, capacity))
    }
}
