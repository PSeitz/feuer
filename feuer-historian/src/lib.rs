//! Request history for immutable objects and byte ranges, independent of cache contents.
//!
//! Keeps decayed request counts and recent request events. Consumers interpret this
//! evidence; history does not assign retrieval costs or make retention decisions.

use std::{
    collections::VecDeque,
    hash::BuildHasher,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering as AtomicOrdering},
    },
};

use feuer_types::{ByteRange, ObjectKeyHash, config::read_env_number};
use rustc_hash::{FxBuildHasher, FxHashMap};

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
    /// `None` records whole-object demand. Do not mix whole-object and range requests for a key.
    pub fn record_access(&self, key: &ObjectKeyHash, requested: impl Into<Option<ByteRange>>) {
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

    /// Reads distinct request counters in insertion order and one global request-clock value.
    /// `None` denotes whole-object demand; unknown keys supply an empty slice.
    /// Counters are borrowed without copying or allocation. Consumers can filter requests
    /// before calling [`DecayedAccessCount::decayed_count`] with the supplied clock.
    /// The key's shard stays locked during `read`; do not re-enter this history from it.
    pub fn with_access_counts<T>(
        &self,
        key: &ObjectKeyHash,
        read: impl FnOnce(&[(Option<ByteRange>, DecayedAccessCount)], u64) -> T,
    ) -> T {
        let objects = self.shard(key).lock().unwrap();
        let counts = objects
            .get(key)
            .map_or(&[][..], |history| history.access_counts.as_slice());
        read(counts, self.request_count())
    }

    /// Snapshots recent requested byte ranges, including repeats but excluding whole-object requests.
    /// Later accesses do not invalidate the snapshot.
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

/// A permanent range-counter index and the global request-counter value assigned when the access was recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RangeAccess {
    count_index: usize,
    observed_at_access: u64,
}

/// Half-life of access counts, in requests across all keys.
/// Reads `FEUER_ACCESS_COUNT_HALF_LIFE` once on first use, defaulting to 262,144.
/// Accepts size suffixes as multipliers; panics unless the result is positive and fits `u64`.
pub static ACCESS_COUNT_HALF_LIFE: LazyLock<u64> = LazyLock::new(|| {
    read_env_number("FEUER_ACCESS_COUNT_HALF_LIFE", 262_144, 1).unwrap_or_else(|error| panic!("{error}"))
});

/// A request count weighted by age in requests across all object keys.
#[derive(Default)]
pub struct DecayedAccessCount {
    count: f64,
    observed_at_access: u64,
}

impl DecayedAccessCount {
    /// Evaluates the count at the supplied request clock, without modifying it.
    /// Earlier clocks leave the count unchanged. Decay uses an approximation that is
    /// up to 6.15% high per evaluation; errors can compound across recorded updates.
    #[inline]
    pub fn decayed_count(&self, access_clock: u64) -> f64 {
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

/// Per-range decayed counts and recent request events.
/// Events have a per-object count limit and an age limit measured in requests across all keys.
/// Unlike events, distinct counters are never removed or capped.
#[derive(Default)]
struct RangeAccessHistory {
    events: VecDeque<RangeAccess>,
    access_count_indices: FxHashMap<Option<ByteRange>, usize>,
    // Each counter is stored once: indexed for recording, scanned for scoring.
    access_counts: Vec<(Option<ByteRange>, DecayedAccessCount)>,
}

impl RangeAccessHistory {
    fn record(&mut self, range: impl Into<Option<ByteRange>>, access_clock: u64) {
        let range = range.into();
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
            count_index,
            observed_at_access: access_clock,
        });
    }

    /// Iterates requested byte ranges from events within the trimming age limit, including repeats.
    pub fn recent_requested_ranges(&self, access_clock: u64) -> impl Iterator<Item = ByteRange> + '_ {
        self.events
            .iter()
            .skip_while(move |event| access_clock.saturating_sub(event.observed_at_access) > *MAX_ACCESS_AGE_ACCESSES)
            .filter_map(|event| self.access_counts[event.count_index].0)
    }
}

#[cfg(test)]
mod tests;
