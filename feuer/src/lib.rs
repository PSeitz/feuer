//! Feuer is a tiered cache for ranges of immutable objects.
//!
//! The current public boundary provides exact requested byte ranges, opaque
//! object identities, a soft-capacity memory tier, integrity-checked disk hits,
//! bounded best-effort disk writes, and per-call asynchronous download callbacks.
//! Opening requires Linux direct I/O and io_uring. Disk recovery runs incrementally in the background.

mod cache;
mod config;
#[cfg(target_os = "linux")]
mod disk_write;
mod metrics;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use cache::{GetOrFetchError, TieredMemoryDiskCache};
pub use config::{CacheConfig, CacheConfigError};
#[cfg(target_os = "linux")]
pub use feuer_storage::DiskRangeCacheError;
pub use feuer_types::{ByteRange, Download, DownloadError, InvalidByteRange};

/// The complete UTF-8 identity supplied at the public cache boundary.
/// Internally only its XXH3-128 hash is retained; collisions are not verified against full keys.
pub type ObjectKey = String;
