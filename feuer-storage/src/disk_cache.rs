//! Cache object bytes on disk, looked up by object key and byte range.

mod metadata;
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
use tokio::sync::OnceCell;
use twox_hash::XxHash64;

use crate::{
    DataFile, DataFileError, DataFileResult, DiskMetrics, IoMetrics,
    allocation::{CHUNK_BYTES, DiskChunkAllocator, ReservedChunks},
    disk_metrics::{DiskLookupOutcome, DiskWriteAttempt, DiskWriteOutcome},
};
#[cfg(test)]
use page_format::{METADATA_PAGE_BYTES, PAGE_CONTENT_BYTES};

// Per shard batch: bound sampled eviction decisions and removal work, including multi-chunk entries.
const MAX_EVICTION_ATTEMPTS: usize = 64;
const MAX_EVICTION_CHUNKS: usize = 4096;

/// Caches object bytes on disk, looked up by object key and byte range.
///
/// Explicit batches pack payloads into 1-MiB chunks. Separate mutable metadata chunks
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
}

/// Shared disk-cache internals: file, shards, access histories, and metrics.
struct DiskCacheInner {
    file: DataFile,
    shards: Box<[DiskCacheShard]>,
    access_histories: Arc<ObjectAccessHistories>,
    metrics: Arc<DiskMetrics>,
}

/// One disk-cache shard's chunk allocator, entry index, and metadata pages.
struct DiskCacheShard {
    reclaim_sample_size: usize,
    allocator: DiskChunkAllocator,
    entry_index: Mutex<DiskEntryIndex>,
    metadata_pages: Mutex<metadata::MetadataPages>,
}

/// Disk entries indexed by object key and range start, with a list sampled in rotation for eviction.
struct DiskEntryIndex {
    entries_by_key: FxHashMap<ObjectKeyHash, BTreeMap<u64, DiskEntry>>,
    eviction_candidates: Vec<(ObjectKeyHash, u64)>,
    next_candidate: usize,
    metrics: Arc<DiskMetrics>,
}

/// An entry's payload, metadata, and metrics awaiting write and publication.
struct PendingEntry {
    key: ObjectKeyHash,
    entry: DiskEntry,
    bytes: Bytes,
    attempt: DiskWriteAttempt,
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
        let file = DataFile::open_with_buffer_pool(directory, capacity, io_metrics, buffer_pool).await?;
        let num_shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
        let shards = (0..num_shards)
            .map(|shard_index| {
                let range = recovery::shard_disk_range(capacity, num_shards, shard_index);
                let allocator = DiskChunkAllocator::with_metrics(range, metrics.clone()).unwrap();
                DiskCacheShard {
                    reclaim_sample_size,
                    allocator,
                    entry_index: Mutex::new(DiskEntryIndex::new(metrics.clone())),
                    metadata_pages: Mutex::new(metadata::MetadataPages::default()),
                }
            })
            .collect();
        let disk = Arc::new(DiskCacheInner {
            file,
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
        while let Some(result) = recovery_tasks.join_next().await {
            result.expect("shard recovery task failed");
        }
        tracing::info!(target: "feuer::storage", elapsed_seconds = started.elapsed().as_secs_f64(), "disk cache recovery finished");
        tokio::spawn(DiskCacheInner::write_metadata_periodically(Arc::downgrade(&disk)));
        Ok(Self { disk })
    }

    /// Shared history for recording requests once, before lookup and outside raw storage operations.
    pub fn access_histories(&self) -> Arc<ObjectAccessHistories> {
        self.disk.access_histories.clone()
    }

    /// Packs an explicit batch into chunks, grouping smaller payloads first within each shard.
    /// Returns the number of entries published; contained entries and entries that do not fit are skipped.
    /// Each entry's payload writes finish before publication; metadata is written every second, best-effort.
    /// Publication rechecks containment. Later entries may replace or evict earlier ones. Does not record accesses.
    ///
    /// Entries share a payload chunk only when their complete aligned payloads fit inside it.
    /// Multi-chunk entries own their chunks exclusively. Partially filled chunks are finalized too.
    /// Each shard batch allows at most 64 eviction attempts and charges at most 4,096 chunks to removed entries.
    /// Unavailable capacity causes admission to be skipped, not waited for.
    /// Completed payload regions publish independently; a later failure does not undo earlier publication.
    /// Callers bound batch size and concurrency: payload slices are retained until writing completes.
    /// Dropping this future does not abort its detached writer.
    pub async fn insert_batch(&self, downloads: Vec<(ObjectKeyHash, Download)>) -> Result<usize, DiskCacheError> {
        let disk = self.disk.clone();
        let mut downloads_by_shard: Vec<Vec<_>> = (0..disk.shards.len()).map(|_| Vec::new()).collect();
        for (key, download) in downloads {
            downloads_by_shard[disk.shard_index_for_key(&key)].push((
                key,
                download,
                DiskWriteAttempt::new(disk.metrics.clone()),
            ));
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DataFileError::RuntimeUnavailable)?;
        runtime
            .spawn(async move {
                let mut published = 0;
                for (shard, downloads) in disk.shards.iter().zip(downloads_by_shard) {
                    published += disk.write_shard(shard, downloads).await?;
                }
                Ok(published)
            })
            .await
            .map_err(DiskCacheError::WriteTaskFailed)?
    }

    /// Checks indexed coverage without reading payload or recording an access.
    pub fn contains(&self, key: &ObjectKeyHash, range: ByteRange) -> bool {
        self.disk.shards[self.disk.shard_index_for_key(key)]
            .entry_index
            .lock()
            .unwrap()
            .covering_entry(key, range)
            .is_some()
    }

    /// Returns exactly requested bytes from one covering entry, or a miss on any I/O/integrity uncertainty.
    /// Concurrent reads of one stored entry share whole-entry I/O and checksum verification.
    /// Reads no neighboring entries or metadata. Results retain no disk ownership.
    pub async fn get(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<Bytes> {
        self.get_with_buffer_capacity(key, requested)
            .await
            .map(|(bytes, _)| bytes)
    }

    /// Returns the requested bytes and their backing buffer's capacity for memory admission.
    pub async fn get_with_buffer_capacity(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<(Bytes, usize)> {
        let started = Instant::now();
        let metrics = &self.disk.metrics;
        let shard = &self.disk.shards[self.disk.shard_index_for_key(key)];
        let (read, in_flight_read) = {
            let mut index = shard.entry_index.lock().unwrap();
            let Some(entry) = index.covering_entry(key, requested) else {
                metrics.record_lookup(DiskLookupOutcome::Absent, started.elapsed());
                return None;
            };
            let start = entry.object_range.start();
            let entry = index.entries_by_key.get_mut(key).unwrap().get_mut(&start).unwrap();
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
                match read.read_verified_range(&self.disk.file, read.object_range).await {
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
                        let mut index = shard.entry_index.lock().unwrap();
                        if let Some(entry) = index.remove_entry_matching_read(key, &read) {
                            shard.remove_payload_and_allow_metadata_overwrite(entry);
                        }
                        Err(outcome)
                    }
                }
            })
            .await;
        match result {
            Ok((bytes, capacity)) => {
                metrics.record_lookup(DiskLookupOutcome::Hit, started.elapsed());
                let start = (requested.start() - read.object_range.start()) as usize;
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
    /// Samples up to `reclaim_sample_size` entries and evicts at most one that fits the remaining chunk budget.
    /// Returns whether sampling occurred, even if no entry was evicted.
    /// Consumes one attempt when sampling; removed entries consume their chunk count. Neighbors are not evicted.
    /// The allocator releases removed payloads without waiting for metadata writes or readers.
    fn sample_and_evict_entry(
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
            let entry = &index.entries_by_key[key][range_start];
            if entry.chunk_count() as usize > *chunks_left {
                continue;
            }
            let retrieval_cost = access_histories.decayed_retrieval_cost(key, entry.object_range);
            let payload_bytes = entry.object_range.len();
            if selected_candidate.is_none_or(|(_, selected_cost, selected_bytes)| {
                compare_cost_per_byte(retrieval_cost, payload_bytes, selected_cost, selected_bytes).is_lt()
            }) {
                selected_candidate = Some((position, retrieval_cost, payload_bytes));
            }
        }
        if let Some((position, ..)) = selected_candidate {
            let (key, start) = index.eviction_candidates[position];
            *chunks_left -= index.entries_by_key[&key][&start].chunk_count() as usize;
            self.remove_payload_and_allow_metadata_overwrite(index.remove(&key, start).unwrap());
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
    /// Removes the payload's use of its chunks and allows its metadata to be overwritten.
    /// Neither index removal nor entry destruction has side effects on disk ownership or metadata.
    fn remove_payload_and_allow_metadata_overwrite(&self, entry: DiskEntry) {
        self.allocator.remove_payload(entry.payload_range.start);
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
            next_candidate: 0,
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
        while let Some((&start, existing)) = self
            .entries_by_key
            .get(&key)
            .and_then(|entries| entries.range(object_range.start()..).next())
        {
            // Retained ranges have increasing ends, so no later range can be contained either.
            if !object_range.contains(existing.object_range) {
                break;
            }
            removed.push(self.remove(&key, start).unwrap());
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

    fn remove(&mut self, key: &ObjectKeyHash, start: u64) -> Option<DiskEntry> {
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
        if self
            .entries_by_key
            .get(key)
            .and_then(|entries| entries.get(&start))
            .is_some_and(|entry| entry.payload_checksum == read.payload_checksum)
        {
            return self.remove(key, start);
        }
        None
    }

    /// Finds the entry whose byte range covers the entire request.
    fn covering_entry(&self, key: &ObjectKeyHash, requested: ByteRange) -> Option<&DiskEntry> {
        // Retained ranges never contain one another, so their ends increase with their starts.
        let (_, entry) = self.entries_by_key.get(key)?.range(..=requested.start()).next_back()?;
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

    /// Packs and writes one shared chunk or one multi-chunk payload at a time.
    async fn write_shard(
        &self,
        shard: &DiskCacheShard,
        mut downloads: Vec<(ObjectKeyHash, Download, DiskWriteAttempt)>,
    ) -> Result<usize, DiskCacheError> {
        downloads.sort_by_key(|(_, download, _)| download.bytes().len());
        let mut region: Option<ReservedChunks> = None;
        let mut entries: Vec<PendingEntry> = Vec::new();
        let mut attempts_left = MAX_EVICTION_ATTEMPTS;
        let mut chunks_left = MAX_EVICTION_CHUNKS;
        let mut published = 0;
        for (key, download, mut attempt) in downloads {
            if shard
                .entry_index
                .lock()
                .unwrap()
                .covering_entry(&key, download.downloaded_range())
                .is_some()
            {
                attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                continue;
            }
            let length = download
                .downloaded_range()
                .len()
                .next_multiple_of(BUFFER_ALIGNMENT as u64);
            if region.as_ref().is_some_and(|region| {
                region.chunk_count() != 1
                    || entries.last().unwrap().entry.payload_range.end + length > region.range().end
            }) {
                published += self
                    .write_and_publish(shard, region.take().unwrap(), std::mem::take(&mut entries))
                    .await?;
            }
            let previous_chunks_left = chunks_left;
            let entry = shard.reserve_entry(
                &download,
                &mut region,
                entries.last().map(|pending| pending.entry.payload_range.end),
                &self.access_histories,
                &mut attempts_left,
                &mut chunks_left,
            );
            // Sampling alone does not count as eviction.
            attempt.evicted = chunks_left != previous_chunks_left;
            let Some(entry) = entry else {
                attempt.set_outcome(DiskWriteOutcome::NoCapacity);
                continue;
            };
            entries.push(PendingEntry {
                key,
                entry,
                bytes: download.bytes().clone(),
                attempt,
            });
        }
        if let Some(region) = region {
            published += self.write_and_publish(shard, region, entries).await?;
        }
        Ok(published)
    }

    /// Writes one reserved region, then publishes entries not already covered in the disk index.
    /// Failure releases this region and its metadata positions without rolling back earlier publication.
    async fn write_and_publish(
        &self,
        shard: &DiskCacheShard,
        region: ReservedChunks,
        mut entries: Vec<PendingEntry>,
    ) -> Result<usize, DiskCacheError> {
        for address in region.range().step_by(CHUNK_BYTES as usize) {
            let end = address + CHUNK_BYTES;
            let mut parts = Vec::new();
            for pending in &entries {
                let start = pending.entry.payload_range.start;
                let from = start.max(address);
                let to = (start + pending.bytes.len() as u64).min(end);
                if from < to {
                    parts.push((
                        (from - address) as usize,
                        pending.bytes.slice((from - start) as usize..(to - start) as usize),
                    ));
                }
            }
            if let Err(error) = self.file.write_parts(address..end, &parts).await {
                let mut pages = shard.metadata_pages.lock().unwrap();
                for pending in &mut entries {
                    pending.attempt.set_outcome(DiskWriteOutcome::Failed);
                    pages.free_entry_positions.push(pending.entry.metadata);
                }
                return Err(error.into());
            }
        }
        self.metrics.written_entries.increase(entries.len() as u64);
        self.metrics
            .packed_payload_bytes
            .increase(entries.iter().map(|pending| pending.entry.object_range.len()).sum());
        self.metrics
            .packed_chunk_bytes
            .increase(region.chunk_count() * CHUNK_BYTES);
        shard.allocator.retain_payloads(region, entries.len());
        let mut index = shard.entry_index.lock().unwrap();
        let mut published = 0;
        // Larger entries in this region can cover smaller neighbors before they publish.
        for PendingEntry {
            key,
            entry,
            mut attempt,
            ..
        } in entries.into_iter().rev()
        {
            if index.covering_entry(&key, entry.object_range).is_some() {
                attempt.set_outcome(DiskWriteOutcome::AlreadyCovered);
                shard.remove_payload_and_allow_metadata_overwrite(entry);
                continue;
            }
            shard.metadata_pages.lock().unwrap().set_entry_metadata(&key, &entry);
            for removed in index.insert(key, entry) {
                shard.remove_payload_and_allow_metadata_overwrite(removed);
            }
            published += 1;
            attempt.set_outcome(DiskWriteOutcome::Published);
        }
        Ok(published)
    }
}

impl DiskCacheShard {
    /// Reserves entry metadata and payload space, evicting within the shard batch's remaining budget.
    /// The caller supplies a region with room at `next_payload`, or `None` to reserve consecutive whole chunks.
    fn reserve_entry(
        &self,
        download: &Download,
        region: &mut Option<ReservedChunks>,
        next_payload: Option<u64>,
        access_histories: &ObjectAccessHistories,
        attempts_left: &mut usize,
        chunks_left: &mut usize,
    ) -> Option<DiskEntry> {
        let length = download
            .downloaded_range()
            .len()
            .next_multiple_of(BUFFER_ALIGNMENT as u64);
        let count = length.div_ceil(CHUNK_BYTES);
        loop {
            let metadata = self.metadata_pages.lock().unwrap().reserve_metadata(&self.allocator);
            let chunks_needed = if let Some(metadata) = metadata {
                if region.is_none() {
                    *region = self.allocator.reserve_chunks(count);
                }
                if let Some(region) = region {
                    let start = next_payload.unwrap_or(region.range().start);
                    return Some(DiskEntry {
                        in_flight_read: Weak::new(),
                        eviction_position: 0,
                        object_range: download.downloaded_range(),
                        payload_checksum: XxHash64::oneshot(0, download.bytes()),
                        payload_range: start..start + length,
                        metadata,
                    });
                }
                self.metadata_pages.lock().unwrap().free_entry_positions.push(metadata);
                count
            } else {
                1
            };
            let metadata_chunks = self.metadata_pages.lock().unwrap().chunks.len() as u64;
            let reserved_chunks = region.as_ref().map_or(0, ReservedChunks::chunk_count);
            // Eviction cannot create more space than the shard has outside metadata and the pending write.
            if chunks_needed > self.allocator.chunk_capacity - metadata_chunks - reserved_chunks
                || !self.sample_and_evict_entry(access_histories, attempts_left, chunks_left)
            {
                return None;
            }
        }
    }
}

impl PayloadRead {
    /// Reads the whole entry, verifies its checksum, and returns only the requested byte range.
    async fn read_verified_range(
        &self,
        file: &DataFile,
        requested: ByteRange,
    ) -> DataFileResult<Option<(Bytes, usize)>> {
        let start = requested.start() - self.object_range.start();
        let end = requested.end() - self.object_range.start();
        let (bytes, capacity) = file
            .read_payload(self.payload_range.clone(), self.object_range.len() as usize)
            .await?;
        if XxHash64::oneshot(0, &bytes) != self.payload_checksum {
            return Ok(None);
        }
        Ok(Some((bytes.slice(start as usize..end as usize), capacity)))
    }
}
