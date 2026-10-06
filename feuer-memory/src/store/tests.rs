use std::{sync::Arc, thread};

use bytes::Bytes;
use feuer_types::{
    ByteRange, Download, ObjectKeyHash,
    retention::{ACCESS_COUNT_HALF_LIFE, MAX_ACCESS_EVENTS_PER_KEY, ObjectAccessHistories},
};

use super::{
    MemoryCache,
    shard::{InsertOrReclaimResult, MIN_REQUESTS_BEFORE_RANGE_TRIM},
    shard_capacity_for,
};
use crate::MemoryMetrics;

#[test]
fn cached_slices_and_idle_buffers_share_allocation_accounting() {
    use crate::test_metrics::{registry, value};

    let (registry, backend) = registry();
    let capacity = 256 * 1024;
    let cache = cache_with_metrics(100 * capacity as u64, MemoryMetrics::new(&backend));
    let pool = cache.buffer_pool();
    assert!(Arc::ptr_eq(&pool, &cache.buffer_pool()));
    let other_cache = MemoryCache::new(cache.capacity());
    assert!(!Arc::ptr_eq(&pool, &other_cache.buffer_pool()));
    let buffer = pool.allocate(capacity).unwrap();
    let bytes = buffer.into_bytes().slice(17..18);
    let key = ObjectKeyHash::from("slice");
    cache.insert_with_allocation_charge(key, Download::new(0, bytes.clone()).unwrap(), capacity);
    assert_eq!(cache.used_bytes(), capacity as u64);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), capacity as f64);
    assert_eq!(cache.get(&key, range(0, 1)).unwrap().as_ptr(), bytes.as_ptr());
    // A redundant insertion does not charge the shared allocation again.
    assert!(!cache.insert_with_allocation_charge(key, Download::new(0, bytes.clone()).unwrap(), capacity));
    assert_eq!(cache.used_bytes(), capacity as u64);
    cache.remove(&key, range(0, 1));
    assert_eq!(cache.used_bytes(), 0); // caller-only results are outside the budget
    drop(bytes);
    assert_eq!(cache.used_bytes(), capacity as u64);
    assert_eq!(pool.idle_bytes(), capacity as u64);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), capacity as f64);
    drop(pool);
    drop(cache);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
    assert_eq!(value(&registry, "feuer_memory_capacity_bytes", &[]), 0.0);
}

#[test]
fn cached_capacity_drives_eviction_not_slice_length() {
    for override_charge in [false, true] {
        let cache = cache(32 * 1024);
        for key in ["first", "second"] {
            let buffer = cache.buffer_pool().allocate(1).unwrap();
            let capacity = buffer.capacity();
            let download = buffer.into_download(0).unwrap();
            if override_charge {
                // The explicit charge replaces an existing download charge, not just the default.
                cache.insert_with_allocation_charge(
                    key.into(),
                    download.with_allocation_charge(capacity / 2),
                    capacity,
                );
            } else {
                cache.insert(key.into(), download);
            }
        }
        assert!(cache.get(&"first".into(), range(0, 1)).is_none());
        assert!(cache.get(&"second".into(), range(0, 1)).is_some());
        assert_eq!(cache.used_bytes(), 32 * 1024);
        assert_eq!(cache.buffer_pool().idle_bytes(), 0);
        cache.remove(&"second".into(), range(0, 1));
        assert_eq!(cache.used_bytes(), 0); // even one idle buffer exceeds the pool's limit
    }
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
    drop(pool.allocate(1).unwrap()); // another bucket cannot exceed the shared idle ceiling
    assert_eq!(pool.idle_bytes(), capacity as u64 * 7 / 100);
    let waiting = pool.allocate(buffer_capacity).unwrap();
    cache.insert("full".into(), Download::new(0, Bytes::from(vec![0; capacity])).unwrap());
    assert_eq!(pool.idle_bytes(), 0);
    drop(waiting);
    assert_eq!(pool.idle_bytes(), 0);
    assert_eq!(cache.used_bytes(), capacity as u64);
    assert!(cache.get(&"full".into(), range(0, 1)).is_some());
}

#[test]
fn replacement_releases_the_old_allocation_charge() {
    let capacity = 256 * 1024;
    let cache = cache(100 * capacity as u64);
    let key = ObjectKeyHash::from("replace");
    let buffer = cache.buffer_pool().allocate(capacity).unwrap();
    cache.insert_with_allocation_charge(key, Download::new(1, buffer.into_bytes().slice(..1)).unwrap(), capacity);
    cache.insert(key, Download::new(0, Bytes::from_static(b"abc")).unwrap());
    assert_eq!(cache.shards[0].lock().used_bytes(), 3);
    assert_eq!(cache.buffer_pool().idle_bytes(), capacity as u64);
    assert_eq!(cache.used_bytes(), capacity as u64 + 3);
    cache.remove(&key, range(0, 3));
    assert_eq!(cache.used_bytes(), capacity as u64);
}

#[test]
fn trimming_replaces_allocation_capacity_with_copied_payload_capacity() {
    let capacity = 256 * 1024;
    let cache = cache(capacity as u64);
    let key = ObjectKeyHash::from("trim");
    let buffer = cache.buffer_pool().allocate(capacity).unwrap();
    cache.insert_with_allocation_charge(
        key,
        Download::new(0, buffer.into_bytes().slice(..100)).unwrap(),
        capacity,
    );
    for _ in 0..MIN_REQUESTS_BEFORE_RANGE_TRIM {
        cache.access_histories.record_access(&key, range(10, 20));
    }
    cache.insert("incoming".into(), Download::new(0, Bytes::from_static(b"x")).unwrap());
    assert_eq!(cache.used_bytes(), 11);
    assert_eq!(cache.buffer_pool().idle_bytes(), 0);
    assert_eq!(cache.get(&key, range(10, 20)).unwrap().len(), 10);
}

#[test]
fn entry_presence_check_accepts_reinsertion_but_rejects_removal_and_replacement() {
    let cache = cache(1024);
    let key = ObjectKeyHash::from("disk-source");
    let payload = Download::new(10, Bytes::from_static(b"abcd")).unwrap();
    let range = payload.downloaded_range();
    assert!(!cache.remove(&key, range));
    assert!(cache.insert(key, payload.clone()));
    assert!(cache.contains_entry(&key, range));
    assert!(!cache.insert(key, payload.clone()));
    assert!(cache.remove(&key, range));
    assert!(!cache.remove(&key, range));
    assert!(!cache.contains_entry(&key, range));
    assert!(cache.insert(key, payload));
    assert!(cache.contains_entry(&key, range));
    cache.insert(key, Download::new(9, Bytes::from_static(b"xabcdy")).unwrap());
    assert!(!cache.contains_entry(&key, range));
}

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

fn download(expected_range: ByteRange, bytes: Bytes) -> Download {
    let download = Download::new(expected_range.start(), bytes).unwrap();
    assert_eq!(download.downloaded_range(), expected_range);
    download
}

fn insert(cache: &MemoryCache, object_key: ObjectKeyHash, download: Download) {
    cache.insert(object_key, download);
}

fn cache(capacity: u64) -> MemoryCache {
    cache_with_metrics(capacity, MemoryMetrics::noop())
}

fn cache_with_metrics(capacity: u64, metrics: Arc<MemoryMetrics>) -> MemoryCache {
    MemoryCache::with_shard_count(capacity, metrics, 1, Arc::new(ObjectAccessHistories::new()))
}

fn accessed_ranges(cache: &MemoryCache, key: &ObjectKeyHash) -> Vec<ByteRange> {
    cache.access_histories.recent_requested_ranges(key)
}

fn access_history_len(cache: &MemoryCache, key: &ObjectKeyHash) -> usize {
    accessed_ranges(cache, key).len()
}

#[test]
fn eviction_triggering_insertions_count_once_per_attempt_not_per_victim() {
    use crate::test_metrics::{registry, value};

    let (registry, backend) = registry();
    let cache = cache_with_metrics(4, MemoryMetrics::new(&backend));
    let triggering = || value(&registry, "feuer_memory_eviction_triggering_insertions_total", &[]);
    let operation = |label| value(&registry, "feuer_memory_operations_total", &[("operation", label)]);
    for key in ["first", "second"] {
        cache.insert(key.into(), Download::new(0, Bytes::from_static(b"ab")).unwrap());
    }
    assert_eq!(triggering(), 0.0);
    // Both residents must be evicted, but only one insertion triggered them.
    cache.insert("large".into(), Download::new(0, Bytes::from_static(b"abcd")).unwrap());
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_memory_entries", &[]), 1.0);
    assert_eq!(triggering(), 1.0);
    // Containment and replacement are not pressure evictions.
    cache.insert("large".into(), Download::new(0, Bytes::from_static(b"ab")).unwrap());
    cache.insert("large".into(), Download::new(0, Bytes::from_static(b"abcde")).unwrap());
    assert_eq!(triggering(), 1.0);
    assert_eq!(operation("insert") + operation("replace") + operation("redundant"), 5.0);
}

#[test]
fn equal_cost_eviction_uses_key_order_not_sample_order() {
    let cache = cache(3);
    let payload = Download::new(0, Bytes::from_static(b"x")).unwrap();
    let range = payload.downloaded_range();
    for key in [0, 1, 2] {
        cache.insert(ObjectKeyHash(key), payload.clone());
    }
    // Removing the first candidate moves key 2 ahead of key 1 in the sample.
    assert!(cache.remove(&ObjectKeyHash(0), range));
    cache.insert(ObjectKeyHash(3), payload.clone());
    cache.insert(ObjectKeyHash(4), payload);
    assert!(cache.get(&ObjectKeyHash(1), range).is_none());
    for key in [2, 3, 4] {
        assert!(cache.get(&ObjectKeyHash(key), range).is_some());
        assert!(cache.remove(&ObjectKeyHash(key), range));
    }
    assert_eq!(cache.entry_count(), 0);
    assert_eq!(cache.used_bytes(), 0);
}

#[test]
fn reclaim_sampling_advances_past_contained_ranges() {
    let cache = cache(4).with_reclaim_sample_size(1);
    let key = ObjectKeyHash::from("object");
    for (start, bytes) in [
        (0, Bytes::from_static(b"a")),
        (1, Bytes::from_static(b"b")),
        (2, Bytes::from_static(b"cd")),
    ] {
        cache.insert(key, Download::new(start, bytes).unwrap());
    }
    let mut shard = cache.shards[0].lock();
    let replacement = Download::new(0, Bytes::from_static(b"abc")).unwrap();
    for (expected_entries, evicted) in [(3, false), (3, false), (2, true)] {
        assert!(matches!(
            shard.try_admit_or_reclaim(&key, &replacement, &cache, false),
            InsertOrReclaimResult::Retry { evicted: removed } if removed == evicted
        ));
        assert_eq!(shard.entry_count(), expected_entries);
    }
    // The partial overlap is eligible; the two ranges fully contained in the incoming download were skipped.
    assert!(matches!(
        shard.try_admit_or_reclaim(&key, &replacement, &cache, false),
        InsertOrReclaimResult::Complete(true)
    ));
    assert_eq!(shard.entry_count(), 1);
    assert_eq!(shard.used_bytes(), 3);
}

#[test]
fn covering_lookup_returns_only_requested_bytes_and_shares_the_allocation() {
    let cache = cache(16);
    let key = ObjectKeyHash::from("object");
    let value = Bytes::from_static(b"abcdefghij");
    for start in [10, u64::MAX - 10] {
        insert(&cache, key, download(range(start, start + 10), value.clone()));
        let result = cache.get(&key, range(start + 3, start + 7)).unwrap();

        assert_eq!(result, Bytes::from_static(b"defg"));
        assert_eq!(result.as_ptr(), value.slice(3..).as_ptr());
        assert_eq!(result.len(), 4);
        assert_eq!(cache.used_bytes(), 10);

        assert!(cache.remove(&key, range(start, start + 10)));
        assert_eq!(cache.used_bytes(), 0);
        assert_eq!(result, Bytes::from_static(b"defg"));
    }
}

#[test]
fn lookups_require_one_entry_covering_the_request() {
    let cache = cache(16);
    let key = ObjectKeyHash::from("object-a");
    insert(&cache, key, download(range(10, 13), Bytes::from_static(b"abc")));

    assert!(cache.get(&ObjectKeyHash::from("object-b"), range(10, 13)).is_none());
    assert!(cache.get(&key, range(9, 12)).is_none());
    assert!(cache.get(&key, range(12, 14)).is_none());
    assert!(cache.get(&key, range(13, 14)).is_none());

    insert(&cache, key, download(range(13, 15), Bytes::from_static(b"de")));
    assert!(cache.get(&key, range(12, 14)).is_none());
}

#[test]
fn insertion_and_accesses_are_independent() {
    let cache = cache(16);
    let key = ObjectKeyHash::from("object");
    insert(&cache, key, download(range(10, 20), Bytes::from_static(b"abcdefghij")));
    assert!(accessed_ranges(&cache, &key).is_empty());

    assert!(cache.get(&key, range(11, 13)).is_some());
    assert!(accessed_ranges(&cache, &key).is_empty());
    cache.access_histories.record_access(&key, range(11, 13));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(11, 13)]);

    insert(&cache, key, download(range(12, 18), Bytes::from_static(b"cdefgh")));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(11, 13)]);

    cache.access_histories.record_access(&key, range(14, 16));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(11, 13), range(14, 16)]);
}

#[test]
fn shared_download_insertion_is_deduplicated_but_each_waiter_records_an_access() {
    let cache = cache(16);
    let key = ObjectKeyHash::from("object");
    let shared = download(range(0, 10), Bytes::from_static(b"abcdefghij"));

    for requested_range in [range(1, 3), range(7, 9), range(1, 3)] {
        cache.insert(key, shared.clone());
        cache.access_histories.record_access(&key, requested_range);
    }

    assert_eq!(cache.used_bytes(), 10);
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(
        accessed_ranges(&cache, &key),
        vec![range(1, 3), range(7, 9), range(1, 3)]
    );
}

#[test]
fn partially_overlapping_downloads_coexist_and_do_not_form_a_hit() {
    let cache = cache(32);
    let key = ObjectKeyHash::from("object");
    insert(&cache, key, download(range(0, 5), Bytes::from_static(b"abcde")));
    insert(&cache, key, download(range(8, 12), Bytes::from_static(b"ijkl")));

    insert(&cache, key, download(range(3, 10), Bytes::from_static(b"defghij")));

    assert_eq!(cache.used_bytes(), 16);
    assert_eq!(cache.entry_count(), 3);
    assert_eq!(cache.get(&key, range(0, 3)).unwrap(), Bytes::from_static(b"abc"));
    assert_eq!(cache.get(&key, range(3, 10)).unwrap(), Bytes::from_static(b"defghij"));
    assert_eq!(cache.get(&key, range(10, 12)).unwrap(), Bytes::from_static(b"kl"));
    assert!(cache.get(&key, range(4, 11)).is_none());
}

#[test]
fn a_larger_download_replaces_contained_entries_but_not_partial_overlaps() {
    let cache = cache(32);
    let key = ObjectKeyHash::from("object");
    insert(&cache, key, download(range(2, 5), Bytes::from_static(b"cde")));
    insert(&cache, key, download(range(10, 15), Bytes::from_static(b"klmno")));

    insert(&cache, key, download(range(0, 12), Bytes::from_static(b"abcdefghijkl")));

    assert_eq!(cache.used_bytes(), 17);
    assert_eq!(cache.entry_count(), 2);
    assert_eq!(
        cache.get(&key, range(0, 12)).unwrap(),
        Bytes::from_static(b"abcdefghijkl")
    );
    assert_eq!(cache.get(&key, range(12, 15)).unwrap(), Bytes::from_static(b"mno"));
    assert!(cache.get(&key, range(9, 13)).is_none());
}

#[test]
fn replacement_releases_contained_charges_including_an_empty_entry_at_the_end() {
    use crate::test_metrics::{registry, value};

    for (capacity, replacement_charge) in [(16, 3), (13, 11)] {
        let (registry, backend) = registry();
        let cache = cache_with_metrics(capacity, MemoryMetrics::new(&backend));
        let key = ObjectKeyHash::from("object");
        for (start, bytes, charge) in [
            (0, Bytes::from_static(b"a"), 4),
            (1, Bytes::from_static(b"b"), 4),
            (3, Bytes::new(), 3),
            (4, Bytes::from_static(b"ef"), 2),
        ] {
            assert!(cache.insert_with_allocation_charge(key, Download::new(start, bytes).unwrap(), charge));
        }
        assert_eq!(cache.used_bytes(), 13);
        assert!(cache.insert_with_allocation_charge(
            key,
            Download::new(0, Bytes::from_static(b"abc")).unwrap(),
            replacement_charge,
        ));
        assert_eq!(cache.used_bytes(), replacement_charge as u64 + 2);
        assert_eq!(cache.entry_count(), 2);
        assert!(!cache.contains_entry(&key, range(3, 3)));
        assert_eq!(cache.get(&key, range(4, 6)).unwrap(), Bytes::from_static(b"ef"));
        for rejected in [range(0, 1), range(0, 4), range(3, 3)] {
            assert!(!cache.remove(&key, rejected));
        }
        assert_eq!(
            value(&registry, "feuer_memory_used_bytes", &[]),
            replacement_charge as f64 + 2.0
        );
        assert_eq!(value(&registry, "feuer_memory_entries", &[]), 2.0);
        assert_eq!(
            value(&registry, "feuer_memory_operations_total", &[("operation", "replace")]),
            1.0
        );
        assert_eq!(
            value(&registry, "feuer_memory_eviction_triggering_insertions_total", &[]),
            0.0
        );
        assert!(cache.remove(&key, range(0, 3)));
        assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 2.0);
        assert_eq!(value(&registry, "feuer_memory_entries", &[]), 1.0);
        let pool = cache.buffer_pool();
        drop(cache);
        assert_eq!(pool.used_bytes(), 0);
        assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_memory_entries", &[]), 0.0);
    }
}

#[test]
fn redundant_insertions_preserve_cached_bytes_and_usage() {
    let cache = cache(16);
    let key = ObjectKeyHash::from("object");
    insert(&cache, key, download(range(0, 10), Bytes::from_static(b"abcdefghij")));

    for (start, length) in [(0, 10), (2, 6)] {
        let redundant = Download::new(start, Bytes::from(vec![b'X'; length])).unwrap();
        for _ in 0..200 {
            assert!(!cache.insert(key, redundant.clone()));
        }
        assert_eq!(cache.used_bytes(), 10);
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(
            cache.get(&key, range(0, 10)).unwrap(),
            Bytes::from_static(b"abcdefghij")
        );
        assert_eq!(cache.get(&key, range(2, 8)).unwrap(), Bytes::from_static(b"cdefgh"));
    }
}

#[test]
fn download_payload_bytes_count_toward_capacity() {
    let cache = cache(5);
    let key = ObjectKeyHash::from("object");
    insert(&cache, key, download(range(0, 3), Bytes::from_static(b"abc")));
    insert(&cache, key, download(range(3, 5), Bytes::from_static(b"de")));
    assert_eq!(cache.used_bytes(), 5);
    assert_eq!(cache.entry_count(), 2);

    insert(&cache, key, download(range(5, 9), Bytes::from_static(b"fghi")));
    assert!(cache.used_bytes() <= cache.capacity());
    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.entry_count(), 1);
    assert!(cache.get(&key, range(5, 9)).is_some());
}

#[test]
fn oversized_insertion_empties_its_shard_and_remains_cached() {
    for capacity in [0, 3] {
        let cache = cache(capacity);
        let key = ObjectKeyHash::from("object");
        insert(&cache, key, download(range(10, 12), Bytes::from_static(b"ok")));

        insert(&cache, key, download(range(0, 4), Bytes::from_static(b"data")));

        assert!(cache.get(&key, range(10, 12)).is_none());
        assert_eq!(cache.get(&key, range(0, 4)).unwrap(), Bytes::from_static(b"data"));
        assert_eq!(cache.used_bytes(), 4);
        assert!(cache.used_bytes() > cache.capacity());
    }
}

#[test]
fn access_history_survives_replacement_with_or_without_eviction() {
    for (capacity, payload, expected_bytes, expected_entries) in [
        (16, b"abcdefghij".as_slice(), 12, 2),
        (10, b"abcdefgh".as_slice(), 8, 1),
    ] {
        let cache = cache(capacity);
        let key = ObjectKeyHash::from("object");
        insert(&cache, key, download(range(0, 4), Bytes::from_static(b"abcd")));
        insert(&cache, key, download(range(6, 10), Bytes::from_static(b"ghij")));
        insert(
            &cache,
            ObjectKeyHash::from("other"),
            download(range(0, 2), Bytes::from_static(b"xx")),
        );
        cache.access_histories.record_access(&key, range(1, 2));
        cache.access_histories.record_access(&key, range(7, 8));
        assert_eq!(accessed_ranges(&cache, &key), vec![range(1, 2), range(7, 8)]);

        cache.insert(key, Download::new(0, Bytes::from_static(payload)).unwrap());

        assert_eq!(cache.used_bytes(), expected_bytes);
        assert_eq!(cache.entry_count(), expected_entries);
        assert_eq!(accessed_ranges(&cache, &key), vec![range(1, 2), range(7, 8)]);
        assert_eq!(cache.get(&key, range(7, 8)).unwrap(), Bytes::from_static(b"h"));
        assert_eq!(cache.get(&key, range(8, 10)).is_some(), payload.len() == 10);
    }
}

#[test]
fn access_history_caps_event_count_and_preserves_repeated_requests() {
    let cache = cache(1);
    let key = ObjectKeyHash::from("object");
    insert(&cache, key, download(range(0, 1), Bytes::from_static(b"a")));

    for _ in 0..*MAX_ACCESS_EVENTS_PER_KEY + 17 {
        cache.access_histories.record_access(&key, range(0, 1));
    }

    assert_eq!(access_history_len(&cache, &key), *MAX_ACCESS_EVENTS_PER_KEY);
    assert_eq!(
        accessed_ranges(&cache, &key),
        vec![range(0, 1); *MAX_ACCESS_EVENTS_PER_KEY]
    );
}

#[test]
fn evicts_entry_with_lower_retrieval_value() {
    for (entry_bytes, hot_request_bytes, hot_accesses) in [(1, 1, 2), (10, 9, 1)] {
        let cache = cache(2 * entry_bytes);
        let hot = ObjectKeyHash::from("hot");
        let cold = ObjectKeyHash::from("cold");
        let incoming = ObjectKeyHash::from("incoming");
        let payload = download(range(0, entry_bytes), Bytes::from(vec![0; entry_bytes as usize]));
        insert(&cache, hot, payload.clone());
        insert(&cache, cold, payload.clone());

        cache.access_histories.record_access(&cold, range(0, 1));
        for _ in 0..hot_accesses {
            cache.access_histories.record_access(&hot, range(0, hot_request_bytes));
        }
        insert(&cache, incoming, payload);

        assert!(cache.get(&hot, range(0, hot_request_bytes)).is_some());
        assert!(cache.get(&cold, range(0, 1)).is_none());
        assert!(cache.get(&incoming, range(0, entry_bytes)).is_some());
    }
}

#[test]
fn retention_credit_is_projected_only_onto_the_requested_interval() {
    let cache = cache(2);
    let key = ObjectKeyHash::from("split-object");
    let incoming = ObjectKeyHash::from("incoming");
    insert(&cache, key, download(range(0, 1), Bytes::from_static(b"a")));
    insert(&cache, key, download(range(1, 2), Bytes::from_static(b"b")));

    cache.access_histories.record_access(&key, range(0, 1));
    insert(&cache, incoming, download(range(0, 1), Bytes::from_static(b"c")));

    assert!(cache.get(&key, range(0, 1)).is_some());
    assert!(cache.get(&key, range(1, 2)).is_none());
}

#[test]
fn stale_frequency_decays_below_recent_accesses() {
    let cache = cache(3);
    let stale = ObjectKeyHash::from("stale");
    let fresh = ObjectKeyHash::from("fresh");
    let clock = ObjectKeyHash::from("clock");
    insert(&cache, stale, download(range(0, 1), Bytes::from_static(b"s")));
    insert(&cache, fresh, download(range(0, 1), Bytes::from_static(b"f")));
    insert(&cache, clock, download(range(0, 1), Bytes::from_static(b"c")));

    for _ in 0..8 {
        cache.access_histories.record_access(&stale, range(0, 1));
    }
    for _ in 0..*ACCESS_COUNT_HALF_LIFE * 4 {
        cache.access_histories.record_access(&clock, range(0, 1));
    }
    cache.access_histories.record_access(&fresh, range(0, 1));
    insert(
        &cache,
        ObjectKeyHash::from("incoming"),
        download(range(0, 1), Bytes::from_static(b"i")),
    );

    assert!(cache.get(&stale, range(0, 1)).is_none());
    assert!(cache.get(&fresh, range(0, 1)).is_some());
}

#[test]
fn range_trim_waits_for_grace_and_pressure_without_recording_accesses() {
    let early_pressure = cache(10);
    let early_key = ObjectKeyHash::from("early-download");
    early_pressure.insert(early_key, download(range(0, 10), Bytes::from_static(b"abcdefghij")));
    early_pressure.access_histories.record_access(&early_key, range(2, 4));
    insert(
        &early_pressure,
        ObjectKeyHash::from("early-pressure"),
        download(range(0, 2), Bytes::from_static(b"xy")),
    );
    assert!(early_pressure.get(&early_key, range(2, 4)).is_none());

    let (registry, backend) = crate::test_metrics::registry();
    let cache = cache_with_metrics(10, MemoryMetrics::new(&backend));
    let key = ObjectKeyHash::from("download");
    let incoming = ObjectKeyHash::from("incoming");
    let original = Bytes::from_static(b"abcdefghij");
    insert(&cache, key, download(range(0, 10), original.clone()));

    let returned = cache.get(&key, range(2, 4)).unwrap();
    assert_eq!(returned, Bytes::from_static(b"cd"));
    assert_eq!(returned.as_ptr(), original.slice(2..).as_ptr());
    for _ in 0..MIN_REQUESTS_BEFORE_RANGE_TRIM {
        cache.access_histories.record_access(&key, range(2, 4));
    }
    let history_len = (*MAX_ACCESS_EVENTS_PER_KEY).min(MIN_REQUESTS_BEFORE_RANGE_TRIM as usize);
    assert_eq!(cache.used_bytes(), 10);
    assert_eq!(access_history_len(&cache, &key), history_len);
    insert(&cache, incoming, download(range(0, 2), Bytes::from_static(b"xy")));

    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.entry_count(), 2);
    assert_eq!(
        crate::test_metrics::value(&registry, "feuer_memory_eviction_triggering_insertions_total", &[]),
        0.0
    );
    assert_eq!(returned, Bytes::from_static(b"cd"));
    let bytes_after_trim = cache.get(&key, range(2, 4)).unwrap();
    assert_eq!(bytes_after_trim, Bytes::from_static(b"cd"));
    assert_ne!(bytes_after_trim.as_ptr(), original.slice(2..).as_ptr());
    assert!(cache.get(&key, range(0, 1)).is_none());
    assert_eq!(access_history_len(&cache, &key), history_len);
    assert!(accessed_ranges(&cache, &key).iter().all(|seen| *seen == range(2, 4)));
}

#[test]
fn range_trim_preserves_disjoint_requested_coverage_without_filling_gaps() {
    let cache = cache(10);
    let key = ObjectKeyHash::from("download");
    insert(&cache, key, download(range(10, 20), Bytes::from_static(b"abcdefghij")));
    cache.access_histories.record_access(&key, range(11, 13));
    cache.access_histories.record_access(&key, range(17, 19));
    for _ in 2..MIN_REQUESTS_BEFORE_RANGE_TRIM {
        cache.access_histories.record_access(&key, range(11, 13));
    }

    insert(
        &cache,
        ObjectKeyHash::from("incoming"),
        download(range(0, 2), Bytes::from_static(b"xy")),
    );

    assert_eq!(cache.used_bytes(), 6);
    assert_eq!(cache.get(&key, range(11, 13)).unwrap(), Bytes::from_static(b"bc"));
    assert_eq!(cache.get(&key, range(17, 19)).unwrap(), Bytes::from_static(b"hi"));
    assert!(cache.get(&key, range(13, 17)).is_none());
}

#[test]
fn candidate_state_tracks_entries_during_oversized_churn() {
    let cache = cache(1);
    for index in 0..300 {
        let key = ObjectKeyHash::from(format!("download-{index}"));
        cache.insert(key, download(range(0, 2), Bytes::from_static(b"ab")));
        cache.access_histories.record_access(&key, range(0, 1));
    }

    assert_eq!(cache.used_bytes(), 2);
    assert_eq!(cache.entry_count(), 1);
}

#[test]
fn range_trim_requires_only_the_exact_source_range() {
    for (change, published, expected_bytes, expected_entries, requested_range, expected_payload) in [
        (0, false, 0, 0, range(0, 10), None),
        (1, true, 2, 1, range(2, 4), Some(b"cd".as_slice())),
        (2, true, 3, 2, range(12, 13), Some(b"x".as_slice())),
        (3, false, 11, 1, range(0, 10), Some(b"abcdefghij".as_slice())),
        (4, true, 9, 1, range(2, 11), Some(b"cdefghijk".as_slice())),
        (5, true, 2, 1, range(2, 4), Some(b"cd".as_slice())),
    ] {
        let (registry, backend) = crate::test_metrics::registry();
        let cache = cache_with_metrics(20, MemoryMetrics::new(&backend));
        let key = ObjectKeyHash::from("source");
        let source = download(range(0, 10), Bytes::from_static(b"abcdefghij"));
        cache.insert(key, source.clone());
        for _ in 0..MIN_REQUESTS_BEFORE_RANGE_TRIM {
            cache.access_histories.record_access(&key, range(2, 4));
        }
        let trim_source = {
            let mut shard = cache.shards[0].lock();
            let InsertOrReclaimResult::Trim(source) = shard.try_admit_or_reclaim(
                &ObjectKeyHash::from("incoming"),
                &Download::new(0, Bytes::from_static(b"01234567890")).unwrap(),
                &cache,
                true,
            ) else {
                panic!("expected a trim");
            };
            source
        };
        let replacement = trim_source.copy_replacement_payloads();
        match change {
            0 => {
                assert!(cache.remove(&key, range(0, 10)));
            }
            1 => {
                assert!(cache.remove(&key, range(0, 10)));
                // The replacement may have a different allocation charge for the same bytes.
                cache.insert_with_allocation_charge(key, source, 16);
            }
            2 => {
                cache.insert(key, download(range(12, 13), Bytes::from_static(b"x")));
            }
            3 => {
                assert!(cache.remove(&key, range(0, 10)));
                cache.insert(key, download(range(0, 11), Bytes::from_static(b"abcdefghijk")));
            }
            4 => {
                // A partial overlap can cover the replacement range without replacing the source.
                cache.insert(key, download(range(2, 11), Bytes::from_static(b"cdefghijk")));
            }
            _ => {}
        }
        let mut shard = cache.shards[0].lock();
        // A request recorded after copying does not invalidate the plan or need the shard lock.
        cache.access_histories.record_access(&key, range(6, 8));
        let used_bytes_before_trim = shard.used_bytes();
        assert_eq!(
            shard.publish_range_trim(replacement, cache.access_histories.request_count()),
            published
        );
        drop(shard);
        assert!(accessed_ranges(&cache, &key).contains(&range(6, 8)));
        assert_eq!(cache.used_bytes(), expected_bytes);
        assert_eq!(cache.entry_count(), expected_entries);
        for (name, expected) in [
            ("feuer_memory_used_bytes", expected_bytes),
            ("feuer_memory_entries", expected_entries),
            (
                "feuer_memory_compacted_payload_bytes_total",
                used_bytes_before_trim - expected_bytes,
            ),
        ] {
            assert_eq!(crate::test_metrics::value(&registry, name, &[]), expected as f64);
        }
        assert_eq!(cache.get(&key, requested_range).as_deref(), expected_payload);
        if published {
            assert_eq!(cache.get(&key, range(2, 4)).unwrap(), Bytes::from_static(b"cd"));
            assert!(cache.get(&key, range(0, 10)).is_none());
            if change != 4 {
                assert!(cache.get(&key, range(6, 8)).is_none());
            }
        }
    }
}

#[test]
fn access_history_survives_removal_reinsertion_and_cache_drop() {
    let cache = cache(1);
    let key = ObjectKeyHash::from("object");
    cache.insert(key, download(range(0, 1), Bytes::from_static(b"a")));
    let history = cache.access_histories.clone();
    history.record_access(&key, range(0, 1));
    assert_eq!(access_history_len(&cache, &key), 1);
    assert!(cache.remove(&key, range(0, 1)));
    assert_eq!(access_history_len(&cache, &key), 1);
    history.record_access(&key, range(10, 11));
    assert_eq!(history.request_count(), 2);
    insert(&cache, key, download(range(0, 1), Bytes::from_static(b"a")));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(0, 1), range(10, 11)]);
    assert!(cache.get(&key, range(0, 1)).is_some());
    assert_eq!(history.request_count(), 2, "raw lookups do not record requests");
    drop(cache);
    assert!(history.decayed_retrieval_cost(&key, range(0, 1)) > 0.0);
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
    let cache = MemoryCache::with_shard_count(2, MemoryMetrics::noop(), 2, Arc::new(ObjectAccessHistories::new()));
    let [first, second] = std::array::from_fn(|shard_index| {
        (0..100)
            .map(|candidate| ObjectKeyHash::from(format!("object-{candidate}")))
            .find(|key| cache.shard_index(key) == shard_index)
            .expect("test keys must cover both shards")
    });

    insert(&cache, first, download(range(0, 2), Bytes::from_static(b"aa")));
    insert(&cache, second, download(range(0, 2), Bytes::from_static(b"bb")));

    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.capacity(), 2);
}

#[test]
fn concurrent_shards_respect_their_targets_for_regular_entries() {
    let cache = MemoryCache::with_shard_count(256, MemoryMetrics::noop(), 8, Arc::new(ObjectAccessHistories::new()));
    thread::scope(|scope| {
        for worker in 0..8_u64 {
            let cache = &cache;
            scope.spawn(move || {
                let key = ObjectKeyHash::from(format!("object-{worker}"));
                for index in 0..500_u64 {
                    let start = index * 8;
                    insert(
                        cache,
                        key,
                        download(range(start, start + 8), Bytes::from(vec![worker as u8; 8])),
                    );
                    assert!(cache.used_bytes() <= cache.capacity());
                }
            });
        }
    });

    assert!(cache.used_bytes() <= cache.capacity());
    assert_eq!(cache.used_bytes(), cache.entry_count() * 8);
}
