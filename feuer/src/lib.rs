//! Feuer is a tiered cache for ranges of immutable objects.
//!
//! The current public boundary provides exact requested byte ranges, opaque
//! object identities, a soft-capacity memory tier, integrity-checked disk hits,
//! bounded best-effort disk population, and per-call asynchronous download callbacks.
//! Opening a cache requires Linux direct I/O and io_uring. Recovery is not implemented.

mod cache;
mod config;
mod metrics;
#[cfg(target_os = "linux")]
mod population;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use cache::{GetOrFetchError, TieredMemoryDiskCache};
pub use config::{CacheConfig, CacheConfigError};
#[cfg(target_os = "linux")]
pub use feuer_storage::DiskRangeCacheError;
pub use feuer_types::{ByteRange, Download, DownloadError, EvictionPolicy, InvalidByteRange, ObjectKey};
