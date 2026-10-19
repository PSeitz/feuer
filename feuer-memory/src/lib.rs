//! Feuer's private in-memory object tier.
//!
//! This is one sharded, soft-capacity cache of complete objects keyed by
//! 128-bit immutable-object key hashes. Lookups return the complete payload,
//! access evidence is separate from insertion, and
//! retention scores are customizable while reclamation remains shard-local.

mod buffer;
mod metrics;
pub mod retention;
mod store;
#[cfg(test)]
#[path = "../../test_support/metrics.rs"]
mod test_metrics;

pub use buffer::{AlignedBuffer, BUFFER_ALIGNMENT, BufferPool, read_idle_buffer_pool_percent};
pub use metrics::MemoryMetrics;
pub use store::MemoryCache;
