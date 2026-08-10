use std::{sync::Arc, thread};

use bytes::Bytes;
use feuer_types::{ByteRange, Download, ObjectKey};

use super::{
    MemoryCache,
    evidence::{ACCESSES_PER_EPOCH, MAX_ACCESS_EVENTS_PER_KEY, MAX_EVIDENCE_AGE_EPOCHS},
    shard::{AdmissionStep, COMPACTION_GRACE_ACCESSES},
    shard_capacity_for,
};
use crate::MemoryMetrics;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

fn download(expected_range: ByteRange, bytes: Bytes) -> Download {
    let download = Download::new(expected_range.start(), bytes).unwrap();
    assert_eq!(download.downloaded_range(), expected_range);
    download
}

fn populate(cache: &MemoryCache, object_key: ObjectKey, download: Download) {
    cache.insert(object_key, download);
}

fn cache(capacity: u64) -> MemoryCache {
    MemoryCache::with_shard_count(capacity, MemoryMetrics::noop(), 1)
}

fn accessed_ranges(cache: &MemoryCache, key: &ObjectKey) -> Vec<ByteRange> {
    cache.shards[cache.shard_index(key)].lock().accessed_ranges(key)
}

fn access_evidence_len(cache: &MemoryCache, key: &ObjectKey) -> usize {
    cache.shards[cache.shard_index(key)].lock().access_evidence_len(key)
}

fn candidate_count(cache: &MemoryCache) -> usize {
    cache.shards[0].lock().candidate_count()
}

#[test]
fn covering_lookup_returns_only_requested_bytes_and_shares_the_allocation() {
    let cache = cache(16);
    let key = ObjectKey::from("object");
    let value = Bytes::from_static(b"abcdefghij");

    populate(&cache, key.clone(), download(range(10, 20), value.clone()));
    let result = cache.get(&key, range(13, 17)).unwrap();

    assert_eq!(result, Bytes::from_static(b"defg"));
    assert_eq!(result.as_ptr(), value.slice(3..).as_ptr());
    assert_eq!(result.len(), 4);
    assert_eq!(cache.used_bytes(), 10);

    assert!(cache.remove(&key, range(10, 20)));
    assert_eq!(cache.used_bytes(), 0);
    assert_eq!(result, Bytes::from_static(b"defg"));
}

#[test]
fn different_identity_or_noncovering_ranges_miss() {
    let cache = cache(16);
    let key = ObjectKey::from("object-a");
    populate(&cache, key.clone(), download(range(10, 13), Bytes::from_static(b"abc")));

    assert!(cache.get(&ObjectKey::from("object-b"), range(10, 13)).is_none());
    assert!(cache.get(&key, range(9, 12)).is_none());
    assert!(cache.get(&key, range(12, 14)).is_none());
    assert!(cache.get(&key, range(13, 14)).is_none());
}

#[test]
fn adjacent_entries_are_not_assembled_into_a_hit() {
    let cache = cache(8);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 2), Bytes::from_static(b"ab")));
    populate(&cache, key.clone(), download(range(2, 4), Bytes::from_static(b"cd")));

    assert!(cache.get(&key, range(1, 3)).is_none());
}

#[test]
fn population_and_accesses_are_independent() {
    let cache = cache(16);
    let key = ObjectKey::from("object");
    populate(
        &cache,
        key.clone(),
        download(range(10, 20), Bytes::from_static(b"abcdefghij")),
    );
    assert!(accessed_ranges(&cache, &key).is_empty());

    assert!(cache.get(&key, range(11, 13)).is_some());
    assert_eq!(accessed_ranges(&cache, &key), vec![range(11, 13)]);

    populate(
        &cache,
        key.clone(),
        download(range(12, 18), Bytes::from_static(b"cdefgh")),
    );
    assert_eq!(accessed_ranges(&cache, &key), vec![range(11, 13)]);

    cache.record_access(&key, range(14, 16));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(11, 13), range(14, 16)]);
}

#[test]
fn shared_download_population_is_deduplicated_but_each_waiter_records_an_access() {
    let cache = cache(16);
    let key = ObjectKey::from("object");
    let shared = download(range(0, 10), Bytes::from_static(b"abcdefghij"));

    for requested_range in [range(1, 3), range(7, 9), range(1, 3)] {
        cache.insert_and_record(key.clone(), shared.clone(), requested_range);
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
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 5), Bytes::from_static(b"abcde")));
    populate(&cache, key.clone(), download(range(8, 12), Bytes::from_static(b"ijkl")));

    populate(
        &cache,
        key.clone(),
        download(range(3, 10), Bytes::from_static(b"defghij")),
    );

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
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(2, 5), Bytes::from_static(b"cde")));
    populate(
        &cache,
        key.clone(),
        download(range(10, 15), Bytes::from_static(b"klmno")),
    );

    populate(
        &cache,
        key.clone(),
        download(range(0, 12), Bytes::from_static(b"abcdefghijkl")),
    );

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
fn a_contained_download_is_discarded_without_replacing_cached_bytes() {
    let cache = cache(16);
    let key = ObjectKey::from("object");
    populate(
        &cache,
        key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
    );

    populate(
        &cache,
        key.clone(),
        download(range(2, 8), Bytes::from_static(b"XXXXXX")),
    );
    assert_eq!(cache.used_bytes(), 10);
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(cache.get(&key, range(2, 8)).unwrap(), Bytes::from_static(b"cdefgh"));
}

#[test]
fn capacity_is_charged_by_retained_download_payload_bytes() {
    let cache = cache(5);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 3), Bytes::from_static(b"abc")));
    populate(&cache, key.clone(), download(range(3, 5), Bytes::from_static(b"de")));
    assert_eq!(cache.used_bytes(), 5);
    assert_eq!(cache.entry_count(), 2);

    populate(&cache, key.clone(), download(range(5, 9), Bytes::from_static(b"fghi")));
    assert!(cache.used_bytes() <= cache.capacity());
    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.entry_count(), 1);
    assert!(cache.get(&key, range(5, 9)).is_some());
}

#[test]
fn repeated_redundant_insertions_do_not_change_usage_or_replace_data() {
    let cache = cache(4);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));

    for _ in 0..200 {
        populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"b")));
    }
    assert_eq!(cache.used_bytes(), 1);
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(cache.get(&key, range(0, 1)).unwrap(), Bytes::from_static(b"a"));
}

#[test]
fn oversized_insertion_empties_its_shard_and_remains_cached() {
    let cache = cache(3);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(10, 12), Bytes::from_static(b"ok")));

    populate(&cache, key.clone(), download(range(0, 4), Bytes::from_static(b"data")));

    assert!(cache.get(&key, range(10, 12)).is_none());
    assert_eq!(cache.get(&key, range(0, 4)).unwrap(), Bytes::from_static(b"data"));
    assert_eq!(cache.used_bytes(), 4);
    assert!(cache.used_bytes() > cache.capacity());
}

#[test]
fn accessed_ranges_survive_downloaded_range_replacement() {
    let cache = cache(16);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 4), Bytes::from_static(b"abcd")));
    populate(&cache, key.clone(), download(range(6, 10), Bytes::from_static(b"ghij")));

    cache.record_access(&key, range(1, 3));
    cache.record_access(&key, range(7, 9));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(1, 3), range(7, 9)]);

    populate(
        &cache,
        key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
    );
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(accessed_ranges(&cache, &key), vec![range(1, 3), range(7, 9)]);
}

#[test]
fn access_evidence_survives_same_object_eviction_during_replacement() {
    let cache = cache(10);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 4), Bytes::from_static(b"abcd")));
    populate(&cache, key.clone(), download(range(6, 10), Bytes::from_static(b"ghij")));
    populate(
        &cache,
        ObjectKey::from("other"),
        download(range(0, 2), Bytes::from_static(b"xx")),
    );
    cache.record_access(&key, range(1, 2));
    cache.record_access(&key, range(7, 8));

    populate(
        &cache,
        key.clone(),
        download(range(0, 8), Bytes::from_static(b"abcdefgh")),
    );

    assert_eq!(cache.used_bytes(), 8);
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(accessed_ranges(&cache, &key), vec![range(1, 2), range(7, 8)]);
    assert_eq!(cache.get(&key, range(7, 8)).unwrap(), Bytes::from_static(b"h"));
    assert!(cache.get(&key, range(8, 10)).is_none());
}

#[test]
fn access_evidence_is_bounded_and_preserves_repeated_exact_requests() {
    let cache = cache(1);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));

    for _ in 0..MAX_ACCESS_EVENTS_PER_KEY + 17 {
        cache.record_access(&key, range(0, 1));
    }

    assert_eq!(access_evidence_len(&cache, &key), MAX_ACCESS_EVENTS_PER_KEY);
    assert_eq!(
        accessed_ranges(&cache, &key),
        vec![range(0, 1); MAX_ACCESS_EVENTS_PER_KEY]
    );
}

#[test]
fn repeated_requested_intervals_are_retained_over_single_accesses() {
    let cache = cache(2);
    let hot = ObjectKey::from("hot");
    let cold = ObjectKey::from("cold");
    let incoming = ObjectKey::from("incoming");
    populate(&cache, hot.clone(), download(range(0, 1), Bytes::from_static(b"h")));
    populate(&cache, cold.clone(), download(range(0, 1), Bytes::from_static(b"c")));

    cache.record_access(&cold, range(0, 1));
    cache.record_access(&hot, range(0, 1));
    cache.record_access(&hot, range(0, 1));
    populate(
        &cache,
        incoming.clone(),
        download(range(0, 1), Bytes::from_static(b"i")),
    );

    assert!(cache.get(&hot, range(0, 1)).is_some());
    assert!(cache.get(&cold, range(0, 1)).is_none());
    assert!(cache.get(&incoming, range(0, 1)).is_some());
}

#[test]
fn retention_credit_is_projected_only_onto_the_requested_interval() {
    let cache = cache(2);
    let key = ObjectKey::from("split-object");
    let incoming = ObjectKey::from("incoming");
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));
    populate(&cache, key.clone(), download(range(1, 2), Bytes::from_static(b"b")));

    cache.record_access(&key, range(0, 1));
    populate(&cache, incoming, download(range(0, 1), Bytes::from_static(b"c")));

    assert!(cache.get(&key, range(0, 1)).is_some());
    assert!(cache.get(&key, range(1, 2)).is_none());
}

#[test]
fn stale_frequency_ages_behind_fresh_evidence() {
    let cache = cache(2);
    let stale = ObjectKey::from("stale");
    let fresh = ObjectKey::from("fresh");
    let incoming = ObjectKey::from("incoming");
    populate(&cache, stale.clone(), download(range(0, 1), Bytes::from_static(b"s")));
    populate(&cache, fresh.clone(), download(range(0, 1), Bytes::from_static(b"f")));

    for _ in 0..8 {
        cache.record_access(&stale, range(0, 1));
    }
    let absent = ObjectKey::from("clock-only");
    for _ in 0..ACCESSES_PER_EPOCH * (MAX_EVIDENCE_AGE_EPOCHS + 1) {
        cache.record_access(&absent, range(0, 1));
    }
    cache.record_access(&fresh, range(0, 1));
    populate(&cache, incoming, download(range(0, 1), Bytes::from_static(b"i")));

    assert!(cache.get(&stale, range(0, 1)).is_none());
    assert!(cache.get(&fresh, range(0, 1)).is_some());
}

#[test]
fn compaction_respects_grace_then_releases_unrequested_payload() {
    let early_pressure = cache(10);
    let early_key = ObjectKey::from("early-download");
    early_pressure.insert_and_record(
        early_key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
        range(2, 4),
    );
    populate(
        &early_pressure,
        ObjectKey::from("early-pressure"),
        download(range(0, 2), Bytes::from_static(b"xy")),
    );
    assert!(early_pressure.get(&early_key, range(2, 4)).is_none());

    let cache = cache(10);
    let key = ObjectKey::from("download");
    let incoming = ObjectKey::from("incoming");
    let original = Bytes::from_static(b"abcdefghij");
    populate(&cache, key.clone(), download(range(0, 10), original.clone()));

    let returned = cache.get(&key, range(2, 4)).unwrap();
    assert_eq!(returned, Bytes::from_static(b"cd"));
    assert_eq!(returned.as_ptr(), original.slice(2..).as_ptr());
    for _ in 1..COMPACTION_GRACE_ACCESSES {
        cache.record_access(&key, range(2, 4));
    }
    populate(&cache, incoming, download(range(0, 2), Bytes::from_static(b"xy")));

    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.entry_count(), 2);
    assert_eq!(returned, Bytes::from_static(b"cd"));
    let retained = cache.get(&key, range(2, 4)).unwrap();
    assert_eq!(retained, Bytes::from_static(b"cd"));
    assert_ne!(retained.as_ptr(), original.slice(2..).as_ptr());
    assert!(cache.get(&key, range(0, 1)).is_none());
    assert_eq!(access_evidence_len(&cache, &key), MAX_ACCESS_EVENTS_PER_KEY);
    assert!(accessed_ranges(&cache, &key).iter().all(|seen| *seen == range(2, 4)));
}

#[test]
fn compaction_preserves_disjoint_requested_coverage_without_filling_gaps() {
    let cache = cache(10);
    let key = ObjectKey::from("download");
    populate(
        &cache,
        key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
    );
    cache.record_access(&key, range(1, 3));
    cache.record_access(&key, range(7, 9));
    for _ in 2..COMPACTION_GRACE_ACCESSES {
        cache.record_access(&key, range(1, 3));
    }

    populate(
        &cache,
        ObjectKey::from("incoming"),
        download(range(0, 2), Bytes::from_static(b"xy")),
    );

    assert_eq!(cache.used_bytes(), 6);
    assert_eq!(cache.get(&key, range(1, 3)).unwrap(), Bytes::from_static(b"bc"));
    assert_eq!(cache.get(&key, range(7, 9)).unwrap(), Bytes::from_static(b"hi"));
    assert!(cache.get(&key, range(3, 7)).is_none());
}

#[test]
fn compaction_waits_for_pressure_and_adds_no_access() {
    let cache = cache(16);
    let key = ObjectKey::from("download");
    let original = Bytes::from_static(b"abcdefghijklmnop");
    populate(&cache, key.clone(), download(range(0, 16), original.clone()));

    let returned = cache.get(&key, range(4, 8)).unwrap();
    for _ in 1..COMPACTION_GRACE_ACCESSES {
        cache.record_access(&key, range(4, 8));
    }

    assert_eq!(cache.used_bytes(), 16);
    assert_eq!(access_evidence_len(&cache, &key), MAX_ACCESS_EVENTS_PER_KEY);
    populate(
        &cache,
        ObjectKey::from("pressure"),
        download(range(0, 1), Bytes::from_static(b"x")),
    );

    assert_eq!(cache.used_bytes(), 5);
    assert_eq!(access_evidence_len(&cache, &key), MAX_ACCESS_EVENTS_PER_KEY);
    assert_eq!(returned, Bytes::from_static(b"efgh"));
    let retained = cache.get(&key, range(4, 8)).unwrap();
    assert_eq!(retained, returned);
    assert_ne!(retained.as_ptr(), original.slice(4..).as_ptr());
}

#[test]
fn candidate_state_tracks_entries_during_oversized_churn() {
    let cache = cache(1);
    for index in 0..300 {
        let key = ObjectKey::from(format!("download-{index}"));
        cache.insert_and_record(key, download(range(0, 2), Bytes::from_static(b"ab")), range(0, 1));
    }

    assert_eq!(cache.used_bytes(), 2);
    assert_eq!(cache.entry_count(), 1);
    assert_eq!(candidate_count(&cache), cache.entry_count() as usize);
}

#[test]
fn copied_compaction_is_revalidated_before_publication_and_can_fall_back() {
    let cache = cache(10);
    let key = ObjectKey::from("download");
    cache.insert_and_record(
        key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
        range(2, 4),
    );
    for _ in 1..COMPACTION_GRACE_ACCESSES {
        cache.record_access(&key, range(2, 4));
    }

    let incoming = ObjectKey::from("incoming");
    let incoming_bytes = Bytes::from_static(b"xy");
    let prepared = {
        let mut shard = cache.shards[0].lock();
        let AdmissionStep::Compact(work) = shard.admission_step(&incoming, range(0, 2), &incoming_bytes, None, true)
        else {
            panic!("pressure should select the cold compactable extent");
        };
        drop(shard);
        work.copy_payload()
    };

    cache.record_access(&key, range(6, 8));
    assert!(!cache.shards[0].lock().publish_compaction(prepared));
    assert_eq!(cache.used_bytes(), 10);
    assert!(cache.get(&key, range(6, 8)).is_some());

    let step = cache.shards[0]
        .lock()
        .admission_step(&incoming, range(0, 2), &incoming_bytes, None, false);
    assert!(matches!(step, AdmissionStep::Retry));
    assert_eq!(cache.used_bytes(), 0, "fallback pressure may evict but cannot starve");
}

#[test]
fn removing_the_last_extent_releases_its_access_metadata() {
    let cache = cache(1);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));
    cache.record_access(&key, range(0, 1));

    assert_eq!(access_evidence_len(&cache, &key), 1);
    assert!(cache.remove(&key, range(0, 1)));
    assert_eq!(access_evidence_len(&cache, &key), 0);
}

#[test]
fn zero_target_still_retains_the_latest_entry() {
    let cache = cache(0);
    let key = ObjectKey::from("object");

    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));
    populate(&cache, key.clone(), download(range(1, 2), Bytes::from_static(b"b")));

    assert!(cache.get(&key, range(0, 1)).is_none());
    assert_eq!(cache.get(&key, range(1, 2)).unwrap(), Bytes::from_static(b"b"));
    assert_eq!(cache.used_bytes(), 1);
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
    let cache = MemoryCache::with_shard_count(2, MemoryMetrics::noop(), 2);
    let mut keys = [None, None];
    for candidate in 0..100 {
        let key = ObjectKey::from(format!("object-{candidate}"));
        let shard = cache.shard_index(&key);
        keys[shard].get_or_insert(key);
        if keys.iter().all(Option::is_some) {
            break;
        }
    }
    let [Some(first), Some(second)] = keys else {
        panic!("test keys must cover both shards");
    };

    populate(&cache, first, download(range(0, 2), Bytes::from_static(b"aa")));
    populate(&cache, second, download(range(0, 2), Bytes::from_static(b"bb")));

    assert_eq!(cache.used_bytes(), 4);
    assert_eq!(cache.capacity(), 2);
}

#[test]
fn concurrent_shards_respect_their_targets_for_regular_entries() {
    let cache = Arc::new(MemoryCache::with_shard_count(256, MemoryMetrics::noop(), 8));
    let mut threads = Vec::new();
    for worker in 0..8_u64 {
        let cache = cache.clone();
        threads.push(thread::spawn(move || {
            let key = ObjectKey::from(format!("object-{worker}"));
            for index in 0..500_u64 {
                let start = index * 8;
                populate(
                    &cache,
                    key.clone(),
                    download(range(start, start + 8), Bytes::from(vec![worker as u8; 8])),
                );
                assert!(cache.used_bytes() <= cache.capacity());
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }

    assert!(cache.used_bytes() <= cache.capacity());
    assert_eq!(cache.used_bytes(), cache.entry_count() * 8);
}
