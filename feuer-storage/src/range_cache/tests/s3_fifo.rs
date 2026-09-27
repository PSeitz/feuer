use super::*;

async fn open_s3_cache(capacity: u64) -> (tempfile::TempDir, DiskRangeCache) {
    let directory = tempfile::tempdir().unwrap();
    let cache = DiskRangeCache::open_with_eviction_policy(
        directory.path(),
        capacity,
        IoMetrics::noop(),
        Arc::new(ObjectAccessHistories::new(1)),
        DiskMetrics::noop(),
        1,
        EvictionPolicy::S3Fifo,
    )
    .await
    .unwrap();
    (directory, cache)
}

#[tokio::test]
async fn verified_hits_promote_and_ghost_readmission_survives_cold_churn() {
    let (_directory, cache) = open_s3_cache(3 * CHUNK_BYTES).await;
    let source = download(0, CHUNK_BYTES as usize / 2);
    for key in ["hot", "b", "c"] {
        assert!(cache.insert(key.to_owned(), source.clone()).await.unwrap());
    }
    for _ in 0..2 {
        assert!(cache.get(&"hot".to_owned(), range(0, 1)).await.is_some());
    }
    assert!(cache.insert("d".to_owned(), source.clone()).await.unwrap());
    assert!(cache.contains(&"hot".to_owned(), source.downloaded_range()));
    assert!(!cache.contains(&"b".to_owned(), source.downloaded_range()));
    // An exact ghost hit enters main even without any disk accesses.
    assert!(cache.insert("b".to_owned(), source.clone()).await.unwrap());
    assert!(cache.insert("e".to_owned(), source.clone()).await.unwrap());
    assert!(cache.contains(&"hot".to_owned(), source.downloaded_range()));
    assert!(cache.contains(&"b".to_owned(), source.downloaded_range()));
    assert!(!cache.contains(&"c".to_owned(), source.downloaded_range()));
    assert!(!cache.contains(&"d".to_owned(), source.downloaded_range()));
}

#[tokio::test]
async fn replacement_and_integrity_invalidation_remove_fifo_state() {
    let (_directory, cache) = open_s3_cache(2 * CHUNK_BYTES).await;
    let key = "replaced".to_owned();
    assert!(cache.insert(key.clone(), download(5, 100)).await.unwrap());
    assert!(cache.insert(key.clone(), download(0, 200)).await.unwrap());
    assert_eq!(cache.get(&key, range(0, 200)).await.unwrap(), download(0, 200).bytes());
    let (payload, _) = entry_disk_ranges(&cache, &key);
    cache
        .disk
        .file
        .write_at(payload[0].start, &Bytes::from(vec![0; 4096]))
        .await
        .unwrap();
    assert!(cache.get(&key, range(0, 1)).await.is_none());
    assert!(!cache.contains(&key, range(0, 1)));
    // Stale queue entries from replacement or invalidation must not be selected.
    for i in 0..10 {
        assert!(cache.insert(format!("new-{i}"), download(0, 100)).await.unwrap());
    }
    let index = cache.disk.shards[0].entry_index.lock().unwrap();
    assert_eq!(index.eviction_candidates.len(), 2);
}

#[tokio::test]
async fn bounded_eviction_does_not_reuse_chunks_held_by_read_guards() {
    let (_directory, cache) = open_s3_cache(2 * CHUNK_BYTES).await;
    let key = "guarded".to_owned();
    let source = download(3, CHUNK_BYTES as usize + 17);
    assert!(cache.insert(key.clone(), source.clone()).await.unwrap());
    let read = {
        let index = cache.disk.shards[0].entry_index.lock().unwrap();
        let entry = index.covering_range(&key, source.downloaded_range()).unwrap();
        GuardedObjectRangeRead {
            object_range: entry.object_range,
            payload_checksum: entry.payload_checksum,
            payload_regions: entry.payload_regions.iter().map(DiskRegion::read_guard).collect(),
        }
    };
    let shard = &cache.disk.shards[0];
    let mut attempts = 0;
    let mut regions = MAX_EVICTION_REGIONS;
    assert!(!shard.evict_candidate(&mut attempts, &mut regions));
    attempts = 1;
    regions = 1;
    assert!(shard.evict_candidate(&mut attempts, &mut regions));
    assert_eq!(attempts, 0);
    assert_eq!(regions, 1);
    assert!(cache.contains(&key, range(3, 4)));
    assert!(!cache.insert("blocked".to_owned(), download(0, 1)).await.unwrap());
    assert!(!cache.contains(&key, range(3, 4)));
    assert_eq!(
        read.read(&cache.disk.file, range(3, 20)).await.unwrap().unwrap(),
        source.bytes().slice(..17)
    );
    drop(read);
    assert!(cache.insert("unblocked".to_owned(), download(0, 1)).await.unwrap());
}
