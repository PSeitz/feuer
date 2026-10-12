use super::*;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

#[test]
fn score_combines_costs_per_request_and_requested_byte() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_access(&key, range(0, 4));
    assert_eq!(AvoidedRetrievalCostScorer::default().cost_per_request, 0.0);
    assert_eq!(AvoidedRetrievalCostScorer::default().cost_per_requested_byte, 1.0);
    for (cost_per_request, cost_per_requested_byte) in [(0.0, 1.0), (256_000.0, 1.0), (2.0, 0.5), (4.0, 0.0)] {
        let scorer = AvoidedRetrievalCostScorer {
            cost_per_request,
            cost_per_requested_byte,
        };
        assert_eq!(
            scorer.score(&key, range(0, 8), 8, &histories),
            (cost_per_request + 4.0 * cost_per_requested_byte) / 8.0
        );
        assert_eq!(scorer.score(&key, range(4, 8), 4, &histories), 0.0);
    }
}

#[test]
fn negative_savings_do_not_cancel_useful_requests() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_access(&key, range(0, 1));
    histories.record_access(&key, range(0, 10));
    histories.record_access(&key, range(20, 21));
    let scorer = AvoidedRetrievalCostScorer {
        cost_per_request: 4.0,
        cost_per_requested_byte: -1.0,
    };
    let demand = decayed_avoided_retrieval_cost(&histories, &key, range(0, 1), 0.0, 1.0);
    assert_eq!(scorer.score(&key, range(0, 10), 10, &histories), demand * 3.0 / 10.0);
    assert_eq!(scorer.score(&key, range(2, 10), 8, &histories), 0.0);
    histories.record_access(&ObjectKeyHash(1), None);
    assert_eq!(scorer.score(&ObjectKeyHash(1), range(0, 10), 10, &histories), 0.0);
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
fn whole_object_requests_score_the_stored_length() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("whole");
    let scorer = AvoidedRetrievalCostScorer::default();
    assert_eq!(scorer.score(&key, range(0, 0), 0, &histories), 0.0);
    histories.record_access(&key, None);
    for cost_per_request in [0, 10_000_000] {
        let scorer = AvoidedRetrievalCostScorer {
            cost_per_request: cost_per_request as f64,
            ..Default::default()
        };
        for cached_range in [range(0, 0), range(0, 100)] {
            let cost = cost_per_request as f64 + cached_range.len() as f64;
            assert_eq!(
                scorer.score(&key, cached_range, cached_range.len(), &histories),
                cost / cached_range.len().max(1) as f64
            );
        }
    }
}

#[test]
fn exact_and_expanded_scores_match_btree_counts() {
    let cost_per_request = 10_000_000;
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    for (start, end) in [(8, 10), (2, 4), (0, 8), (4, 8), (8, 10), (6, 10), (2, 8)] {
        histories.record_access(&key, range(start, end));
    }
    histories.record_access(&key, range(u64::MAX - 1, u64::MAX));
    let reference = histories.read_request_counts(&key, |request_counts, request_clock| {
        request_counts
            .iter()
            .map(|(requested, request_count)| {
                let requested = requested.unwrap();
                (
                    (requested.start(), requested.end()),
                    request_count.decayed_count(request_clock),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    });
    for start in 0..18 {
        for end in start + 1..20 {
            let expected: f64 = reference
                .range((start, 0)..(end, 0))
                .filter(|&(&(_, requested_end), _)| requested_end <= end)
                .map(|(&(start, end), count)| count * (cost_per_request as f64 + (end - start) as f64))
                .sum();
            let actual =
                decayed_avoided_retrieval_cost(&histories, &key, range(start, end), cost_per_request as f64, 1.0);
            // Sums follow insertion order rather than sorted range order.
            assert!(
                (actual - expected).abs() <= expected.abs() * 1e-12,
                "{start}..{end}: {actual} != {expected}"
            );
        }
    }
    assert_eq!(
        decayed_avoided_retrieval_cost(
            &histories,
            &ObjectKeyHash::from("unknown"),
            range(0, 1),
            cost_per_request as f64,
            1.0
        ),
        0.0
    );
    assert_eq!(
        decayed_avoided_retrieval_cost(
            &histories,
            &key,
            range(u64::MAX - 1, u64::MAX),
            cost_per_request as f64,
            1.0
        ),
        cost_per_request as f64 + 1.0
    );
}
