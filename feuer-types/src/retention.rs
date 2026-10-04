//! Internal shared access evidence and payload-value comparison for both cache tiers.

use std::{
    cmp::Ordering,
    collections::VecDeque,
    hash::BuildHasher,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
};

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::{ByteRange, ObjectKeyHash, config::read_env_number};

/// Default maximum entries inspected in one retention-policy sample.
pub const RECLAIM_SAMPLE_SIZE: usize = 64;

/// Chooses up to `sample_size` candidates, advancing through the list and wrapping at its end.
pub fn sample_candidates(next_candidate: &mut usize, candidate_count: usize, sample_size: usize) -> (usize, usize) {
    if candidate_count == 0 {
        return (0, 0);
    }
    let sample_start = *next_candidate % candidate_count;
    let sample_count = candidate_count.min(sample_size);
    *next_candidate = (sample_start + sample_count) % candidate_count;
    (sample_start, sample_count)
}

/// Compares decayed retrieval value per payload byte without division.
pub fn compare_cost_per_byte(left_cost: f64, left_bytes: u64, right_cost: f64, right_bytes: u64) -> Ordering {
    (left_cost * right_bytes as f64).total_cmp(&(right_cost * left_bytes as f64))
}

/// Independent of the number of memory or disk cache shards.
const ACCESS_HISTORY_SHARDS: usize = 64;

/// Standalone request history shared by both cache tiers, independent of their shards and entries.
/// Object keys select one of 64 independently locked history maps; the request clock remains global.
/// Every distinct key and requested-range counter lasts for this object's lifetime, even after
/// cache eviction. Trimming retains at most `MAX_ACCESS_EVENTS_PER_KEY` events per object key.
/// History is not persisted across restarts.
pub struct ObjectAccessHistories {
    shards: [Mutex<FxHashMap<ObjectKeyHash, RangeAccessHistory>>; ACCESS_HISTORY_SHARDS],
    request_count: AtomicU64,
}

impl Default for ObjectAccessHistories {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::default()),
            request_count: AtomicU64::new(0),
        }
    }
}

impl ObjectAccessHistories {
    /// Creates empty request history with one clock across all keys and cache tiers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one request when it starts, before any cache lookup or source fetch.
    /// Failed and canceled requests contribute to demand just like successful requests.
    /// Exact repeats update the same counter.
    /// Cache insertion and eviction do not record or remove history.
    pub fn record_access(&self, key: &ObjectKeyHash, requested: ByteRange) {
        let mut objects = self.shard(key).lock().unwrap();
        // Assign the clock under the shard lock so this key's updates cannot arrive out of order.
        // The atomic only measures request age; the shard mutex protects the history itself.
        let clock = self.request_count.fetch_add(1, AtomicOrdering::Relaxed) + 1;
        objects.entry(*key).or_default().record(requested, clock);
    }

    /// Number of requests recorded across all keys and cache tiers.
    /// Other shards may still be applying their counter updates.
    pub fn request_count(&self) -> u64 {
        self.request_count.load(AtomicOrdering::Relaxed)
    }

    /// Sums retrieval costs for contained requests, weighted by decayed access counts.
    /// Eviction divides this cost by the memory allocation charge or disk payload length.
    /// Unknown keys have zero cost.
    pub fn decayed_retrieval_cost(&self, key: &ObjectKeyHash, cached_range: ByteRange) -> f64 {
        let objects = self.shard(key).lock().unwrap();
        objects.get(key).map_or(0.0, |history| {
            history.decayed_retrieval_cost(cached_range, self.request_count())
        })
    }

    /// Snapshots recent requested ranges for trimming. Later accesses do not invalidate the snapshot:
    /// it guides a retention policy, not the correctness of the cached bytes.
    pub fn recent_requested_ranges(&self, key: &ObjectKeyHash) -> Vec<ByteRange> {
        let objects = self.shard(key).lock().unwrap();
        objects.get(key).map_or_else(Vec::new, |history| {
            history.recent_requested_ranges(self.request_count()).collect()
        })
    }

    fn shard(&self, key: &ObjectKeyHash) -> &Mutex<FxHashMap<ObjectKeyHash, RangeAccessHistory>> {
        &self.shards[FxBuildHasher.hash_one(key) as usize % self.shards.len()]
    }
}

/// Fixed source-request cost as equivalent transferred bytes; zero scores only bytes.
/// Reads `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` once on first use, defaulting to
/// 10,000,000 (125 ms at 80 MB/s). Accepts size suffixes; panics unless the value fits `u64`.
pub static FIXED_RETRIEVAL_EQUIVALENT_BYTES: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES", 10_000_000, 0).unwrap_or_else(|error| panic!("{error}"))
});
/// Maximum exact access events in one object key's range-trimming history.
/// Reads `FEUER_MAX_ACCESS_EVENTS_PER_KEY` once on first use, defaulting to 64.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `usize`.
pub static MAX_ACCESS_EVENTS_PER_KEY: LazyLock<usize> = LazyLock::new(|| {
    read_env_number("FEUER_MAX_ACCESS_EVENTS_PER_KEY", 64, 1).unwrap_or_else(|error| panic!("{error}"))
});
/// Maximum age in requests across all keys that still contributes to range trimming.
/// Reads `FEUER_MAX_ACCESS_AGE_ACCESSES` once on first use, defaulting to 262,144.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `u64`.
pub static MAX_ACCESS_AGE_ACCESSES: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_MAX_ACCESS_AGE_ACCESSES", 262_144, 1).unwrap_or_else(|error| panic!("{error}"))
});

/// A requested byte range and the global request-counter value assigned when the access was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RangeAccess {
    range: ByteRange,
    observed_at_access: u64,
}

/// Half-life of access counts, in requests across all keys.
/// Reads `FEUER_ACCESS_COUNT_HALF_LIFE` once on first use, defaulting to 262,144.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `u64`.
pub static ACCESS_COUNT_HALF_LIFE: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_ACCESS_COUNT_HALF_LIFE", 262_144, 1).unwrap_or_else(|error| panic!("{error}"))
});

#[derive(Default)]
struct DecayedAccessCount {
    count: f64,
    observed_at_access: u64,
}

impl DecayedAccessCount {
    fn decayed_count(&self, access_clock: u64) -> f64 {
        // Age is measured in requests across all keys, not wall-clock time.
        // An earlier clock leaves the count unchanged rather than increasing it.
        let elapsed_accesses = access_clock.saturating_sub(self.observed_at_access);
        let half_lives_elapsed = elapsed_accesses as f64 / *ACCESS_COUNT_HALF_LIFE as f64;

        if half_lives_elapsed >= 126.0 {
            return 0.0; // Discard negligible counts rather than constructing subnormal floats.
        }

        // Approximate 2^(-age) through IEEE-754 bits: the exponent gives powers of two,
        // and the mantissa interpolates between them. Each decay is up to ~6.15% high;
        // repeated updates can compound that error.
        let exponent_bias = 127.0;
        let mantissa_scale = (1u32 << 23) as f64;
        let float_bits = ((exponent_bias - half_lives_elapsed) * mantissa_scale) as u32;
        self.count * f32::from_bits(float_bits) as f64
    }
}

/// Per-range decayed counts for scoring and recent request events for range trimming.
/// Trimming events have a per-object count limit and an age limit measured in requests across all keys.
/// Cached ranges receive credit only for fully contained requests.
/// Unlike trimming events, distinct counters are never removed or capped.
#[derive(Default)]
struct RangeAccessHistory {
    events: VecDeque<RangeAccess>,
    access_count_indices: FxHashMap<ByteRange, usize>,
    // Each counter is stored once: indexed for recording, scanned for scoring.
    access_counts: Vec<(ByteRange, DecayedAccessCount)>,
}

impl RangeAccessHistory {
    fn record(&mut self, range: ByteRange, access_clock: u64) {
        let count_index = *self.access_count_indices.entry(range).or_insert_with(|| {
            let count_index = self.access_counts.len();
            self.access_counts.push((range, DecayedAccessCount::default()));
            count_index
        });
        let access_count = &mut self.access_counts[count_index].1;
        access_count.count = access_count.decayed_count(access_clock) + 1.0;
        access_count.observed_at_access = access_clock;
        if self.events.len() == *MAX_ACCESS_EVENTS_PER_KEY {
            self.events.pop_front();
        }
        self.events.push_back(RangeAccess {
            range,
            observed_at_access: access_clock,
        });
    }

    /// Iterates requested byte ranges from events within the trimming age limit, including repeats.
    pub fn recent_requested_ranges(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_ {
        self.events
            .iter()
            .skip_while(move |event| access_clock.saturating_sub(event.observed_at_access) > *MAX_ACCESS_AGE_ACCESSES)
            .map(|event| event.range)
    }

    /// Sums retrieval costs for contained requests, weighted by decayed access counts.
    /// The caller divides this cost by the memory allocation charge or disk payload length
    /// to select an entry to evict.
    pub fn decayed_retrieval_cost(&self, cached_range: ByteRange, access_clock: u64) -> f64 {
        let fixed_retrieval_cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64;
        self.access_counts
            .iter()
            .filter(|(requested_range, _)| cached_range.contains(*requested_range))
            .map(|(requested_range, access_count)| {
                access_count.decayed_count(access_clock) * (fixed_retrieval_cost + requested_range.len() as f64)
            })
            .sum()
    }

    #[cfg(test)]
    pub(super) fn ranges(&self) -> Vec<ByteRange> {
        self.events.iter().map(|event| event.range).collect()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    fn environment_test_command(test: &str) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", &format!("retention::tests::{test}")]);
        command
    }

    // Fresh processes isolate environment settings and LazyLock initialization.
    fn assert_command_succeeds(command: &mut std::process::Command) {
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn fixed_retrieval_cost_environment_override() {
        if let Ok(value) = std::env::var("FEUER_TEST_FIXED_RETRIEVAL_CHILD") {
            let fixed_cost = value.parse::<u64>().unwrap();
            assert_eq!(*FIXED_RETRIEVAL_EQUIVALENT_BYTES, fixed_cost);
            let mut history = RangeAccessHistory::default();
            history.record(range(0, 4), 0);
            history.record(range(0, 4), 0);
            assert_eq!(
                history.decayed_retrieval_cost(range(0, 8), 0),
                (fixed_cost as f64 + 4.0) * 2.0
            );
            assert_eq!(history.decayed_retrieval_cost(range(4, 8), 0), 0.0);
            return;
        }
        for fixed_cost in [0, 1_000_000, 10_000_000, u64::MAX] {
            assert_command_succeeds(
                environment_test_command("fixed_retrieval_cost_environment_override")
                    .env("FEUER_TEST_FIXED_RETRIEVAL_CHILD", fixed_cost.to_string())
                    .env("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES", fixed_cost.to_string())
                    .env("FEUER_MAX_ACCESS_EVENTS_PER_KEY", "64"),
            );
        }
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
                history.record(requested, 0);
            }
            assert_eq!(history.len(), limit);
            assert_eq!(history.ranges(), requested_ranges[3..]);
            assert_eq!(
                history.decayed_retrieval_cost(range(0, 1), 0),
                *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0
            );
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
    fn access_count_half_life_environment_override() {
        if let Ok(value) = std::env::var("FEUER_TEST_HALF_LIFE_CHILD") {
            let half_life = value.parse::<u64>().unwrap();
            assert_eq!(*ACCESS_COUNT_HALF_LIFE, half_life);
            let mut history = RangeAccessHistory::default();
            history.record(range(0, 1), 0);
            let cost = history.decayed_retrieval_cost(range(0, 1), 0);
            assert_eq!(history.decayed_retrieval_cost(range(0, 1), half_life), cost * 0.5);
            return;
        }
        for (setting, half_life) in [
            (None, 262_144),
            (Some("1"), 1),
            (Some("256"), 256),
            (Some("64KiB"), 65_536),
            (Some("18446744073709551615"), u64::MAX),
        ] {
            let mut command = environment_test_command("access_count_half_life_environment_override");
            command
                .env("FEUER_TEST_HALF_LIFE_CHILD", half_life.to_string())
                .env_remove("FEUER_ACCESS_COUNT_HALF_LIFE");
            if let Some(setting) = setting {
                command.env("FEUER_ACCESS_COUNT_HALF_LIFE", setting);
            }
            assert_command_succeeds(&mut command);
        }
    }

    #[test]
    fn access_age_environment_override() {
        if std::env::var_os("FEUER_TEST_ACCESS_AGE_CHILD").is_some() {
            assert_eq!(*MAX_ACCESS_AGE_ACCESSES, 65_536);
            let mut history = RangeAccessHistory::default();
            history.record(range(0, 1), 0);
            assert!(history.decayed_retrieval_cost(range(0, 1), 65_536) > 0.0);
            assert!(history.decayed_retrieval_cost(range(0, 1), 65_537) > 0.0);
            assert_eq!(history.recent_requested_ranges(65_537).count(), 0);
            history.record(range(1, 2), 65_537);
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
        assert_eq!(histories.decayed_retrieval_cost(&key, range(0, 1)), 0.0);
        assert!(histories.recent_requested_ranges(&key).is_empty());
        assert!(histories.shards.iter().all(|shard| shard.lock().unwrap().is_empty()));
        histories.record_access(&key, range(0, 1));
        for _ in 0..*MAX_ACCESS_EVENTS_PER_KEY + 1 {
            histories.record_access(&key, range(2, 3));
        }
        assert!(!histories.recent_requested_ranges(&key).contains(&range(0, 1)));
        assert!(histories.decayed_retrieval_cost(&key, range(0, 1)) > 0.0);
        let objects = histories.shard(&key).lock().unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[&key].access_counts.len(), 2);
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
        assert_eq!(actual.access_counts.len(), 1);
        let mut expected = RangeAccessHistory::default();
        for clock in 1..=8000 {
            expected.record(range(0, 1), clock);
        }
        assert_eq!(
            actual.decayed_retrieval_cost(range(0, 1), 8000),
            expected.decayed_retrieval_cost(range(0, 1), 8000)
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
                assert!(histories.decayed_retrieval_cost(&other, range(0, 1)) > 0.0);
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
            assert_eq!(history.access_counts.len(), 1);
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
        assert_eq!(histories.shard(&old).lock().unwrap()[&old].access_counts.len(), 1);
        histories.record_access(&old, range(0, 1));
        assert_eq!(histories.shard(&old).lock().unwrap()[&old].access_counts.len(), 1);
    }

    #[test]
    fn samples_at_most_64_candidates_and_compares_large_costs_per_byte_without_overflow() {
        let mut cursor = 0;
        assert_eq!(sample_candidates(&mut cursor, 0, RECLAIM_SAMPLE_SIZE), (0, 0));
        assert_eq!(sample_candidates(&mut cursor, 100, RECLAIM_SAMPLE_SIZE), (0, 64));
        assert_eq!(sample_candidates(&mut cursor, 100, RECLAIM_SAMPLE_SIZE), (64, 64));
        assert_eq!(
            compare_cost_per_byte(u64::MAX as f64 * 2.0, u64::MAX, u64::MAX as f64, u64::MAX),
            Ordering::Greater
        );
        assert_eq!(compare_cost_per_byte(1.0, 2, 2.0, 4), Ordering::Equal);
    }

    #[test]
    fn retrieval_value_decays_and_new_accesses_add_one() {
        let requested = range(2, 4);
        let cached_range = range(0, 8);
        let mut history = RangeAccessHistory::default();
        let cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + requested.len() as f64;
        history.record(requested, 0);
        assert_eq!(history.decayed_retrieval_cost(cached_range, 0), cost);
        assert_eq!(
            history.decayed_retrieval_cost(cached_range, *ACCESS_COUNT_HALF_LIFE),
            cost * 0.5
        );
        history.record(requested, *ACCESS_COUNT_HALF_LIFE);
        assert_eq!(
            history.decayed_retrieval_cost(cached_range, *ACCESS_COUNT_HALF_LIFE),
            cost * 1.5
        );
        assert_eq!(
            history.decayed_retrieval_cost(cached_range, *ACCESS_COUNT_HALF_LIFE * 2),
            cost * 0.75
        );
    }

    #[test]
    fn exact_counts_stay_indexed_when_the_vec_grows() {
        let mut history = RangeAccessHistory::default();
        let first = range(1000, 1001);
        history.record(first, 0);
        for start in (0..256).rev() {
            history.record(range(start, start + 1), 0);
        }
        history.record(first, *ACCESS_COUNT_HALF_LIFE);
        assert_eq!(history.access_counts.len(), 257);
        assert_eq!(history.access_count_indices.len(), 257);
        for (&requested, &index) in &history.access_count_indices {
            assert_eq!(history.access_counts[index].0, requested);
        }
        assert_eq!(
            history.decayed_retrieval_cost(first, *ACCESS_COUNT_HALF_LIFE),
            (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0) * 1.5
        );
    }

    #[test]
    fn exact_and_expanded_scores_match_btree_counts() {
        let mut history = RangeAccessHistory::default();
        let mut reference = std::collections::BTreeMap::<(u64, u64), DecayedAccessCount>::new();
        for (index, (start, end)) in [(8, 10), (2, 4), (0, 8), (4, 8), (8, 10), (6, 10), (2, 8)]
            .into_iter()
            .enumerate()
        {
            let clock = index as u64 * 1024;
            history.record(range(start, end), clock);
            let accesses = reference.entry((start, end)).or_default();
            accesses.count = accesses.decayed_count(clock) + 1.0;
            accesses.observed_at_access = clock;
        }
        for start in 0..18 {
            for end in start + 1..20 {
                let expected: f64 = reference
                    .range((start, 0)..(end, 0))
                    .filter(|&(&(_, requested_end), _)| requested_end <= end)
                    .map(|(&(start, end), accesses)| {
                        accesses.decayed_count(10_000)
                            * (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + (end - start) as f64)
                    })
                    .sum();
                let actual = history.decayed_retrieval_cost(range(start, end), 10_000);
                // Sums follow insertion order rather than sorted range order.
                assert!(
                    (actual - expected).abs() <= expected.abs() * 1e-12,
                    "{start}..{end}: {actual} != {expected}"
                );
            }
        }
        assert_eq!(
            RangeAccessHistory::default().decayed_retrieval_cost(range(0, 1), 0),
            0.0
        );
    }

    #[test]
    fn decayed_count_never_increases_and_relative_error_is_at_most_6_15_percent() {
        let accesses = DecayedAccessCount {
            count: 1.0,
            observed_at_access: 1,
        };
        assert_eq!(accesses.decayed_count(0), 1.0);
        let mut previous = 1.0;
        for sample in 0..=256 {
            let elapsed_accesses = *ACCESS_COUNT_HALF_LIFE * sample / 64;
            let actual = accesses.decayed_count(1 + elapsed_accesses);
            let exact = (-(elapsed_accesses as f64) / *ACCESS_COUNT_HALF_LIFE as f64).exp2();
            assert!(actual <= previous);
            assert!(actual >= exact * (1.0 - 1e-7));
            assert!(actual <= exact * 1.0615);
            previous = actual;
        }
        assert_eq!(accesses.decayed_count(1 + *ACCESS_COUNT_HALF_LIFE * 126), 0.0);
        assert_eq!(accesses.decayed_count(u64::MAX), 0.0);
    }

    #[test]
    fn approximate_decayed_counts_bound_individual_event_weights() {
        let mut history = RangeAccessHistory::default();
        let clocks = [1, 17, 4097, 9001, 20_000];
        for clock in clocks {
            history.record(range(0, 100), clock);
        }
        let expected: f64 = clocks
            .into_iter()
            .map(|clock| (-((30_000 - clock) as f64) / *ACCESS_COUNT_HALF_LIFE as f64).exp2())
            .sum();
        let actual =
            history.decayed_retrieval_cost(range(0, 100), 30_000) / (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 100.0);
        // Each event undergoes at most one decay per recorded access, including the final read.
        let max_decay_steps = clocks.len() as i32;
        assert!(actual >= expected * (1.0_f64 - 1e-7).powi(max_decay_steps));
        assert!(actual <= expected * 1.0615_f64.powi(max_decay_steps));
    }

    #[test]
    fn credits_only_cached_ranges_covering_the_exact_request() {
        let fixed_cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64;
        let mut history = RangeAccessHistory::default();
        history.record(range(3, 7), 0);
        for (cached_range, expected) in [(range(0, 8), fixed_cost + 4.0), (range(3, 5), 0.0), (range(5, 8), 0.0)] {
            assert_eq!(history.decayed_retrieval_cost(cached_range, 0), expected);
        }

        let mut history = RangeAccessHistory::default();
        history.record(range(3, 5), 0);
        history.record(range(5, 8), 0);
        history.record(range(u64::MAX - 1, u64::MAX), 0);
        for (cached_range, expected) in [
            (range(3, 5), fixed_cost + 2.0),
            (range(0, 8), (fixed_cost + 2.0) + (fixed_cost + 3.0)),
            (range(u64::MAX - 1, u64::MAX), fixed_cost + 1.0),
        ] {
            assert_eq!(history.decayed_retrieval_cost(cached_range, 0), expected);
        }
    }
}
