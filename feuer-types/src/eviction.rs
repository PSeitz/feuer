//! Eviction policy selection.

mod s3fifo;

#[doc(hidden)]
pub use s3fifo::S3Fifo;

/// Policy used independently by each memory and disk shard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Sample recent retrieval cost per payload byte; memory victims may be trimmed.
    #[default]
    CostAware,
    /// Byte-weighted S3-FIFO with a 10% small queue and a 90% ghost history.
    /// Only successful accesses to resident entries increase their frequency.
    S3Fifo,
}
