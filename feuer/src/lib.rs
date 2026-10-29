//! Feuer is a tiered cache for immutable objects.
//!
//! The public boundary provides object payloads, opaque object identities,
//! a soft-capacity memory tier, integrity-checked disk hits,
//! best-effort disk writes through a 512-entry queue, and per-call asynchronous download callbacks.
//! Zero disk capacity disables the disk tier on every platform without requiring a Tokio runtime.
//! With disk enabled, opening requires Linux direct I/O and io_uring, and waits for metadata recovery.
//! Other platforms reject nonzero disk capacity. Access tracing requires Tokio.

mod access_trace;
mod cache;
mod config;
mod metrics;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use cache::{Cache, GetOrFetchError};
pub use config::{CacheConfig, CacheConfigError, SampledEvictionConfig};
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

/// I/O queues are available only on Linux. Pass `None` to cache constructors on other platforms.
#[cfg(not(target_os = "linux"))]
#[derive(Clone, Debug)]
pub enum IoQueues {}

/// The UTF-8 identity supplied at the public cache boundary.
/// Internally only its XXH3-128 hash is stored. Colliding keys identify the same object.
pub type ObjectKey = String;
