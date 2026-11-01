//! ![Feuer — cost-aware tiered cache, with a flaming horse logo](https://raw.githubusercontent.com/PSeitz/feuer/main/full_logo.svg?v=cfeda6a)
//!
//! Feuer caches immutable objects in memory and on disk.
//!
//! On a cache miss, it calls your function to fetch or compute the object.
//! Eviction considers how often an object is requested, its size, and the cost of fetching it again.
//!
//! # Quick start
//!
//! Create a memory-only cache and fetch an object. This example runs on every supported
//! platform. Replace the callback body with your application's source access.
//!
//! ```
//! use std::convert::Infallible;
//! use bytes::Bytes;
//! use feuer::Cache;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let cache = Cache::builder()
//!         .memory_capacity(64 * 1024 * 1024)
//!         .open()
//!         .await?;
//!
//!     let bytes = cache
//!         .get_or_fetch("object-v1".to_owned(), || async {
//!             Ok::<_, Infallible>(Bytes::from_static(b"hello world"))
//!         })
//!         .await?;
//!
//!     assert_eq!(bytes.as_ref(), b"hello world");
//!     Ok(())
//! }
//! ```
//!
//! Capacities are in bytes. Memory capacity must be positive. Disk caching is disabled
//! until you set a nonzero [`disk_capacity`](CacheBuilder::disk_capacity).
//!
//! # How lookups work
//!
//! [`Cache::get_or_fetch`] checks memory first, then disk when enabled. A disk hit is
//! checksum-verified and promoted to memory. On a miss, Feuer calls your callback,
//! caches the result in memory, and queues a disk write when disk caching is enabled.
//!
//! Use [`Cache::get`] to check the cache without fetching from the source.
//! Cloning a [`Cache`] shares its contents and in-flight work, so handles can be used
//! across tasks. Callbacks that share a fetch must be interchangeable.
//!
//! # Object identity and recovery
//!
//! Each key must identify one immutable payload. Use a new key when the content changes.
//! Feuer has no invalidation API. Your application remains responsible for source retries
//! and concurrency limits.
//!
//! Treat cached data as disposable. Disk writes and recovery are best-effort, and your
//! source must remain able to supply objects after eviction or a restart. Each disk cache
//! needs its own backing file. See [`CacheConfig`] for capacity and payload-version rules.
//!
//! # Disk caching and retention
//!
//! To enable SSD caching, configure a nonzero disk capacity and open the cache on Linux
//! with Tokio, `io_uring`, and direct-I/O support:
//!
//! ```no_run
//! # async fn example() -> Result<(), feuer::CacheOpenError> {
//! let cache = feuer::Cache::builder()
//!     .memory_capacity(64 * 1024 * 1024)
//!     .disk_capacity(1024 * 1024 * 1024)
//!     .directory("feuer_cache")
//!     .file_name("objects")
//!     .open()
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! This stores disk data in `feuer_cache/objects`. The directory defaults to `feuer_cache` and the
//! filename to `data`. Opening waits for disk metadata recovery.
//! Memory-only operation works on every supported platform without a Tokio runtime.
//! Access tracing through [`Cache::with_trace`] requires Tokio.
//!
//! The default retention policy is [`AvoidedRetrievalCostScorer`]. For workload-specific
//! decisions, implement [`RetentionScorer`] and configure each tier through [`CacheBuilder`].

#![doc(html_logo_url = "https://raw.githubusercontent.com/PSeitz/feuer/main/horse_logo_transparent.svg?v=2")]

mod cache;
mod config;
mod metrics;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use cache::{BytesAndMemoryCharge, Cache, CacheBuilder, CacheOpenError, GetOrFetchError};
pub use config::{CacheConfig, CacheConfigError};
pub use feuer_historian::ObjectAccessHistories;
pub use feuer_memory::{
    AlignedBuffer,
    retention::{AvoidedRetrievalCostScorer, RetentionScorer},
};
#[cfg(target_os = "linux")]
pub use feuer_storage::{DiskCacheError, IoQueues};
pub use feuer_types::ObjectKeyHash;

/// An error opening the disk cache on a platform without disk caching support.
#[cfg(not(target_os = "linux"))]
#[derive(Debug, thiserror::Error)]
pub enum DiskCacheError {
    /// Disk caching requires Linux. Set disk capacity to zero for memory-only operation.
    #[error("disk caching requires Linux; set disk capacity to zero for memory-only operation")]
    UnsupportedPlatform,
}

/// I/O queues are available only on Linux. Omit queue configuration on other platforms.
#[cfg(not(target_os = "linux"))]
#[derive(Clone, Debug)]
pub enum IoQueues {}

/// The UTF-8 identity supplied at the public cache boundary.
/// Internally only its XXH3-128 hash is stored. Colliding keys identify the same object.
pub type ObjectKey = String;
