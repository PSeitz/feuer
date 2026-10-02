use super::*;
use crate::range_cache::tests::{download, open_test_cache, range};

async fn reopen(directory: &Path, cache: DiskRangeCache) -> DiskRangeCache {
    let capacity = cache.disk.file.capacity();
    drop(cache);
    DiskRangeCache::open(directory, capacity, IoMetrics::noop())
        .await
        .unwrap()
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
        let pages = cache.disk.shards[index].metadata.lock().unwrap();
        assert_eq!(pages.chunks[0].region.range().start, shard_disk_range.start);
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
    let lengths = [1, 4097, CHUNK_BYTES as usize, 3 * CHUNK_BYTES as usize + 7];
    let entries = lengths
        .iter()
        .enumerate()
        .map(|(i, &length)| (ObjectKeyHash(i as u128), download(17, length)))
        .collect();
    assert_eq!(cache.insert_batch(entries).await.unwrap(), lengths.len());
    let cache = reopen(directory.path(), cache).await;
    for (i, length) in lengths.into_iter().enumerate() {
        assert_eq!(
            cache
                .get(&ObjectKeyHash(i as u128), range(17, 17 + length as u64))
                .await
                .unwrap(),
            download(17, length).bytes()
        );
    }
    assert_eq!(cache.disk.shards[0].metadata.lock().unwrap().chunks.len(), 1);
}

#[tokio::test]
async fn follows_last_page_link_and_recovers_every_record_across_chunks() {
    let (directory, cache) = open_test_cache(100 * CHUNK_BYTES).await;
    let entries = RECORDS_PER_CHUNK + 17;
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
    {
        let pages = cache.disk.shards[0].metadata.lock().unwrap();
        assert_eq!(pages.chunks.len(), 2);
        assert_eq!(
            pages.chunks[0].next_chunk_address(),
            Some(pages.chunks[1].region.range().start)
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
    assert_eq!(shard.metadata.lock().unwrap().chunks.len(), 1);
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
    assert_eq!(cache.disk.shards[0].metadata.lock().unwrap().chunks.len(), 1);
}

#[tokio::test]
async fn index_entry_destruction_does_not_change_metadata_or_payload_occupancy() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let shard = &cache.disk.shards[0];
    let entry = shard.entry_index.lock().unwrap().remove(&key, 0).unwrap();
    let address = entry.payload_range.start;
    let (chunk_index, entry_metadata_index) = entry.metadata.unwrap();
    let metadata = shard.metadata.lock().unwrap();
    let entry_metadata_bytes = metadata.chunks[chunk_index]
        .entry_metadata_bytes(entry_metadata_index)
        .to_vec();
    drop(entry); // Must not lock metadata or release the allocator's payload.
    assert_eq!(
        metadata.chunks[chunk_index].entry_metadata_bytes(entry_metadata_index),
        entry_metadata_bytes
    );
    assert!(shard.allocator.reserve_chunks(1).is_none());
    shard.allocator.remove_payload(address);
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
async fn torn_record_page_does_not_reject_other_pages() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let entries = RECORDS_PER_PAGE + 1;
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
    assert!(cache.get(&ObjectKeyHash(0), range(0, 1)).await.is_none());
    assert!(
        cache
            .get(&ObjectKeyHash(RECORDS_PER_PAGE as u128), range(0, 1))
            .await
            .is_some()
    );
    assert!(cache.insert(ObjectKeyHash(999), download(0, 1)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&ObjectKeyHash(999), range(0, 1)).await.is_some());
    assert!(
        cache
            .get(&ObjectKeyHash(RECORDS_PER_PAGE as u128), range(0, 1))
            .await
            .is_some()
    );
}

#[tokio::test]
async fn corrupt_or_cyclic_link_terminates_recovery_and_can_be_repaired() {
    for cyclic in [false, true] {
        let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
        assert!(cache.insert(ObjectKeyHash(1), download(0, 1)).await.unwrap());
        let address: u64 = 0;
        let offset = address + (RECORD_PAGES * METADATA_PAGE_BYTES) as u64;
        let mut page = vec![0; METADATA_PAGE_BYTES];
        encode_page(&mut page, NEXT_CHUNK_PAGE_TAG, 1, &address.to_le_bytes());
        if !cyclic {
            page[0] ^= 1;
        }
        cache.disk.file.write_at(offset, &Bytes::from(page)).await.unwrap();
        let cache = reopen(directory.path(), cache).await;
        assert!(cache.get(&ObjectKeyHash(1), range(0, 1)).await.is_some());
        assert!(cache.insert(ObjectKeyHash(2), download(0, 1)).await.unwrap());
        assert_eq!(
            cache.disk.shards[0].metadata.lock().unwrap().chunks[0].next_chunk_address(),
            Some(NO_CHUNK)
        );
    }
}

#[tokio::test]
async fn payload_corruption_remains_a_checksum_miss() {
    let (directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let payload = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key][&0]
        .payload_range
        .clone();
    cache
        .disk
        .file
        .write_at(payload.start, &Bytes::from(vec![99; 4096]))
        .await
        .unwrap();
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&key, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn failed_metadata_flush_does_not_delay_payload_reuse() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let entries = RECORDS_PER_PAGE + 1;
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
    for i in 0..entries {
        let entry = shard
            .entry_index
            .lock()
            .unwrap()
            .remove(&ObjectKeyHash(i as u128), 0)
            .unwrap();
        shard.remove_payload(entry);
    }
    let directory = tempfile::tempdir().unwrap();
    let short_file = DataFile::open(directory.path(), METADATA_PAGE_BYTES as u64, IoMetrics::noop())
        .await
        .unwrap();
    let _io = shard.metadata_io.lock().await;
    // Making entry metadata available for overwrite produces no writes, even across multiple pages.
    shard.flush_metadata(&short_file).await.unwrap();
    shard.metadata.lock().unwrap().set_last_chunk_link(NO_CHUNK);
    assert!(shard.flush_metadata(&short_file).await.is_err());
    assert!(shard.allocator.reserve_chunks(1).is_some());
    shard.flush_metadata(&cache.disk.file).await.unwrap();
    assert!(shard.allocator.reserve_chunks(1).is_some());
}

#[tokio::test]
async fn a_write_after_open_replaces_recovered_entries() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(10, 10)).await.unwrap());
    let capacity = cache.disk.file.capacity();
    drop(cache);
    let cache = DiskRangeCache::open(directory.path(), capacity, IoMetrics::noop())
        .await
        .unwrap();
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
    assert!(cache.get(&ObjectKeyHash(1), range(0, 1)).await.is_none());
    assert!(cache.get(&ObjectKeyHash(2), range(0, 1)).await.is_some());
}

#[tokio::test]
async fn recovery_deduplicates_starts_and_restores_allocator_availability() {
    for second_length in [100, 200, CHUNK_BYTES as usize + 1] {
        let (directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
        let key = ObjectKeyHash(1);
        cache
            .insert_batch(vec![
                (key, download(0, 100)),
                (ObjectKeyHash(2), download(0, second_length)),
            ])
            .await
            .unwrap();
        // Give the second record the first record's key and start, keeping distinct payload addresses.
        let mut page = cache.disk.file.read_at(0, METADATA_PAGE_BYTES).await.unwrap().to_vec();
        let offset = PAGE_HEADER_BYTES + ENTRY_METADATA_BYTES;
        page[offset..offset + 16].copy_from_slice(&key.0.to_le_bytes());
        let checksum = XxHash64::oneshot(0, &page[8..]);
        page[..8].copy_from_slice(&checksum.to_le_bytes());
        cache.disk.file.write_at(0, &Bytes::from(page)).await.unwrap();
        let capacity = cache.disk.file.capacity();
        drop(cache);
        let (registry, backend) = crate::test_metrics::registry();
        let cache = DiskRangeCache::open_with_metrics(
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
            let index = shard.entry_index.lock().unwrap();
            assert_eq!(index.entries_by_key[&key].len(), 1);
            assert_eq!(index.eviction_candidates, vec![(key, 0)]);
            assert_eq!(index.entries_by_key[&key][&0].eviction_position, 0);
        }
        assert_eq!(shard.allocator.available_metadata_count(), RECORDS_PER_CHUNK - 1);
        assert_eq!(crate::test_metrics::value(&registry, "feuer_disk_entries", &[]), 1.0);
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_payload_bytes", &[]),
            second_length as f64
        );
        assert_eq!(
            cache.get(&key, range(0, second_length as u64)).await.unwrap(),
            download(0, second_length).bytes()
        );
        let recovered_chunks = if second_length <= CHUNK_BYTES as usize {
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
        let (_, entry_metadata_index) = shard.entry_index.lock().unwrap().entries_by_key[&ObjectKeyHash(3)][&0]
            .metadata
            .unwrap();
        assert_ne!(entry_metadata_index, if second_length == 100 { 0 } else { 1 });
        assert_eq!(shard.metadata.lock().unwrap().chunks.len(), 1);
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
    let entry = shard.entry_index.lock().unwrap().remove(&key, 0).unwrap();
    shard.allocator.remove_payload(entry.payload_range.start);
    let reused = shard.allocator.reserve_chunks(1).unwrap();
    cache
        .disk
        .file
        .write_parts(reused.range(), &[(0, Bytes::from(vec![99; CHUNK_BYTES as usize]))])
        .await
        .unwrap();
    drop(reused);
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.contains(&key, range(0, 100)));
    assert!(cache.get(&key, range(0, 100)).await.is_none());
}

#[test]
fn decoder_rejects_invalid_object_ranges_and_payload_addresses() {
    let record = encode_entry_metadata(&ObjectKeyHash(1), range(0, 100), &(0..4096), 9);
    assert!(decode_entry_metadata(&record, 0..4 * CHUNK_BYTES).is_some());
    for (offset, value) in [(16, u64::MAX), (24, 0), (24, u64::MAX), (32, 1), (32, 4 * CHUNK_BYTES)] {
        let mut invalid = record.to_vec();
        invalid[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        assert!(decode_entry_metadata(&invalid, 0..4 * CHUNK_BYTES).is_none());
    }
}
