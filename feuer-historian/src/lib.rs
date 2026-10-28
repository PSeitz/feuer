//! Request history for immutable objects, independent of cache contents.
//!
//! Keeps decayed request counts for retrieval costs and retention decisions.

use std::{
    hash::BuildHasher,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use feuer_types::ObjectKeyHash;
use rustc_hash::{FxBuildHasher, FxHashMap};

/// Independent of the number of memory or disk cache shards.
const ACCESS_HISTORY_SHARDS: usize = 64;

/// Request history shared by both cache tiers, independent of their shards and entries.
/// Object keys select one of 64 independently locked history maps. The request clock remains global.
/// Each key's counter lasts for the history's lifetime, including after cache eviction.
/// A restart begins with empty history.
pub struct ObjectAccessHistories {
    shards: [Mutex<FxHashMap<ObjectKeyHash, DecayedAccessCount>>; ACCESS_HISTORY_SHARDS],
    request_count: AtomicU64,
    half_life: u64,
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
    /// Panics if the half-life is zero.
    pub fn with_half_life(half_life: u64) -> Self {
        assert!(half_life > 0, "access count half-life must be greater than zero");
        Self {
            shards: std::array::from_fn(|_| Mutex::default()),
            request_count: AtomicU64::new(0),
            half_life,
        }
    }

    /// Returns the decay half-life used when recording and evaluating request counts.
    pub fn half_life(&self) -> u64 {
        self.half_life
    }

    /// Records one request when it starts, before any cache lookup or source fetch.
    /// Failed and canceled requests contribute to demand just like successful requests.
    /// Repeated requests update the same counter.
    /// Cache insertion and eviction do not record or remove history.
    pub fn record_access(&self, key: &ObjectKeyHash) {
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
            .map_or(0.0, |count| count.decayed_count(self.request_count(), self.half_life))
    }

    fn shard(&self, key: &ObjectKeyHash) -> &Mutex<FxHashMap<ObjectKeyHash, DecayedAccessCount>> {
        &self.shards[FxBuildHasher.hash_one(key) as usize % self.shards.len()]
    }
}

/// Default half-life of access counts, in requests across all keys.
pub const DEFAULT_ACCESS_COUNT_HALF_LIFE: u64 = 262_144;

/// A request count that decays as requests accumulate across all object keys.
#[derive(Default)]
pub struct DecayedAccessCount {
    count: f64,
    last_update_clock: u64,
}

impl DecayedAccessCount {
    fn add_request(&mut self, request_clock: u64, half_life: u64) {
        self.count = self.decayed_count(request_clock, half_life) + 1.0;
        self.last_update_clock = request_clock;
    }

    /// Evaluates the count at the supplied request clock, without modifying it.
    /// `half_life` must be positive and match the half-life used to record this count.
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
    pub fn decayed_count(&self, request_clock: u64, half_life: u64) -> f64 {
        let requests_since_update = request_clock.saturating_sub(self.last_update_clock);
        let half_lives_elapsed = requests_since_update as f64 / half_life as f64;

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
mod tests;
