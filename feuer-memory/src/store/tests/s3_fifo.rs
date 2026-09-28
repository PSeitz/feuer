use super::*;
use feuer_types::EvictionPolicy;

fn s3_cache(capacity: u64) -> MemoryCache {
    cache(capacity)
        .with_eviction_policy(EvictionPolicy::S3Fifo)
        .with_reclaim_sample_size(1)
}

fn insert(cache: &MemoryCache, key: &str, size: usize) {
    cache.insert(key.to_owned(), Download::new(0, Bytes::from(vec![1; size])).unwrap());
}

#[test]
fn covering_hit_promotes_while_population_does_not() {
    let cache = s3_cache(30);
    let hot = "hot".to_owned();
    cache.insert_and_record(
        hot.clone(),
        download(range(0, 10), Bytes::from(vec![1; 10])),
        range(0, 1),
    );
    cache.get(&hot, range(5, 6)).unwrap();
    insert(&cache, "cold-a", 10);
    insert(&cache, "cold-b", 10);
    insert(&cache, "new", 20);
    assert_eq!(cache.used_bytes(), 30);
    assert!(cache.get(&hot, range(0, 10)).is_some());
    assert!(cache.get(&"cold-a".to_owned(), range(0, 1)).is_none());
    assert!(cache.get(&"cold-b".to_owned(), range(0, 1)).is_none());
}

#[test]
fn callback_misses_do_not_promote_one_hit_ranges() {
    let cache = s3_cache(30);
    let hot = "hot".to_owned();
    let requested = range(0, 1);
    cache.insert_and_record(
        hot.clone(),
        download(range(0, 10), Bytes::from(vec![1; 10])),
        requested,
    );
    assert_eq!(accessed_ranges(&cache, &hot), vec![requested]);
    cache.get(&hot, range(5, 6)).unwrap();
    for key in ["a", "b", "c", "d", "e"] {
        let key = key.to_owned();
        assert!(cache.get(&key, requested).is_none());
        cache.insert_and_record(
            key.clone(),
            download(range(0, 10), Bytes::from(vec![1; 10])),
            requested,
        );
        assert_eq!(accessed_ranges(&cache, &key), vec![requested]);
    }
    assert_eq!(cache.used_bytes(), 30);
    assert!(cache.get(&hot, range(0, 10)).is_some());
}

#[test]
fn redundant_download_records_access_but_does_not_reinsert() {
    let cache = s3_cache(30);
    let hot = "hot".to_owned();
    let source = download(range(0, 10), Bytes::from(vec![1; 10]));
    let id = cache
        .insert_and_record(hot.clone(), source.clone(), range(0, 1))
        .unwrap();
    insert(&cache, "cold-a", 10);
    insert(&cache, "cold-b", 10);
    assert_eq!(cache.insert_and_record(hot.clone(), source, range(5, 6)), None);
    insert(&cache, "new", 20);
    assert_eq!(cache.with_current_entry(&hot, range(0, 10), id, || true), Some(true));
}

#[test]
fn ghost_readmission_bypasses_small_queue() {
    let cache = s3_cache(30);
    for key in ["a", "b", "c", "d"] {
        insert(&cache, key, 10);
    }
    assert!(cache.get(&"a".to_owned(), range(0, 1)).is_none());
    insert(&cache, "a", 10); // Ghost hit: goes to main with no accesses.
    insert(&cache, "e", 10);
    insert(&cache, "f", 10);
    assert!(cache.get(&"a".to_owned(), range(0, 10)).is_some());
    assert_eq!(cache.used_bytes(), 30);
}

#[test]
fn superseded_small_entries_do_not_starve_main_eviction() {
    let cache = s3_cache(100);
    insert(&cache, "replaced", 40);
    insert(&cache, "hot", 60);
    for _ in 0..2 {
        cache.get(&"hot".to_owned(), range(0, 1)).unwrap();
    }
    insert(&cache, "replaced", 60);
    assert_eq!(cache.used_bytes(), 60);
    assert_eq!(cache.entry_count(), 1);
    assert!(cache.get(&"replaced".to_owned(), range(0, 60)).is_some());
    assert!(cache.get(&"hot".to_owned(), range(0, 1)).is_none());
}

#[test]
fn s3_fifo_evicts_whole_ranges_instead_of_trimming_and_retains_oversized_downloads() {
    let cache = s3_cache(100);
    insert(&cache, "hot", 100);
    for _ in 0..RANGE_TRIM_GRACE_ACCESSES {
        cache.get(&"hot".to_owned(), range(0, 1)).unwrap();
    }
    insert(&cache, "new", 10);
    assert!(cache.get(&"hot".to_owned(), range(0, 1)).is_none());
    assert_eq!(cache.used_bytes(), 10);
    insert(&cache, "oversized", 200);
    assert_eq!(cache.used_bytes(), 200);
    assert_eq!(cache.entry_count(), 1);
    insert(&cache, "next", 10);
    assert_eq!(cache.used_bytes(), 10);
}

#[test]
fn replacement_partial_overlap_and_removal_keep_candidate_indexes_consistent() {
    let cache = s3_cache(100);
    let key = "ranges".to_owned();
    for _ in 0..100 {
        cache.insert(key.clone(), download(range(0, 40), Bytes::from(vec![1; 40])));
        cache.insert(key.clone(), download(range(20, 60), Bytes::from(vec![2; 40])));
        assert_eq!(cache.entry_count(), 2);
        cache.insert(key.clone(), download(range(0, 60), Bytes::from(vec![3; 60])));
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(candidate_count(&cache), 1);
        assert!(cache.remove(&key, range(0, 60)));
        assert_eq!(cache.used_bytes(), 0);
        assert_eq!(candidate_count(&cache), 0);
    }
    insert(&cache, "after-churn", 100);
    insert(&cache, "next", 100);
    assert_eq!(cache.used_bytes(), 100);
}
