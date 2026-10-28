use super::*;

#[test]
fn history_needs_no_cached_entry_and_keeps_one_counter_per_key() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    assert_eq!(histories.decayed_request_count(&key), 0.0);
    assert_eq!(histories.request_count(), 0);
    assert!(histories.shards.iter().all(|shard| shard.lock().unwrap().is_empty()));
    histories.record_access(&key);
    assert_eq!(histories.decayed_request_count(&key), 1.0);
    histories.record_access(&key);
    assert!(histories.decayed_request_count(&key) > 1.0);
    assert_eq!(histories.request_count(), 2);
    assert_eq!(histories.shard(&key).lock().unwrap().len(), 1);
}

#[test]
fn concurrent_recording_counts_every_request() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..1000 {
                    histories.record_access(&key);
                }
            });
        }
    });
    assert_eq!(histories.request_count(), 8000);
    assert_eq!(histories.shard(&key).lock().unwrap().len(), 1);
    let mut expected = DecayedAccessCount::default();
    for clock in 1..=8000 {
        expected.add_request(clock, histories.half_life());
    }
    assert_eq!(
        histories.decayed_request_count(&key),
        expected.decayed_count(8000, histories.half_life())
    );
}

#[test]
fn another_history_shard_can_record_and_read_while_one_is_locked() {
    let histories = ObjectAccessHistories::new();
    let locked_key = ObjectKeyHash::from("locked");
    let other = (0..1000)
        .map(|index| ObjectKeyHash::from(format!("other-{index}")))
        .find(|key| !std::ptr::eq(histories.shard(&locked_key), histories.shard(key)))
        .unwrap();
    let (done, received) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let _locked = histories.shard(&locked_key).lock().unwrap();
        scope.spawn(|| {
            histories.record_access(&other);
            assert_eq!(histories.decayed_request_count(&other), 1.0);
            done.send(()).unwrap();
        });
        received.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(histories.request_count(), 1);
    });
}

#[test]
fn concurrent_keys_share_one_clock() {
    let histories = ObjectAccessHistories::new();
    let keys: Vec<_> = (0..32)
        .map(|index| ObjectKeyHash::from(format!("object-{index}")))
        .collect();
    std::thread::scope(|scope| {
        for key in &keys {
            let histories = &histories;
            scope.spawn(move || {
                for _ in 0..200 {
                    histories.record_access(key);
                }
            });
        }
    });
    assert_eq!(histories.request_count(), 32 * 200);
    for key in &keys {
        assert!(histories.decayed_request_count(key) > 190.0);
        assert!(histories.decayed_request_count(key) <= 200.0);
    }
}

#[test]
fn counts_decay_and_new_accesses_add_one() {
    let key = ObjectKeyHash::from("object");
    let other = ObjectKeyHash::from("other");
    for half_life in [2, 4, DEFAULT_ACCESS_COUNT_HALF_LIFE] {
        let histories = ObjectAccessHistories::with_half_life(half_life);
        histories.record_access(&key);
        for _ in 1..half_life {
            histories.record_access(&other);
        }
        histories.record_access(&key);
        assert_eq!(histories.decayed_request_count(&key), 1.5);
        for _ in 0..half_life {
            histories.record_access(&other);
        }
        assert_eq!(histories.decayed_request_count(&key), 0.75);
    }
}

#[test]
fn counters_remain_after_their_demand_decays_to_zero() {
    let histories = ObjectAccessHistories::with_half_life(1);
    let key = ObjectKeyHash::from("old");
    histories.record_access(&key);
    for _ in 0..126 {
        histories.record_access(&ObjectKeyHash::from("other"));
    }
    assert_eq!(histories.decayed_request_count(&key), 0.0);
    assert!(histories.shard(&key).lock().unwrap().contains_key(&key));
    histories.record_access(&key);
    assert_eq!(histories.decayed_request_count(&key), 1.0);
}

#[test]
#[should_panic(expected = "access count half-life must be greater than zero")]
fn zero_half_life_is_rejected() {
    ObjectAccessHistories::with_half_life(0);
}

#[test]
fn decayed_count_never_increases_and_relative_error_is_at_most_6_15_percent() {
    let request_count = DecayedAccessCount {
        count: 1.0,
        last_update_clock: 1,
    };
    assert_eq!(request_count.decayed_count(0, DEFAULT_ACCESS_COUNT_HALF_LIFE), 1.0);
    let mut previous = 1.0;
    for sample in 0..=256 {
        let requests_since_update = DEFAULT_ACCESS_COUNT_HALF_LIFE * sample / 64;
        let actual = request_count.decayed_count(1 + requests_since_update, DEFAULT_ACCESS_COUNT_HALF_LIFE);
        let exact = (-(requests_since_update as f64) / DEFAULT_ACCESS_COUNT_HALF_LIFE as f64).exp2();
        assert!(actual <= previous);
        assert!(actual >= exact * (1.0 - 1e-7));
        assert!(actual <= exact * 1.0615);
        previous = actual;
    }
    assert_eq!(
        request_count.decayed_count(1 + DEFAULT_ACCESS_COUNT_HALF_LIFE * 126, DEFAULT_ACCESS_COUNT_HALF_LIFE),
        0.0
    );
    assert_eq!(
        request_count.decayed_count(u64::MAX, DEFAULT_ACCESS_COUNT_HALF_LIFE),
        0.0
    );
}

#[test]
fn repeated_updates_compound_decay_error() {
    let mut count = DecayedAccessCount::default();
    let clocks = [1, 17, 4097, 9001, 20_000];
    for clock in clocks {
        count.add_request(clock, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    }
    let expected: f64 = clocks
        .into_iter()
        .map(|clock| (-((30_000 - clock) as f64) / DEFAULT_ACCESS_COUNT_HALF_LIFE as f64).exp2())
        .sum();
    let actual = count.decayed_count(30_000, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    // Each event undergoes at most one decay per recorded access, including the final read.
    let max_decay_steps = clocks.len() as i32;
    assert!(actual >= expected * (1.0_f64 - 1e-7).powi(max_decay_steps));
    assert!(actual <= expected * 1.0615_f64.powi(max_decay_steps));
}
