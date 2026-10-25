//! Feuer's private in-memory object tier.
//!
//! A sharded, soft-capacity cache keyed by 128-bit immutable-object key hashes.
//! Access evidence is separate from insertion. Retention scores are customizable
//! while reclamation remains shard-local.

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
