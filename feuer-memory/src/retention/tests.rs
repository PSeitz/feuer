use super::*;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

#[test]
fn fixed_retrieval_cost_environment_override() {
    if let Ok(value) = std::env::var("FEUER_TEST_FIXED_RETRIEVAL_CHILD") {
        let fixed_cost = value.parse::<u64>().unwrap();
        assert_eq!(*FIXED_RETRIEVAL_EQUIVALENT_BYTES, fixed_cost);
        let histories = ObjectAccessHistories::new();
        let key = ObjectKeyHash::from("object");
        histories.record_access(&key, range(0, 4));
        assert_eq!(
            decayed_retrieval_cost(&histories, &key, range(0, 8)),
            fixed_cost as f64 + 4.0
        );
        assert_eq!(decayed_retrieval_cost(&histories, &key, range(4, 8)), 0.0);
        return;
    }
    // Fresh processes isolate environment settings and LazyLock initialization.
    for fixed_cost in [0, 1_000_000, 10_000_000, u64::MAX] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "retention::tests::fixed_retrieval_cost_environment_override"])
            .env("FEUER_TEST_FIXED_RETRIEVAL_CHILD", fixed_cost.to_string())
            .env("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES", fixed_cost.to_string())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
}

#[test]
fn samples_at_most_64_candidates_and_compares_large_costs_per_byte_without_overflow() {
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
    assert_eq!(
        compare_cost_per_byte(u64::MAX as f64 * 2.0, u64::MAX, u64::MAX as f64, u64::MAX),
        Ordering::Greater
    );
    assert_eq!(compare_cost_per_byte(1.0, 2, 2.0, 4), Ordering::Equal);
}

#[test]
fn whole_object_requests_score_the_stored_length() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("whole");
    histories.record_access(&key, None);
    for cached_range in [range(0, 0), range(0, 100)] {
        assert_eq!(
            decayed_retrieval_cost(&histories, &key, cached_range),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + cached_range.len() as f64
        );
    }
}

#[test]
fn exact_and_expanded_scores_match_btree_counts() {
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
                .map(|(&(start, end), count)| count * (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + (end - start) as f64))
                .sum();
            let actual = decayed_retrieval_cost(&histories, &key, range(start, end));
            // Sums follow insertion order rather than sorted range order.
            assert!(
                (actual - expected).abs() <= expected.abs() * 1e-12,
                "{start}..{end}: {actual} != {expected}"
            );
        }
    }
    assert_eq!(
        decayed_retrieval_cost(&histories, &ObjectKeyHash::from("unknown"), range(0, 1)),
        0.0
    );
    assert_eq!(
        decayed_retrieval_cost(&histories, &key, range(u64::MAX - 1, u64::MAX)),
        *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0
    );
}
