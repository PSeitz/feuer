//! Feuer is a tiered cache for immutable objects and their byte ranges.
//!
//! The current public boundary provides whole objects or exact requested byte ranges, opaque
//! object identities, a soft-capacity memory tier, integrity-checked disk hits,
//! best-effort disk writes through a 512-entry queue, and per-call asynchronous download callbacks.
//! Zero disk capacity disables the disk tier. With disk enabled, opening requires
//! Linux direct I/O and io_uring, and waits for every disk shard's metadata recovery.

mod access_trace;
mod cache;
mod config;
mod metrics;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use cache::{GetOrFetchError, TieredMemoryDiskCache};
pub use config::{CacheConfig, CacheConfigError};
pub use feuer_memory::AlignedBuffer;
#[cfg(target_os = "linux")]
pub use feuer_storage::DiskCacheError;
pub use feuer_types::{ByteRange, Download, DownloadError, InvalidByteRange};

/// The complete UTF-8 identity supplied at the public cache boundary.
/// Internally only its XXH3-128 hash is stored; collisions are not verified against full keys.
pub type ObjectKey = String;
