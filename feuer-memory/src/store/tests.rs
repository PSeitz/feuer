use std::{sync::Arc, thread};

use bytes::Bytes;
use feuer_types::{
    ByteRange, Download, ObjectKey,
    retention::{ACCESS_COUNT_HALF_LIFE, MAX_ACCESS_EVENTS_PER_KEY},
};

use super::{
    MemoryCache,
    shard::{AdmissionProgress, MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION},
    shard_capacity_for,
};
use crate::MemoryMetrics;

#[test]
fn disk_population_identity_expires_on_removal_replacement_and_readmission() {
    let cache = cache(1024);
    let key = ObjectKey::from("disk-source");
    let payload = Download::new(10, Bytes::from_static(b"abcd")).unwrap();
    let range = payload.downloaded_range();
    let id = cache.insert_and_record(key.clone(), payload.clone(), range).unwrap();
    assert_eq!(cache.with_current_entry(&key, range, id, || 7), Some(7));
    assert_eq!(cache.insert_and_record(key.clone(), payload.clone(), range), None);
    assert!(cache.remove(&key, range));
    let replacement_id = cache.insert_and_record(key.clone(), payload, range).unwrap();
    assert_ne!(id, replacement_id);
    assert!(
        cache
            .with_current_entry(&key, range, id, || panic!("stale admission"))
            .is_none()
    );
    cache.insert(key.clone(), Download::new(9, Bytes::from_static(b"abcdef")).unwrap());
    assert!(
        cache
            .with_current_entry(&key, range, replacement_id, || panic!("superseded admission"))
            .is_none()
    );
}

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

fn access_history_len(cache: &MemoryCache, key: &ObjectKey) -> usize {
    cache.shards[cache.shard_index(key)].lock().access_history_len(key)
}

fn candidate_count(cache: &MemoryCache) -> usize {
    cache.shards[0].lock().candidate_count()
}

#[test]
fn equal_cost_eviction_uses_entry_age_not_sample_order() {
    let cache = cache(3);
    let payload = Download::new(0, Bytes::from_static(b"x")).unwrap();
    let range = payload.downloaded_range();
    for key in ["a", "b", "c"] {
        cache.insert(key.to_owned(), payload.clone());
    }
    // Removing the first candidate moves c ahead of the older b in the sample.
    assert!(cache.remove(&"a".to_owned(), range));
    cache.insert("d".to_owned(), payload.clone());
    cache.insert("e".to_owned(), payload);
    assert!(cache.get(&"b".to_owned(), range).is_none());
    for key in ["c", "d", "e"] {
        assert!(cache.get(&key.to_owned(), range).is_some());
    }
}

#[test]
fn reclaim_sampling_advances_past_superseded_ranges() {
    let cache = cache(4).with_reclaim_sample_size(1);
    let key = "object".to_owned();
    for (start, bytes) in [
        (0, Bytes::from_static(b"a")),
        (1, Bytes::from_static(b"b")),
        (2, Bytes::from_static(b"cd")),
    ] {
        cache.insert(key.clone(), Download::new(start, bytes).unwrap());
    }
    let mut shard = cache.shards[0].lock();
    let replacement = Bytes::from_static(b"abc");
    for expected_entries in [3, 3, 2] {
        assert!(matches!(
            shard.advance_admission(&key, range(0, 3), &replacement, None, false),
            AdmissionProgress::Retry
        ));
        assert_eq!(shard.entry_count(), expected_entries);
    }
    // The partial overlap is eligible; the two fully superseded entries were skipped.
    assert!(matches!(
        shard.advance_admission(&key, range(0, 3), &replacement, None, false),
        AdmissionProgress::Complete(Some(_))
    ));
    assert_eq!(shard.entry_count(), 1);
    assert_eq!(shard.used_bytes(), 3);
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
fn access_history_survives_same_object_eviction_during_replacement() {
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
fn access_history_is_bounded_and_preserves_repeated_exact_requests() {
    let cache = cache(1);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));

    for _ in 0..*MAX_ACCESS_EVENTS_PER_KEY + 17 {
        cache.record_access(&key, range(0, 1));
    }

    assert_eq!(access_history_len(&cache, &key), *MAX_ACCESS_EVENTS_PER_KEY);
    assert_eq!(
        accessed_ranges(&cache, &key),
        vec![range(0, 1); *MAX_ACCESS_EVENTS_PER_KEY]
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
fn requested_bytes_contribute_to_retrieval_value() {
    let cache = cache(20);
    let small = ObjectKey::from("small-request");
    let large = ObjectKey::from("large-request");
    populate(
        &cache,
        small.clone(),
        download(range(0, 10), Bytes::from_static(b"0123456789")),
    );
    populate(
        &cache,
        large.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
    );

    cache.record_access(&small, range(0, 1));
    cache.record_access(&large, range(0, 9));
    populate(
        &cache,
        ObjectKey::from("incoming"),
        download(range(0, 10), Bytes::from_static(b"klmnopqrst")),
    );

    assert!(cache.get(&small, range(0, 1)).is_none());
    assert!(cache.get(&large, range(0, 9)).is_some());
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
fn stale_frequency_decays_below_recent_accesses() {
    let cache = cache(3);
    let stale = ObjectKey::from("stale");
    let fresh = ObjectKey::from("fresh");
    let clock = ObjectKey::from("clock");
    populate(&cache, stale.clone(), download(range(0, 1), Bytes::from_static(b"s")));
    populate(&cache, fresh.clone(), download(range(0, 1), Bytes::from_static(b"f")));
    populate(&cache, clock.clone(), download(range(0, 1), Bytes::from_static(b"c")));

    for _ in 0..8 {
        cache.record_access(&stale, range(0, 1));
    }
    for _ in 0..*ACCESS_COUNT_HALF_LIFE * 4 {
        cache.record_access(&clock, range(0, 1));
    }
    cache.record_access(&fresh, range(0, 1));
    populate(
        &cache,
        ObjectKey::from("incoming"),
        download(range(0, 1), Bytes::from_static(b"i")),
    );

    assert!(cache.get(&stale, range(0, 1)).is_none());
    assert!(cache.get(&fresh, range(0, 1)).is_some());
}

#[test]
fn range_trim_respects_grace_then_releases_unrequested_payload() {
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
    for _ in 1..MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION {
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
    assert_eq!(
        access_history_len(&cache, &key),
        (*MAX_ACCESS_EVENTS_PER_KEY).min(MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION as usize + 1)
    );
    assert!(accessed_ranges(&cache, &key).iter().all(|seen| *seen == range(2, 4)));
}

#[test]
fn range_trim_preserves_disjoint_requested_coverage_without_filling_gaps() {
    let cache = cache(10);
    let key = ObjectKey::from("download");
    populate(
        &cache,
        key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
    );
    cache.record_access(&key, range(1, 3));
    cache.record_access(&key, range(7, 9));
    for _ in 2..MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION {
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
fn range_trim_waits_for_pressure_and_adds_no_access() {
    let cache = cache(16);
    let key = ObjectKey::from("download");
    let original = Bytes::from_static(b"abcdefghijklmnop");
    populate(&cache, key.clone(), download(range(0, 16), original.clone()));

    let returned = cache.get(&key, range(4, 8)).unwrap();
    for _ in 1..MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION {
        cache.record_access(&key, range(4, 8));
    }

    assert_eq!(cache.used_bytes(), 16);
    let history_len = (*MAX_ACCESS_EVENTS_PER_KEY).min(MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION as usize);
    assert_eq!(access_history_len(&cache, &key), history_len);
    populate(
        &cache,
        ObjectKey::from("pressure"),
        download(range(0, 1), Bytes::from_static(b"x")),
    );

    assert_eq!(cache.used_bytes(), 5);
    assert_eq!(access_history_len(&cache, &key), history_len);
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
fn copied_range_trim_is_revalidated_before_publication_and_can_fall_back() {
    let cache = cache(10);
    let key = ObjectKey::from("download");
    cache.insert_and_record(
        key.clone(),
        download(range(0, 10), Bytes::from_static(b"abcdefghij")),
        range(2, 4),
    );
    for _ in 1..MIN_SHARD_ACCESSES_BEFORE_PAYLOAD_COMPACTION {
        cache.record_access(&key, range(2, 4));
    }

    let incoming = ObjectKey::from("incoming");
    let incoming_bytes = Bytes::from_static(b"xy");
    let replacement = {
        let mut shard = cache.shards[0].lock();
        let AdmissionProgress::Trim(source) =
            shard.advance_admission(&incoming, range(0, 2), &incoming_bytes, None, true)
        else {
            panic!("pressure should select the cold compactable cached range");
        };
        drop(shard);
        source.copy_payload()
    };

    // Simulate a disk-served request: it changes shared evidence without taking the memory shard lock.
    cache.access_histories().record_access(&key, range(6, 8));
    assert!(!cache.shards[0].lock().publish_range_trim(replacement));
    assert_eq!(cache.used_bytes(), 10);
    assert!(cache.get(&key, range(6, 8)).is_some());

    let step = cache.shards[0]
        .lock()
        .advance_admission(&incoming, range(0, 2), &incoming_bytes, None, false);
    assert!(matches!(step, AdmissionProgress::Retry));
    assert_eq!(cache.used_bytes(), 0, "fallback pressure may evict but cannot starve");
}

#[test]
fn removing_the_last_cached_range_releases_its_access_history() {
    let cache = cache(1);
    let key = ObjectKey::from("object");
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));
    cache.record_access(&key, range(0, 1));

    assert_eq!(access_history_len(&cache, &key), 1);
    assert!(cache.remove(&key, range(0, 1)));
    assert_eq!(access_history_len(&cache, &key), 0);
}

#[test]
fn shared_evidence_survives_memory_eviction_and_records_disk_only_requests() {
    let cache = cache(1);
    let key = "object".to_owned();
    cache.insert_and_record(
        key.clone(),
        download(range(0, 1), Bytes::from_static(b"a")),
        range(0, 1),
    );
    let disk_owner = cache.access_histories().for_key(&key);
    assert!(cache.remove(&key, range(0, 1)));
    cache.record_access(&key, range(10, 11));
    assert_eq!(disk_owner.lock().generation(), 2);
    populate(&cache, key.clone(), download(range(0, 1), Bytes::from_static(b"a")));
    assert_eq!(accessed_ranges(&cache, &key), vec![range(0, 1), range(10, 11)]);
    assert!(cache.get(&key, range(0, 1)).is_some());
    assert_eq!(disk_owner.lock().generation(), 3);
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
