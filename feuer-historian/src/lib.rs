//! Request history for immutable objects, independent of cache contents.
//!
//! Keeps decayed request counts for retrieval costs and retention decisions.

use std::{
    array::from_fn,
    hash::BuildHasher,
    num::{NonZeroU64, NonZeroUsize},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use feuer_types::ObjectKeyHash;
use rustc_hash::{FxBuildHasher, FxHashMap};

/// Independent of the number of memory or disk cache shards.
const ACCESS_HISTORY_SHARDS: usize = 64;

/// Request history shared by both cache tiers.
/// Object keys select one of 64 independently locked history maps. The request clock remains global.
/// Counters survive cache eviction. Once per second, one background worker checks the entry limit
/// and removes the least recently requested keys, targeting 90% of the limit.
/// The limit is a soft threshold. A restart begins with empty history.
pub struct ObjectAccessHistories {
    shards: Arc<[Mutex<FxHashMap<ObjectKeyHash, DecayedRequestCount>>; ACCESS_HISTORY_SHARDS]>,
    request_count: AtomicU64,
    half_life: NonZeroU64,
}

impl Default for ObjectAccessHistories {
    fn default() -> Self {
        Self::with_half_life(DEFAULT_ACCESS_COUNT_HALF_LIFE)
    }
}

impl ObjectAccessHistories {
    /// Creates empty request history with one clock across all keys and cache tiers.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates empty history with a fixed decay half-life in requests across all keys.
    pub fn with_half_life(half_life: NonZeroU64) -> Self {
        Self::with_half_life_and_entry_limit(half_life, DEFAULT_HISTORY_ENTRY_LIMIT)
    }

    /// Creates empty history with a fixed decay half-life and an entry-count cleanup threshold.
    /// Cleanup needs no async runtime and locks only one history shard at a time.
    pub fn with_half_life_and_entry_limit(half_life: NonZeroU64, entry_limit: NonZeroUsize) -> Self {
        let shards = Arc::new(from_fn(|_| Mutex::default()));
        let weak = Arc::downgrade(&shards);
        thread::spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(1));
                let Some(shards) = weak.upgrade() else { break };
                Self::remove_oldest(&*shards, entry_limit.get());
            }
        });
        Self {
            shards,
            request_count: AtomicU64::new(0),
            half_life,
        }
    }

    fn remove_oldest(shards: &[Mutex<FxHashMap<ObjectKeyHash, DecayedRequestCount>>], limit: usize) {
        let count: usize = shards.iter().map(|shard| shard.lock().unwrap().len()).sum();
        if count <= limit {
            return;
        }
        let mut clocks = Vec::with_capacity(count);
        for shard in shards {
            clocks.extend(shard.lock().unwrap().values().map(|count| count.last_update_clock));
        }
        let cutoff_index = clocks.len() - (limit - limit / 10) - 1;
        let cutoff = *clocks.select_nth_unstable(cutoff_index).1;
        for shard in shards {
            let mut histories = shard.lock().unwrap();
            // Requests since the snapshot have newer clocks and keep their history.
            histories.retain(|_, count| count.last_update_clock > cutoff);
            histories.shrink_to_fit();
        }
    }

    /// Returns the decay half-life in requests across all keys.
    pub fn half_life(&self) -> NonZeroU64 {
        self.half_life
    }

    /// Records one request when it starts, before any cache lookup or source fetch.
    /// Failed and canceled requests contribute to demand just like successful requests.
    /// Repeated requests update the same counter.
    /// Cache insertion and eviction do not record or remove history.
    pub fn record_request(&self, key: &ObjectKeyHash) {
        let mut histories = self.shard(key).lock().unwrap();
        // Assign the clock under the shard lock so this key's updates cannot arrive out of order.
        // The atomic only measures request age. The shard mutex protects the history itself.
        let request_clock = self.request_count.fetch_add(1, Ordering::Relaxed) + 1;
        histories
            .entry(*key)
            .or_default()
            .add_request(request_clock, self.half_life);
    }

    /// Number of requests recorded across all keys and cache tiers.
    /// Other shards may still be applying their counter updates.
    pub fn request_count(&self) -> u64 {
        self.request_count.load(Ordering::Relaxed)
    }

    /// Returns the object's request count decayed to the global request clock.
    /// Unknown keys have a count of zero.
    pub fn decayed_request_count(&self, key: &ObjectKeyHash) -> f64 {
        let histories = self.shard(key).lock().unwrap();
        histories
            .get(key)
            .map(|count| count.decayed_count(self.request_count(), self.half_life))
            .unwrap_or(0.0)
    }

    fn shard(&self, key: &ObjectKeyHash) -> &Mutex<FxHashMap<ObjectKeyHash, DecayedRequestCount>> {
        &self.shards[FxBuildHasher.hash_one(key) as usize % self.shards.len()]
    }
}

/// Default half-life of access counts, in requests across all keys.
pub const DEFAULT_ACCESS_COUNT_HALF_LIFE: NonZeroU64 = NonZeroU64::new(262_144).unwrap();

/// Default entry count above which background cleanup removes old history.
pub const DEFAULT_HISTORY_ENTRY_LIMIT: NonZeroUsize = NonZeroUsize::new(2_000_000).unwrap();

/// A request count that decays as requests accumulate across all object keys.
#[derive(Default)]
pub struct DecayedRequestCount {
    count: f64,
    last_update_clock: u64,
}

impl DecayedRequestCount {
    fn add_request(&mut self, request_clock: u64, half_life: NonZeroU64) {
        self.count = self.decayed_count(request_clock, half_life) + 1.0;
        self.last_update_clock = request_clock;
    }

    /// Evaluates the count at the supplied request clock, without modifying it.
    /// `half_life` must match the half-life used to record this count.
    /// The result is between zero and the count stored at the last update, inclusive.
    /// Each matching request decays the stored count and adds one,
    /// so repeated requests can make it exceed one.
    ///
    /// Age is measured in requests across all object keys.
    /// With the default half-life of 262,144 requests, starting at one with no further matching requests:
    ///
    /// | Additional requests across all keys | Value |
    /// | ---: | ---: |
    /// | 0 | 1 |
    /// | 262,144 | 0.5 |
    /// | 524,288 | 0.25 |
    ///
    /// After 126 half-lives without an update, the result is zero.
    /// Earlier clocks return the stored count unchanged.
    ///
    /// Decay approximates `count * 2^(-requests_since_update / half_life)` and is
    /// up to 6.15% high per evaluation. Errors can compound across recorded updates.
    ///
    /// # Repeated requests
    ///
    /// Requests for the same object key update the count as
    /// `next = retained_fraction * previous + 1`. With equally spaced matching requests,
    /// the count after each update approaches `1 / (1 - retained_fraction)`.
    /// These examples use the default half-life and this implementation's approximation:
    ///
    /// | Matching request frequency | Retained fraction per update | Count approached after each update |
    /// | --- | ---: | ---: |
    /// | Every request | `1 - 1/524288` | 524,288 |
    /// | Every second request | `1 - 1/262144` | 262,144 |
    /// | Every 128,000 requests | 0.755859375 | 4.096 |
    /// | Every 131,072 requests | 0.75 | 4 |
    ///
    /// At 131,072 requests apart, the counts after matching requests are
    /// `1 -> 1.75 -> 2.3125 -> 2.734375 -> ... -> 4`.
    /// Between matching requests, a count near 4 decays toward 3 before the next update adds one.
    /// At 128,000 requests apart, the corresponding limits are 4.096 after adding one and 3.096 before.
    #[inline]
    pub fn decayed_count(&self, request_clock: u64, half_life: NonZeroU64) -> f64 {
        let requests_since_update = request_clock.saturating_sub(self.last_update_clock);
        let half_lives_elapsed = requests_since_update as f64 / half_life.get() as f64;

        if half_lives_elapsed >= 126.0 {
            return 0.0; // Discard negligible counts rather than constructing subnormal floats.
        }

        // Approximate 2^(-half_lives_elapsed) through IEEE-754 bits: the exponent gives powers of two,
        // and the mantissa interpolates between them. Each decay is up to ~6.15% high.
        // Repeated updates can compound that error.
        let exponent_bias = 127.0;
        let mantissa_scale = (1u32 << 23) as f64;
        let float_bits = ((exponent_bias - half_lives_elapsed) * mantissa_scale) as u32;
        self.count * f32::from_bits(float_bits) as f64
    }
}

#[cfg(test)]
mod cleanup_tests;
#[cfg(test)]
mod tests;
