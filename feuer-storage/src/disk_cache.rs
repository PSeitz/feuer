//! Cache objects on disk by object key.

#[cfg(test)]
mod adversarial_tests;
#[cfg(test)]
mod fallthrough_tests;
mod header;
mod metadata;
mod small_entry_buffer;
mod write_queue;
mod writer;
use small_entry_buffer::SmallEntryBuffer;
use write_queue::PendingDiskWrite;
#[cfg(test)]
mod metrics_tests;
mod page_format;
mod recovery;
#[cfg(test)]
mod tests;

use std::{
    fmt,
    path::Path,
    sync::{Arc, Mutex, Weak},
    time::Instant,
};

use bytes::Bytes;
use feuer_historian::ObjectAccessHistories;
use feuer_memory::{
    BufferPool,
    retention::{AvoidedRetrievalCostScorer, RECLAIM_SAMPLE_SIZE, RetentionScorer, sample_candidates},
};
use feuer_types::ObjectKeyHash;
#[cfg(test)]
use page_format::METADATA_PAGE_BYTES;
use rustc_hash::FxHashMap;
use tokio::{
    runtime::Handle,
    sync::{Mutex as AsyncMutex, OnceCell, mpsc},
    task::{JoinError, JoinSet},
};
use twox_hash::XxHash64;

use crate::{
    DataFile, DataFileError, DataFileResult, DiskMetrics, IoMetrics, IoQueues,
    allocation::{CHUNK_BYTES, DiskChunkAllocator, ReservedChunks},
    disk_metrics::{DiskLookupOutcome, DiskWriteAttempt, DiskWriteOutcome},
    file::payload_disk_bytes,
};

// Per admission: limit sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_CHUNKS: usize = 4096;

/// Caches objects on disk by object key.
///
/// Background writes use a nonblocking 512-entry queue. Partial small-entry buffers flush every 60 seconds.
/// Closing finishes queued writes and discards remaining partial chunks.
/// Small entries share immutable 1-MiB payload chunks. Separate mutable metadata chunks
/// hold entry metadata and links between metadata chunks. Each shard's chain starts at its first chunk.
/// Entries have plain payload bytes, 4-KiB-aligned storage, and a checksum in their entry metadata.
/// Payload reuse requires a wholly free chunk.
/// The key hash selects an independently allocated shard. Admission may fail despite space elsewhere.
///
/// Pressure eviction uses sampled retention scores, defaulting to retrieval value per payload byte.
/// The first chunk holds a global header with the application's payload version and reserved space.
/// Opening clears incompatible files, then waits for all shards to recover before making the cache available.
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
    retention_scorer: Arc<dyn RetentionScorer>,
    metrics: Arc<DiskMetrics>,
    reclaim_sample_size: usize,
}

/// A shard's small-entry buffer, allocator, published entry index, and metadata pages.
struct DiskCacheShard {
    allocator: DiskChunkAllocator,
    entry_index: Mutex<DiskEntryIndex>,
    metadata_pages: Mutex<metadata::MetadataChunks>,
    small_entry_buffer: AsyncMutex<SmallEntryBuffer>,
}

/// Disk entries indexed by object key, with a list sampled in rotation for eviction.
struct DiskEntryIndex {
    entries_by_key: FxHashMap<ObjectKeyHash, DiskEntry>,
    eviction_candidates: Vec<ObjectKeyHash>,
    /// Index where the next eviction sample starts.
    next_sample_start: usize,
    metrics: Arc<DiskMetrics>,
}

/// An entry's payload length, address, checksum, shared read, metadata, and eviction position.
struct DiskEntry {
    // Weak references share reads without keeping payload bytes alive after callers finish.
    shared_read: Weak<PayloadRead>,
    eviction_position: usize,
    payload_len: u64,
    payload_checksum: u64,
    payload_address: u64,
    metadata: metadata::EntryMetadataLocation,
}

/// A payload read's disk address, length, checksum, and result shared by concurrent readers.
struct PayloadRead {
    payload_len: u64,
    payload_checksum: u64,
    payload_address: u64,
    result: OnceCell<Result<(Bytes, usize), DiskLookupOutcome>>,
}

/// An error opening or writing the disk cache. Read uncertainty becomes a miss.
#[derive(Debug, thiserror::Error)]
pub enum DiskCacheError {
    /// The requested disk capacity is unsupported.
    #[error("disk cache capacity must be at least 2 MiB and at most i64::MAX")]
    InvalidCapacity,
    /// Raw storage failed.
    #[error(transparent)]
    DataFile(#[from] DataFileError),
    /// The task performing a disk write failed. Submitted I/O may still finish.
    #[error("disk write task failed: {0}")]
    WriteTaskFailed(#[source] JoinError),
}

impl fmt::Debug for DiskCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiskCache")
            .field("capacity", &self.disk.file.capacity())
            .field("shards", &self.disk.shards.len())
            .finish_non_exhaustive()
    }
}

/// Optional settings and shared resources for opening a disk cache.
/// Defaults use no-op metrics, private history and queues, no buffer pooling, and retrieval-cost scoring.
pub struct DiskCacheOptions {
    /// Backing filename within the cache directory. Defaults to `data`.
    /// Must be a single filename, without NUL bytes or the reserved `.feuer.` prefix.
    pub file_name: String,
    /// Application payload format version, independent of Feuer's disk format. Defaults to zero.
    /// Any mismatch on open clears previous disk entries, including when changing to a lower version.
    pub payload_version: u64,
    /// Raw file I/O metrics.
    pub io_metrics: Arc<IoMetrics>,
    /// Disk-cache lookup, write, and capacity metrics.
    pub metrics: Arc<DiskMetrics>,
    /// Request history consulted for retention.
    /// Insertion and `get` do not record requests.
    /// Public request handling records once before lookup.
    pub access_histories: Arc<ObjectAccessHistories>,
    /// Maximum candidates inspected per eviction decision. Must be positive.
    pub reclaim_sample_size: usize,
    /// Aligned buffer pool for reads, normally shared with the memory cache.
    pub buffer_pool: Arc<BufferPool>,
    /// I/O queues shared with other caches. `None` creates dedicated queues.
    pub io_queues: Option<IoQueues>,
    /// Retention scorer, also applied to recovered entries.
    pub retention_scorer: Arc<dyn RetentionScorer>,
}

impl Default for DiskCacheOptions {
    fn default() -> Self {
        Self {
            file_name: "data".to_owned(),
            payload_version: 0,
            io_metrics: IoMetrics::noop(),
            metrics: DiskMetrics::noop(),
            access_histories: Arc::new(ObjectAccessHistories::new()),
            reclaim_sample_size: RECLAIM_SAMPLE_SIZE,
            buffer_pool: BufferPool::unpooled(),
            io_queues: None,
            retention_scorer: Arc::new(AvoidedRetrievalCostScorer::default()),
        }
    }
}

impl DiskCache {
    /// Opens an exclusively locked, fixed-capacity file after scanning every shard's metadata.
    pub async fn open(
        directory: impl AsRef<Path>,
        capacity: u64,
        options: DiskCacheOptions,
    ) -> Result<Self, DiskCacheError> {
        let DiskCacheOptions {
            file_name,
            payload_version,
            io_metrics,
            metrics,
            access_histories,
            reclaim_sample_size,
            buffer_pool,
            io_queues,
            retention_scorer,
        } = options;
        assert!(reclaim_sample_size > 0, "reclaim sample size must be greater than zero");
        if capacity < 2 * CHUNK_BYTES || capacity > i64::MAX as u64 {
            return Err(DiskCacheError::InvalidCapacity);
        }
        let capacity = capacity / CHUNK_BYTES * CHUNK_BYTES;
        let file =
            DataFile::open_with_io_queues(directory, &file_name, capacity, io_metrics, buffer_pool, io_queues).await?;
        let num_shards = ((capacity - CHUNK_BYTES) / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        header::clear_incompatible_metadata(&file, payload_version, num_shards).await?;
        let shards = (0..num_shards)
            .map(|shard_index| {
                let range = recovery::shard_disk_range(capacity, num_shards, shard_index);
                let allocator = DiskChunkAllocator::with_metrics(range, metrics.clone());
                Ok(DiskCacheShard {
                    allocator,
                    entry_index: Mutex::new(DiskEntryIndex::new(metrics.clone())),
                    small_entry_buffer: AsyncMutex::new(SmallEntryBuffer::new()?),
                    metadata_pages: Mutex::new(metadata::MetadataChunks::default()),
                })
            })
            .collect::<Result<_, DataFileError>>()?;
        let disk = Arc::new(DiskCacheInner {
            file,
            shards,
            access_histories,
            retention_scorer,
            metrics,
            reclaim_sample_size,
        });
        let started = Instant::now();
        tracing::info!(target: "feuer::storage", "starting disk cache recovery");
        let mut recovery_tasks = JoinSet::new();
        for index in 0..disk.shards.len() {
            let disk = disk.clone();
            // Poll recovery on a blocking thread, including metadata parsing and index rebuilding.
            // Its io_uring reads still use the runtime for asynchronous completion.
            recovery_tasks.spawn_blocking(move || Handle::current().block_on(disk.recover_shard(index)));
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
    pub async fn insert_batch(&self, mut entries: Vec<(ObjectKeyHash, Bytes)>) -> Result<usize, DiskCacheError> {
        let disk = self.disk.clone();
        entries.sort_unstable_by_key(|(_, bytes)| bytes.len());
        tokio::spawn(async move {
            let mut published = 0;
            for (key, bytes) in entries {
                published += disk.write(key, bytes, ()).await?;
            }
            Ok(published + disk.flush().await?)
        })
        .await
        .map_err(DiskCacheError::WriteTaskFailed)?
    }

    /// Buffers small entries. Full chunks, chunks with no room for the next entry,
    /// and larger entries write immediately.
    /// Holds completion state until publication or discard. Cancellation may leave submitted I/O running.
    pub async fn write(
        &self,
        key: ObjectKeyHash,
        bytes: Bytes,
        completion: impl Send + 'static,
    ) -> Result<usize, DiskCacheError> {
        self.disk.write(key, bytes, completion).await
    }

    /// Shared history for recording requests once, before lookup and outside raw storage operations.
    pub fn access_histories(&self) -> Arc<ObjectAccessHistories> {
        self.disk.access_histories.clone()
    }

    /// Checks for a buffered or disk entry, without disk I/O or recording a request.
    pub fn contains(&self, key: &ObjectKeyHash) -> bool {
        self.disk.shards[self.disk.shard_index_for_key(key)].contains(key)
    }

    /// Returns the object, or a miss on I/O/integrity uncertainty.
    /// Concurrent callers share reads. Results retain no disk ownership.
    pub async fn get(&self, key: &ObjectKeyHash) -> Option<Bytes> {
        self.get_with_fallthrough(key, || false).await.map(|(bytes, _)| bytes)
    }

    /// Like [`Self::get`], but returns bytes and backing capacity and allows waiting reads to fall through.
    /// At 64 payload reads waiting for a permit, every unchecked waiter is scheduled to call its own
    /// `should_fall_through`, even if other fallthroughs subsequently relieve the pressure.
    /// Each callback runs at most once, in its caller's task and without holding a cache lock.
    /// Buffered hits, absent entries, admitted reads, and shared-read waiters skip the check.
    /// A waiter taking over after cancellation or fallthrough may be checked while waiting for a permit.
    /// Returning true produces `None` without reading or removing the entry. The decision is
    /// local to this lookup and is not shared with other callers waiting for the same read.
    pub async fn get_with_fallthrough(
        &self,
        key: &ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool,
    ) -> Option<(Bytes, usize)> {
        let started = Instant::now();
        let metrics = &self.disk.metrics;
        let shard = &self.disk.shards[self.disk.shard_index_for_key(key)];
        // Check the buffer only when unlocked. Flushing entries may miss until disk publication.
        if let Ok(buffer) = shard.small_entry_buffer.try_lock()
            && let Some(bytes) = buffer.get(key)
        {
            metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
            let capacity = bytes.len();
            return Some((bytes, capacity));
        }
        let read = {
            let mut disk_index = shard.entry_index.lock().unwrap();
            let Some(entry) = disk_index.entries_by_key.get_mut(key) else {
                drop(disk_index);
                metrics.record_lookup(DiskLookupOutcome::Absent, started.elapsed());
                return None;
            };
            entry.share_read()
        };
        // Cancellation or fallthrough leaves the shared result unset so another caller can read.
        let Ok(result) = read
            .result
            .get_or_try_init(|| async {
                let Some(_read_permit) = self.disk.file.acquire_payload_read_permit(should_fall_through).await else {
                    return Err(());
                };
                let result = read.read_and_verify_payload(&self.disk.file).await;
                if matches!(result, Err(DiskLookupOutcome::ChecksumFailed)) {
                    let mut disk_index = shard.entry_index.lock().unwrap();
                    // Compare read identity so a late checksum failure cannot remove a replacement entry.
                    if disk_index
                        .entries_by_key
                        .get(key)
                        .is_some_and(|entry| entry.shared_read.ptr_eq(&Arc::downgrade(&read)))
                    {
                        shard.remove_entry_from_index(&mut disk_index, key);
                    }
                }
                Ok(result)
            })
            .await
        else {
            metrics.record_lookup(DiskLookupOutcome::Fallthrough, started.elapsed());
            return None;
        };
        match result {
            Ok((bytes, capacity)) => {
                metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
                Some((bytes.clone(), *capacity))
            }
            Err(outcome) => {
                metrics.record_lookup(*outcome, started.elapsed());
                None
            }
        }
    }
}

impl DiskCacheShard {
    /// Checks for a buffered or disk entry without copying payload bytes.
    fn contains(&self, key: &ObjectKeyHash) -> bool {
        self.small_entry_buffer
            .try_lock()
            .is_ok_and(|buffer| buffer.entry(key).is_some())
            || self.entry_index.lock().unwrap().entries_by_key.contains_key(key)
    }

    /// Samples up to `reclaim_sample_size` entries and evicts at most one that fits the remaining chunk budget.
    /// Returns whether sampling occurred, even if no entry was evicted.
    /// Sampling consumes one attempt. Removed entries consume their chunk count. Neighbors are not evicted.
    /// The allocator releases removed payloads without waiting for metadata writes or readers.
    fn sample_and_evict_entry(
        &self,
        disk: &DiskCacheInner,
        remaining_eviction_attempts: &mut usize,
        remaining_eviction_chunks: &mut usize,
    ) -> bool {
        if *remaining_eviction_attempts == 0 || *remaining_eviction_chunks == 0 {
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
        .filter_map(|key| {
            let entry = &disk_index.entries_by_key[key];
            let chunk_count = entry.chunk_count() as usize;
            (chunk_count <= *remaining_eviction_chunks).then(|| {
                let score =
                    disk.retention_scorer
                        .score(key, entry.payload_len, entry.payload_len, &disk.access_histories);
                (key, score, chunk_count)
            })
        })
        .min_by(|(_, left_score, _), (_, right_score, _)| left_score.total_cmp(right_score));
        if let Some((&key, _, chunk_count)) = selected_candidate {
            *remaining_eviction_chunks -= chunk_count;
            self.remove_entry_from_index(disk_index, &key);
        }
        true
    }
}

impl DiskEntry {
    /// An entry without readers, described by its metadata at `location`.
    fn new(entry_metadata: &metadata::EntryMetadata, location: metadata::EntryMetadataLocation) -> Self {
        Self {
            shared_read: Weak::new(),
            eviction_position: 0,
            payload_len: entry_metadata.payload_len,
            payload_checksum: entry_metadata.payload_checksum,
            payload_address: entry_metadata.payload_address,
            metadata: location,
        }
    }

    /// Shares a read still held by a caller, or creates one. The caller starts I/O through its result cell.
    fn share_read(&mut self) -> Arc<PayloadRead> {
        self.shared_read.upgrade().unwrap_or_else(|| {
            let read = Arc::new(PayloadRead {
                payload_len: self.payload_len,
                payload_checksum: self.payload_checksum,
                payload_address: self.payload_address,
                result: OnceCell::new(),
            });
            self.shared_read = Arc::downgrade(&read);
            read
        })
    }

    fn chunk_count(&self) -> u64 {
        // Small payloads stay within one chunk. Larger payloads start at a chunk boundary.
        self.payload_len.max(1).div_ceil(CHUNK_BYTES)
    }
}

impl DiskCacheShard {
    /// Locks the disk index, removes an entry, releases its payload, and frees its metadata position.
    #[cfg(test)]
    fn remove_entry(&self, key: &ObjectKeyHash) {
        self.remove_entry_from_index(&mut self.entry_index.lock().unwrap(), key);
    }

    /// Removes and releases an entry while the caller holds the disk-index lock.
    fn remove_entry_from_index(&self, disk_index: &mut DiskEntryIndex, key: &ObjectKeyHash) {
        let Some(entry) = disk_index.take_entry(key) else {
            return;
        };
        self.release_entry(entry, &mut self.metadata_pages.lock().unwrap());
    }

    /// Releases a held payload and frees its metadata position.
    fn release_entry(&self, entry: DiskEntry, metadata: &mut metadata::MetadataChunks) {
        self.allocator.release_payload(entry.payload_address);
        metadata.free_entry_metadata(entry.metadata);
    }

    /// Inserts an entry and releases any previous entry for its key.
    /// The caller holds both locks, has reserved the payload and metadata position,
    /// and records inserted entry counts and payload bytes once its batch is complete.
    fn insert_entry(
        &self,
        disk_index: &mut DiskEntryIndex,
        metadata: &mut metadata::MetadataChunks,
        key: ObjectKeyHash,
        mut entry: DiskEntry,
    ) {
        if let Some(previous) = disk_index.take_entry(&key) {
            self.release_entry(previous, metadata);
        }
        entry.eviction_position = disk_index.eviction_candidates.len();
        disk_index.eviction_candidates.push(key);
        disk_index.entries_by_key.insert(key, entry);
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

    /// Removes an entry from the index and eviction candidates. Its payload and metadata remain reserved.
    fn take_entry(&mut self, key: &ObjectKeyHash) -> Option<DiskEntry> {
        let entry = self.entries_by_key.remove(key)?;
        self.metrics.entries.decrease(1);
        self.metrics.payload_bytes.decrease(entry.payload_len);
        self.eviction_candidates.swap_remove(entry.eviction_position);
        if let Some(moved_key) = self.eviction_candidates.get(entry.eviction_position) {
            self.entries_by_key.get_mut(moved_key).unwrap().eviction_position = entry.eviction_position;
        }
        Some(entry)
    }
}

impl Drop for DiskEntryIndex {
    fn drop(&mut self) {
        self.metrics.entries.decrease(self.eviction_candidates.len() as u64);
        self.metrics
            .payload_bytes
            .decrease(self.entries_by_key.values().map(|entry| entry.payload_len).sum());
    }
}

impl DiskCacheInner {
    fn shard_index_for_key(&self, key: &ObjectKeyHash) -> usize {
        (key.0 % self.shards.len() as u128) as usize
    }
}

impl PayloadRead {
    /// Reads the payload and verifies its checksum.
    async fn read_and_verify_payload(&self, file: &DataFile) -> Result<(Bytes, usize), DiskLookupOutcome> {
        let (bytes, capacity) = file
            .read_payload(self.payload_address, self.payload_len as usize)
            .await
            .map_err(|error| {
                tracing::warn!(target: "feuer::storage", %error, "disk read failed");
                DiskLookupOutcome::IoError
            })?;
        if XxHash64::oneshot(0, &bytes) != self.payload_checksum {
            tracing::warn!(target: "feuer::storage", "disk checksum failed");
            return Err(DiskLookupOutcome::ChecksumFailed);
        }
        Ok((bytes, capacity))
    }
}
