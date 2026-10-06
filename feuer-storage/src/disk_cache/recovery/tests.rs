use super::*;
use crate::disk_cache::tests::{download, open_test_cache, range, with_manual_metadata_writes};

async fn reopen(directory: &Path, cache: DiskCache) -> DiskCache {
    cache.write_dirty_metadata_pages().await;
    let capacity = cache.disk.file.capacity();
    drop(cache);
    with_manual_metadata_writes(DiskCache::open(directory, capacity, IoMetrics::noop()).await.unwrap())
}

#[tokio::test]
async fn metadata_chains_start_at_each_shards_first_chunk_without_a_sidecar() {
    let (directory, cache) = open_test_cache(256 * CHUNK_BYTES).await;
    let cache = reopen(directory.path(), cache).await;
    assert_eq!(cache.disk.shards.len(), 2);
    for index in 0..cache.disk.shards.len() {
        assert!(
            cache
                .insert(ObjectKeyHash(index as u128), download(0, 1))
                .await
                .unwrap()
        );
        let shard_disk_range = shard_disk_range(cache.disk.file.capacity(), cache.disk.shards.len(), index);
        let pages = cache.disk.shards[index].metadata_pages.lock().unwrap();
        assert_eq!(
            pages.chunks[0].reserved_chunk.disk_byte_range().start,
            shard_disk_range.start
        );
    }
    assert!(!directory.path().join("recovery-heads").exists());
    let cache = reopen(directory.path(), cache).await;
    for index in 0..cache.disk.shards.len() {
        assert_eq!(
            cache.get(&ObjectKeyHash(index as u128), range(0, 1)).await.unwrap(),
            download(0, 1).bytes()
        );
    }
}

#[tokio::test]
async fn recovers_separate_metadata_and_contiguous_multi_chunk_payloads() {
    let (directory, cache) = open_test_cache(12 * CHUNK_BYTES).await;
    let lengths = [0, 1, 4096, 4097, CHUNK_BYTES as usize, 3 * CHUNK_BYTES as usize + 7];
    let entries = lengths
        .iter()
        .enumerate()
        .map(|(i, &length)| (ObjectKeyHash(i as u128), download(u64::MAX - length as u64, length)))
        .collect();
    assert_eq!(cache.insert_batch(entries).await.unwrap(), lengths.len());
    let cache = reopen(directory.path(), cache).await;
    for (i, length) in lengths.into_iter().enumerate() {
        assert_eq!(
            cache
                .get(&ObjectKeyHash(i as u128), range(u64::MAX - length as u64, u64::MAX))
                .await
                .unwrap(),
            download(u64::MAX - length as u64, length).bytes()
        );
        cache.disk.shards[0].remove_entry(&ObjectKeyHash(i as u128), u64::MAX - length as u64);
    }
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 12 * CHUNK_BYTES);
    assert_eq!(cache.disk.shards[0].metadata_pages.lock().unwrap().chunks.len(), 1);
}

#[tokio::test]
async fn concurrent_batches_grow_metadata_chain_and_recover_every_entry() {
    let (directory, cache) = open_test_cache(100 * CHUNK_BYTES).await;
    let entries = ENTRIES_PER_METADATA_CHUNK + 17;
    let inputs = |keys: Range<usize>| keys.map(|i| (ObjectKeyHash(i as u128), download(0, 1))).collect();
    let (first, second) = tokio::join!(
        cache.insert_batch(inputs(0..entries / 2)),
        cache.insert_batch(inputs(entries / 2..entries)),
    );
    assert_eq!(first.unwrap() + second.unwrap(), entries);
    {
        let pages = cache.disk.shards[0].metadata_pages.lock().unwrap();
        assert_eq!(pages.chunks.len(), 2);
        assert_eq!(
            pages.chunks[0].next_chunk_address(),
            Some(pages.chunks[1].reserved_chunk.disk_byte_range().start)
        );
        assert_eq!(pages.chunks[1].next_chunk_address(), Some(NO_CHUNK));
    }
    let cache = reopen(directory.path(), cache).await;
    for i in 0..entries {
        assert_eq!(
            cache.get(&ObjectKeyHash(i as u128), range(0, 1)).await.unwrap(),
            download(0, 1).bytes()
        );
    }
}

#[tokio::test]
async fn multiple_batches_update_one_metadata_chunk_and_reuse_entry_metadata() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let shard = &cache.disk.shards[0];
    for i in 0..20 {
        assert!(
            cache
                .insert(ObjectKeyHash(i), download(0, CHUNK_BYTES as usize))
                .await
                .unwrap()
        );
    }
    assert_eq!(shard.metadata_pages.lock().unwrap().chunks.len(), 1);
    let live_keys: Vec<_> = shard
        .entry_index
        .lock()
        .unwrap()
        .entries_by_key
        .keys()
        .copied()
        .collect();
    let cache = reopen(directory.path(), cache).await;
    for key in live_keys {
        assert!(cache.get(&key, range(0, 1)).await.is_some());
    }
    assert_eq!(cache.disk.shards[0].metadata_pages.lock().unwrap().chunks.len(), 1);
}

#[tokio::test]
async fn index_entry_destruction_does_not_change_metadata_or_payload_occupancy() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let shard = &cache.disk.shards[0];
    let entry = shard.entry_index.lock().unwrap().take_entry(&key, 0).unwrap();
    let address = entry.payload_address;
    let (chunk_index, entry_metadata_index) = entry.metadata;
    let metadata = shard.metadata_pages.lock().unwrap();
    let entry_metadata_bytes = metadata.chunks[chunk_index]
        .entry_metadata_bytes(entry_metadata_index)
        .to_vec();
    drop(entry); // Must not lock metadata or release the allocator's payload.
    assert_eq!(
        metadata.chunks[chunk_index].entry_metadata_bytes(entry_metadata_index),
        entry_metadata_bytes
    );
    assert!(shard.allocator.reserve_chunks(1).is_none());
    shard.allocator.release_payload(address);
    assert!(shard.allocator.reserve_chunks(1).is_some());
    assert_eq!(
        metadata.chunks[chunk_index].entry_metadata_bytes(entry_metadata_index),
        entry_metadata_bytes
    );
}

#[tokio::test]
async fn eviction_does_not_recover_an_old_record_for_reused_payload_chunks() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let old = ObjectKeyHash(1);
    let new = ObjectKeyHash(2);
    assert!(cache.insert(old, download(0, 4096)).await.unwrap());
    assert!(cache.insert(new, download(100, 4096)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&old, range(0, 1)).await.is_none());
    assert_eq!(
        cache.get(&new, range(100, 4196)).await.unwrap(),
        download(100, 4096).bytes()
    );
}

#[tokio::test]
async fn torn_entry_metadata_page_does_not_reject_other_pages() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let entries = ENTRIES_PER_METADATA_PAGE + 1;
    assert_eq!(
        cache
            .insert_batch(
                (0..entries)
                    .map(|i| (ObjectKeyHash(i as u128), download(0, 1)))
                    .collect()
            )
            .await
            .unwrap(),
        entries
    );
    cache.write_dirty_metadata_pages().await;
    let address = 0;
    let mut bytes = cache
        .disk
        .file
        .read_at(address, METADATA_PAGE_BYTES)
        .await
        .unwrap()
        .to_vec();
    bytes[PAGE_HEADER_BYTES] ^= 1;
    cache.disk.file.write_at(address, &Bytes::from(bytes)).await.unwrap();
    let cache = reopen(directory.path(), cache).await;
    let shard = &cache.disk.shards[0];
    assert!(shard.metadata_pages.lock().unwrap().dirty_pages.is_empty());
    // Reverse publication assigns key 0 a position on the second page.
    assert!(cache.get(&ObjectKeyHash(0), range(0, 1)).await.is_some());
    assert!(
        cache
            .get(&ObjectKeyHash(ENTRIES_PER_METADATA_PAGE as u128), range(0, 1))
            .await
            .is_none()
    );
    assert!(cache.insert(ObjectKeyHash(999), download(0, 1)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&ObjectKeyHash(999), range(0, 1)).await.is_some());
    assert!(cache.get(&ObjectKeyHash(0), range(0, 1)).await.is_some());
}

#[tokio::test]
async fn corrupt_or_cyclic_link_terminates_recovery_and_can_be_repaired() {
    for (next_chunk_address, corrupt_checksum) in [
        (0, true),
        (0, false),
        (2 * CHUNK_BYTES, false),
        (4 * CHUNK_BYTES, false),
    ] {
        let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
        assert!(cache.insert(ObjectKeyHash(1), download(0, 1)).await.unwrap());
        cache.write_dirty_metadata_pages().await;
        let offset = (ENTRY_METADATA_PAGES_PER_CHUNK * METADATA_PAGE_BYTES) as u64;
        let mut page = vec![0; METADATA_PAGE_BYTES];
        encode_page(&mut page, NEXT_CHUNK_PAGE_TAG, 1, &next_chunk_address.to_le_bytes());
        if corrupt_checksum {
            page[0] ^= 1;
        }
        cache.disk.file.write_at(offset, &Bytes::from(page)).await.unwrap();
        let cache = reopen(directory.path(), cache).await;
        assert!(cache.get(&ObjectKeyHash(1), range(0, 1)).await.is_some());
        assert!(cache.insert(ObjectKeyHash(2), download(0, 1)).await.unwrap());
        assert_eq!(
            cache.disk.shards[0].metadata_pages.lock().unwrap().chunks[0].next_chunk_address(),
            Some(NO_CHUNK)
        );
        let cache = reopen(directory.path(), cache).await;
        assert!(cache.get(&ObjectKeyHash(1), range(0, 1)).await.is_some());
        assert!(cache.get(&ObjectKeyHash(2), range(0, 1)).await.is_some());
    }
}

#[tokio::test]
async fn payload_corruption_remains_a_checksum_miss() {
    let (directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let address = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key][&0].payload_address;
    cache
        .disk
        .file
        .write_at(address, &Bytes::from(vec![99; 4096]))
        .await
        .unwrap();
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&key, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn failed_metadata_writes_do_not_retry_or_block_payload_use() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let entries = ENTRIES_PER_METADATA_PAGE + 1;
    assert_eq!(
        cache
            .insert_batch(
                (0..entries)
                    .map(|i| (ObjectKeyHash(i as u128), download(0, 1)))
                    .collect()
            )
            .await
            .unwrap(),
        entries
    );
    let shard = &cache.disk.shards[0];
    let directory = tempfile::tempdir().unwrap();
    let short_file = DataFile::open(directory.path(), METADATA_PAGE_BYTES as u64, IoMetrics::noop())
        .await
        .unwrap();
    assert!(shard.write_dirty_metadata_pages(&short_file).await.is_err());
    assert!(shard.metadata_pages.lock().unwrap().dirty_pages.is_empty());
    shard.write_dirty_metadata_pages(&short_file).await.unwrap();
    assert_eq!(
        cache.get(&ObjectKeyHash(0), range(0, 1)).await.unwrap(),
        download(0, 1).bytes()
    );
    for i in 0..entries {
        shard.remove_entry(&ObjectKeyHash(i as u128), 0);
    }
    // Returning slots across multiple pages needs no invalidation writes or metadata retry.
    assert!(shard.metadata_pages.lock().unwrap().dirty_pages.is_empty());
    assert!(shard.allocator.reserve_chunks(1).is_some());
}

#[tokio::test]
async fn a_write_after_open_replaces_recovered_entries() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(10, 10)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.insert(key, download(0, 100)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert_eq!(cache.get(&key, range(0, 100)).await.unwrap(), download(0, 100).bytes());
    assert_eq!(
        cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key].len(),
        1
    );
}

#[tokio::test]
async fn metadata_cannot_claim_a_metadata_chunk_as_payload() {
    let (directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    assert_eq!(
        cache
            .insert_batch(vec![
                (ObjectKeyHash(1), download(0, 1)),
                (ObjectKeyHash(2), download(0, 1))
            ])
            .await
            .unwrap(),
        2
    );
    cache.write_dirty_metadata_pages().await;
    let address = 0;
    let mut bytes = cache
        .disk
        .file
        .read_at(address, METADATA_PAGE_BYTES)
        .await
        .unwrap()
        .to_vec();
    bytes[PAGE_HEADER_BYTES + 32..PAGE_HEADER_BYTES + 40].copy_from_slice(&address.to_le_bytes());
    let checksum = XxHash64::oneshot(0, &bytes[8..]);
    bytes[..8].copy_from_slice(&checksum.to_le_bytes());
    cache.disk.file.write_at(address, &Bytes::from(bytes)).await.unwrap();
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&ObjectKeyHash(2), range(0, 1)).await.is_none());
    assert!(cache.get(&ObjectKeyHash(1), range(0, 1)).await.is_some());
}

#[tokio::test]
async fn recovery_deduplicates_starts_and_restores_allocator_availability() {
    for second_length in [50, 100, 200, CHUNK_BYTES as usize + 1] {
        let retained_length = second_length.max(100);
        let (directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
        let key = ObjectKeyHash(1);
        cache
            .insert_batch(vec![
                (key, download(0, 100)),
                (ObjectKeyHash(2), download(0, second_length)),
            ])
            .await
            .unwrap();
        cache.write_dirty_metadata_pages().await;
        // Give key 2's record key 1's identity, regardless of which payload was written first.
        let position = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&ObjectKeyHash(2)][&0]
            .metadata
            .1;
        let mut page = cache.disk.file.read_at(0, METADATA_PAGE_BYTES).await.unwrap().to_vec();
        let offset = PAGE_HEADER_BYTES + position * ENTRY_METADATA_BYTES;
        page[offset..offset + 16].copy_from_slice(&key.0.to_le_bytes());
        let checksum = XxHash64::oneshot(0, &page[8..]);
        page[..8].copy_from_slice(&checksum.to_le_bytes());
        cache.disk.file.write_at(0, &Bytes::from(page)).await.unwrap();
        let capacity = cache.disk.file.capacity();
        drop(cache);
        let (registry, backend) = crate::test_metrics::registry();
        let cache = DiskCache::open_with_metrics(
            directory.path(),
            capacity,
            IoMetrics::noop(),
            Arc::new(ObjectAccessHistories::new()),
            DiskMetrics::new(&backend),
            RECLAIM_SAMPLE_SIZE,
        )
        .await
        .unwrap();
        let shard = &cache.disk.shards[0];
        {
            let disk_index = shard.entry_index.lock().unwrap();
            assert_eq!(disk_index.entries_by_key[&key].len(), 1);
            assert_eq!(disk_index.eviction_candidates, vec![(key, 0)]);
            assert_eq!(disk_index.entries_by_key[&key][&0].eviction_position, 0);
        }
        assert_eq!(
            shard.metadata_pages.lock().unwrap().free_entry_positions.len(),
            ENTRIES_PER_METADATA_CHUNK - 1
        );
        assert_eq!(crate::test_metrics::value(&registry, "feuer_disk_entries", &[]), 1.0);
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_payload_bytes", &[]),
            retained_length as f64
        );
        assert_eq!(
            cache.get(&key, range(0, retained_length as u64)).await.unwrap(),
            download(0, retained_length).bytes()
        );
        let recovered_chunks = if retained_length <= CHUNK_BYTES as usize {
            2.0
        } else {
            3.0
        };
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_recovered_chunks", &[]),
            recovered_chunks
        );
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_chunks", &[("state", "allocated")]),
            recovered_chunks
        );
        assert!(cache.insert(ObjectKeyHash(3), download(0, 1)).await.unwrap());
        let (_, entry_metadata_index) =
            shard.entry_index.lock().unwrap().entries_by_key[&ObjectKeyHash(3)][&0].metadata;
        let retained_position = shard.entry_index.lock().unwrap().entries_by_key[&key][&0].metadata.1;
        assert_ne!(entry_metadata_index, retained_position);
        assert_eq!(shard.metadata_pages.lock().unwrap().chunks.len(), 1);
        drop(cache);
        assert_eq!(crate::test_metrics::value(&registry, "feuer_disk_entries", &[]), 0.0);
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_chunks", &[("state", "allocated")]),
            0.0
        );
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_recovered_chunks", &[]),
            0.0
        );
    }
}

#[tokio::test]
async fn stale_metadata_after_payload_reuse_recovers_as_a_checksum_miss() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 100)).await.unwrap());
    let shard = &cache.disk.shards[0];
    shard.remove_entry(&key, 0);
    let reused = shard.allocator.reserve_chunks(1).unwrap();
    cache
        .disk
        .file
        .write_at(
            reused.disk_byte_range().start,
            &Bytes::from(vec![99; CHUNK_BYTES as usize]),
        )
        .await
        .unwrap();
    drop(reused);
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.covers_range(&key, range(0, 100)));
    assert!(cache.get(&key, range(0, 100)).await.is_none());
}
