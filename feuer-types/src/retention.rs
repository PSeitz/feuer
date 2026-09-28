//! Internal shared access evidence and payload-value comparison for both cache tiers.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, VecDeque, hash_map::DefaultHasher},
    ffi::OsStr,
    hash::{Hash, Hasher},
    sync::{
        Arc, LazyLock, Mutex, MutexGuard, Weak,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
};

use crate::{ByteRange, ObjectKey};

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
/// 10,000,000 (125 ms at 80 MB/s). Panics unless set to a nonnegative `u64` integer.
pub static FIXED_RETRIEVAL_EQUIVALENT_BYTES: LazyLock<u64> = LazyLock::new(|| {
    parse_fixed_retrieval_equivalent_bytes(std::env::var_os("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES").as_deref())
});
/// Maximum exact access events retained for range trimming for one object key.
/// Reads `FEUER_MAX_ACCESS_EVENTS_PER_KEY` once on first use, defaulting to 64.
/// Panics if set to anything other than a positive `usize` integer.
pub static MAX_ACCESS_EVENTS_PER_KEY: LazyLock<usize> =
    LazyLock::new(|| parse_max_access_events_per_key(std::env::var_os("FEUER_MAX_ACCESS_EVENTS_PER_KEY").as_deref()));
/// Maximum same-shard successful-access age that still contributes to range trimming.
/// Reads `FEUER_MAX_ACCESS_AGE_ACCESSES` once on first use, defaulting to 262,144.
/// Panics if set to anything other than a positive `u64` integer.
pub static MAX_ACCESS_AGE_ACCESSES: LazyLock<u64> =
    LazyLock::new(|| parse_max_access_age(std::env::var_os("FEUER_MAX_ACCESS_AGE_ACCESSES").as_deref()));

fn parse_fixed_retrieval_equivalent_bytes(value: Option<&OsStr>) -> u64 {
    let Some(value) = value else {
        return 10_000_000;
    };
    value
        .to_str()
        .and_then(|value| value.parse::<u64>().ok())
        .expect("FEUER_FIXED_RETRIEVAL_EQUIVALENT_BYTES must be a nonnegative u64 integer")
}

fn parse_max_access_events_per_key(value: Option<&OsStr>) -> usize {
    let Some(value) = value else {
        return 64;
    };
    value
        .to_str()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&value| value > 0)
        .expect("FEUER_MAX_ACCESS_EVENTS_PER_KEY must be a positive usize integer")
}

fn parse_max_access_age(value: Option<&OsStr>) -> u64 {
    let Some(value) = value else {
        return 262_144;
    };
    value
        .to_str()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&value| value > 0)
        .expect("FEUER_MAX_ACCESS_AGE_ACCESSES must be a positive u64 integer")
}

/// One exact requested interval and its shard-local observation clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RangeAccess {
    range: ByteRange,
    observed_at_access: u64,
}

/// Half-life of access counts, in successful accesses to the same shard.
/// Reads `FEUER_ACCESS_COUNT_HALF_LIFE` once on first use, defaulting to 8192.
/// Panics unless set to a positive `u64` integer.
pub static ACCESS_COUNT_HALF_LIFE: LazyLock<u64> =
    LazyLock::new(|| parse_access_count_half_life(std::env::var_os("FEUER_ACCESS_COUNT_HALF_LIFE").as_deref()));

fn parse_access_count_half_life(value: Option<&OsStr>) -> u64 {
    let Some(value) = value else {
        return 8192;
    };
    value
        .to_str()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&value| value > 0)
        .expect("FEUER_ACCESS_COUNT_HALF_LIFE must be a positive u64 integer")
}

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
/// Counts are retained until the object's final history owner is released; unlike
/// trimming events, their number is not capped per object.
#[derive(Default)]
pub struct RangeAccessHistory {
    events: VecDeque<RangeAccess>,
    generation: u64,
    access_counts: BTreeMap<(u64, u64), DecayedAccessCount>,
}

impl RangeAccessHistory {
    fn record(&mut self, range: ByteRange, access_clock: u64) {
        self.generation = self.generation.saturating_add(1);
        let accesses = self.access_counts.entry((range.start(), range.end())).or_default();
        accesses.count = accesses.decayed_count(access_clock) + 1.0;
        accesses.observed_at_access = access_clock;
        self.expire(access_clock);
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
            .filter(move |event| is_active(**event, access_clock))
            .map(|event| event.range)
    }

    /// Sums decay-weighted retrieval costs for requests fully contained in `cached_range`.
    /// Eviction compares this score per payload byte.
    pub fn retention_score(&self, cached_range: ByteRange, access_clock: u64) -> f64 {
        let fixed_retrieval_cost = *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64;
        self.access_counts
            .range((cached_range.start(), 0)..(cached_range.end(), 0))
            .filter(|&(&(_, end), _)| end <= cached_range.end())
            .map(|(&(start, end), accesses)| {
                accesses.decayed_count(access_clock) * (fixed_retrieval_cost + (end - start) as f64)
            })
            .sum()
    }

    /// Changes on every recorded request, including accesses served by the other tier.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn expire(&mut self, access_clock: u64) {
        while self
            .events
            .front()
            .is_some_and(|event| !is_active(*event, access_clock))
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

fn is_active(event: RangeAccess, access_clock: u64) -> bool {
    access_clock.saturating_sub(event.observed_at_access) <= *MAX_ACCESS_AGE_ACCESSES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn parses_fixed_retrieval_equivalent_bytes() {
        assert_eq!(parse_fixed_retrieval_equivalent_bytes(None), 10_000_000);
        for value in [0, 1, 1_000_000, 10_000_000, u64::MAX] {
            assert_eq!(
                parse_fixed_retrieval_equivalent_bytes(Some(OsStr::new(&value.to_string()))),
                value
            );
        }
        for value in ["", "-1", "1MB", "1.5", " 1", "1 ", "18446744073709551616"] {
            assert!(
                std::panic::catch_unwind(|| parse_fixed_retrieval_equivalent_bytes(Some(OsStr::new(value)))).is_err()
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(
                std::panic::catch_unwind(|| parse_fixed_retrieval_equivalent_bytes(Some(OsStr::from_bytes(b"\xff"))))
                    .is_err()
            );
        }
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
    fn parses_max_access_events_per_key() {
        assert_eq!(parse_max_access_events_per_key(None), 64);
        for value in [1, 16, 64, 256, 1024, usize::MAX] {
            assert_eq!(
                parse_max_access_events_per_key(Some(OsStr::new(&value.to_string()))),
                value
            );
        }
        for value in ["", "0", "-1", "64k", "1.5", " 256", "256 "] {
            assert!(std::panic::catch_unwind(|| parse_max_access_events_per_key(Some(OsStr::new(value)))).is_err());
        }
        let overflow = (usize::MAX as u128 + 1).to_string();
        assert!(std::panic::catch_unwind(|| parse_max_access_events_per_key(Some(OsStr::new(&overflow)))).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(
                std::panic::catch_unwind(|| parse_max_access_events_per_key(Some(OsStr::from_bytes(b"\xff")))).is_err()
            );
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
        for half_life in [1, 256, 65_536, u64::MAX] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "retention::tests::access_count_half_life_environment_override",
                ])
                .env("FEUER_TEST_HALF_LIFE_CHILD", half_life.to_string())
                .env("FEUER_ACCESS_COUNT_HALF_LIFE", half_life.to_string())
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }

    #[test]
    fn parses_max_access_age() {
        assert_eq!(parse_max_access_age(None), 262_144);
        for value in [1, 32_768, 65_536, 131_072, 262_144, u64::MAX] {
            assert_eq!(parse_max_access_age(Some(OsStr::new(&value.to_string()))), value);
        }
        for value in ["", "0", "-1", "64k", "1.5", " 65536", "18446744073709551616"] {
            assert!(std::panic::catch_unwind(|| parse_max_access_age(Some(OsStr::new(value)))).is_err());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(std::panic::catch_unwind(|| parse_max_access_age(Some(OsStr::from_bytes(b"\xff")))).is_err());
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
            .env("FEUER_MAX_ACCESS_AGE_ACCESSES", "65536")
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

        history.record(range(3, 5), 0);
        history.record(range(5, 8), 0);
        assert_eq!(
            history.retention_score(range(3, 5), 0),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 2.0
        );
        assert_eq!(
            history.retention_score(range(0, 8), 0),
            (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 4.0)
                + (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 2.0)
                + (*FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 3.0)
        );
        history.record(range(u64::MAX - 1, u64::MAX), 0);
        assert_eq!(
            history.retention_score(range(u64::MAX - 1, u64::MAX), 0),
            *FIXED_RETRIEVAL_EQUIVALENT_BYTES as f64 + 1.0
        );
    }
}
