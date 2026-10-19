use super::*;

#[test]
fn score_combines_costs_per_request_and_payload_byte() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_access(&key);
    assert_eq!(AvoidedRetrievalCostScorer::default().cost_per_request, 0.0);
    assert_eq!(AvoidedRetrievalCostScorer::default().cost_per_requested_byte, 1.0);
    for (cost_per_request, cost_per_requested_byte) in [(0.0, 1.0), (256_000.0, 1.0), (2.0, 0.5), (4.0, 0.0)] {
        let scorer = AvoidedRetrievalCostScorer {
            cost_per_request,
            cost_per_requested_byte,
        };
        assert_eq!(
            scorer.score(&key, 4, 8, &histories),
            (cost_per_request + 4.0 * cost_per_requested_byte) / 8.0
        );
        assert_eq!(scorer.score(&ObjectKeyHash::from("unknown"), 4, 8, &histories), 0.0);
    }
}

#[test]
fn negative_savings_are_clamped_to_zero() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_access(&key);
    histories.record_access(&key);
    let scorer = AvoidedRetrievalCostScorer {
        cost_per_request: 4.0,
        cost_per_requested_byte: -1.0,
    };
    let demand = histories.decayed_request_count(&key);
    assert_eq!(scorer.score(&key, 1, 10, &histories), demand * 3.0 / 10.0);
    assert_eq!(scorer.score(&key, 10, 10, &histories), 0.0);
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
fn scores_use_payload_length_and_handle_empty_objects() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    let scorer = AvoidedRetrievalCostScorer::default();
    assert_eq!(scorer.score(&key, 0, 0, &histories), 0.0);
    histories.record_access(&key);
    for cost_per_request in [0, 10_000_000] {
        let scorer = AvoidedRetrievalCostScorer {
            cost_per_request: cost_per_request as f64,
            ..Default::default()
        };
        for payload_len in [0, 100, u64::MAX] {
            let cost = cost_per_request as f64 + payload_len as f64;
            assert_eq!(
                scorer.score(&key, payload_len, payload_len, &histories),
                cost / payload_len.max(1) as f64
            );
        }
    }
}

#[test]
fn scores_follow_decayed_object_demand() {
    let histories = ObjectAccessHistories::with_half_life(2);
    let key = ObjectKeyHash::from("object");
    histories.record_access(&key);
    let scorer = AvoidedRetrievalCostScorer::default();
    assert_eq!(scorer.score(&key, 10, 10, &histories), 1.0);
    for _ in 0..2 {
        histories.record_access(&ObjectKeyHash::from("other"));
    }
    assert_eq!(scorer.score(&key, 10, 10, &histories), 0.5);
    assert_eq!(scorer.score(&key, 10, 20, &histories), 0.25);
}
