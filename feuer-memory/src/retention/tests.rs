use super::*;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

#[test]
fn fixed_retrieval_cost_is_explicit() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.record_access(&key, range(0, 4));
    assert_eq!(RetrievalCostScorer::default().fixed_retrieval_equivalent_bytes, 0);
    for fixed_cost in [0, 1_000_000, 10_000_000, u64::MAX] {
        let scorer = RetrievalCostScorer {
            fixed_retrieval_equivalent_bytes: fixed_cost,
        };
        assert_eq!(
            scorer.score(&key, range(0, 8), 8, &histories),
            (fixed_cost as f64 + 4.0) / 8.0
        );
        assert_eq!(scorer.score(&key, range(4, 8), 4, &histories), 0.0);
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
fn whole_object_requests_score_the_stored_length() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("whole");
    let scorer = RetrievalCostScorer::default();
    assert_eq!(scorer.score(&key, range(0, 0), 0, &histories), 0.0);
    histories.record_access(&key, None);
    for fixed_cost in [0, 10_000_000] {
        let scorer = RetrievalCostScorer {
            fixed_retrieval_equivalent_bytes: fixed_cost,
        };
        for cached_range in [range(0, 0), range(0, 100)] {
            let cost = fixed_cost as f64 + cached_range.len() as f64;
            assert_eq!(
                scorer.score(&key, cached_range, cached_range.len(), &histories),
                cost / cached_range.len().max(1) as f64
            );
        }
    }
}

#[test]
fn exact_and_expanded_scores_match_btree_counts() {
    let fixed_cost = 10_000_000;
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    for (start, end) in [(8, 10), (2, 4), (0, 8), (4, 8), (8, 10), (6, 10), (2, 8)] {
        histories.record_access(&key, range(start, end));
    }
    histories.record_access(&key, range(u64::MAX - 1, u64::MAX));
    let reference = histories.with_access_counts(&key, |counts, clock| {
        counts
            .iter()
            .map(|(requested, count)| {
                let requested = requested.unwrap();
                ((requested.start(), requested.end()), count.decayed_count(clock))
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    });
    for start in 0..18 {
        for end in start + 1..20 {
            let expected: f64 = reference
                .range((start, 0)..(end, 0))
                .filter(|&(&(_, requested_end), _)| requested_end <= end)
                .map(|(&(start, end), count)| count * (fixed_cost as f64 + (end - start) as f64))
                .sum();
            let actual = decayed_retrieval_cost(&histories, &key, range(start, end), fixed_cost);
            // Sums follow insertion order rather than sorted range order.
            assert!(
                (actual - expected).abs() <= expected.abs() * 1e-12,
                "{start}..{end}: {actual} != {expected}"
            );
        }
    }
    assert_eq!(
        decayed_retrieval_cost(&histories, &ObjectKeyHash::from("unknown"), range(0, 1), fixed_cost),
        0.0
    );
    assert_eq!(
        decayed_retrieval_cost(&histories, &key, range(u64::MAX - 1, u64::MAX), fixed_cost),
        fixed_cost as f64 + 1.0
    );
}
