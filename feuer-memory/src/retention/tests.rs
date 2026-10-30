use std::num::NonZeroU64;

use super::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn scores_match_avoided_cost_per_charged_byte(
        payload_len in any::<u64>(),
        entry_size in any::<u64>(),
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
        for (payload_len, entry_size) in [(0, 0), (u64::MAX, u64::MAX), (payload_len, entry_size)] {
            let savings = (scorer.cost_per_request + payload_len as f64 * scorer.cost_per_requested_byte).max(0.0);
            prop_assert_eq!(
                scorer.score(&key, payload_len, entry_size, &histories),
                demand * savings / entry_size.max(1) as f64
            );
            prop_assert_eq!(scorer.score(&ObjectKeyHash::from("unknown"), payload_len, entry_size, &histories), 0.0);
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
    assert_eq!(scorer.score(&key, 10, 10, &histories), 1.0);
    for _ in 0..2 {
        histories.record_request(&ObjectKeyHash::from("other"));
    }
    assert_eq!(scorer.score(&key, 10, 10, &histories), 0.5);
    assert_eq!(scorer.score(&key, 10, 20, &histories), 0.25);
}
