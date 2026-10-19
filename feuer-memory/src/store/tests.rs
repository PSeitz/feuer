use std::{sync::Arc, thread};

use bytes::Bytes;
use feuer_historian::ObjectAccessHistories;
use feuer_types::{Download, ObjectKeyHash};

use super::{MemoryCache, shard_capacity_for};
use crate::{
    BufferPool, MemoryMetrics,
    test_metrics::{registry, value},
};

fn cache(capacity: u64) -> MemoryCache {
    cache_with_metrics(capacity, MemoryMetrics::noop())
}

fn cache_with_metrics(capacity: u64, metrics: Arc<MemoryMetrics>) -> MemoryCache {
    MemoryCache::with_shard_count(BufferPool::new(capacity, metrics), 1)
}

fn repeat_request(cache: &MemoryCache, key: ObjectKeyHash, count: u64) {
    for _ in 0..count {
        cache.access_histories.record_access(&key);
    }
}

#[test]
fn idle_buffer_pool_percent_is_independent_per_cache() {
    let size = 32 * 1024;
    let capacity = 4 * size as u64;
    let caches: Vec<_> = [0, 25, 100]
        .into_iter()
        .map(|percent| MemoryCache::with_metrics(capacity, MemoryMetrics::noop(), percent))
        .collect();
    for (cache, expected_buffers) in caches.iter().zip([0, 1, 4]) {
        let pool = cache.buffer_pool();
        let buffers: Vec<_> = (0..5).map(|_| pool.allocate(size).unwrap()).collect();
        drop(buffers);
        assert_eq!(pool.idle_bytes(), expected_buffers * size as u64);
        assert_eq!(cache.used_bytes(), expected_buffers * size as u64);
    }
}

#[test]
fn cached_payloads_and_idle_buffers_share_allocation_accounting() {
    let (registry, backend) = registry();
    let capacity = 256 * 1024;
    let cache = cache_with_metrics(100 * capacity as u64, MemoryMetrics::new(&backend));
    let pool = cache.buffer_pool();
    assert!(Arc::ptr_eq(&pool, &cache.buffer_pool()));
    let other_cache = MemoryCache::new(cache.capacity());
    assert!(!Arc::ptr_eq(&pool, &other_cache.buffer_pool()));
    let buffer = pool.allocate(1).unwrap();
    let charge = buffer.capacity();
    let download = buffer.into_download();
    let bytes = download.bytes().clone();
    let key = ObjectKeyHash::from("object");
    assert!(cache.insert(key, download.clone()));
    assert_eq!(cache.used_bytes(), charge as u64);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), charge as f64);
    assert_eq!(cache.get(&key).unwrap().as_ptr(), bytes.as_ptr());
    assert!(!cache.insert(key, download));
    assert_eq!(cache.used_bytes(), charge as u64);
    assert!(cache.remove(&key));
    assert_eq!(cache.used_bytes(), 0);
    drop(bytes);
    assert_eq!(cache.used_bytes(), charge as u64);
    assert_eq!(pool.idle_bytes(), charge as u64);
    drop(pool);
    drop(cache);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_memory_capacity_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_memory_entries", &[]), 0.0);
}

#[test]
fn cached_capacity_drives_eviction_not_payload_length() {
    let cache = cache(32 * 1024);
    for key in ["first", "second"] {
        let buffer = cache.buffer_pool().allocate(1).unwrap();
        cache.insert(key.into(), buffer.into_download());
    }
    assert!(cache.get(&"first".into()).is_none());
    assert!(cache.get(&"second".into()).is_some());
    assert_eq!(cache.used_bytes(), 32 * 1024);
    assert_eq!(cache.buffer_pool().idle_bytes(), 0);
    cache.remove(&"second".into());
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn cached_entries_displace_idle_buffers_and_released_bursts_cannot_displace_entries() {
    let buffer_capacity = 256 * 1024;
    let capacity = 100 * buffer_capacity;
    let cache = cache(capacity as u64);
    let pool = cache.buffer_pool();
    let burst: Vec<_> = (0..8).map(|_| pool.allocate(buffer_capacity).unwrap()).collect();
    drop(burst);
    assert_eq!(pool.idle_bytes(), capacity as u64 * 7 / 100);
    drop(pool.allocate(1).unwrap());
    assert_eq!(pool.idle_bytes(), capacity as u64 * 7 / 100);
    let waiting = pool.allocate(buffer_capacity).unwrap();
    cache.insert("full".into(), Download::new(Bytes::from(vec![0; capacity])));
    assert_eq!(pool.idle_bytes(), 0);
    drop(waiting);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(cache.used_bytes(), capacity as u64);
    assert_eq!(cache.get(&"full".into()).unwrap().len(), capacity);
}

#[test]
fn entry_presence_tracks_removal_and_reinsertion() {
    let cache = cache(1024);
    let key = ObjectKeyHash::from("object");
    let payload = Download::new(Bytes::from_static(b"abcd"));
    assert!(!cache.remove(&key));
    assert!(!cache.contains_entry(&key));
    assert!(cache.insert(key, payload.clone()));
    assert!(cache.contains_entry(&key));
    assert!(!cache.insert(key, payload.clone()));
    assert!(cache.remove(&key));
    assert!(!cache.remove(&key));
    assert!(!cache.contains_entry(&key));
    assert!(cache.insert(key, payload));
    assert!(cache.contains_entry(&key));
}

#[test]
fn redundant_insertions_preserve_cached_bytes_and_usage() {
    let cache = cache(16);
    let key = ObjectKeyHash::from("object");
    let bytes = Bytes::from_static(b"abcdefghij");
    cache.insert(key, Download::new(bytes.clone()));
    for length in [0, 6, 10, 20] {
        let redundant = Download::new(Bytes::from(vec![b'X'; length])).with_allocation_charge(100);
        for _ in 0..200 {
            assert!(!cache.insert(key, redundant.clone()));
        }
        assert_eq!(cache.used_bytes(), 10);
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.get(&key).unwrap(), bytes);
        assert_eq!(cache.get(&key).unwrap().as_ptr(), bytes.as_ptr());
    }
    assert!(cache.get(&"missing".into()).is_none());
}

#[test]
fn empty_objects_are_present_without_payload_charges() {
    let cache = cache(0);
    let key = ObjectKeyHash::from("empty");
    assert!(cache.insert(key, Download::new(Bytes::new())));
    assert_eq!(cache.get(&key), Some(Bytes::new()));
    assert!(cache.contains_entry(&key));
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(cache.used_bytes(), 0);
    assert!(cache.remove(&key));
    assert_eq!(cache.entry_count(), 0);
}

#[test]
fn eviction_triggering_insertions_count_once_per_attempt_not_per_victim() {
    let (registry, backend) = registry();
    let cache = cache_with_metrics(4, MemoryMetrics::new(&backend));
    let triggering = || value(&registry, "feuer_memory_eviction_triggering_insertions_total", &[]);
    let operation = |label| value(&registry, "feuer_memory_operations_total", &[("operation", label)]);
    for key in ["first", "second"] {
        cache.insert(key.into(), Download::new(Bytes::from_static(b"ab")));
    }
    assert_eq!(triggering(), 0.0);
    cache.insert("large".into(), Download::new(Bytes::from_static(b"abcd")));
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_memory_entries", &[]), 1.0);
    assert_eq!(triggering(), 1.0);
    assert!(!cache.insert("large".into(), Download::new(Bytes::from_static(b"abcd"))));
    assert_eq!(triggering(), 1.0);
    assert_eq!(operation("insert"), 3.0);
    assert_eq!(operation("redundant"), 1.0);
    assert!(cache.remove(&"large".into()));
    assert!(!cache.remove(&"large".into()));
    assert_eq!(operation("remove"), 1.0);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_memory_entries", &[]), 0.0);
}

#[test]
fn custom_scores_use_live_application_metadata_without_cache_byte_normalization() {
    use crate::retention::RetentionScorer;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct ComputationCost(AtomicU64);
    impl RetentionScorer for ComputationCost {
        fn score(
            &self,
            key: &ObjectKeyHash,
            payload_len: u64,
            entry_size_bytes: u64,
            _: &ObjectAccessHistories,
        ) -> f64 {
            assert_eq!(payload_len, 1);
            if *key == ObjectKeyHash(1) {
                assert_eq!(entry_size_bytes, 100);
                self.0.load(Ordering::Relaxed) as f64
            } else {
                assert_eq!(entry_size_bytes, 1);
                1.0
            }
        }
    }
    let scorer = Arc::new(ComputationCost(AtomicU64::new(2)));
    let cache = cache(101).with_retention_scorer(scorer.clone());
    let payload = Download::new(Bytes::from_static(b"x"));
    cache.insert(ObjectKeyHash(1), payload.clone().with_allocation_charge(100));
    cache.insert(ObjectKeyHash(2), payload.clone());
    cache.insert(ObjectKeyHash(3), payload.clone());
    assert!(cache.get(&ObjectKeyHash(1)).is_some());
    assert!(cache.get(&ObjectKeyHash(2)).is_none());
    scorer.0.store(0, Ordering::Relaxed);
    cache.insert(ObjectKeyHash(4), payload);
    assert!(cache.get(&ObjectKeyHash(1)).is_none());
    assert!(cache.get(&ObjectKeyHash(3)).is_some());
    assert!(cache.get(&ObjectKeyHash(4)).is_some());
}

#[test]
fn equal_cost_eviction_uses_key_order_not_sample_order() {
    let cache = cache(3);
    let payload = Download::new(Bytes::from_static(b"x"));
    for key in [0, 1, 2] {
        cache.insert(ObjectKeyHash(key), payload.clone());
    }
    // Removing the first candidate moves key 2 ahead of key 1 in the sample.
    assert!(cache.remove(&ObjectKeyHash(0)));
    cache.insert(ObjectKeyHash(3), payload.clone());
    cache.insert(ObjectKeyHash(4), payload);
    assert!(cache.get(&ObjectKeyHash(1)).is_none());
    for key in [2, 3, 4] {
        assert!(cache.get(&ObjectKeyHash(key)).is_some());
        assert!(cache.remove(&ObjectKeyHash(key)));
    }
    assert_eq!(cache.entry_count(), 0);
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn eviction_samples_advance_between_decisions() {
    let cache = cache(3).with_reclaim_sample_size(1);
    let payload = Download::new(Bytes::from_static(b"x"));
    for key in 0..5 {
        cache.insert(ObjectKeyHash(key), payload.clone());
    }
    assert!(!cache.contains_entry(&ObjectKeyHash(0)));
    assert!(!cache.contains_entry(&ObjectKeyHash(1)));
    for key in 2..5 {
        assert!(cache.contains_entry(&ObjectKeyHash(key)));
    }
}

#[test]
fn oversized_insertion_empties_its_shard_and_remains_cached() {
    for capacity in [0, 3] {
        let cache = cache(capacity);
        cache.insert("old".into(), Download::new(Bytes::from_static(b"ok")));
        cache.insert("new".into(), Download::new(Bytes::from_static(b"data")));
        assert!(cache.get(&"old".into()).is_none());
        assert_eq!(cache.get(&"new".into()).unwrap(), Bytes::from_static(b"data"));
        assert_eq!(cache.used_bytes(), 4);
        assert_eq!(cache.entry_count(), 1);
        assert!(cache.used_bytes() > cache.capacity());
    }
}

#[test]
fn maximum_allocation_charge_is_evicted_before_admission() {
    let cache = cache(usize::MAX as u64);
    cache.insert(
        "old".into(),
        Download::new(Bytes::from_static(b"x")).with_allocation_charge(usize::MAX),
    );
    cache.insert("new".into(), Download::new(Bytes::from_static(b"y")));
    assert_eq!(cache.used_bytes(), 1);
    assert!(!cache.contains_entry(&"old".into()));
    assert!(cache.contains_entry(&"new".into()));
}

#[test]
fn evicts_entry_with_lower_retrieval_value() {
    let cache = cache(2);
    let hot = ObjectKeyHash::from("hot");
    let cold = ObjectKeyHash::from("cold");
    let payload = Download::new(Bytes::from_static(b"x"));
    cache.insert(hot, payload.clone());
    cache.insert(cold, payload.clone());
    cache.access_histories.record_access(&cold);
    repeat_request(&cache, hot, 2);
    cache.insert("incoming".into(), payload);
    assert!(cache.get(&hot).is_some());
    assert!(cache.get(&cold).is_none());
    assert!(cache.get(&"incoming".into()).is_some());
}

#[test]
fn stale_frequency_decays_below_recent_accesses() {
    let cache = cache(3).with_access_histories(Arc::new(ObjectAccessHistories::with_half_life(16)));
    let stale = ObjectKeyHash::from("stale");
    let fresh = ObjectKeyHash::from("fresh");
    let clock = ObjectKeyHash::from("clock");
    let payload = Download::new(Bytes::from_static(b"x"));
    for key in [stale, fresh, clock] {
        cache.insert(key, payload.clone());
    }
    repeat_request(&cache, stale, 8);
    repeat_request(&cache, clock, cache.access_histories.half_life() * 4);
    cache.access_histories.record_access(&fresh);
    cache.insert("incoming".into(), payload);
    assert!(cache.get(&stale).is_none());
    assert!(cache.get(&fresh).is_some());
}

#[test]
fn access_history_is_independent_of_cache_operations_and_lifetime() {
    let cache = cache(1);
    let key = ObjectKeyHash::from("object");
    let history = cache.access_histories().clone();
    let payload = Download::new(Bytes::from_static(b"a"));
    cache.insert(key, payload.clone());
    assert!(cache.get(&key).is_some());
    assert_eq!(history.request_count(), 0);
    history.record_access(&key);
    assert!(cache.remove(&key));
    assert_eq!(history.decayed_request_count(&key), 1.0);
    cache.insert(key, payload.clone());
    cache.insert("incoming".into(), payload.clone());
    assert!(!cache.contains_entry(&key));
    assert_eq!(history.decayed_request_count(&key), 1.0);
    history.record_access(&key);
    cache.insert(key, payload);
    assert_eq!(history.request_count(), 2);
    let demand = history.decayed_request_count(&key);
    drop(cache);
    assert_eq!(history.decayed_request_count(&key), demand);
}

#[test]
fn configured_target_is_divided_without_losing_remainder_bytes() {
    assert_eq!(
        (0..2).map(|index| shard_capacity_for(3, 2, index)).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(
        (0..4).map(|index| shard_capacity_for(2, 4, index)).collect::<Vec<_>>(),
        vec![1, 1, 0, 0]
    );
}

#[test]
fn shard_targets_can_collectively_exceed_the_configured_capacity() {
    let cache = MemoryCache::with_shard_count(BufferPool::new(2, MemoryMetrics::noop()), 2);
    let [first, second] = std::array::from_fn(|shard_index| {
        (0..100)
            .map(|candidate| ObjectKeyHash::from(format!("object-{candidate}")))
            .find(|key| cache.shard_index(key) == shard_index)
            .expect("test keys must cover both shards")
    });
    cache.insert(first, Download::new(Bytes::from_static(b"aa")));
    cache.insert(second, Download::new(Bytes::from_static(b"bb")));
    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.capacity(), 2);
}

#[test]
fn concurrent_shards_respect_their_targets_for_regular_entries() {
    let cache = MemoryCache::with_shard_count(BufferPool::new(256, MemoryMetrics::noop()), 8);
    thread::scope(|scope| {
        for worker in 0..8_u64 {
            let cache = &cache;
            scope.spawn(move || {
                for index in 0..500_u64 {
                    let key = ObjectKeyHash::from(format!("object-{worker}-{index}"));
                    cache.insert(key, Download::new(Bytes::from(vec![worker as u8; 8])));
                    assert!(cache.used_bytes() <= cache.capacity());
                }
            });
        }
    });
    assert!(cache.used_bytes() <= cache.capacity());
    assert_eq!(cache.used_bytes(), cache.entry_count() * 8);
    for shard in &cache.shards {
        let shard = shard.lock();
        assert_eq!(shard.used_bytes(), shard.entry_count() as u64 * 8);
    }
}
