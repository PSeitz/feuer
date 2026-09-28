//! Internal shared access evidence and payload-value comparison for both cache tiers.

use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
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

/// Compares retrieval value per payload byte without division or rounding.
pub fn compare_cost_per_byte(left_cost: u64, left_bytes: u64, right_cost: u64, right_bytes: u64) -> Ordering {
    (u128::from(left_cost) * u128::from(right_bytes)).cmp(&(u128::from(right_cost) * u128::from(left_bytes)))
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

    /// Recent retrieval value covered by this entry's exact range.
    pub fn covered_retrieval_cost(&self, range: ByteRange) -> u64 {
        self.lock().covered_retrieval_cost(range, self.clock())
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

/// Target 125-ms source-request cost at 80 MB/s, as equivalent transferred bytes.
pub const FIXED_RETRIEVAL_EQUIVALENT_BYTES: u64 = 10_000_000;
/// Maximum exact access events retained for one object key.
pub const MAX_ACCESS_EVENTS_PER_KEY: usize = 64;
/// Maximum same-shard successful-access age that still contributes.
/// Reads `FEUER_MAX_ACCESS_AGE_ACCESSES` once on first use, defaulting to 262,144.
/// Panics if set to anything other than a positive `u64` integer.
pub static MAX_ACCESS_AGE_ACCESSES: LazyLock<u64> =
    LazyLock::new(|| parse_max_access_age(std::env::var_os("FEUER_MAX_ACCESS_AGE_ACCESSES").as_deref()));

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

/// Bounded history of requested byte ranges for one complete object key.
///
/// Events are deliberately not coalesced: repeated requests remain separate
/// records until they expire or are displaced by the per-key bound.
#[derive(Default)]
pub struct RangeAccessHistory {
    events: VecDeque<RangeAccess>,
    generation: u64,
}

impl RangeAccessHistory {
    fn record(&mut self, range: ByteRange, access_clock: u64) {
        self.generation = self.generation.saturating_add(1);
        self.expire(access_clock);
        if self.events.len() == MAX_ACCESS_EVENTS_PER_KEY {
            self.events.pop_front();
        }
        self.events.push_back(RangeAccess {
            range,
            observed_at_access: access_clock,
        });
    }

    /// Iterates exact requested ranges that still have policy value.
    pub fn active_ranges(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_ {
        self.events
            .iter()
            .filter(move |event| is_active(**event, access_clock))
            .map(|event| event.range)
    }

    /// Sums modeled source retrieval cost for active requests covered by a cached range.
    pub fn covered_retrieval_cost(&self, cached_range: ByteRange, access_clock: u64) -> u64 {
        self.events
            .iter()
            .filter(|event| is_active(**event, access_clock) && cached_range.contains(event.range))
            .map(|event| FIXED_RETRIEVAL_EQUIVALENT_BYTES.saturating_add(event.range.len()))
            .fold(0, u64::saturating_add)
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
            assert!(history.covered_retrieval_cost(range(0, 1), 65_536) > 0);
            assert_eq!(history.covered_retrieval_cost(range(0, 1), 65_537), 0);
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
        drop(disk);
        assert!(histories.shards[0].objects.lock().unwrap().is_empty());
        // Lookup evidence without any retained entry must not leave an unbounded key registry.
        histories.record_access(&key, range(0, 1));
        assert!(histories.shards[0].objects.lock().unwrap().is_empty());
        assert_eq!(histories.for_key(&key).lock().generation(), 0);
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
            compare_cost_per_byte(u64::MAX, u64::MAX, u64::MAX - 1, u64::MAX),
            Ordering::Greater
        );
        assert_eq!(compare_cost_per_byte(1, 2, 2, 4), Ordering::Equal);
    }

    #[test]
    fn bounds_events_without_coalescing_repeated_ranges() {
        let repeated = range(10, 20);
        let mut history = RangeAccessHistory::default();
        for index in 0..MAX_ACCESS_EVENTS_PER_KEY + 3 {
            let requested = if index >= MAX_ACCESS_EVENTS_PER_KEY {
                repeated
            } else {
                range(index as u64, index as u64 + 1)
            };
            history.record(requested, 0);
        }

        assert_eq!(history.len(), MAX_ACCESS_EVENTS_PER_KEY);
        assert_eq!(history.ranges()[MAX_ACCESS_EVENTS_PER_KEY - 3..], [repeated; 3]);
        assert_eq!(history.ranges()[0], range(3, 4));
    }

    #[test]
    fn retains_full_retrieval_value_until_expiration() {
        let requested = range(2, 4);
        let cached_range = range(0, 8);
        let mut history = RangeAccessHistory::default();
        history.record(requested, 0);
        history.record(requested, 0);

        let expected = 2 * (FIXED_RETRIEVAL_EQUIVALENT_BYTES + requested.len());
        assert_eq!(history.covered_retrieval_cost(cached_range, 0), expected);
        assert_eq!(
            history.covered_retrieval_cost(cached_range, *MAX_ACCESS_AGE_ACCESSES),
            expected
        );
        assert_eq!(
            history.covered_retrieval_cost(cached_range, *MAX_ACCESS_AGE_ACCESSES + 1),
            0
        );

        history.record(range(6, 7), *MAX_ACCESS_AGE_ACCESSES + 1);
        assert_eq!(history.ranges(), vec![range(6, 7)]);
    }

    #[test]
    fn credits_only_cached_ranges_covering_the_exact_request() {
        let mut history = RangeAccessHistory::default();
        history.record(range(3, 7), 0);

        assert_eq!(
            history.covered_retrieval_cost(range(0, 8), 0),
            FIXED_RETRIEVAL_EQUIVALENT_BYTES + 4
        );
        assert_eq!(history.covered_retrieval_cost(range(3, 5), 0), 0);
        assert_eq!(history.covered_retrieval_cost(range(5, 8), 0), 0);
    }
}
