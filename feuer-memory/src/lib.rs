//! Feuer's private in-memory range tier.
//!
//! This is one sharded, soft-capacity cache of downloaded ranges keyed by
//! fully compared immutable-object identities. Covering lookups return exactly
//! the requested bytes, bounded ageable access evidence is separate from
//! population, and shard-local retention and compaction remain private policy.

mod metrics;
mod store;

pub use metrics::MemoryMetrics;
pub use store::MemoryCache;
