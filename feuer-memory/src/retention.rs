//! Shared retention policy for the memory and disk tiers.

use feuer_historian::ObjectAccessHistories;
use feuer_types::{ByteRange, ObjectKeyHash};

/// Scores how valuable a cached byte range is to retain.
///
/// Scores must be finite. The lowest-scoring eligible sampled entry is evicted.
/// Normalization is up to the scorer. `charged_bytes` is memory allocation charge
/// or disk payload length, possibly zero.
/// Both tiers share the scorer. It may own application metadata keyed by [`ObjectKeyHash`].
/// Calls run concurrently under shard locks: avoid I/O, cache re-entry, and holding
/// application locks across cache operations if scoring acquires those locks.
/// The cache owns sampling, trimming, and removal. The application owns custom metadata.
pub trait RetentionScorer: Send + Sync {
    /// Returns the retention score for one object's cached range and tier-specific byte charge.
    fn score(
        &self,
        key: &ObjectKeyHash,
        cached_range: ByteRange,
        charged_bytes: u64,
        histories: &ObjectAccessHistories,
    ) -> f64;
}

/// Scores cached ranges by decayed retrieval cost per charged byte.
/// Empty entries use a one-byte denominator to keep scores finite.
#[derive(Debug, Default)]
pub struct RetrievalCostScorer {
    /// Fixed source-request cost in equivalent transferred bytes. Defaults to zero.
    /// Zero scores only requested bytes.
    pub fixed_retrieval_equivalent_bytes: u64,
}

impl RetentionScorer for RetrievalCostScorer {
    fn score(
        &self,
        key: &ObjectKeyHash,
        cached_range: ByteRange,
        charged_bytes: u64,
        histories: &ObjectAccessHistories,
    ) -> f64 {
        decayed_retrieval_cost(histories, key, cached_range, self.fixed_retrieval_equivalent_bytes)
            / charged_bytes.max(1) as f64
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

/// Sums retrieval costs for contained requests, weighted by decayed request counts.
/// Whole-object requests use the cached payload length. Unknown keys have zero cost.
/// Eviction divides this cost by the memory allocation charge or disk payload length.
pub fn decayed_retrieval_cost(
    histories: &ObjectAccessHistories,
    key: &ObjectKeyHash,
    cached_range: ByteRange,
    fixed_retrieval_equivalent_bytes: u64,
) -> f64 {
    histories.read_request_counts(key, |request_counts, request_clock| {
        let fixed_retrieval_cost = fixed_retrieval_equivalent_bytes as f64;
        request_counts
            .iter()
            .map(|(requested, request_count)| (requested.unwrap_or(cached_range), request_count))
            .filter(|(requested_range, _)| cached_range.contains(*requested_range))
            .map(|(requested_range, request_count)| {
                request_count.decayed_count(request_clock) * (fixed_retrieval_cost + requested_range.len() as f64)
            })
            .sum()
    })
}

#[cfg(test)]
mod tests;
