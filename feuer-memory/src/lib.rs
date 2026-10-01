//! Feuer's private in-memory range tier.
//!
//! This is one sharded, soft-capacity cache of downloaded ranges keyed by
//! 128-bit immutable-object key hashes. Covering lookups return exactly
//! the requested bytes, access evidence is separate from insertion, and
//! shard-local retention and range trimming remain private policy.

mod buffer;
mod metrics;
mod store;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use buffer::{AlignedBuffer, BUFFER_ALIGNMENT, BufferPool};
pub use metrics::MemoryMetrics;
pub use store::MemoryCache;
