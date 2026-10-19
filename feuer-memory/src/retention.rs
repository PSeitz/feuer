//! Shared retention policy for the memory and disk tiers.

use feuer_historian::ObjectAccessHistories;
use feuer_types::ObjectKeyHash;

/// Scores how valuable a cached object is to retain.
///
/// Scores must be finite. The lowest-scoring eligible sampled entry is evicted.
/// Normalization is up to the scorer. `entry_size_bytes` is the entry's size for this tier:
/// memory allocation charge or disk payload length, possibly zero.
/// Each tier can use its own scorer or share one. It may own application metadata keyed by [`ObjectKeyHash`].
/// Calls run concurrently under shard locks: avoid I/O, cache re-entry, and holding
/// application locks across cache operations if scoring acquires those locks.
/// The cache owns sampling and removal. The application owns custom metadata.
pub trait RetentionScorer: Send + Sync {
    /// Returns the retention score for one object's payload length and tier-specific entry size.
    fn score(
        &self,
        key: &ObjectKeyHash,
        payload_len: u64,
        entry_size_bytes: u64,
        histories: &ObjectAccessHistories,
    ) -> f64;
}

/// Scores entries by retrieval cost avoided per byte of entry size, weighted by decayed request counts.
///
/// The object's decayed request count multiplies
/// `max(cost_per_request + payload_len * cost_per_requested_byte, 0)`.
/// Empty entries use a one-byte denominator.
/// Costs must use one consistent unit, such as equivalent bytes, I/O time, or CPU time.
/// Coefficients must be finite and keep scores finite over the application's sizes and demand.
///
/// Defaults are zero fixed cost and one cost unit per byte, preserving byte-weighted demand.
/// With a nonempty request whose size equals the entry size, the default score equals its
/// decayed count. Using the default half-life and current decay approximation:
///
/// | Matching request frequency | Score approached immediately after a matching request |
/// | --- | ---: |
/// | Every request | 524,288 |
/// | Every second request | 262,144 |
///
/// Separate tier scorers can model memory's assumed fallback cost and disk's source-minus-disk
/// cost. Negative savings are clamped to zero. Both use caller-demand history.
/// This linear model does not inspect disk residency or model alignment and whole-entry reads.
#[derive(Debug)]
pub struct AvoidedRetrievalCostScorer {
    /// Cost avoided once per request, regardless of requested byte count. Defaults to zero.
    pub cost_per_request: f64,
    /// Cost avoided for each requested byte. Defaults to one.
    pub cost_per_requested_byte: f64,
}

impl Default for AvoidedRetrievalCostScorer {
    fn default() -> Self {
        Self {
            cost_per_request: 0.0,
            cost_per_requested_byte: 1.0,
        }
    }
}

impl RetentionScorer for AvoidedRetrievalCostScorer {
    fn score(
        &self,
        key: &ObjectKeyHash,
        payload_len: u64,
        entry_size_bytes: u64,
        histories: &ObjectAccessHistories,
    ) -> f64 {
        decayed_avoided_retrieval_cost(
            histories,
            key,
            payload_len,
            self.cost_per_request,
            self.cost_per_requested_byte,
        ) / entry_size_bytes.max(1) as f64
    }
}

/// Default maximum entries inspected in one retention-policy sample.
pub const RECLAIM_SAMPLE_SIZE: usize = 64;

/// Chooses up to `sample_size` candidates, advancing through the list and wrapping at its end.
pub fn sample_candidates<'a, T>(
    next_candidate: &mut usize,
    candidates: &'a [T],
    sample_size: usize,
) -> impl Iterator<Item = &'a T> {
    let candidate_count = candidates.len();
    let sample_start = *next_candidate % candidate_count.max(1);
    if candidate_count > 0 {
        *next_candidate = (sample_start + candidate_count.min(sample_size)) % candidate_count;
    }
    let (before_start, from_start) = candidates.split_at(sample_start);
    from_start.iter().chain(before_start).take(sample_size)
}

/// Weights the object's avoided retrieval cost by its decayed request count.
/// Unknown keys have zero cost.
/// Eviction divides this cost by the memory allocation charge or disk payload length.
pub fn decayed_avoided_retrieval_cost(
    histories: &ObjectAccessHistories,
    key: &ObjectKeyHash,
    payload_len: u64,
    cost_per_request: f64,
    cost_per_requested_byte: f64,
) -> f64 {
    histories.decayed_request_count(key) * (cost_per_request + payload_len as f64 * cost_per_requested_byte).max(0.0)
}

#[cfg(test)]
mod tests;
