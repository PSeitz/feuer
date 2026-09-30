use super::super::tests::{download, entry_disk_ranges, open_test_cache, range};
use super::*;

pub(crate) async fn wait_for_recovery(cache: &DiskRangeCache) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while cache.disk.recovery.running.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

// Open the same production state without starting the scan, to control write/scan ordering.
async fn open_paused(directory: &Path, capacity: u64) -> DiskRangeCache {
    let file = DataFile::open(directory, capacity, IoMetrics::noop()).await.unwrap();
    let count = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64) as usize;
    let (recovery, ends) = RecoveryState::open(directory, capacity, count).unwrap();
    let shards = ends
        .iter()
        .enumerate()
        .map(|(index, &end)| {
            let range = shard_range(capacity, count, index);
            let allocator = DiskChunkAllocator::for_disk_range(range.clone()).unwrap();
            allocator.start_recovery(range.start, end);
            DiskCacheShard {
                allocator,
                reclaim_sample_size: RECLAIM_SAMPLE_SIZE,
                entry_index: Mutex::new(DiskEntryIndex::new(DiskMetrics::noop())),
                written_end: AtomicU64::new(end),
            }
        })
        .collect();
    DiskRangeCache {
        disk: Arc::new(DiskRangeCacheState {
            file,
            shards,
            recovery,
            access_histories: Arc::new(ObjectAccessHistories::new()),
            metrics: DiskMetrics::noop(),
        }),
    }
}

#[test]
fn scan_ends_validate_checksum_layout_and_bounds() {
    let bounds = RecoveryBounds {
        generation: [1; 16],
        capacity: 256 * CHUNK_BYTES,
        ends: vec![CHUNK_BYTES, 129 * CHUNK_BYTES],
    };
    let bytes = bounds.encode();
    let decoded = RecoveryBounds::decode(&bytes).unwrap();
    assert_eq!(decoded.ends, bounds.ends);
    assert_eq!(decoded.capacity, bounds.capacity);
    for index in 0..bytes.len() {
        let mut corrupt = bytes.clone();
        corrupt[index] ^= 1;
        assert!(RecoveryBounds::decode(&corrupt).is_none());
    }
    assert!(RecoveryBounds::decode(&bytes[..16]).is_none());
    for capacity in [0, 1, u64::MAX] {
        let invalid = RecoveryBounds {
            capacity,
            ends: vec![0],
            ..bounds
        };
        assert!(RecoveryBounds::decode(&invalid.encode()).is_none());
    }
    for ends in [
        vec![],
        vec![0; 65],
        vec![1, 129 * CHUNK_BYTES],
        vec![129 * CHUNK_BYTES, 129 * CHUNK_BYTES],
        vec![0, 0],
    ] {
        let invalid = RecoveryBounds { ends, ..bounds };
        assert!(RecoveryBounds::decode(&invalid.encode()).is_none());
    }
}

#[test]
fn resets_are_persistent_and_interrupted_updates_leave_the_previous_file() {
    let directory = tempfile::tempdir().unwrap();
    let (first, _) = RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 2).unwrap();
    let (same, _) = RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 2).unwrap();
    assert_eq!(first.generation, same.generation);
    assert_ne!(first.next_batch_id(), same.next_batch_id());
    assert_ne!(same.next_batch_id(), same.next_batch_id());
    fs::write(first.path.with_extension("tmp"), b"interrupted update").unwrap();
    let (same, _) = RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 2).unwrap();
    assert_eq!(first.generation, same.generation);
    // Same capacity, different shard count: a new persistent generation, not reinterpreted ends.
    let (reset, ends) = RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 1).unwrap();
    assert_ne!(first.generation, reset.generation);
    assert_eq!(ends, vec![0]);
    let (again, _) = RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 1).unwrap();
    assert_eq!(reset.generation, again.generation);

    let (resized, ends) = RecoveryState::open(directory.path(), 128 * CHUNK_BYTES, 1).unwrap();
    assert_ne!(reset.generation, resized.generation);
    assert_eq!(ends, vec![0]);

    fs::write(&reset.path, b"torn inventory").unwrap();
    let (corrupt_reset, _) = RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 1).unwrap();
    assert_ne!(resized.generation, corrupt_reset.generation);
}

#[test]
fn layout_change_warnings_describe_previous_and_current_layouts() {
    let directory = tempfile::tempdir().unwrap();
    let log_path = directory.path().join("recovery.log");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Mutex::new(File::create(&log_path).unwrap()))
        .finish();
    let _logs = tracing::subscriber::set_default(subscriber);

    RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 2).unwrap();
    RecoveryState::open(directory.path(), 256 * CHUNK_BYTES, 1).unwrap();
    RecoveryState::open(directory.path(), 128 * CHUNK_BYTES, 1).unwrap();
    fs::write(directory.path().join(BOUNDS_FILE), b"torn inventory").unwrap();
    RecoveryState::open(directory.path(), 128 * CHUNK_BYTES, 1).unwrap();

    let log = fs::read_to_string(log_path).unwrap();
    let warnings: Vec<_> = log
        .lines()
        .filter(|line| line.contains("discarding entire disk cache"))
        .collect();
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("reason=\"shard count changed\""));
    assert!(warnings[0].contains("previous_shard_count=2"));
    assert!(warnings[0].contains("shard_count=1"));
    assert!(warnings[1].contains("reason=\"capacity changed\""));
    assert!(warnings[1].contains("previous_capacity_bytes=268435456"));
    assert!(warnings[1].contains("capacity_bytes=134217728"));
}

#[tokio::test]
async fn recovery_logs_start_and_completion_at_info() {
    let (directory, cache) = open_test_cache(8 * CHUNK_BYTES).await;
    cache.insert("object".to_owned(), download(0, 100)).await.unwrap();
    cache.disk.save_recovery_ends().unwrap();
    drop(cache);

    let log_path = directory.path().join("recovery.log");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .with_writer(Mutex::new(File::create(&log_path).unwrap()))
        .finish();
    let _logs = tracing::subscriber::set_default(subscriber);
    let cache = DiskRangeCache::open(directory.path(), 8 * CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    wait_for_recovery(&cache).await;

    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("starting disk cache recovery"));
    assert!(log.contains("shard_count=1"));
    assert!(log.contains("capacity_bytes=8388608"));
    assert!(log.contains("disk cache recovery finished"));
    assert!(log.contains("elapsed_seconds="));
    assert!(!log.contains("starting disk shard recovery scan"));
}

#[tokio::test]
async fn incrementally_recovers_shared_chunks_and_multi_chunk_entries() {
    let (directory, cache) = open_test_cache(8 * CHUNK_BYTES).await;
    let inputs = vec![
        ("small-a".to_owned(), download(17, 123)),
        ("small-b".to_owned(), download(9, 444)),
        ("large".to_owned(), download(3, 2 * CHUNK_BYTES as usize + 17)),
        ("metadata-only/".repeat(90_000), download(0, 512)),
    ];
    assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), inputs.len());
    cache.disk.save_recovery_ends().unwrap();
    drop(cache);
    let cache = open_paused(directory.path(), 8 * CHUNK_BYTES).await;
    let end = cache.disk.shards[0].written_end.load(Ordering::Relaxed);
    cache.disk.recover_chunk(0, 0, end).await;
    for (key, source) in &inputs[..2] {
        assert_eq!(cache.get(key, source.downloaded_range()).await.unwrap(), source.bytes());
    }
    assert!(!cache.contains(&inputs[2].0, inputs[2].1.downloaded_range()));
    for address in (CHUNK_BYTES..end).step_by(CHUNK_BYTES as usize) {
        cache.disk.recover_chunk(0, address, end).await;
    }
    for (key, source) in &inputs {
        assert_eq!(cache.get(key, source.downloaded_range()).await.unwrap(), source.bytes());
        assert_eq!(cache.access_histories().clock(), 0);
    }
    // Metadata-only chunks and partially occupied chunks must remain fully reserved.
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 8 * CHUNK_BYTES - end);
}

#[tokio::test]
async fn writes_can_win_before_scanning_and_recovery_does_not_resurrect_freed_entries() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    cache.insert("old".to_owned(), download(0, 100)).await.unwrap();
    cache.insert("untouched".to_owned(), download(0, 100)).await.unwrap();
    cache.disk.save_recovery_ends().unwrap();
    drop(cache);
    let cache = open_paused(directory.path(), 3 * CHUNK_BYTES).await;
    // Both lookups and writes work while the scan has not advanced at all.
    assert!(cache.get(&"old".to_owned(), range(0, 1)).await.is_none());
    assert!(cache.insert("new".to_owned(), download(0, 17)).await.unwrap());
    assert_eq!(
        cache.get(&"new".to_owned(), range(0, 17)).await.unwrap(),
        download(0, 17).bytes()
    );
    cache.disk.shards[0].entry_index.lock().unwrap().remove("new", 0);
    assert!(cache.disk.recover_chunk(0, 0, 2 * CHUNK_BYTES).await.is_none());
    assert!(!cache.contains(&"old".to_owned(), range(0, 1)));
    assert!(!cache.contains(&"new".to_owned(), range(0, 1)));
    cache.disk.recover_chunk(0, CHUNK_BYTES, 2 * CHUNK_BYTES).await.unwrap();
    assert!(cache.contains(&"untouched".to_owned(), range(0, 1)));
    cache.disk.shards[0].entry_index.lock().unwrap().remove("untouched", 0);
    assert!(
        cache
            .disk
            .recover_chunk(0, CHUNK_BYTES, 2 * CHUNK_BYTES)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn recovery_does_not_replace_a_range_published_by_a_write() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    cache.insert("discarded".to_owned(), download(0, 100)).await.unwrap();
    cache.insert("object".to_owned(), download(0, 100)).await.unwrap();
    cache.disk.save_recovery_ends().unwrap();
    drop(cache);
    let cache = open_paused(directory.path(), 3 * CHUNK_BYTES).await;
    cache.insert("object".to_owned(), download(0, 20)).await.unwrap();
    cache.disk.recover_chunk(0, CHUNK_BYTES, 2 * CHUNK_BYTES).await.unwrap();
    assert!(cache.contains(&"object".to_owned(), range(0, 20)));
    assert!(!cache.contains(&"object".to_owned(), range(0, 100)));
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 2 * CHUNK_BYTES);
}

#[tokio::test]
async fn stale_ends_bound_recovery_and_new_writes_do_not_extend_the_scan() {
    let (directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    cache.insert("saved".to_owned(), download(0, 100)).await.unwrap();
    cache.disk.save_recovery_ends().unwrap();
    cache.insert("beyond-end".to_owned(), download(0, 100)).await.unwrap();
    drop(cache);
    let cache = DiskRangeCache::open(directory.path(), 4 * CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    wait_for_recovery(&cache).await;
    assert!(cache.contains(&"saved".to_owned(), range(0, 1)));
    assert!(!cache.contains(&"beyond-end".to_owned(), range(0, 1)));
    assert!(cache.insert("new".to_owned(), download(0, 100)).await.unwrap());
    assert!(!cache.disk.recovery.running.load(Ordering::Acquire));
}

#[tokio::test]
async fn corrupt_payload_is_recovered_only_as_a_candidate_and_misses_on_read() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    cache.insert("object".to_owned(), download(0, 100)).await.unwrap();
    let payload = entry_disk_ranges(&cache, "object").0[0].clone();
    cache
        .disk
        .file
        .write_at(payload.start, &Bytes::from(vec![0xff; METADATA_PAGE_BYTES]))
        .await
        .unwrap();
    cache.disk.save_recovery_ends().unwrap();
    drop(cache);
    let cache = DiskRangeCache::open(directory.path(), CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    wait_for_recovery(&cache).await;
    assert!(cache.contains(&"object".to_owned(), range(0, 1)));
    assert!(cache.get(&"object".to_owned(), range(0, 1)).await.is_none());
    assert!(!cache.contains(&"object".to_owned(), range(0, 1)));
}

#[tokio::test]
async fn torn_metadata_and_reused_multi_chunk_addresses_are_rejected() {
    for corrupt_metadata in [false, true] {
        let (directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
        let key = "large".to_owned();
        cache
            .insert(key.clone(), download(0, 2 * CHUNK_BYTES as usize))
            .await
            .unwrap();
        let (payload, metadata) = entry_disk_ranges(&cache, &key);
        if corrupt_metadata {
            cache
                .disk
                .file
                .write_at(metadata[0].start, &Bytes::from(vec![0; METADATA_PAGE_BYTES]))
                .await
                .unwrap();
        } else {
            // Reuse just a payload chunk while old metadata survives elsewhere on disk.
            cache.disk.shards[0].entry_index.lock().unwrap().remove(&key, 0);
            assert!(cache.insert("replacement".to_owned(), download(0, 100)).await.unwrap());
            assert_eq!(entry_disk_ranges(&cache, "replacement").0[0].start, payload[0].start);
        }
        cache.disk.save_recovery_ends().unwrap();
        drop(cache);
        let cache = DiskRangeCache::open(directory.path(), 4 * CHUNK_BYTES, IoMetrics::noop())
            .await
            .unwrap();
        wait_for_recovery(&cache).await;
        assert!(!cache.contains(&key, range(0, 1)));
    }
}

#[tokio::test]
async fn a_payload_chunk_from_another_batch_cannot_join_old_entry_metadata() {
    let (directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = "large".to_owned();
    cache
        .insert(key.clone(), download(0, 2 * CHUNK_BYTES as usize))
        .await
        .unwrap();
    let page = cache.disk.file.read_at(0, METADATA_PAGE_BYTES).await.unwrap();
    let mut contents = page[96..96 + page_format::CHUNK_METADATA_CONTENT_BYTES].to_vec();
    assert_eq!(&contents[..32], &[0; 32]); // Payload-only chunk, no entry to recover independently.
    contents[48] ^= 1; // Valid page for a different batch, not just a checksum failure.
    let mut replaced = vec![0; METADATA_PAGE_BYTES];
    page_format::encode_page(
        &mut replaced,
        page_format::CHUNK_METADATA_PAGE_TAG,
        blake3::hash(&contents).as_bytes(),
        0,
        0,
        0,
        &contents,
    );
    cache.disk.file.write_at(0, &Bytes::from(replaced)).await.unwrap();
    cache.disk.save_recovery_ends().unwrap();
    drop(cache);
    let cache = DiskRangeCache::open(directory.path(), 4 * CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    wait_for_recovery(&cache).await;
    assert!(!cache.contains(&key, range(0, 1)));
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 4 * CHUNK_BYTES);
}

#[tokio::test]
async fn generation_reset_rejects_old_chunks_even_when_new_scan_bounds_cover_them() {
    let (directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    cache.insert("old-a".to_owned(), download(0, 100)).await.unwrap();
    cache.insert("old-b".to_owned(), download(0, 100)).await.unwrap();
    cache.disk.save_recovery_ends().unwrap();
    let old_generation = cache.disk.recovery.generation;
    drop(cache);
    fs::remove_file(directory.path().join(BOUNDS_FILE)).unwrap();
    let cache = open_paused(directory.path(), 4 * CHUNK_BYTES).await;
    assert_ne!(cache.disk.recovery.generation, old_generation);
    cache.disk.shards[0]
        .written_end
        .store(2 * CHUNK_BYTES, Ordering::Relaxed);
    cache.disk.save_recovery_ends().unwrap();
    let new_generation = cache.disk.recovery.generation;
    drop(cache);
    let cache = DiskRangeCache::open(directory.path(), 4 * CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    wait_for_recovery(&cache).await;
    assert_eq!(cache.disk.recovery.generation, new_generation);
    assert!(!cache.contains(&"old-a".to_owned(), range(0, 1)));
    assert!(!cache.contains(&"old-b".to_owned(), range(0, 1)));
}

#[test]
fn decoder_rejects_malformed_lengths_ranges_mappings_keys_and_batch_ids() {
    let allocator = DiskChunkAllocator::for_disk_range(0..CHUNK_BYTES).unwrap();
    let chunk = allocator.reserve_chunks(1).unwrap().pop().unwrap();
    let payload = vec![chunk.slice(4096..8192)];
    let bytes =
        page_format::encode_entry_metadata("key", range(7, 17), &payload, &blake3::hash(b"0123456789"), &[1; 16]);
    assert!(decode_entry(&bytes, &[1; 16], 0..CHUNK_BYTES).is_some());
    assert!(decode_entry(&bytes, &[2; 16], 0..CHUNK_BYTES).is_none());
    for (offset, value) in [
        (0, u64::MAX),
        (8, u64::MAX),
        (16, 17),
        (24, 7),
        (32, u64::MAX),
        (72, 0),
        (72, 4097),
        (80, 2 * CHUNK_BYTES),
        (80, 4096),
    ] {
        let mut invalid = bytes.to_vec();
        invalid[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        assert!(decode_entry(&invalid, &[1; 16], 0..CHUNK_BYTES).is_none());
    }
    let mut invalid = bytes.to_vec();
    invalid[88] = 0xff;
    assert!(decode_entry(&invalid, &[1; 16], 0..CHUNK_BYTES).is_none());
    for length in 0..bytes.len() {
        assert!(decode_entry(&bytes[..length], &[1; 16], 0..CHUNK_BYTES).is_none());
    }
}
