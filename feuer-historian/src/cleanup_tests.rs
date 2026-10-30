use std::time::Instant;

use super::*;

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "history cleanup did not finish");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn cleanup_removes_least_recent_requests_across_shards() {
    let histories = ObjectAccessHistories::new();
    let keys: Vec<_> = (0..11).map(|i| ObjectKeyHash::from(format!("key-{i}"))).collect();
    for key in &keys {
        histories.record_request(key);
    }
    histories.record_request(&keys[0]);
    let recent_count = histories.decayed_request_count(&keys[0]);
    ObjectAccessHistories::remove_oldest(&*histories.shards, 10);

    assert_eq!(histories.request_count(), 12);
    assert_eq!(histories.decayed_request_count(&keys[0]), recent_count);
    for key in &keys[1..3] {
        assert_eq!(histories.decayed_request_count(key), 0.0);
    }
    for key in &keys[3..] {
        assert!(histories.decayed_request_count(key) > 0.0);
    }
    histories.record_request(&keys[1]);
    assert_eq!(histories.decayed_request_count(&keys[1]), 1.0);
}

#[test]
fn background_cleanup_handles_concurrent_requests_and_releases_history_on_drop() {
    let histories = ObjectAccessHistories::with_half_life_and_entry_limit(
        DEFAULT_ACCESS_COUNT_HALF_LIFE,
        NonZeroUsize::new(100).unwrap(),
    );
    let weak = Arc::downgrade(&histories.shards);
    thread::scope(|scope| {
        for worker in 0..8 {
            let histories = &histories;
            scope.spawn(move || {
                for i in 0..1000 {
                    histories.record_request(&ObjectKeyHash::from(format!("{worker}-{}", i / 2)));
                }
            });
        }
    });
    wait_until(|| {
        histories
            .shards
            .iter()
            .map(|shard| shard.lock().unwrap().len())
            .sum::<usize>()
            <= 100
    });
    assert_eq!(histories.request_count(), 8000);
    drop(histories);
    wait_until(|| weak.upgrade().is_none());
}
