use super::*;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

fn range_count(history: &RangeAccessHistory, requested: ByteRange, request_clock: u64) -> f64 {
    history.request_counts[history.request_count_indices[&Some(requested)]]
        .1
        .decayed_count(request_clock, DEFAULT_ACCESS_COUNT_HALF_LIFE)
}

fn environment_test_command(test: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", &format!("tests::{test}")]);
    command
}

// Fresh processes isolate environment settings and LazyLock initialization.
fn assert_command_succeeds(command: &mut std::process::Command) {
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn access_event_limit_environment_override() {
    if let Ok(value) = std::env::var("FEUER_TEST_ACCESS_EVENTS_CHILD") {
        let limit = value.parse::<usize>().unwrap();
        assert_eq!(*MAX_ACCESS_EVENTS_PER_KEY, limit);
        let requested_ranges: Vec<_> = (0..limit)
            .map(|index| range(index as u64, index as u64 + 1))
            .chain(std::iter::repeat_n(range(10, 20), 3))
            .collect();
        let mut history = RangeAccessHistory::default();
        for &requested in &requested_ranges {
            history.record(requested, 0, DEFAULT_ACCESS_COUNT_HALF_LIFE);
        }
        assert_eq!(history.events.len(), limit);
        assert_eq!(
            history.recent_requested_ranges(0).collect::<Vec<_>>(),
            requested_ranges[3..]
        );
        assert_eq!(range_count(&history, range(0, 1), 0), 1.0);
        return;
    }
    for limit in [1, 64, 256] {
        assert_command_succeeds(
            environment_test_command("access_event_limit_environment_override")
                .env("FEUER_TEST_ACCESS_EVENTS_CHILD", limit.to_string())
                .env("FEUER_MAX_ACCESS_EVENTS_PER_KEY", limit.to_string()),
        );
    }
}

#[test]
fn histories_use_their_own_half_lives_for_updates_and_reads() {
    let histories = [
        ObjectAccessHistories::with_half_life(2),
        ObjectAccessHistories::with_half_life(4),
    ];
    let key = ObjectKeyHash::from("object");
    let other = ObjectKeyHash::from("other");
    for history in &histories {
        history.record_access(&key, None);
        for _ in 0..3 {
            history.record_access(&other, None);
        }
        history.record_access(&key, None);
    }
    for (history, expected) in histories.iter().zip([1.25, 1.5]) {
        history.read_request_counts(&key, |counts, clock| {
            assert_eq!(counts[0].1.decayed_count(clock, history.half_life()), expected);
        });
        for _ in 0..4 {
            history.record_access(&other, None);
        }
    }
    for (history, expected) in histories.iter().zip([0.3125, 0.75]) {
        history.read_request_counts(&key, |counts, clock| {
            assert_eq!(counts[0].1.decayed_count(clock, history.half_life()), expected);
        });
    }
}

#[test]
fn access_age_environment_override() {
    if std::env::var_os("FEUER_TEST_ACCESS_AGE_CHILD").is_some() {
        assert_eq!(*MAX_ACCESS_AGE_ACCESSES, 65_536);
        let mut history = RangeAccessHistory::default();
        history.record(range(0, 1), 0, DEFAULT_ACCESS_COUNT_HALF_LIFE);
        assert!(range_count(&history, range(0, 1), 65_536) > 0.0);
        assert!(range_count(&history, range(0, 1), 65_537) > 0.0);
        assert_eq!(history.recent_requested_ranges(65_537).count(), 0);
        history.record(range(1, 2), 65_537, DEFAULT_ACCESS_COUNT_HALF_LIFE);
        assert_eq!(
            history.recent_requested_ranges(65_537).collect::<Vec<_>>(),
            vec![range(1, 2)]
        );
        return;
    }
    assert_command_succeeds(
        environment_test_command("access_age_environment_override")
            .env("FEUER_TEST_ACCESS_AGE_CHILD", "1")
            .env("FEUER_MAX_ACCESS_AGE_ACCESSES", "64KiB"),
    );
}

#[test]
fn history_needs_no_cached_entry_and_keeps_distinct_counters() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("object");
    histories.read_request_counts(&key, |request_counts, request_clock| {
        assert!(request_counts.is_empty());
        assert_eq!(request_clock, 0);
    });
    assert!(histories.recent_requested_ranges(&key).is_empty());
    assert!(histories.shards.iter().all(|shard| shard.lock().unwrap().is_empty()));
    histories.record_access(&key, range(0, 1));
    for _ in 0..*MAX_ACCESS_EVENTS_PER_KEY + 1 {
        histories.record_access(&key, range(2, 3));
    }
    assert!(!histories.recent_requested_ranges(&key).contains(&range(0, 1)));
    histories.read_request_counts(&key, |request_counts, request_clock| {
        assert_eq!(request_counts.len(), 2);
        assert_eq!(request_counts[0].0, Some(range(0, 1)));
        assert_eq!(request_counts[1].0, Some(range(2, 3)));
        assert!(request_counts[0].1.decayed_count(request_clock, histories.half_life()) > 0.0);
        assert_eq!(request_clock, *MAX_ACCESS_EVENTS_PER_KEY as u64 + 2);
    });
    let objects = histories.shard(&key).lock().unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[&key].request_counts.len(), 2);
}

#[test]
fn concurrent_recording_deduplicates_ranges_and_counts_every_request() {
    let histories = ObjectAccessHistories::new();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..1000 {
                    histories.record_access(&ObjectKeyHash::from("object"), range(0, 1));
                }
            });
        }
    });
    assert_eq!(histories.request_count(), 8000);
    let objects = histories.shard(&ObjectKeyHash::from("object")).lock().unwrap();
    assert_eq!(objects.len(), 1);
    let actual = &objects[&ObjectKeyHash::from("object")];
    assert_eq!(actual.request_counts.len(), 1);
    let mut expected = RangeAccessHistory::default();
    for clock in 1..=8000 {
        expected.record(range(0, 1), clock, histories.half_life());
    }
    assert_eq!(
        range_count(actual, range(0, 1), 8000),
        range_count(&expected, range(0, 1), 8000)
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
            histories.record_access(&other, range(0, 1));
            histories.read_request_counts(&other, |request_counts, request_clock| {
                assert_eq!(request_counts.len(), 1);
                assert_eq!(
                    request_counts[0].1.decayed_count(request_clock, histories.half_life()),
                    1.0
                );
            });
            assert_eq!(histories.recent_requested_ranges(&other), vec![range(0, 1)]);
            done.send(()).unwrap();
        });
        received.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(histories.request_count(), 1);
    });
}

#[test]
fn concurrent_keys_share_one_clock_and_keep_ordered_events() {
    let histories = ObjectAccessHistories::new();
    let keys: Vec<_> = (0..32)
        .map(|index| ObjectKeyHash::from(format!("object-{index}")))
        .collect();
    std::thread::scope(|scope| {
        for key in &keys {
            let histories = &histories;
            scope.spawn(move || {
                for _ in 0..200 {
                    histories.record_access(key, range(0, 1));
                }
            });
        }
    });
    assert_eq!(histories.request_count(), 32 * 200);
    for key in &keys {
        let objects = histories.shard(key).lock().unwrap();
        let history = &objects[key];
        assert_eq!(history.request_counts.len(), 1);
        assert!(history.events.iter().map(|event| event.observed_at_access).is_sorted());
    }
}

#[test]
fn all_keys_advance_the_same_clock_without_expiring_counters() {
    let histories = ObjectAccessHistories::new();
    let old = ObjectKeyHash::from("old");
    histories.record_access(&old, range(0, 1));
    for _ in 0..*MAX_ACCESS_AGE_ACCESSES + 1 {
        histories.record_access(&ObjectKeyHash::from("other"), range(0, 1));
    }
    assert!(histories.recent_requested_ranges(&old).is_empty());
    assert_eq!(histories.shard(&old).lock().unwrap()[&old].request_counts.len(), 1);
    histories.record_access(&old, range(0, 1));
    assert_eq!(histories.shard(&old).lock().unwrap()[&old].request_counts.len(), 1);
}

#[test]
fn whole_object_requests_share_one_counter_and_have_no_byte_range_events() {
    let histories = ObjectAccessHistories::new();
    let key = ObjectKeyHash::from("whole");
    histories.record_access(&key, None);
    histories.record_access(&key, None);
    histories.read_request_counts(&key, |request_counts, request_clock| {
        assert_eq!(request_counts.len(), 1);
        assert_eq!(request_counts[0].0, None);
        assert_eq!(request_clock, 2);
    });
    assert!(histories.recent_requested_ranges(&key).is_empty());
}

#[test]
fn counts_decay_and_new_accesses_add_one() {
    let requested = range(2, 4);
    let mut history = RangeAccessHistory::default();
    history.record(requested, 0, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    assert_eq!(range_count(&history, requested, 0), 1.0);
    assert_eq!(range_count(&history, requested, DEFAULT_ACCESS_COUNT_HALF_LIFE), 0.5);
    history.record(
        requested,
        DEFAULT_ACCESS_COUNT_HALF_LIFE,
        DEFAULT_ACCESS_COUNT_HALF_LIFE,
    );
    assert_eq!(range_count(&history, requested, DEFAULT_ACCESS_COUNT_HALF_LIFE), 1.5);
    assert_eq!(
        range_count(&history, requested, DEFAULT_ACCESS_COUNT_HALF_LIFE * 2),
        0.75
    );
}

#[test]
fn exact_counts_stay_indexed_when_the_vec_grows() {
    let mut history = RangeAccessHistory::default();
    let first = range(1000, 1001);
    history.record(first, 0, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    for start in (0..256).rev() {
        history.record(range(start, start + 1), 0, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    }
    history.record(first, DEFAULT_ACCESS_COUNT_HALF_LIFE, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    assert_eq!(history.request_counts.len(), 257);
    let recent = history.recent_requested_ranges(DEFAULT_ACCESS_COUNT_HALF_LIFE);
    assert_eq!(recent.last(), Some(first));
    assert_eq!(history.request_count_indices.len(), 257);
    for (&requested, &index) in &history.request_count_indices {
        assert_eq!(history.request_counts[index].0, requested);
    }
    assert_eq!(range_count(&history, first, DEFAULT_ACCESS_COUNT_HALF_LIFE), 1.5);
}

#[test]
fn decayed_count_never_increases_and_relative_error_is_at_most_6_15_percent() {
    let request_count = DecayedAccessCount {
        count: 1.0,
        observed_at_access: 1,
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
fn approximate_decayed_counts_bound_individual_event_weights() {
    let mut history = RangeAccessHistory::default();
    let clocks = [1, 17, 4097, 9001, 20_000];
    for clock in clocks {
        history.record(range(0, 100), clock, DEFAULT_ACCESS_COUNT_HALF_LIFE);
    }
    let expected: f64 = clocks
        .into_iter()
        .map(|clock| (-((30_000 - clock) as f64) / DEFAULT_ACCESS_COUNT_HALF_LIFE as f64).exp2())
        .sum();
    let actual = range_count(&history, range(0, 100), 30_000);
    // Each event undergoes at most one decay per recorded access, including the final read.
    let max_decay_steps = clocks.len() as i32;
    assert!(actual >= expected * (1.0_f64 - 1e-7).powi(max_decay_steps));
    assert!(actual <= expected * 1.0615_f64.powi(max_decay_steps));
}
