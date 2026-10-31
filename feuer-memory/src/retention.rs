//! Shared retention policy for the memory and disk tiers.

use feuer_historian::ObjectAccessHistories;
use feuer_types::ObjectKeyHash;

/// Scores how valuable a cached object is to retain.
///
/// Scores must be finite. The lowest-scoring eligible sampled entry is evicted.
/// Normalization is up to the scorer. `payload_len` is the object's data size, possibly zero.
/// Each tier can use its own scorer or share one. It may own application metadata keyed by [`ObjectKeyHash`].
/// Calls run concurrently under shard locks: avoid I/O, cache re-entry, and holding
/// application locks across cache operations if scoring acquires those locks.
/// The cache owns sampling and removal. The application owns custom metadata.
pub trait RetentionScorer: Send + Sync {
    /// Returns the retention score for an object's payload length and request history.
    fn score(&self, key: &ObjectKeyHash, payload_len: u64, histories: &ObjectAccessHistories) -> f64;
}

/// Scores entries by retrieval cost avoided per payload byte, weighted by decayed request counts.
///
/// The object's decayed request count multiplies
/// `max(cost_per_request + payload_len * cost_per_requested_byte, 0)`.
/// A zero payload length uses a one-byte denominator.
/// Costs must use one consistent unit, such as equivalent bytes, I/O time, or CPU time.
/// Coefficients must be finite and keep scores finite over the application's sizes and demand.
///
/// Defaults are zero fixed cost and one cost unit per byte, preserving byte-weighted demand.
/// With a nonzero payload length, the default score equals the object's
/// decayed count. Using the default half-life and current decay approximation:
///
/// | Matching request frequency | Score approached immediately after a matching request |
/// | --- | ---: |
/// | Every request | 524,288 |
/// | Every second request | 262,144 |
///
/// Separate tier scorers can model memory's assumed fallback cost and disk's source-minus-disk
/// cost. Negative savings are clamped to zero. Both use request history.
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
    fn score(&self, key: &ObjectKeyHash, payload_len: u64, histories: &ObjectAccessHistories) -> f64 {
        decayed_avoided_retrieval_cost(
            histories,
            key,
            payload_len,
            self.cost_per_request,
            self.cost_per_requested_byte,
        ) / payload_len.max(1) as f64
    }
}

/// Default maximum entries inspected per retention-policy sample.
pub const RECLAIM_SAMPLE_SIZE: usize = 64;

/// Chooses up to `sample_size` candidates, advancing through the list and wrapping at its end.
pub fn sample_candidates<'a, T>(
    next_sample_start: &mut usize,
    candidates: &'a [T],
    sample_size: usize,
) -> impl Iterator<Item = &'a T> {
    let candidate_count = candidates.len();
    let sample_start = *next_sample_start % candidate_count.max(1);
    if candidate_count > 0 {
        *next_sample_start = (sample_start + candidate_count.min(sample_size)) % candidate_count;
    }
    let (before_start, from_start) = candidates.split_at(sample_start);
    from_start.iter().chain(before_start).take(sample_size)
}

/// Weights the object's avoided retrieval cost by its decayed request count.
/// Unknown keys have zero cost.
/// The default scorer divides this cost by the payload length, using one for empty payloads.
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
use proptest::prelude::*;
#[cfg(test)]
use std::num::NonZeroU64;

#[cfg(test)]
proptest! {
    #[test]
    fn scores_match_avoided_cost_per_payload_byte(
        payload_len in any::<u64>(),
        fixed_cost in -1_000_000i32..=1_000_000,
        byte_cost in -16i32..=16,
        requests in 0i32..8,
    ) {
        let histories = ObjectAccessHistories::with_half_life(NonZeroU64::MIN);
        let key = ObjectKeyHash::from("object");
        for _ in 0..requests {
            histories.record_request(&key);
        }
        // One half-life between requests makes this geometric sum exact.
        let demand = 2.0 - 2.0_f64.powi(1 - requests);
        let scorer = AvoidedRetrievalCostScorer {
            cost_per_request: fixed_cost as f64 / 8.0,
            cost_per_requested_byte: byte_cost as f64 / 8.0,
        };
        for payload_len in [0, u64::MAX, payload_len] {
            let savings = (scorer.cost_per_request + payload_len as f64 * scorer.cost_per_requested_byte).max(0.0);
            prop_assert_eq!(
                scorer.score(&key, payload_len, &histories),
                demand * savings / payload_len.max(1) as f64
            );
            prop_assert_eq!(scorer.score(&ObjectKeyHash::from("unknown"), payload_len, &histories), 0.0);
        }
    }
}

#[test]
fn samples_at_most_64_candidates() {
    let mut cursor = 0;
    let candidates: Vec<_> = (0..100).collect();
    let mut sample = |count| {
        sample_candidates(&mut cursor, &candidates[..count], RECLAIM_SAMPLE_SIZE)
            .copied()
            .collect::<Vec<_>>()
    };
    assert_eq!(sample(100), (0..64).collect::<Vec<_>>());
    assert_eq!(sample(100), (64..100).chain(0..28).collect::<Vec<_>>());
    assert_eq!(sample(0), []);
    assert_eq!(sample(3), [1, 2, 0]);
}

#[test]
fn scores_follow_decayed_object_demand() {
    let histories = ObjectAccessHistories::with_half_life(NonZeroU64::new(2).unwrap());
    let key = ObjectKeyHash::from("object");
    histories.record_request(&key);
    let scorer = AvoidedRetrievalCostScorer::default();
    assert_eq!(scorer.cost_per_request, 0.0);
    assert_eq!(scorer.cost_per_requested_byte, 1.0);
    assert_eq!(scorer.score(&key, 10, &histories), 1.0);
    for _ in 0..2 {
        histories.record_request(&ObjectKeyHash::from("other"));
    }
    assert_eq!(scorer.score(&key, 10, &histories), 0.5);
    assert_eq!(scorer.score(&key, 20, &histories), 0.5);
}
