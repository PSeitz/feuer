//! Internal shared access evidence and payload-value comparison for both cache tiers.

use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard, Weak,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
};

use fnv::FnvHashMap;

use crate::{ByteRange, ObjectKey, config::read_env_number};

/// Default maximum entries inspected in one retention-policy sample.
pub const RECLAIM_SAMPLE_SIZE: usize = 64;

/// Chooses the next bounded, rotating sample from a dense candidate list.
pub fn sample_candidates(cursor: &mut usize, length: usize, sample_size: usize) -> (usize, usize) {
    if length == 0 {
        return (0, 0);
    }
    let start = *cursor % length;
    let count = length.min(sample_size);
    *cursor = (start + count) % length;
    (start, count)
}

/// Compares decayed retrieval value per payload byte without division.
pub fn compare_cost_per_byte(left_cost: f64, left_bytes: u64, right_cost: f64, right_bytes: u64) -> Ordering {
    (left_cost * right_bytes as f64).total_cmp(&(right_cost * left_bytes as f64))
}

/// Per-object access histories shared by memory and disk entries. Not persisted.
/// The registry holds weak references; removing the last entry owner releases its history.
pub struct ObjectAccessHistories {
    shards: Box<[Arc<AccessHistoryShard>]>,
}

#[derive(Default)]
struct AccessHistoryShard {
    objects: Mutex<HashMap<ObjectKey, Weak<ObjectAccessHistory>>>,
    clock: AtomicU64,
}

impl ObjectAccessHistories {
    /// Creates independently clocked shards. Use the memory tier's shard count.
    pub fn new(shards: usize) -> Self {
        assert!(shards > 0);
        Self {
            shards: (0..shards).map(|_| Arc::new(AccessHistoryShard::default())).collect(),
        }
    }

    /// Keeps a key's history alive for a cached entry or active population.
    pub fn for_key(&self, key: &ObjectKey) -> Arc<ObjectAccessHistory> {
        let shard = &self.shards[self.shard_index(key)];
        let mut objects = shard.objects.lock().unwrap();
        if let Some(history) = objects.get(key).and_then(Weak::upgrade) {
            return history;
        }
        let history = Arc::new(ObjectAccessHistory {
            key: key.clone(),
            shard: shard.clone(),
            history: Mutex::new(RangeAccessHistory::default()),
        });
        objects.insert(key.clone(), Arc::downgrade(&history));
        history
    }

    /// Records one successful public lookup, regardless of which tier supplied it.
    /// Population alone must not call this. With no entry owners, the evidence is immediately released.
    pub fn record_access(&self, key: &ObjectKey, requested: ByteRange) {
        self.for_key(key).record(requested);
    }

    /// Selects the shared evidence shard by complete key identity.
    pub fn shard_index(&self, key: &ObjectKey) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % self.shards.len() as u64) as usize
    }
}

/// One object's exact access evidence, retained by entries in either tier.
pub struct ObjectAccessHistory {
    key: ObjectKey,
    shard: Arc<AccessHistoryShard>,
    history: Mutex<RangeAccessHistory>,
}

impl ObjectAccessHistory {
    /// Records exactly one request and advances the shared shard's successful-access clock.
    /// Distinct requested ranges for an object must not overlap; exact repeats are allowed.
    pub fn record(&self, requested: ByteRange) {
        let mut history = self.lock();
        let clock = self
            .shard
            .clock
            .fetch_update(AtomicOrdering::Relaxed, AtomicOrdering::Relaxed, |clock| {
                Some(clock.saturating_add(1))
            })
            .unwrap()
            .saturating_add(1);
        history.record(requested, clock);
    }

    /// Current successful-access clock, shared across both tiers.
    pub fn clock(&self) -> u64 {
        self.shard.clock.load(AtomicOrdering::Relaxed)
    }

    /// Locks evidence for scoring or compaction generation validation. Never acquire a tier lock while held.
    pub fn lock(&self) -> MutexGuard<'_, RangeAccessHistory> {
        self.history.lock().unwrap()
    }

    /// Sums decay-weighted retrieval costs for requests fully contained in `cached_range`.
    /// Eviction compares this score per payload byte.
    pub fn retention_score(&self, cached_range: ByteRange) -> f64 {
        self.lock().retention_score(cached_range, self.clock())
    }
}

impl Drop for ObjectAccessHistory {
    fn drop(&mut self) {
        let mut objects = self.shard.objects.lock().unwrap();
        // A concurrent admission may already have replaced an expired weak reference.
        if objects
            .get(&self.key)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
        {
            objects.remove(&self.key);
        }
    }
}

/// Fixed source-request cost as equivalent transferred bytes; zero scores only bytes.
/// Reads `FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES` once on first use, defaulting to
/// 10,000,000 (125 ms at 80 MB/s). Accepts size suffixes; panics unless the value fits `u64`.
pub static FIXED_RETRIEVAL_EQUIVALENT_BYTES: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES", 10_000_000, 0).unwrap_or_else(|error| panic!("{error}"))
});
/// Maximum exact access events retained for range trimming for one object key.
/// Reads `FEUER_MAX_ACCESS_EVENTS_PER_KEY` once on first use, defaulting to 64.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `usize`.
pub static MAX_ACCESS_EVENTS_PER_KEY: LazyLock<usize> = LazyLock::new(|| {
    read_env_number("FEUER_MAX_ACCESS_EVENTS_PER_KEY", 64, 1).unwrap_or_else(|error| panic!("{error}"))
});
/// Maximum same-shard successful-access age that still contributes to range trimming.
/// Reads `FEUER_MAX_ACCESS_AGE_ACCESSES` once on first use, defaulting to 262,144.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `u64`.
pub static MAX_ACCESS_AGE_ACCESSES: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_MAX_ACCESS_AGE_ACCESSES", 262_144, 1).unwrap_or_else(|error| panic!("{error}"))
});

/// One exact requested interval and its shard-local observation clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RangeAccess {
    range: ByteRange,
    observed_at_access: u64,
}

/// Half-life of access counts, in successful accesses to the same shard.
/// Reads `FEUER_ACCESS_COUNT_HALF_LIFE` once on first use, defaulting to 8192.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `u64`.
pub static ACCESS_COUNT_HALF_LIFE: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_ACCESS_COUNT_HALF_LIFE", 8192, 1).unwrap_or_else(|error| panic!("{error}"))
});

#[derive(Default)]
struct DecayedAccessCount {
    count: f64,
    observed_at_access: u64,
}

impl DecayedAccessCount {
    fn decayed_count(&self, access_clock: u64) -> f64 {
        // Age is measured in successful accesses to the same shard, not wall-clock time.
        // An earlier clock leaves the count unchanged rather than increasing it.
        let elapsed_accesses = access_clock.saturating_sub(self.observed_at_access);
        let half_lives_elapsed = elapsed_accesses as f64 / *ACCESS_COUNT_HALF_LIFE as f64;

        // Approximate 2^(-age) through IEEE-754 bits: the exponent gives powers of two,
        // and the mantissa interpolates between them. Each decay is up to ~6.15% high;
        // repeated updates can compound that error.
        let decay_factor = if half_lives_elapsed >= 126.0 {
            0.0 // Discard negligible counts rather than constructing subnormal floats.
        } else {
            let exponent_bias = 127.0;
            let mantissa_scale = (1u32 << 23) as f64;
            let float_bits = ((exponent_bias - half_lives_elapsed) * mantissa_scale) as u32;
            f32::from_bits(float_bits) as f64
        };
        self.count * decay_factor
    }
}

/// Per-range decayed counts for scoring and bounded request events for range trimming.
/// Distinct requested ranges must not overlap; cached ranges may cover several requests.
/// Counts are retained until the object's final history owner is released; unlike
/// trimming events, their number is not capped per object.
#[derive(Default)]
pub struct RangeAccessHistory {
    events: VecDeque<RangeAccess>,
    generation: u64,
    access_count_indices: FnvHashMap<ByteRange, usize>,
    // Each counter is stored once: indexed for exact matches, scanned for expanded ranges.
    access_counts: Vec<(ByteRange, DecayedAccessCount)>,
}

impl RangeAccessHistory {
    fn record(&mut self, range: ByteRange, access_clock: u64) {
        self.generation = self.generation.saturating_add(1);
        let index = *self.access_count_indices.entry(range).or_insert_with(|| {
            let index = self.access_counts.len();
            self.access_counts.push((range, DecayedAccessCount::default()));
            index
        });
        let accesses = &mut self.access_counts[index].1;
        accesses.count = accesses.decayed_count(access_clock) + 1.0;
        accesses.observed_at_access = access_clock;
        self.remove_expired_events(access_clock);
        if self.events.len() == *MAX_ACCESS_EVENTS_PER_KEY {
            self.events.pop_front();
        }
        self.events.push_back(RangeAccess {
            range,
            observed_at_access: access_clock,
        });
    }

    /// Iterates exact requested ranges that still contribute to range trimming.
    pub fn active_ranges(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_ {
        self.events
            .iter()
            .filter(move |event| is_within_range_trim_age_limit(**event, access_clock))
            .map(|event| event.range)
    }

    /// Sums decay-weighted retrieval costs for requests fully contained in `cached_range`.
    /// Eviction compares this score per payload byte.
    pub fn retention_score(&self, cached_range: ByteRange, access_clock: u64) -> f64 {
        let fixed_retrieval_cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64;
        if let Some(&index) = self.access_count_indices.get(&cached_range) {
            // Non-overlapping requests mean an exact match cannot contain another request.
            return self.access_counts[index].1.decayed_count(access_clock)
                * (fixed_retrieval_cost + cached_range.len() as f64);
        }
        self.access_counts
            .iter()
            .filter(|(requested, _)| cached_range.contains(*requested))
            .map(|(requested, accesses)| {
                accesses.decayed_count(access_clock) * (fixed_retrieval_cost + requested.len() as f64)
            })
            .sum()
    }

    /// Changes on every recorded request, including accesses served by the other tier.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Removes expired trimming events without discarding the decayed access counts.
    fn remove_expired_events(&mut self, access_clock: u64) {
        while self
            .events
            .front()
            .is_some_and(|event| !is_within_range_trim_age_limit(*event, access_clock))
        {
            self.events.pop_front();
        }
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

/// Checks whether the access is within the configured range-trimming age limit,
/// measured in successful accesses to the same shard rather than elapsed time.
fn is_within_range_trim_age_limit(event: RangeAccess, access_clock: u64) -> bool {
    access_clock.saturating_sub(event.observed_at_access) <= *MAX_ACCESS_AGE_ACCESSES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn fixed_retrieval_cost_environment_override() {
        // Separate processes exercise environment loading without mutating this process's environment.
        if let Ok(value) = std::env::var("FEUER_TEST_FIXED_RETRIEVAL_CHILD") {
            let fixed_cost = value.parse::<u64>().unwrap();
            assert_eq!(*FIXED_RETRIEVAL_EQUIVALENT_BYTES, fixed_cost);
            let mut history = RangeAccessHistory::default();
            history.record(range(0, 4), 0);
            history.record(range(0, 4), 0);
            assert_eq!(
                history.retention_score(range(0, 8), 0),
                (fixed_cost as f64 + 4.0) * 2.0
            );
            assert_eq!(history.retention_score(range(4, 8), 0), 0.0);
            return;
        }
        for fixed_cost in [0, 1_000_000, 10_000_000, u64::MAX] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "retention::tests::fixed_retrieval_cost_environment_override"])
                .env("FEUER_TEST_FIXED_RETRIEVAL_CHILD", fixed_cost.to_string())
                .env("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES", fixed_cost.to_string())
                .env("FEUER_MAX_ACCESS_EVENTS_PER_KEY", "64")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn access_event_limit_environment_override() {
        // Separate processes avoid mutating the environment or reusing an initialized LazyLock.
        if let Ok(value) = std::env::var("FEUER_TEST_ACCESS_EVENTS_CHILD") {
            let limit = value.parse::<usize>().unwrap();
            assert_eq!(*MAX_ACCESS_EVENTS_PER_KEY, limit);
            let mut history = RangeAccessHistory::default();
            for index in 0..limit + 3 {
                history.record(range(index as u64, index as u64 + 1), 0);
            }
            assert_eq!(history.len(), limit);
            assert_eq!(history.ranges()[0], range(3, 4));
            assert_eq!(history.ranges()[limit - 1], range(limit as u64 + 2, limit as u64 + 3));
            assert!(history.retention_score(range(0, 1), 0) > 0.0);
            return;
        }
        for limit in [1, 256] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "retention::tests::access_event_limit_environment_override"])
                .env("FEUER_TEST_ACCESS_EVENTS_CHILD", limit.to_string())
                .env("FEUER_MAX_ACCESS_EVENTS_PER_KEY", limit.to_string())
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn access_count_half_life_environment_override() {
        // Separate processes avoid mutating the environment or reusing an initialized LazyLock.
        if let Ok(value) = std::env::var("FEUER_TEST_HALF_LIFE_CHILD") {
            let half_life = value.parse::<u64>().unwrap();
            assert_eq!(*ACCESS_COUNT_HALF_LIFE, half_life);
            let mut history = RangeAccessHistory::default();
            history.record(range(0, 1), 0);
            let cost = history.retention_score(range(0, 1), 0);
            assert_eq!(history.retention_score(range(0, 1), half_life), cost * 0.5);
            return;
        }
        for (setting, half_life) in [
            ("1", 1),
            ("256", 256),
            ("64KiB", 65_536),
            ("18446744073709551615", u64::MAX),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "retention::tests::access_count_half_life_environment_override",
                ])
                .env("FEUER_TEST_HALF_LIFE_CHILD", half_life.to_string())
                .env("FEUER_ACCESS_COUNT_HALF_LIFE", setting)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn access_age_environment_override() {
        // A fresh process tests environment loading without mutating this test process's environment.
        if std::env::var_os("FEUER_TEST_ACCESS_AGE_CHILD").is_some() {
            assert_eq!(*MAX_ACCESS_AGE_ACCESSES, 65_536);
            let mut history = RangeAccessHistory::default();
            history.record(range(0, 1), 0);
            assert!(history.retention_score(range(0, 1), 65_536) > 0.0);
            assert!(history.retention_score(range(0, 1), 65_537) > 0.0);
            assert_eq!(history.active_ranges(65_537).count(), 0);
            history.record(range(1, 2), 65_537);
            assert_eq!(history.ranges(), vec![range(1, 2)]);
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "retention::tests::access_age_environment_override"])
            .env("FEUER_TEST_ACCESS_AGE_CHILD", "1")
            .env("FEUER_MAX_ACCESS_AGE_ACCESSES", "64KiB")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn owners_share_evidence_and_release_registry_state() {
        let histories = ObjectAccessHistories::new(1);
        let key = "object".to_owned();
        let memory = histories.for_key(&key);
        let disk = histories.for_key(&key);
        assert!(Arc::ptr_eq(&memory, &disk));
        histories.record_access(&key, range(0, 1));
        assert_eq!(disk.lock().generation(), 1);
        drop(memory);
        histories.record_access(&key, range(2, 3));
        assert_eq!(disk.lock().generation(), 2);
        assert!(disk.retention_score(range(0, 1)) > 0.0);
        drop(disk);
        assert!(histories.shards[0].objects.lock().unwrap().is_empty());
        // Lookup evidence without any retained entry must not leave an unbounded key registry.
        histories.record_access(&key, range(0, 1));
        assert!(histories.shards[0].objects.lock().unwrap().is_empty());
        let fresh = histories.for_key(&key);
        assert_eq!(fresh.lock().generation(), 0);
        assert_eq!(fresh.retention_score(range(0, 1)), 0.0);
    }

    #[test]
    fn concurrent_last_owner_release_does_not_remove_a_new_history() {
        let histories = Arc::new(ObjectAccessHistories::new(1));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let histories = histories.clone();
            threads.push(std::thread::spawn(move || {
                let key = "object".to_owned();
                for _ in 0..1000 {
                    let first = histories.for_key(&key);
                    first.record(range(0, 1));
                    let second = histories.for_key(&key);
                    assert!(Arc::ptr_eq(&first, &second));
                }
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(histories.shards[0].objects.lock().unwrap().is_empty());
    }

    #[test]
    fn sampling_is_bounded_and_value_comparison_does_not_overflow() {
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
    fn bounds_events_without_coalescing_repeated_ranges() {
        let repeated = range(10, 20);
        let mut history = RangeAccessHistory::default();
        let limit = *MAX_ACCESS_EVENTS_PER_KEY;
        for index in 0..limit + 3 {
            let requested = if index >= limit {
                repeated
            } else {
                range(index as u64, index as u64 + 1)
            };
            history.record(requested, 0);
        }

        assert_eq!(history.len(), limit);
        let mut expected: Vec<_> = (3..limit).map(|index| range(index as u64, index as u64 + 1)).collect();
        expected.extend(std::iter::repeat_n(repeated, limit.min(3)));
        assert_eq!(history.ranges(), expected);
    }

    #[test]
    fn retrieval_value_decays_and_new_accesses_add_one() {
        let requested = range(2, 4);
        let cached_range = range(0, 8);
        let mut history = RangeAccessHistory::default();
        let cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + requested.len() as f64;
        history.record(requested, 0);
        assert_eq!(history.retention_score(cached_range, 0), cost);
        assert_eq!(
            history.retention_score(cached_range, *ACCESS_COUNT_HALF_LIFE),
            cost * 0.5
        );
        history.record(requested, *ACCESS_COUNT_HALF_LIFE);
        assert_eq!(
            history.retention_score(cached_range, *ACCESS_COUNT_HALF_LIFE),
            cost * 1.5
        );
        assert_eq!(
            history.retention_score(cached_range, *ACCESS_COUNT_HALF_LIFE * 2),
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
            history.retention_score(first, *ACCESS_COUNT_HALF_LIFE),
            (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0) * 1.5
        );
    }

    #[test]
    fn exact_and_expanded_scores_match_non_overlapping_btree_counts() {
        let mut history = RangeAccessHistory::default();
        let mut reference = std::collections::BTreeMap::<(u64, u64), DecayedAccessCount>::new();
        for (index, (start, end)) in [(8, 10), (2, 4), (0, 1), (4, 8), (8, 10), (12, 16), (2, 4)]
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
                let actual = history.retention_score(range(start, end), 10_000);
                // Expanded-range sums follow insertion order rather than sorted range order.
                assert!(
                    (actual - expected).abs() <= expected.abs() * 1e-12,
                    "{start}..{end}: {actual} != {expected}"
                );
            }
        }
        assert_eq!(
            RangeAccessHistory::default().retention_score(range(0, 1), 0),
            0.0
        );
    }

    #[test]
    fn other_ranges_do_not_displace_decayed_counts() {
        let mut history = RangeAccessHistory::default();
        history.record(range(0, 1), 0);
        for _ in 0..*MAX_ACCESS_EVENTS_PER_KEY {
            history.record(range(2, 3), 0);
        }
        assert!(!history.ranges().contains(&range(0, 1)));
        assert_eq!(
            history.retention_score(range(0, 1), 0),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0
        );
    }

    #[test]
    fn approximate_decay_is_monotonic_and_bounded() {
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
            history.retention_score(range(0, 100), 30_000) / (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 100.0);
        // Each event undergoes at most one decay per recorded access, including the final read.
        let max_decay_steps = clocks.len() as i32;
        assert!(actual >= expected * (1.0_f64 - 1e-7).powi(max_decay_steps));
        assert!(actual <= expected * 1.0615_f64.powi(max_decay_steps));
    }

    #[test]
    fn credits_only_cached_ranges_covering_the_exact_request() {
        let mut history = RangeAccessHistory::default();
        history.record(range(3, 7), 0);

        assert_eq!(
            history.retention_score(range(0, 8), 0),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 4.0
        );
        assert_eq!(history.retention_score(range(3, 5), 0), 0.0);
        assert_eq!(history.retention_score(range(5, 8), 0), 0.0);

        let mut history = RangeAccessHistory::default();
        history.record(range(3, 5), 0);
        history.record(range(5, 8), 0);
        assert_eq!(
            history.retention_score(range(3, 5), 0),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 2.0
        );
        assert_eq!(
            history.retention_score(range(0, 8), 0),
            (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 2.0) + (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 3.0)
        );
        history.record(range(u64::MAX - 1, u64::MAX), 0);
        assert_eq!(
            history.retention_score(range(u64::MAX - 1, u64::MAX), 0),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0
        );
    }
}
