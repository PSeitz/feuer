//! Shared retention policy for the memory and disk tiers.

use std::{cmp::Ordering, sync::LazyLock};

use feuer_historian::ObjectAccessHistories;
use feuer_types::{ByteRange, ObjectKeyHash, config::read_env_number};

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

/// Compares decayed retrieval value per payload byte without division.
pub fn compare_cost_per_byte(left_cost: f64, left_bytes: u64, right_cost: f64, right_bytes: u64) -> Ordering {
    (left_cost * right_bytes as f64).total_cmp(&(right_cost * left_bytes as f64))
}

/// Fixed source-request cost as equivalent transferred bytes; zero scores only bytes.
/// Reads `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` once on first use, defaulting to
/// 10,000,000 (125 ms at 80 MB/s). Accepts size suffixes; panics unless the value fits `u64`.
pub static FIXED_RETRIEVAL_EQUIVALENT_BYTES: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES", 10_000_000, 0).unwrap_or_else(|error| panic!("{error}"))
});

/// Sums retrieval costs for contained requests, weighted by decayed access counts.
/// Whole-object requests use the cached payload length. Unknown keys have zero cost.
/// Eviction divides this cost by the memory allocation charge or disk payload length.
pub fn decayed_retrieval_cost(histories: &ObjectAccessHistories, key: &ObjectKeyHash, cached_range: ByteRange) -> f64 {
    histories.with_access_counts(key, |counts, clock| {
        let fixed_retrieval_cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64;
        counts
            .iter()
            .map(|(requested, access_count)| (requested.unwrap_or(cached_range), access_count))
            .filter(|(requested_range, _)| cached_range.contains(*requested_range))
            .map(|(requested_range, access_count)| {
                access_count.decayed_count(clock) * (fixed_retrieval_cost + requested_range.len() as f64)
            })
            .sum()
    })
}

#[cfg(test)]
mod tests;
