use super::*;
use crate::disk_cache::tests::{download, entry_disk_ranges, open_test_cache, with_manual_metadata_writes};
use feuer_memory::BUFFER_ALIGNMENT;
use proptest::prelude::*;
use std::collections::BTreeSet;

async fn reopen(directory: &Path, cache: DiskCache) -> DiskCache {
    cache.write_dirty_metadata_pages().await;
    let capacity = cache.disk.file.capacity();
    drop(cache);
    with_manual_metadata_writes(
        DiskCache::open(directory, capacity, DiskCacheOptions::default())
            .await
            .unwrap(),
    )
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
        assert_eq!(cache.get(&ObjectKeyHash(index as u128)).await.unwrap(), download(0, 1));
    }
}

fn payload_lengths() -> impl Strategy<Value = Vec<usize>> {
    let chunk = CHUNK_BYTES as usize;
    let alignment = BUFFER_ALIGNMENT;
    let half = chunk / 2;
    prop::collection::vec(
        prop_oneof![
            0..=2 * alignment,
            half - 2 * alignment..=half + 2 * alignment,
            chunk - 2 * alignment..=chunk + 2 * alignment,
            2 * chunk - 2 * alignment..=2 * chunk + 2 * alignment,
        ],
        0..8,
    )
    .prop_map(move |mut lengths| {
        // Keep every boundary even when shrinking removes all generated lengths.
        lengths.extend([
            0,
            1,
            alignment - 1,
            alignment,
            alignment + 1,
            half - alignment,
            half - alignment + 1,
            half - 1,
            half,
            half + 1,
            chunk - 1,
            chunk,
            chunk + 1,
            2 * chunk,
            3 * chunk + 7,
        ]);
        lengths
    })
    .prop_shuffle()
}

proptest! {
    // Every case writes and checks all boundary sizes, including several multi-chunk payloads.
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn payload_packing_and_recovery_preserve_bytes_and_chunk_ownership(
        lengths in payload_lengths(),
        seed in any::<u64>(),
    ) {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            // Even a separate chunk range for every entry fits, so admission must succeed.
            let payload_capacity = lengths.iter()
                .map(|&length| (length as u64).max(1).div_ceil(CHUNK_BYTES))
                .sum::<u64>() * CHUNK_BYTES;
            let (directory, mut cache) = open_test_cache(payload_capacity).await;
            let inputs: Vec<_> = lengths.into_iter().enumerate()
                .map(|(i, length)| (ObjectKeyHash(i as u128), download(seed.wrapping_add(i as u64), length)))
                .collect();
            assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), inputs.len());
            for recovered in [false, true] {
                if recovered {
                    cache = reopen(directory.path(), cache).await;
                }
                let mut entries = Vec::new();
                for (key, source) in &inputs {
                    let (payload, metadata) = entry_disk_ranges(&cache, key).await;
                    entries.push((key, source, payload, metadata));
                }
                // Equal-length entries may be ordered either way within the batch.
                entries.sort_unstable_by_key(|(_, source, payload, _)| (source.len(), payload.start));
                let mut chunks = BTreeSet::new();
                let mut shared_end = 0;
                for (key, source, payload, metadata) in entries {
                    let disk_bytes = (source.len() as u64).max(1).next_multiple_of(BUFFER_ALIGNMENT as u64);
                    let small = disk_bytes < 512 * 1024;
                    let offset = if small && shared_end % CHUNK_BYTES + disk_bytes <= CHUNK_BYTES {
                        shared_end % CHUNK_BYTES
                    } else {
                        0
                    };
                    assert_eq!(payload.start % CHUNK_BYTES, offset);
                    assert_eq!(payload.end - payload.start, disk_bytes);
                    assert!(payload.start >= 2 * CHUNK_BYTES && payload.end <= cache.disk.file.capacity());
                    assert_eq!(metadata, CHUNK_BYTES..CHUNK_BYTES + METADATA_PAGE_BYTES as u64);
                    let first_chunk = payload.start / CHUNK_BYTES;
                    let end_chunk = payload.end.div_ceil(CHUNK_BYTES);
                    if offset != 0 {
                        assert_eq!(payload.start, shared_end);
                    } else {
                        for chunk in first_chunk..end_chunk {
                            assert!(chunks.insert(chunk), "standalone payloads and separate packed groups must not share chunks");
                        }
                    }
                    if small {
                        assert_eq!(end_chunk - first_chunk, 1);
                        shared_end = payload.end;
                    }
                    assert_eq!(cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[key].chunk_count(), end_chunk - first_chunk);
                    // Read disk directly so lookup logic cannot hide headers or metadata gaps.
                    let stored = cache.disk.file.read_at(payload.start, disk_bytes as usize).await.unwrap();
                    assert_eq!(&stored[..source.len()], source.as_ref());
                    assert!(stored[source.len()..].iter().all(|&byte| byte == 0));
                    let (bytes, capacity) = cache.get_with_fallthrough(key, || false).await.unwrap();
                    assert_eq!(bytes, *source);
                    assert!(capacity >= bytes.len());
                }
                assert_eq!(cache.disk.shards[0].allocator.available_bytes(), payload_capacity - chunks.len() as u64 * CHUNK_BYTES);
            }
            for (key, _) in inputs {
                cache.disk.shards[0].remove_entry(&key);
            }
            assert_eq!(cache.disk.shards[0].allocator.available_bytes(), payload_capacity);
            assert_eq!(cache.disk.shards[0].metadata_pages.lock().unwrap().chunks.len(), 1);
        });
    }
}

#[tokio::test]
async fn concurrent_batches_grow_metadata_chain_and_recover_every_entry() {
    let (directory, cache) = open_test_cache(110 * CHUNK_BYTES).await;
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
            pages.chunks[0].next_chunk_address,
            pages.chunks[1].reserved_chunk.disk_byte_range().start
        );
        assert_eq!(pages.chunks[1].next_chunk_address, NO_CHUNK);
    }
    let cache = reopen(directory.path(), cache).await;
    for i in 0..entries {
        assert_eq!(cache.get(&ObjectKeyHash(i as u128)).await.unwrap(), download(0, 1));
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
        assert!(cache.get(&key).await.is_some());
    }
    assert_eq!(cache.disk.shards[0].metadata_pages.lock().unwrap().chunks.len(), 1);
}

#[tokio::test]
async fn index_entry_destruction_does_not_change_metadata_or_payload_occupancy() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let shard = &cache.disk.shards[0];
    let entry = shard.entry_index.lock().unwrap().take_entry(&key).unwrap();
    let address = entry.payload_address;
    let location = entry.metadata;
    let metadata = shard.metadata_pages.lock().unwrap();
    let slot = |metadata: &metadata::MetadataChunks| {
        metadata.chunks[location.chunk_index as usize].entries[location.entry_index as usize]
    };
    let entry_metadata = slot(&metadata);
    assert!(entry_metadata.is_some());
    drop(entry); // Must not lock metadata or release the allocator's payload.
    assert_eq!(slot(&metadata), entry_metadata);
    assert!(shard.allocator.reserve_chunks(1).is_none());
    shard.allocator.release_payload(address);
    assert!(shard.allocator.reserve_chunks(1).is_some());
    assert_eq!(slot(&metadata), entry_metadata);
}

#[tokio::test]
async fn eviction_does_not_recover_old_metadata_for_reused_payload_chunks() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let old = ObjectKeyHash(1);
    let new = ObjectKeyHash(2);
    assert!(cache.insert(old, download(0, 4096)).await.unwrap());
    assert!(cache.insert(new, download(100, 4096)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&old).await.is_none());
    assert_eq!(cache.get(&new).await.unwrap(), download(100, 4096));
}

#[tokio::test]
async fn metadata_reuse_preserves_multi_chunk_recovery() {
    for recover_before_removal in [false, true] {
        for reverse_removal in [false, true] {
            let (directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
            let first = ObjectKeyHash(1);
            let second = ObjectKeyHash(2);
            let large = ObjectKeyHash(3);
            for key in [first, second] {
                assert!(cache.insert(key, download(0, CHUNK_BYTES as usize)).await.unwrap());
            }
            let cache = if recover_before_removal {
                reopen(directory.path(), cache).await
            } else {
                cache
            };
            cache.write_dirty_metadata_pages().await;
            let shard = &cache.disk.shards[0];
            let removed = if reverse_removal {
                [second, first]
            } else {
                [first, second]
            };
            for key in removed {
                shard.remove_entry(&key);
            }
            let length = CHUNK_BYTES as usize + 1;
            assert!(cache.insert(large, download(0, length)).await.unwrap());
            let cache = reopen(directory.path(), cache).await;
            assert!(cache.contains(&large));
            assert_eq!(cache.get(&large).await.unwrap(), download(0, length));
        }
    }
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
    let address = CHUNK_BYTES;
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
    assert!(cache.get(&ObjectKeyHash(0)).await.is_some());
    assert!(
        cache
            .get(&ObjectKeyHash(ENTRIES_PER_METADATA_PAGE as u128))
            .await
            .is_none()
    );
    assert!(cache.insert(ObjectKeyHash(999), download(0, 1)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&ObjectKeyHash(999)).await.is_some());
    assert!(cache.get(&ObjectKeyHash(0)).await.is_some());
}

#[tokio::test]
async fn corrupt_unaligned_or_cyclic_link_terminates_recovery_and_can_be_repaired() {
    for (next_chunk_address, corrupt_checksum) in [
        (CHUNK_BYTES, true),
        (CHUNK_BYTES, false),
        (2 * CHUNK_BYTES + METADATA_PAGE_BYTES as u64, false),
        (3 * CHUNK_BYTES, false),
        (5 * CHUNK_BYTES, false),
    ] {
        let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
        assert!(cache.insert(ObjectKeyHash(1), download(0, 1)).await.unwrap());
        cache.write_dirty_metadata_pages().await;
        let offset = CHUNK_BYTES + (ENTRY_METADATA_PAGES_PER_CHUNK * METADATA_PAGE_BYTES) as u64;
        let mut page = vec![0; METADATA_PAGE_BYTES];
        encode_page(&mut page, NEXT_CHUNK_PAGE_TAG, 1, &next_chunk_address.to_le_bytes());
        if corrupt_checksum {
            page[0] ^= 1;
        }
        let page = Bytes::from(page);
        if !next_chunk_address.is_multiple_of(CHUNK_BYTES) {
            // Keep the target nonzero so recovery cannot dismiss it as an unwritten chunk.
            cache.disk.file.write_at(next_chunk_address, &page).await.unwrap();
        }
        cache.disk.file.write_at(offset, &page).await.unwrap();
        let cache = reopen(directory.path(), cache).await;
        assert!(cache.get(&ObjectKeyHash(1)).await.is_some());
        assert!(cache.insert(ObjectKeyHash(2), download(0, 1)).await.unwrap());
        assert_eq!(
            cache.disk.shards[0].metadata_pages.lock().unwrap().chunks[0].next_chunk_address,
            NO_CHUNK
        );
        let cache = reopen(directory.path(), cache).await;
        assert!(cache.get(&ObjectKeyHash(1)).await.is_some());
        assert!(cache.get(&ObjectKeyHash(2)).await.is_some());
    }
}

#[tokio::test]
async fn payload_corruption_remains_a_checksum_miss() {
    let (directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let address = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key].payload_address;
    cache
        .disk
        .file
        .write_at(address, &Bytes::from(vec![99; 4096]))
        .await
        .unwrap();
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&key).await.is_none());
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
    let short_file = DataFile::open(
        directory.path(),
        CHUNK_BYTES + METADATA_PAGE_BYTES as u64,
        IoMetrics::noop(),
    )
    .await
    .unwrap();
    assert!(shard.write_dirty_metadata_pages(&short_file).await.is_err());
    assert!(shard.metadata_pages.lock().unwrap().dirty_pages.is_empty());
    shard.write_dirty_metadata_pages(&short_file).await.unwrap();
    assert_eq!(cache.get(&ObjectKeyHash(0)).await.unwrap(), download(0, 1));
    for i in 0..entries {
        shard.remove_entry(&ObjectKeyHash(i as u128));
    }
    // Removing entry metadata marks its pages for rewriting.
    assert_eq!(shard.metadata_pages.lock().unwrap().dirty_pages.len(), 2);
    assert!(shard.allocator.reserve_chunks(1).is_some());
    // A failed initialization still requires a full-chunk write, including the unchanged link page.
    shard.write_dirty_metadata_pages(&cache.disk.file).await.unwrap();
    let link_page = cache
        .disk
        .file
        .read_at(2 * CHUNK_BYTES - METADATA_PAGE_BYTES as u64, METADATA_PAGE_BYTES)
        .await
        .unwrap();
    assert_eq!(
        validate_page(&link_page, NEXT_CHUNK_PAGE_TAG).map(|bytes| read_u64(bytes, 0)),
        Some(NO_CHUNK)
    );
}

#[tokio::test]
async fn a_write_after_open_preserves_recovered_entries() {
    let (directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    assert!(cache.insert(key, download(10, 10)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert!(!cache.insert(key, download(0, 100)).await.unwrap());
    let cache = reopen(directory.path(), cache).await;
    assert_eq!(cache.get(&key).await.unwrap(), download(10, 10));
    assert_eq!(cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key.len(), 1);
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
    let address = CHUNK_BYTES;
    let mut bytes = cache
        .disk
        .file
        .read_at(address, METADATA_PAGE_BYTES)
        .await
        .unwrap()
        .to_vec();
    bytes[PAGE_HEADER_BYTES + 24..PAGE_HEADER_BYTES + 32].copy_from_slice(&address.to_le_bytes());
    let checksum = XxHash64::oneshot(0, &bytes[8..]);
    bytes[..8].copy_from_slice(&checksum.to_le_bytes());
    cache.disk.file.write_at(address, &Bytes::from(bytes)).await.unwrap();
    let cache = reopen(directory.path(), cache).await;
    assert!(cache.get(&ObjectKeyHash(2)).await.is_none());
    assert!(cache.get(&ObjectKeyHash(1)).await.is_some());
}

#[tokio::test]
async fn recovery_deduplicates_keys_and_restores_allocator_availability() {
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
        // Replace key 2 with key 1 in its metadata, regardless of which payload was written first.
        let position = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&ObjectKeyHash(2)]
            .metadata
            .entry_index as usize;
        let mut page = cache
            .disk
            .file
            .read_at(CHUNK_BYTES, METADATA_PAGE_BYTES)
            .await
            .unwrap()
            .to_vec();
        let offset = PAGE_HEADER_BYTES + position * ENTRY_METADATA_BYTES;
        page[offset..offset + 16].copy_from_slice(&key.0.to_le_bytes());
        let checksum = XxHash64::oneshot(0, &page[8..]);
        page[..8].copy_from_slice(&checksum.to_le_bytes());
        cache.disk.file.write_at(CHUNK_BYTES, &Bytes::from(page)).await.unwrap();
        let capacity = cache.disk.file.capacity();
        drop(cache);
        let (registry, backend) = crate::test_metrics::registry();
        let cache = DiskCache::open(
            directory.path(),
            capacity,
            DiskCacheOptions {
                metrics: DiskMetrics::new(&backend),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let shard = &cache.disk.shards[0];
        {
            let disk_index = shard.entry_index.lock().unwrap();
            assert_eq!(disk_index.entries_by_key.len(), 1);
            assert_eq!(disk_index.eviction_candidates, vec![key]);
            assert_eq!(disk_index.entries_by_key[&key].eviction_position, 0);
        }
        assert_eq!(
            shard.metadata_pages.lock().unwrap().free_slot_count(),
            ENTRIES_PER_METADATA_CHUNK - 1
        );
        assert_eq!(crate::test_metrics::value(&registry, "feuer_disk_entries", &[]), 1.0);
        assert_eq!(
            crate::test_metrics::value(&registry, "feuer_disk_payload_bytes", &[]),
            retained_length as f64
        );
        assert_eq!(cache.get(&key).await.unwrap(), download(0, retained_length));
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
        let location = shard.entry_index.lock().unwrap().entries_by_key[&ObjectKeyHash(3)].metadata;
        let retained_location = shard.entry_index.lock().unwrap().entries_by_key[&key].metadata;
        assert_ne!(location, retained_location);
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
    cache.write_dirty_metadata_pages().await;
    let shard = &cache.disk.shards[0];
    shard.remove_entry(&key);
    // Simulate a crash before the writer clears the removed entry's metadata.
    shard.metadata_pages.lock().unwrap().dirty_pages.clear();
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
    assert!(cache.contains(&key));
    assert!(cache.get(&key).await.is_none());
}
