use std::{
    ffi::OsStr,
    fmt,
    future::Future,
    hash::Hash,
    io,
    num::{NonZeroU64, NonZeroUsize},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use bytes::Bytes;
use feuer_historian::{DEFAULT_ACCESS_COUNT_HALF_LIFE, DEFAULT_HISTORY_ENTRY_LIMIT, ObjectAccessHistories};
use feuer_memory::{
    AlignedBuffer, MemoryCache, MemoryMetrics,
    retention::{FetchCostScorer, RECLAIM_SAMPLE_SIZE, RetentionScorer},
};
#[cfg(all(target_os = "linux", feature = "disk"))]
use feuer_storage::{DiskCache, DiskCacheOptions, DiskMetrics, IoMetrics};
use feuer_utils::{ObjectKeyHash, config::read_env_number};
use futures_util::FutureExt;
use mixtrics::{metrics::BoxedRegistry, registry::noop::NoopMetricsRegistry};
use thiserror::Error;

use crate::{
    CacheConfig, CacheConfigError, DiskCacheError, IoQueues,
    access_trace::AccessTrace,
    metrics::{LookupMetrics, LookupOutcome},
};

mod pending_fetches;
use pending_fetches::{PendingFetch, PendingFetches};

type FetchResult<E> = (LookupOutcome, Result<Bytes, GetOrFetchError<E>>);

// Decides whether an object's payload may enter a cache tier.
type AdmissionFilter = dyn Fn(&ObjectKeyHash, u64, &ObjectAccessHistories) -> bool + Send + Sync;

/// Bytes and the amount charged toward the cache's memory usage.
///
/// Converting [`Bytes`] charges its length. Converting [`AlignedBuffer`] charges
/// its capacity and produces [`Bytes`] without copying the payload.
pub struct BytesAndMemoryCharge {
    bytes: Bytes,
    memory_charge: usize,
}

impl From<Bytes> for BytesAndMemoryCharge {
    fn from(bytes: Bytes) -> Self {
        let memory_charge = bytes.len();
        Self { bytes, memory_charge }
    }
}

impl From<AlignedBuffer> for BytesAndMemoryCharge {
    fn from(buffer: AlignedBuffer) -> Self {
        let memory_charge = buffer.capacity();
        Self {
            bytes: buffer.into_bytes(),
            memory_charge,
        }
    }
}

/// Configuration and cache tiers shared by cloned handles.
struct CacheInner {
    config: CacheConfig,
    memory: MemoryCache,
    metrics: LookupMetrics,
    pending_fetches: PendingFetches,
    memory_admission_filter: Option<Box<AdmissionFilter>>,
    #[cfg(all(target_os = "linux", feature = "disk"))]
    disk_admission_filter: Option<Box<AdmissionFilter>>,
    #[cfg(all(target_os = "linux", feature = "disk"))]
    disk: Option<DiskCache>,
}

/// A cloneable handle to a Feuer cache.
///
/// Each key identifies one immutable payload. All insertions and fetch callbacks for
/// the same key must supply equivalent bytes, including across cache reopenings.
/// Lookups check memory, then integrity-checked disk, then the per-call callback.
/// Disk writes are best-effort. A full 512-entry queue skips new writes. Zero disk capacity disables disk.
/// Otherwise, opening requires the `disk` feature, Linux direct I/O and io_uring, and waits for metadata recovery.
#[derive(Clone)]
pub struct Cache {
    inner: Arc<CacheInner>,
    access_trace: Option<AccessTrace>,
}

impl fmt::Debug for Cache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cache")
            .field("memory_capacity", &self.inner.config.memory_capacity())
            .field("disk_capacity", &self.inner.config.disk_capacity())
            .finish_non_exhaustive()
    }
}

/// An error configuring or opening a [`Cache`].
#[derive(Debug, Error)]
pub enum CacheOpenError {
    /// The cache settings are invalid.
    #[error(transparent)]
    Config(#[from] CacheConfigError),
    /// The disk cache could not be opened.
    #[error(transparent)]
    Disk(#[from] DiskCacheError),
}

/// Settings and optional resources used to open a [`Cache`].
///
/// Created by [`Cache::builder`]. Set a positive memory capacity before calling
/// [`open`](Self::open). Disk caching is disabled by default. Settings are validated
/// when opening. Defaults use no-op metrics, dedicated I/O queues,
/// and [`FetchCostScorer`] for both tiers.
pub struct CacheBuilder<'a> {
    directory: PathBuf,
    file_name: String,
    disk_capacity: u64,
    memory_capacity: u64,
    payload_version: u64,
    access_count_half_life: Option<u64>,
    history_entry_limit: Option<usize>,
    reclaim_sample_size: Option<usize>,
    metrics_registry: Option<&'a BoxedRegistry>,
    io_queues: Option<IoQueues>,
    memory_scorer: Arc<dyn RetentionScorer>,
    disk_scorer: Arc<dyn RetentionScorer>,
    memory_admission_filter: Option<Box<AdmissionFilter>>,
    disk_admission_filter: Option<Box<AdmissionFilter>>,
}

impl<'a> CacheBuilder<'a> {
    /// Sets the memory eviction target in bytes. Must be positive.
    pub fn memory_capacity(mut self, bytes: u64) -> Self {
        self.memory_capacity = bytes;
        self
    }

    /// Sets disk capacity in bytes, including metadata and alignment overhead.
    /// Defaults to zero, which disables disk caching. Nonzero capacity must be at least 2 MiB.
    pub fn disk_capacity(mut self, bytes: u64) -> Self {
        self.disk_capacity = bytes;
        self
    }

    /// Sets the disk cache directory. Defaults to `feuer_cache`. Unused when disk caching is disabled.
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = directory.into();
        self
    }

    /// Sets the backing filename within the directory. Defaults to `data`.
    /// Must be a single filename without NUL bytes or the reserved `.feuer.` prefix.
    pub fn file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = file_name.into();
        self
    }

    /// Sets the application's payload format version. Defaults to zero.
    /// Changing the version clears previous disk entries when opening.
    pub fn payload_version(mut self, version: u64) -> Self {
        self.payload_version = version;
        self
    }

    /// Sets the number of requests after which demand weight halves. Must be positive.
    /// Overrides `FEUER_ACCESS_COUNT_HALF_LIFE`, whose default is 262,144.
    pub fn access_count_half_life(mut self, requests: u64) -> Self {
        self.access_count_half_life = Some(requests);
        self
    }

    /// Sets the entry-count cleanup threshold for request history. Must be positive.
    /// Overrides `FEUER_HISTORY_ENTRY_LIMIT`, whose default is 2,000,000.
    pub fn history_entry_limit(mut self, entries: usize) -> Self {
        self.history_entry_limit = Some(entries);
        self
    }

    /// Sets the number of candidates sampled per eviction. Must be positive.
    /// Overrides `FEUER_RECLAIM_SAMPLE_SIZE`, whose default is 64.
    pub fn reclaim_sample_size(mut self, candidates: usize) -> Self {
        self.reclaim_sample_size = Some(candidates);
        self
    }

    pub(super) fn resolve_config(&self) -> Result<CacheConfig, CacheConfigError> {
        if self.memory_capacity == 0 {
            return Err(CacheConfigError::InvalidMemoryCapacity);
        }
        if Path::new(&self.file_name).file_name() != Some(OsStr::new(&self.file_name))
            || self.file_name.contains('\0')
            || self.file_name.starts_with(".feuer.")
        {
            return Err(CacheConfigError::InvalidFileName);
        }
        let sample_size = self
            .reclaim_sample_size
            .map(Ok)
            .unwrap_or_else(|| read_env_number("FEUER_RECLAIM_SAMPLE_SIZE", RECLAIM_SAMPLE_SIZE, 1))
            .map_err(|_| CacheConfigError::InvalidReclaimSampleSize)?;
        if sample_size == 0 {
            return Err(CacheConfigError::InvalidReclaimSampleSize);
        }
        let half_life = self
            .access_count_half_life
            .map(Ok)
            .unwrap_or_else(|| read_env_number("FEUER_ACCESS_COUNT_HALF_LIFE", DEFAULT_ACCESS_COUNT_HALF_LIFE.get(), 1))
            .map_err(|_| CacheConfigError::InvalidAccessCountHalfLife)?;
        let history_limit = self
            .history_entry_limit
            .map(Ok)
            .unwrap_or_else(|| read_env_number("FEUER_HISTORY_ENTRY_LIMIT", DEFAULT_HISTORY_ENTRY_LIMIT.get(), 1))
            .map_err(|_| CacheConfigError::InvalidHistoryEntryLimit)?;
        Ok(CacheConfig {
            directory: self.directory.clone(),
            file_name: self.file_name.clone(),
            payload_version: self.payload_version,
            disk_capacity: self.disk_capacity,
            memory_capacity: self.memory_capacity,
            access_count_half_life: NonZeroU64::new(half_life).ok_or(CacheConfigError::InvalidAccessCountHalfLife)?,
            history_entry_limit: NonZeroUsize::new(history_limit).ok_or(CacheConfigError::InvalidHistoryEntryLimit)?,
            reclaim_sample_size: sample_size,
        })
    }

    /// Records metrics in this registry. Sharing a registry aggregates metrics.
    /// Labels contain no object identities or cache names.
    pub fn with_metrics(mut self, registry: &'a BoxedRegistry) -> Self {
        self.metrics_registry = Some(registry);
        self
    }

    /// Shares I/O concurrency with other caches using clones of these queues.
    /// Available on Linux with the `disk` feature. Zero disk capacity ignores the queues.
    /// Contents, capacities, eviction, buffer pools, file locks, and background-write queues
    /// remain independent.
    pub fn with_io_queues(mut self, queues: IoQueues) -> Self {
        self.io_queues = Some(queues);
        self
    }

    /// Sets the memory retention scorer. See [`RetentionScorer`] for the scoring contract.
    /// Both tiers receive the same caller-demand history, including memory hits.
    pub fn with_memory_retention_scorer(mut self, scorer: Arc<dyn RetentionScorer>) -> Self {
        self.memory_scorer = scorer;
        self
    }

    /// Sets the disk retention scorer, also used during recovery.
    /// See [`RetentionScorer`] for the scoring contract. Zero disk capacity ignores this scorer.
    pub fn with_disk_retention_scorer(mut self, scorer: Arc<dyn RetentionScorer>) -> Self {
        self.disk_scorer = scorer;
        self
    }

    /// Filters memory admission for explicit inserts, fetched objects, and disk-hit promotions.
    /// Defaults to accepting all objects. Disk admission is independent.
    ///
    /// Receives the key hash, payload length in bytes (possibly zero), and shared request history.
    /// Lookups record demand before admission. Explicit inserts record no demand.
    /// Returning false skips insertion while preserving returned bytes and existing entries.
    /// The filter runs synchronously and may run concurrently, outside cache shard locks.
    /// Keep it cheap and avoid cache re-entry. Concurrent fetches sharing a result share admission.
    pub fn with_memory_admission_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&ObjectKeyHash, u64, &ObjectAccessHistories) -> bool + Send + Sync + 'static,
    {
        self.memory_admission_filter = Some(Box::new(filter));
        self
    }

    /// Filters disk writes from explicit inserts and fetched objects before queueing.
    /// Defaults to accepting all objects. Rejecting memory admission still permits a disk write.
    /// Inserts that retain an existing memory entry skip the disk write and its filter.
    /// Disabled disk caching ignores this filter. Existing disk entries remain readable,
    /// including entries recovered at open. Hits and recovery do not invoke the disk filter.
    ///
    /// Uses the same arguments and execution contract as [`Self::with_memory_admission_filter`].
    /// Returning true still leaves writes subject to queue capacity and disk reclamation.
    pub fn with_disk_admission_filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&ObjectKeyHash, u64, &ObjectAccessHistories) -> bool + Send + Sync + 'static,
    {
        self.disk_admission_filter = Some(Box::new(filter));
        self
    }

    /// Opens a cache synchronously, using only memory when disk capacity is zero.
    /// Memory-only operation works on every platform without a Tokio runtime.
    /// Nonzero disk capacity returns an error unless built for Linux with the `disk` feature.
    /// Disk caching requires an entered Tokio runtime for background workers, io_uring
    /// and direct I/O. Opening locks the backing file and blocks until metadata recovery
    /// completes. Disk failures never fall back to memory.
    /// Validates settings and reads environment defaults before opening any files.
    pub fn open(self) -> Result<Cache, CacheOpenError> {
        let config = self.resolve_config()?;
        #[cfg(not(all(target_os = "linux", feature = "disk")))]
        {
            if config.disk_capacity() != 0 {
                return Err(DiskCacheError::UnsupportedPlatform.into());
            }
            let _ = (self.io_queues, self.disk_scorer, self.disk_admission_filter);
        }
        let noop_metrics_registry: BoxedRegistry = Box::new(NoopMetricsRegistry);
        let metrics_registry = self.metrics_registry.unwrap_or(&noop_metrics_registry);
        let memory = MemoryCache::with_metrics(config.memory_capacity(), MemoryMetrics::new(metrics_registry))
            .with_access_histories(Arc::new(ObjectAccessHistories::with_half_life_and_entry_limit(
                config.access_count_half_life(),
                config.history_entry_limit(),
            )))
            .with_reclaim_sample_size(config.reclaim_sample_size())
            .with_retention_scorer(self.memory_scorer);
        #[cfg(all(target_os = "linux", feature = "disk"))]
        let disk = if config.disk_capacity() == 0 {
            None
        } else {
            let options = DiskCacheOptions {
                file_name: config.file_name().to_owned(),
                payload_version: config.payload_version(),
                io_metrics: IoMetrics::new(metrics_registry),
                metrics: DiskMetrics::new(metrics_registry),
                access_histories: memory.access_histories().clone(),
                reclaim_sample_size: config.reclaim_sample_size(),
                buffer_pool: memory.buffer_pool(),
                io_queues: self.io_queues,
                retention_scorer: self.disk_scorer,
            };
            let disk = DiskCache::open(config.directory(), config.disk_capacity(), options)?;
            Some(disk)
        };
        Ok(Cache {
            inner: Arc::new(CacheInner {
                config,
                memory,
                metrics: LookupMetrics::new(metrics_registry),
                pending_fetches: PendingFetches::default(),
                memory_admission_filter: self.memory_admission_filter,
                #[cfg(all(target_os = "linux", feature = "disk"))]
                disk_admission_filter: self.disk_admission_filter,
                #[cfg(all(target_os = "linux", feature = "disk"))]
                disk,
            }),
            access_trace: None,
        })
    }
}

impl Cache {
    /// Starts configuring a memory-only cache. Set its memory capacity, then call
    /// [`CacheBuilder::open`]. Enable disk caching by setting a nonzero disk capacity.
    pub fn builder<'a>() -> CacheBuilder<'a> {
        let scorer = Arc::new(FetchCostScorer::default());
        CacheBuilder {
            directory: PathBuf::from("feuer_cache"),
            file_name: "data".to_owned(),
            disk_capacity: 0,
            memory_capacity: 0,
            payload_version: 0,
            access_count_half_life: None,
            history_entry_limit: None,
            reclaim_sample_size: None,
            metrics_registry: None,
            io_queues: None,
            memory_scorer: scorer.clone(),
            disk_scorer: scorer,
            memory_admission_filter: None,
            disk_admission_filter: None,
        }
    }

    /// Delivers uncompressed binary access-trace packages as [`Bytes`] for this handle and its clones.
    /// Requires Tokio. Persist packages unchanged. The callback owns retries and error reporting.
    pub fn with_trace<F, Fut>(mut self, deliver_package: F) -> Self
    where
        F: FnMut(Bytes) -> Fut + Send + 'static,
        Fut: Future<Output = io::Result<()>> + Send + 'static,
    {
        self.access_trace = Some(AccessTrace::new(deliver_package));
        self
    }

    async fn trace_request_start(&self, object_key: ObjectKeyHash) -> Option<u64> {
        let access_trace = self.access_trace.as_ref()?;
        Some(access_trace.send_request_start(object_key).await)
    }

    /// Returns this cache's configuration.
    pub fn config(&self) -> &CacheConfig {
        &self.inner.config
    }

    /// Acquires an aligned buffer from the same pool used by disk reads.
    /// Fill its exposed slice and return it from [`Self::get_or_fetch`]'s callback
    /// without copying. It returns to the pool after its last owner releases it,
    /// subject to the idle capacity limit.
    pub fn allocate_buffer(&self, length: usize) -> io::Result<AlignedBuffer> {
        self.inner.memory.buffer_pool().allocate(length)
    }

    /// Admits immutable bytes synchronously, without recording lookup demand.
    ///
    /// Accepts [`Bytes`] or [`AlignedBuffer`], including empty objects, without copying
    /// the payload. Charges memory by length for [`Bytes`] and capacity for
    /// [`AlignedBuffer`]. The memory target is soft and entries remain subject to eviction.
    /// Memory admission that finds an existing entry retains it without another disk enqueue.
    ///
    /// Memory and disk admission filters apply independently. Rejection preserves existing entries.
    /// Eligible entries are queued to disk when enabled, using best-effort admission.
    /// A full or closed queue skips the disk write while preserving any memory insertion.
    /// This call performs no disk lookup, waits for no I/O, and requires no Tokio runtime.
    /// The runtime that opened the disk cache drives background writes.
    ///
    /// All insertions and fetch callbacks for a key must supply equivalent immutable bytes.
    /// In-flight fetches continue independently and may return their own equivalent bytes.
    /// Keys must follow the identity contract in [`ObjectKeyHash::from_hash`].
    pub fn insert<K: Hash + ?Sized, V: Into<BytesAndMemoryCharge>>(&self, object_key: &K, value: V) {
        let object_key = ObjectKeyHash::from_hash(object_key);
        let BytesAndMemoryCharge { bytes, memory_charge } = value.into();
        self.insert_by_hash(object_key, &bytes, memory_charge);
    }

    fn allows_admission(&self, filter: &Option<Box<AdmissionFilter>>, key: &ObjectKeyHash, bytes: &Bytes) -> bool {
        filter
            .as_ref()
            .is_none_or(|filter| filter(key, bytes.len() as u64, self.inner.memory.access_histories()))
    }

    fn insert_by_hash(&self, object_key: ObjectKeyHash, bytes: &Bytes, memory_charge: usize) {
        let duplicate = self.allows_admission(&self.inner.memory_admission_filter, &object_key, bytes)
            && !self.inner.memory.insert(object_key, bytes.clone(), memory_charge);
        #[cfg(not(all(target_os = "linux", feature = "disk")))]
        let _ = duplicate;
        #[cfg(all(target_os = "linux", feature = "disk"))]
        if let Some(disk) = &self.inner.disk {
            if duplicate {
                self.inner.metrics.disk_write_redundant.increase(1);
            } else if self.allows_admission(&self.inner.disk_admission_filter, &object_key, bytes) {
                disk.enqueue_if_space_available(object_key, bytes.clone());
            }
        }
    }

    /// Returns an object from memory or disk, or `None` on a miss.
    /// Does not invoke or wait for source callbacks. Disk hits are subject to memory admission.
    /// The key is hashed immediately. The returned future retains only its fingerprint.
    /// Keys must follow the identity contract in [`ObjectKeyHash::from_hash`].
    pub fn get<'cache, K: Hash + ?Sized>(
        &'cache self,
        object_key: &K,
    ) -> impl Future<Output = Option<Bytes>> + Send + use<'cache, K> {
        let object_key = ObjectKeyHash::from_hash(object_key);
        async move {
            self.inner.memory.access_histories().record_request(&object_key);
            let request_id = self.trace_request_start(object_key).await;
            let started = Instant::now();
            let (outcome, bytes) = self.lookup(object_key, || false).await;
            let payload_length = bytes.as_ref().map(|bytes| bytes.len() as u64).unwrap_or(0);
            self.inner.metrics.record(outcome, started.elapsed(), payload_length);
            if let (Some(access_trace), Some(request_id)) = (&self.access_trace, request_id) {
                access_trace
                    .send_outcome(request_id, object_key, outcome, payload_length)
                    .await;
            }
            bytes
        }
    }

    /// Returns the object from memory, disk, or a shared callback.
    ///
    /// Disk hits promote the object subject to the memory admission filter. Concurrent memory misses with
    /// the same key and exact error type share one disk-or-callback operation, including errors.
    /// Callbacks for matching requests must be interchangeable. Each callback runs at most once.
    /// Any waiter can drive the shared fetch. Cancelling one caller preserves the original callback
    /// for remaining waiters. Dropping the last waiter cancels the fetch.
    /// Callbacks and their futures must own their captures and be `Send + 'static`.
    /// Completed failures are not cached for later requests.
    ///
    /// The callback can return [`Bytes`] or [`AlignedBuffer`], including empty objects.
    /// The cache charges [`Bytes`] by length and [`AlignedBuffer`] by capacity.
    /// Both produce [`Bytes`] without copying the payload. Concurrent misses share work
    /// across both callback types when their error types match. The memory target is soft.
    ///
    /// Every started request records demand exactly once, before checking either tier.
    /// Failures and cancellation still count. Insertion records no request.
    /// Callback results are admitted independently to memory and disk through the configured filters.
    /// Eligible disk writes are queued immediately, subject to queue capacity.
    /// Rejected results still reach all callers sharing the fetch.
    /// Tracing can backpressure lookups but never changes their results.
    /// The key is hashed immediately. The returned future retains only its fingerprint.
    /// Keys must follow the identity contract in [`ObjectKeyHash::from_hash`].
    pub fn get_or_fetch<'cache, K, F, Fut, V, E>(
        &'cache self,
        object_key: &K,
        callback: F,
    ) -> impl Future<Output = Result<Bytes, GetOrFetchError<E>>> + Send + use<'cache, K, F, Fut, V, E>
    where
        K: Hash + ?Sized,
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<V, E>> + Send + 'static,
        V: Into<BytesAndMemoryCharge>,
    {
        self.get_or_fetch_with_disk_fallthrough(object_key, || false, callback)
    }

    /// Like [`Self::get_or_fetch`], but may fetch from the source instead of a busy disk.
    ///
    /// When the queue reaches 64 waiting payload reads, calls `should_fall_through`
    /// at most once per waiting read before I/O. Caches sharing `IoQueues` share this queue.
    /// True invokes `callback`, false waits for disk. The decision runs synchronously
    /// in a waiting caller's task and should be cheap. Shared fetches retain the original
    /// caller's decision closure even if that caller cancels.
    ///
    /// Fallthrough preserves the disk entry and applies memory admission to successful downloads
    /// without rewriting them to disk. Source failures return without retrying disk.
    /// Keys must follow the identity contract in [`ObjectKeyHash::from_hash`].
    pub fn get_or_fetch_with_disk_fallthrough<'cache, K, S, F, Fut, V, E>(
        &'cache self,
        object_key: &K,
        should_fall_through: S,
        callback: F,
    ) -> impl Future<Output = Result<Bytes, GetOrFetchError<E>>> + Send + use<'cache, K, S, F, Fut, V, E>
    where
        K: Hash + ?Sized,
        S: FnOnce() -> bool + Send + 'static,
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<V, E>> + Send + 'static,
        V: Into<BytesAndMemoryCharge>,
    {
        let object_key = ObjectKeyHash::from_hash(object_key);
        async move {
            self.get_or_fetch_by_hash(object_key, should_fall_through, move || async move {
                callback().await.map(Into::into)
            })
            .await
        }
    }

    async fn get_or_fetch_by_hash<F, Fut, E>(
        &self,
        object_key: ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool + Send + 'static,
        callback: F,
    ) -> Result<Bytes, GetOrFetchError<E>>
    where
        E: Clone + Send + Sync + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<BytesAndMemoryCharge, E>> + Send + 'static,
    {
        self.inner.memory.access_histories().record_request(&object_key);
        let request_id = self.trace_request_start(object_key).await;
        let started = Instant::now();
        let (outcome, result) = if let Some(bytes) = self.inner.memory.get(&object_key) {
            (LookupOutcome::MemoryHit, Ok(bytes))
        } else {
            let fetch = PendingFetch::<E>::join(&self.inner.pending_fetches, object_key);
            let result = fetch
                .result
                .get_or_init(|| {
                    let cache = self.clone();
                    async move { cache.fetch(object_key, should_fall_through, callback).await }
                        .boxed()
                        .shared()
                })
                .clone()
                .await;
            fetch.remove_from_registry();
            result
        };
        let payload_length = result.as_ref().map(|bytes| bytes.len() as u64).unwrap_or(0);
        self.inner.metrics.record(outcome, started.elapsed(), payload_length);
        if let (Some(access_trace), Some(request_id)) = (&self.access_trace, request_id) {
            access_trace
                .send_outcome(request_id, object_key, outcome, payload_length)
                .await;
        }
        result
    }

    /// Checks memory, then disk if configured, promoting disk hits to memory.
    /// Unlike `fetch`, returns a miss without invoking a source callback.
    async fn lookup(
        &self,
        object_key: ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool,
    ) -> (LookupOutcome, Option<Bytes>) {
        if let Some(bytes) = self.inner.memory.get(&object_key) {
            return (LookupOutcome::MemoryHit, Some(bytes));
        }
        #[cfg(not(all(target_os = "linux", feature = "disk")))]
        let _ = should_fall_through;
        #[cfg(all(target_os = "linux", feature = "disk"))]
        if let Some(disk) = &self.inner.disk
            && let Some((bytes, buffer_capacity)) = disk.get_with_fallthrough(&object_key, should_fall_through).await
        {
            if self.allows_admission(&self.inner.memory_admission_filter, &object_key, &bytes) {
                self.inner.memory.insert(object_key, bytes.clone(), buffer_capacity);
            }
            return (LookupOutcome::DiskHit, Some(bytes));
        }
        (LookupOutcome::Miss, None)
    }

    /// Looks up the object in memory, then disk if configured, before invoking the source callback.
    /// The memory recheck handles insertions before this caller joined the registry.
    async fn fetch<F, Fut, E>(
        &self,
        object_key: ObjectKeyHash,
        should_fall_through: impl FnOnce() -> bool,
        callback: F,
    ) -> FetchResult<E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<BytesAndMemoryCharge, E>>,
    {
        let mut fell_through = false;
        let (outcome, bytes) = self
            .lookup(object_key, || {
                fell_through = should_fall_through();
                fell_through
            })
            .await;
        if let Some(bytes) = bytes {
            return (outcome, Ok(bytes));
        }

        let BytesAndMemoryCharge { bytes, memory_charge } = match callback().await {
            Ok(result) => result,
            Err(error) => {
                return (LookupOutcome::CallbackError, Err(GetOrFetchError::Callback(error)));
            }
        };
        #[cfg(all(target_os = "linux", feature = "disk"))]
        if let Some(disk) = &self.inner.disk
            && disk.contains(&object_key)
        {
            self.inner.metrics.disk_write_already_covered.increase(1);
            if fell_through && self.allows_admission(&self.inner.memory_admission_filter, &object_key, &bytes) {
                self.inner.memory.insert(object_key, bytes.clone(), memory_charge);
            }
            return (LookupOutcome::Callback, Ok(bytes));
        }
        self.insert_by_hash(object_key, &bytes, memory_charge);
        (LookupOutcome::Callback, Ok(bytes))
    }
}

/// A failed cache lookup or download operation.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GetOrFetchError<E> {
    /// The callback used by this lookup's shared fetch failed.
    #[error("download callback failed: {0}")]
    Callback(#[source] E),
}

#[cfg(all(test, target_os = "linux", feature = "disk"))]
mod disk_fallthrough_tests;

#[cfg(test)]
mod portable_tests;

#[cfg(test)]
mod key_tests;

#[cfg(test)]
mod insert_tests;

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod sharing_tests;

#[cfg(test)]
mod download_buffer_tests;

#[cfg(test)]
mod object_allocation_charge_tests;

#[cfg(all(test, not(all(target_os = "linux", feature = "disk"))))]
mod request_history_tests;

#[cfg(all(test, target_os = "linux", feature = "disk"))]
mod tests {
    use std::{
        convert::Infallible,
        future::pending,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        time::Duration,
    };

    use tempfile::TempDir;
    use tokio::{
        sync::Notify,
        task::yield_now,
        time::{sleep, timeout},
    };

    use super::*;
    use crate::test_metrics::{registry, value};

    #[tokio::test]
    async fn tier_scorers_receive_payload_lengths_and_shared_history() {
        #[derive(Default)]
        struct ScoredEntries(Mutex<Vec<(ObjectKeyHash, u64, usize)>>);

        impl RetentionScorer for ScoredEntries {
            fn score(&self, key: &ObjectKeyHash, payload_len: u64, histories: &ObjectAccessHistories) -> f64 {
                self.0
                    .lock()
                    .unwrap()
                    .push((*key, payload_len, histories as *const ObjectAccessHistories as usize));
                1.0
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let scorer = Arc::new(ScoredEntries::default());
        let disk_scorer = Arc::new(ScoredEntries::default());
        let cache = Cache::builder()
            .directory(directory.path())
            .disk_capacity(4 << 20)
            .memory_capacity(1)
            .with_memory_retention_scorer(scorer.clone())
            .with_disk_retention_scorer(disk_scorer.clone())
            .open()
            .unwrap();
        // More entries than memory shards force the memory scorer to run.
        for index in 0..65 {
            cache.inner.memory.insert(
                ObjectKeyHash::from(format!("memory-{index}")),
                Bytes::from_static(b"x"),
                100,
            );
        }
        let disk = cache.inner.disk.as_ref().unwrap();
        for index in 0..3 {
            disk.insert_batch(vec![(
                ObjectKeyHash::from(format!("disk-{index}")),
                Bytes::from(vec![0; 1 << 20]),
            )])
            .await
            .unwrap();
        }
        let history = Arc::as_ptr(cache.inner.memory.access_histories()) as usize;
        let memory_scores = scorer.0.lock().unwrap();
        assert!(!memory_scores.is_empty());
        assert!(
            memory_scores
                .iter()
                .all(|(_, payload_len, histories)| (*payload_len, *histories) == (1, history))
        );
        let disk_scores = disk_scorer.0.lock().unwrap();
        assert!(!disk_scores.is_empty());
        assert!(
            disk_scores
                .iter()
                .all(|(_, payload_len, histories)| (*payload_len, *histories) == (1 << 20, history))
        );
    }

    #[tokio::test]
    async fn public_registry_observes_memory_disk_callbacks_and_writes() {
        let directory = tempfile::tempdir().unwrap();
        let (registry, backend) = registry();
        let cache = Cache::builder()
            .directory(directory.path())
            .disk_capacity(4 << 20)
            .memory_capacity(32)
            .with_metrics(&backend)
            .open()
            .unwrap();
        let key = "object".to_owned();
        assert!(Arc::ptr_eq(
            cache.inner.memory.access_histories(),
            &cache.inner.disk.as_ref().unwrap().access_histories()
        ));
        assert!(
            cache
                .get_or_fetch(&key, || async { Err::<Bytes, _>("failed") })
                .await
                .is_err()
        );
        cache
            .get_or_fetch(&key, || async { Ok::<_, Infallible>(Bytes::from_static(b"bc")) })
            .await
            .unwrap();
        wait_for_buffered(&cache, &key).await;
        cache.inner.disk.as_ref().unwrap().flush().await.unwrap();
        cache
            .get_or_fetch(&key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert!(cache.inner.memory.remove(&ObjectKeyHash::from(key.as_str())));
        cache
            .get_or_fetch(&key, || async { Err::<Bytes, _>("disk hit must not fetch") })
            .await
            .unwrap();
        for outcome in ["memory_hit", "disk_hit", "callback", "callback_error"] {
            assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), 1.0);
        }
        for outcome in ["disk_hit", "callback"] {
            assert_eq!(
                value(
                    &registry,
                    "feuer_lookup_duration_seconds",
                    &[("outcome", outcome), ("size_range", "lt_100kib")],
                ),
                1.0
            );
        }
        for source in ["memory", "disk", "callback"] {
            assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 2.0);
        }
        assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", "hit")]), 1.0);
        // Metadata is written periodically, independently of payload publication.
        timeout(Duration::from_secs(10), async {
            while value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "write"), ("outcome", "success")],
            ) < 4.0
            {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "write"), ("outcome", "success")]
            ),
            4.0 // Global header, one payload chunk, one metadata page, and one next-chunk link.
        );
        assert_eq!(
            value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
            1.0
        );
        assert_eq!(
            cache.inner.memory.access_histories().request_count(),
            4,
            "every request is recorded, including errors"
        );
        // Closing the cache also releases the detached disk-write worker's gauges.
        drop(cache);
        timeout(Duration::from_secs(10), async {
            while value(&registry, "feuer_disk_chunks", &[("state", "free")]) != 0.0 {
                yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(value(&registry, "feuer_disk_write_pending_bytes", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_write_queued_entries", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 0.0);
    }

    fn cache(memory_capacity: u64) -> (TempDir, Cache) {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::builder()
            .directory(directory.path())
            .disk_capacity(4 << 20)
            .memory_capacity(memory_capacity)
            .open()
            .unwrap();
        (directory, cache)
    }

    async fn wait_for_buffered(cache: &Cache, key: &(impl Hash + ?Sized)) {
        let disk = cache.inner.disk.as_ref().unwrap();
        let hash = ObjectKeyHash::from_hash(key);
        timeout(Duration::from_secs(10), async {
            while !disk.contains(&hash) {
                yield_now().await;
            }
        })
        .await
        .expect("disk write was not buffered");
    }

    #[tokio::test]
    async fn shared_io_queues_preserve_independent_whole_object_caches_in_one_directory() {
        let directory = tempfile::tempdir().unwrap();
        let (metrics_registry, backend) = registry();
        let queues = IoQueues::new().unwrap();
        let first = Cache::builder()
            .directory(directory.path())
            .disk_capacity(4 << 20)
            .memory_capacity(1 << 20)
            .access_count_half_life(2)
            .with_metrics(&backend)
            .with_io_queues(queues.clone())
            .open()
            .unwrap();
        // Queue sharing does not require the caches to use the same metrics configuration.
        let second = Cache::builder()
            .directory(directory.path())
            .disk_capacity(4 << 20)
            .memory_capacity(1 << 20)
            .file_name("objects")
            .access_count_half_life(4)
            .with_io_queues(queues)
            .open()
            .unwrap();
        assert!(!Arc::ptr_eq(
            &first.inner.memory.buffer_pool(),
            &second.inner.memory.buffer_pool()
        ));
        assert!(!Arc::ptr_eq(
            first.inner.memory.access_histories(),
            second.inner.memory.access_histories()
        ));
        for (cache, half_life) in [(&first, 2), (&second, 4)] {
            assert_eq!(cache.inner.memory.access_histories().half_life().get(), half_life);
            assert_eq!(
                cache.inner.disk.as_ref().unwrap().access_histories().half_life().get(),
                half_life
            );
        }
        let key = "object-v1";
        let hash = ObjectKeyHash::from(key);
        let first_bytes = Bytes::from_static(b"first object");
        let second_bytes = Bytes::from_static(b"second object");
        let first_download = first_bytes.clone();
        let second_download = second_bytes.clone();
        let (first_result, second_result) = tokio::join!(
            first.get_or_fetch(key, move || async move { Ok::<_, Infallible>(first_download) }),
            second.get_or_fetch(key, move || async move { Ok::<_, Infallible>(second_download) }),
        );
        assert_eq!(first_result.unwrap(), first_bytes);
        assert_eq!(second_result.unwrap(), second_bytes);
        assert_eq!(
            value(&metrics_registry, "feuer_lookup_total", &[("outcome", "callback")]),
            1.0
        );
        wait_for_buffered(&first, key).await;
        wait_for_buffered(&second, key).await;
        let (first_flush, second_flush) = tokio::join!(
            first.inner.disk.as_ref().unwrap().flush(),
            second.inner.disk.as_ref().unwrap().flush(),
        );
        first_flush.unwrap();
        second_flush.unwrap();
        assert!(first.inner.memory.remove(&hash));
        assert!(second.inner.memory.remove(&hash));
        let (first_result, second_result) = tokio::join!(
            first.get_or_fetch(key, || async { Err::<Bytes, _>("must hit first disk") }),
            second.get_or_fetch(key, || async { Err::<Bytes, _>("must hit second disk") }),
        );
        assert_eq!(first_result.unwrap(), first_bytes);
        assert_eq!(second_result.unwrap(), second_bytes);
        assert_eq!(
            value(&metrics_registry, "feuer_lookup_total", &[("outcome", "disk_hit")]),
            1.0
        );
        drop(first);
        assert!(second.inner.memory.remove(&hash));
        assert_eq!(
            second
                .get_or_fetch(key, || async { Err::<Bytes, _>("must hit surviving disk") })
                .await
                .unwrap(),
            second_bytes,
        );
    }

    #[tokio::test]
    async fn whole_object_buffer_and_disk_hits_promote_all_bytes() {
        let (_directory, cache) = cache(32);
        let key = ("whole-object".to_owned(), 100u64..106);
        let hash = ObjectKeyHash::from_hash(&key);
        let payload = Bytes::from_static(b"abcdef");
        let download = payload.clone();
        cache
            .get_or_fetch(&key, move || async move { Ok::<_, &str>(download) })
            .await
            .unwrap();
        wait_for_buffered(&cache, &key).await;
        for flush in [false, true] {
            if flush {
                cache.inner.disk.as_ref().unwrap().flush().await.unwrap();
            }
            for lookup_only in [false, true] {
                assert!(cache.inner.memory.remove(&hash));
                let bytes = if lookup_only {
                    cache.get(&key).await.unwrap()
                } else {
                    cache
                        .get_or_fetch(&key, || async { Err::<Bytes, _>("must hit disk") })
                        .await
                        .unwrap()
                };
                assert_eq!(bytes, payload);
                assert_eq!(cache.inner.memory.get(&hash).unwrap().as_ptr(), bytes.as_ptr());
            }
        }
    }

    #[tokio::test]
    async fn disk_promotion_charges_buffer_capacity_and_preserves_returned_bytes_after_removal() {
        let (_directory, cache) = cache(32);
        let key = "large-entry".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let disk = cache.inner.disk.as_ref().unwrap();
        disk.insert_batch(vec![(hash, Bytes::from(vec![0x77; 40 * 1024]))])
            .await
            .unwrap();
        let bytes = cache
            .get_or_fetch(&key, || async { Err::<Bytes, _>("disk hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(&bytes[..], &[0x77; 40 * 1024]);
        assert_eq!(cache.inner.memory.used_bytes(), 256 * 1024);
        assert_eq!(cache.inner.memory.get(&hash).unwrap().as_ptr(), bytes.as_ptr());
        assert_eq!(cache.inner.memory.buffer_pool().idle_bytes(), 0);
        assert!(cache.inner.memory.remove(&hash));
        assert_eq!(cache.inner.memory.used_bytes(), 0);
        assert_eq!(&bytes[..], &[0x77; 40 * 1024]);
    }

    #[tokio::test]
    async fn concurrent_disk_promotions_share_the_object_and_charge_once() {
        let (_directory, cache) = cache(8 << 20);
        let key = "shared-entry".to_owned();
        let hash = ObjectKeyHash::from(key.as_str());
        let payload = Bytes::from(vec![0x77; 40 * 1024]);
        let disk = cache.inner.disk.as_ref().unwrap();
        disk.insert_batch(vec![(hash, payload.clone())]).await.unwrap();
        disk.flush().await.unwrap();
        let (first, second) = tokio::join!(
            cache.get_or_fetch(&key, || async { Err::<Bytes, _>("disk hit must not fetch") }),
            cache.get_or_fetch(&key, || async { Err::<Bytes, _>("disk hit must not fetch") }),
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(first, payload);
        assert_eq!(second, payload);
        assert_eq!(first.as_ptr(), second.as_ptr());
        assert_eq!(cache.inner.memory.get(&hash).unwrap().as_ptr(), first.as_ptr());
        assert_eq!(
            cache.inner.memory.used_bytes(),
            cache.inner.memory.buffer_pool().idle_bytes() + 256 * 1024
        );
    }

    #[tokio::test]
    async fn callback_cancellation_keeps_the_recorded_request_without_inserting() {
        let (_directory, cache) = cache(32);
        let key = String::from("canceled");
        let history = cache.inner.memory.access_histories();
        let entered = Arc::new(Notify::new());
        let task = {
            let cache = cache.clone();
            let key = key.clone();
            let entered = entered.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch(&key, || async move {
                        entered.notify_one();
                        pending::<Result<Bytes, Infallible>>().await
                    })
                    .await
            })
        };
        entered.notified().await;
        assert_eq!(history.request_count(), 1, "recorded before the callback completes");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(cache.inner.memory.get(&ObjectKeyHash::from(key.as_str())).is_none());
        let disk = cache.inner.disk.as_ref().unwrap();
        assert!(!disk.contains(&ObjectKeyHash::from(key.as_str())));
        assert_eq!(history.request_count(), 1);
    }

    #[tokio::test]
    async fn opening_validates_disk_capacity_and_exclusive_file_ownership() {
        let (directory, cache) = cache(32);
        assert!(
            Cache::builder()
                .directory(cache.config().directory())
                .disk_capacity(cache.config().disk_capacity())
                .memory_capacity(cache.config().memory_capacity())
                .open()
                .is_err()
        );
        assert!(matches!(
            Cache::builder()
                .directory(directory.path())
                .disk_capacity(1024)
                .memory_capacity(32)
                .open(),
            Err(CacheOpenError::Disk(DiskCacheError::InvalidCapacity))
        ));
    }

    #[test]
    fn cache_handle_is_send_sync_static() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<Cache>();
    }

    #[tokio::test]
    async fn callback_and_memory_hit_share_the_object_without_creating_disk_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unused");
        let cache = Cache::builder().directory(&path).memory_capacity(32).open().unwrap();
        let key = String::from("object");
        let payload = Bytes::from_static(b"abcdefghij");
        let download = payload.clone();
        let result = cache
            .get_or_fetch(&key, move || async move { Ok::<_, Infallible>(download) })
            .await
            .unwrap();
        assert_eq!(result, payload);
        assert_eq!(result.as_ptr(), payload.as_ptr());

        let result = cache
            .get_or_fetch(&key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(result, payload);
        assert_eq!(result.as_ptr(), payload.as_ptr());
        assert!(cache.inner.disk.is_none());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn callback_errors_are_returned_without_retry_or_insertion() {
        let (_directory, cache) = cache(8);
        let key = String::from("object");
        let callback_count = Arc::new(AtomicU64::new(0));

        for expected_count in 1..=2 {
            let invocation_count = callback_count.clone();
            let error = cache
                .get_or_fetch(&key, move || async move {
                    invocation_count.fetch_add(1, Ordering::Relaxed);
                    Err::<Bytes, _>("source unavailable")
                })
                .await
                .unwrap_err();

            assert_eq!(error, GetOrFetchError::Callback("source unavailable"));
            assert_eq!(callback_count.load(Ordering::Relaxed), expected_count);
        }
    }

    #[tokio::test]
    async fn retains_oversized_callback_results() {
        let (_directory, cache) = cache(4);
        let key = String::from("object");
        let result = cache
            .get_or_fetch(&key, || async { Ok::<_, Infallible>(Bytes::from_static(b"abcde")) })
            .await
            .unwrap();
        assert_eq!(result, Bytes::from_static(b"abcde"));
        assert_eq!(cache.inner.memory.used_bytes(), 5);
        let hit = cache
            .get_or_fetch(&key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(hit, result);
        assert_eq!(hit.as_ptr(), result.as_ptr());
    }

    #[tokio::test]
    async fn a_racing_memory_insertion_is_not_replaced_by_the_callback() {
        let (_directory, cache) = cache(32);
        let key = String::from("object");
        let callback_entered = Arc::new(Notify::new());
        let release_callback = Arc::new(Notify::new());

        let pending = {
            let cache = cache.clone();
            let key = key.clone();
            let callback_entered = callback_entered.clone();
            let release_callback = release_callback.clone();
            tokio::spawn(async move {
                cache
                    .get_or_fetch(&key, move || async move {
                        callback_entered.notify_one();
                        release_callback.notified().await;
                        Ok::<_, Infallible>(Bytes::from(vec![b'x'; 6]))
                    })
                    .await
                    .unwrap()
            })
        };

        callback_entered.notified().await;
        let payload = Bytes::from_static(b"xxxxxx");
        cache
            .inner
            .memory
            .insert(ObjectKeyHash::from(key.as_str()), payload.clone(), payload.len());
        release_callback.notify_one();

        let returned = pending.await.unwrap();
        assert_eq!(returned, payload);
        let cached = cache
            .get_or_fetch(&key, || async { Err::<Bytes, _>("memory hit must not fetch") })
            .await
            .unwrap();
        assert_eq!(cached, payload);
        assert_eq!(cached.as_ptr(), payload.as_ptr());
        assert_ne!(returned.as_ptr(), cached.as_ptr());
        assert_eq!(cache.inner.memory.used_bytes(), payload.len() as u64);
    }
}
