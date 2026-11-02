//! Shared retention policy for the memory and disk tiers.

use feuer_historian::ObjectAccessHistories;
use feuer_utils::ObjectKeyHash;

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

/// Scores objects by fetch cost, weighted by demand and divided by payload size.
///
/// Decayed request counts estimate future demand. Payload length represents keeping cost.
/// Each tier uses total request history, independently of the contents of other tiers.
///
/// ```text
/// score = decayed_request_count * (cost_per_request + payload_len * cost_per_byte)
///         / max(payload_len, 1)
/// ```
///
/// The coefficients estimate fetching from the fallback. This model treats cache reads
/// as free. Use [`CacheReadSavingsScorer`] to account for both retrieval paths.
/// Costs must be finite, nonnegative, and use one consistent unit, such as I/O time or
/// equivalent bytes. They must keep intermediate costs and scores finite for the workload.
/// Unknown keys score zero. Empty payloads use a one-byte denominator.
///
/// Defaults are zero fixed cost and one cost unit per byte. For nonempty objects, the
/// default score equals the decayed request count. This is a frequency-based default.
#[derive(Debug)]
pub struct FetchCostScorer {
    /// Fetch cost per request, independent of payload size. Defaults to zero.
    pub cost_per_request: f64,
    /// Fetch cost per payload byte. Defaults to one.
    pub cost_per_byte: f64,
}

impl Default for FetchCostScorer {
    fn default() -> Self {
        Self {
            cost_per_request: 0.0,
            cost_per_byte: 1.0,
        }
    }
}

impl RetentionScorer for FetchCostScorer {
    fn score(&self, key: &ObjectKeyHash, payload_len: u64, histories: &ObjectAccessHistories) -> f64 {
        let fetch_cost = self.cost_per_request + payload_len as f64 * self.cost_per_byte;
        histories.decayed_request_count(key) * fetch_cost / payload_len.max(1) as f64
    }
}

/// Scores objects by the cost saved by cache reads, weighted by demand and divided by payload size.
///
/// Each cache read replaces a fetch from the assumed fallback. The caller supplies linear
/// cost estimates for both paths. For disk, the fallback is the source. For memory, choose
/// an assumed disk or source fallback. Costs are independent of the contents of other tiers.
/// Decayed total request counts estimate future demand.
///
/// ```text
/// fallback_cost = fallback_cost_per_request + payload_len * fallback_cost_per_byte
/// cache_read_cost = cache_read_cost_per_request + payload_len * cache_read_cost_per_byte
/// score = decayed_request_count * (fallback_cost - cache_read_cost) / max(payload_len, 1)
/// ```
///
/// Both paths' costs must be finite, nonnegative, and use the same unit. They must keep
/// intermediate costs and scores finite for the workload. Negative savings produce negative
/// scores, prioritizing eviction when cache reads cost more than fallback fetches.
/// Payload length represents keeping cost. Empty payloads
/// use a one-byte denominator. Unknown keys score zero.
#[derive(Debug)]
pub struct CacheReadSavingsScorer {
    /// Fallback fetch cost per request, independent of payload size.
    pub fallback_cost_per_request: f64,
    /// Fallback fetch cost per payload byte.
    pub fallback_cost_per_byte: f64,
    /// Cost of reading from the cache per request, independent of payload size.
    pub cache_read_cost_per_request: f64,
    /// Cost of reading from the cache per payload byte.
    pub cache_read_cost_per_byte: f64,
}

impl RetentionScorer for CacheReadSavingsScorer {
    fn score(&self, key: &ObjectKeyHash, payload_len: u64, histories: &ObjectAccessHistories) -> f64 {
        let fallback_cost = self.fallback_cost_per_request + payload_len as f64 * self.fallback_cost_per_byte;
        let cache_read_cost = self.cache_read_cost_per_request + payload_len as f64 * self.cache_read_cost_per_byte;
        histories.decayed_request_count(key) * (fallback_cost - cache_read_cost) / payload_len.max(1) as f64
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

#[cfg(test)]
use proptest::prelude::*;
#[cfg(test)]
use std::num::NonZeroU64;

#[cfg(test)]
proptest! {
    #[test]
    fn fetch_cost_scores_preserve_previous_nonnegative_scores(
        payload_len in any::<u64>(),
        fixed_cost in 0i32..=1_000_000,
        byte_cost in 0i32..=16,
        requests in 0i32..8,
    ) {
        let histories = ObjectAccessHistories::with_half_life(NonZeroU64::MIN);
        let key = ObjectKeyHash::from("object");
        for _ in 0..requests {
            histories.record_request(&key);
        }
        // One half-life between requests makes this geometric sum exact.
        let demand = 2.0 - 2.0_f64.powi(1 - requests);
        let scorer = FetchCostScorer {
            cost_per_request: fixed_cost as f64 / 8.0,
            cost_per_byte: byte_cost as f64 / 8.0,
        };
        for payload_len in [0, u64::MAX, payload_len] {
            // Compare against the previous formula, including its clamp.
            let expected = demand * (fixed_cost as f64 / 8.0 + payload_len as f64 * (byte_cost as f64 / 8.0)).max(0.0)
                / payload_len.max(1) as f64;
            prop_assert_eq!(scorer.score(&key, payload_len, &histories).to_bits(), expected.to_bits());
            prop_assert_eq!(scorer.score(&ObjectKeyHash::from("unknown"), payload_len, &histories), 0.0);
            let savings_scorer = CacheReadSavingsScorer {
                fallback_cost_per_request: scorer.cost_per_request,
                fallback_cost_per_byte: scorer.cost_per_byte,
                cache_read_cost_per_request: 0.0,
                cache_read_cost_per_byte: 0.0,
            };
            prop_assert_eq!(savings_scorer.score(&key, payload_len, &histories).to_bits(), expected.to_bits());
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
    let scorer = FetchCostScorer::default();
    assert_eq!(scorer.cost_per_request, 0.0);
    assert_eq!(scorer.cost_per_byte, 1.0);
    assert_eq!(scorer.score(&key, 10, &histories), 1.0);
    for _ in 0..2 {
        histories.record_request(&ObjectKeyHash::from("other"));
    }
    assert_eq!(scorer.score(&key, 10, &histories), 0.5);
    assert_eq!(scorer.score(&key, 20, &histories), 0.5);
    assert_eq!(scorer.score(&key, 0, &histories), 0.0);
}

#[test]
fn source_minus_disk_costs_determine_retention_benefit() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_request(&key);
    // Source retrieval costs 100 + 3 * size. Disk retrieval costs 120 + size.
    let scorer = CacheReadSavingsScorer {
        fallback_cost_per_request: 100.0,
        fallback_cost_per_byte: 3.0,
        cache_read_cost_per_request: 120.0,
        cache_read_cost_per_byte: 1.0,
    };
    assert_eq!(scorer.score(&key, 5, &histories), -2.0);
    assert_eq!(scorer.score(&key, 10, &histories), 0.0);
    assert_eq!(scorer.score(&key, 20, &histories), 1.0);
    assert_eq!(scorer.score(&key, 0, &histories), -20.0);
    assert_eq!(scorer.score(&ObjectKeyHash::from("unknown"), 20, &histories), 0.0);

    let cheaper_reads = CacheReadSavingsScorer {
        cache_read_cost_per_request: 80.0,
        ..scorer
    };
    assert_eq!(cheaper_reads.score(&key, 20, &histories), 3.0);
    assert_eq!(cheaper_reads.score(&key, 0, &histories), 20.0);
}

#[test]
fn fixed_fetch_cost_favors_smaller_objects_at_equal_demand() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_request(&key);
    let scorer = FetchCostScorer {
        cost_per_request: 100.0,
        cost_per_byte: 1.0,
    };
    assert_eq!(scorer.score(&key, 10, &histories), 11.0);
    assert_eq!(scorer.score(&key, 20, &histories), 6.0);
    assert_eq!(scorer.score(&key, 0, &histories), 100.0);
}
