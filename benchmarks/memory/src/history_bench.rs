//! Opt-in history-only replay: no cache payloads, lookups, or disk I/O.

use std::{collections::HashSet, sync::Barrier, thread};

use super::*;

#[test]
#[ignore = "manual throughput benchmark; run in release mode with --nocapture"]
fn history_recording_contention() {
    let requests = load_trace().unwrap();
    assert!(!requests.is_empty());
    for (index, request) in requests.iter().enumerate() {
        request.check_range_fits_object(index).unwrap();
    }
    let keys: HashSet<_> = requests.iter().map(|request| &request.object_key).collect();
    let ranges: HashSet<_> = requests
        .iter()
        .map(|request| (&request.object_key, request.requested_range))
        .collect();
    println!(
        "requests={}, keys={}, distinct_key_ranges={}, available_parallelism={}",
        requests.len(),
        keys.len(),
        ranges.len(),
        thread::available_parallelism().unwrap(),
    );
    println!("Five trials per case; each timed trial distributes one full trace across the workers.");
    println!("threads,history,median_ms,requests_per_second,min_ms,max_ms");
    for threads in [1, 32] {
        for prepopulated in [false, true] {
            let mut durations = Vec::new();
            for _ in 0..5 {
                let history = ObjectAccessHistories::new();
                if prepopulated {
                    for request in &requests {
                        history.record_access(&request.object_key, request.requested_range);
                    }
                }
                let initial_clock = history.clock();
                let barrier = Barrier::new(threads + 1);
                let elapsed = thread::scope(|scope| {
                    for worker in 0..threads {
                        let (history, requests, barrier) = (&history, &requests, &barrier);
                        scope.spawn(move || {
                            barrier.wait(); // All workers ready before timing starts.
                            barrier.wait();
                            for request in requests.iter().skip(worker).step_by(threads) {
                                history.record_access(&request.object_key, request.requested_range);
                            }
                            barrier.wait();
                        });
                    }
                    barrier.wait();
                    let started = Instant::now();
                    barrier.wait();
                    barrier.wait();
                    started.elapsed()
                });
                assert_eq!(history.clock() - initial_clock, requests.len() as u64);
                durations.push(elapsed);
            }
            durations.sort_unstable();
            let median = durations[2].as_secs_f64();
            println!(
                "{threads},{},{:.3},{:.0},{:.3},{:.3}",
                if prepopulated { "prepopulated" } else { "empty" },
                median * 1000.0,
                requests.len() as f64 / median,
                durations[0].as_secs_f64() * 1000.0,
                durations[4].as_secs_f64() * 1000.0,
            );
        }
    }
}
