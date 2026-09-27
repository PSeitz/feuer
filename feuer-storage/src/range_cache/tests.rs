use super::*;
use std::time::Duration;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

fn download(start: u64, length: usize) -> Download {
    Download::new(
        start,
        Bytes::from(
            (start..start + length as u64)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        ),
    )
    .unwrap()
}

async fn open_test_cache(capacity: u64) -> (tempfile::TempDir, DiskRangeCache) {
    let directory = tempfile::tempdir().unwrap();
    let cache = DiskRangeCache::open(directory.path(), capacity, IoMetrics::noop())
        .await
        .unwrap();
    (directory, cache)
}

fn entry_disk_ranges(cache: &DiskRangeCache, key: &str) -> (Vec<std::ops::Range<u64>>, Vec<std::ops::Range<u64>>) {
    let index = cache.disk.arenas[cache.disk.arena_index_for_key(key)]
        .entry_index
        .lock()
        .unwrap();
    let storage = index.ranges_by_key[key].first_key_value().unwrap().1;
    (
        storage.payload_regions.iter().map(DiskRegion::range).collect(),
        storage.entry_metadata_regions.iter().map(DiskRegion::range).collect(),
    )
}

#[tokio::test]
async fn exact_unaligned_reads_containment_and_full_key_identity() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = "complete immutable identity".to_owned();
    assert!(cache.get(&key, range(3, 4)).await.is_none());
    let source = download(3, 20_007);
    assert!(cache.insert(key.clone(), source.clone()).await.unwrap());
    for request in [range(3, 4), range(13, 99), range(4001, 8043), source.downloaded_range()] {
        assert_eq!(
            cache.get(&key, request).await.unwrap(),
            source
                .bytes()
                .slice((request.start() - 3) as usize..(request.end() - 3) as usize)
        );
    }
    assert!(cache.get(&key, range(2, 4)).await.is_none());
    assert!(cache.get(&key, range(20_009, 20_011)).await.is_none());
    assert!(
        cache
            .get(&"different immutable identity".to_owned(), range(3, 4))
            .await
            .is_none()
    );
    assert!(!cache.insert(key.clone(), source).await.unwrap());
    assert!(!cache.insert(key.clone(), download(7, 100)).await.unwrap());
    let index = cache.disk.arenas[0].entry_index.lock().unwrap();
    assert_eq!(index.ranges_by_key[&key].len(), 1);
}

#[tokio::test]
async fn aligned_variable_length_entries_share_a_chunk_without_payload_headers() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let mut previous_end = BLOCK_BYTES;
    for length in [1, 1024, 4016, 4096, 4097, 3 * 4096 + 9] {
        let key = format!("entry-{length}");
        let source = download(17, length);
        assert!(cache.insert(key.clone(), source.clone()).await.unwrap());
        let (payload, entry_metadata) = entry_disk_ranges(&cache, &key);
        assert_eq!(payload.len(), 1);
        let allocated = &payload[0];
        assert!(allocated.start >= previous_end);
        assert!(allocated.start.is_multiple_of(BLOCK_BYTES));
        assert!(allocated.end <= CHUNK_BYTES);
        assert_eq!(
            allocated.end - allocated.start,
            (length as u64).next_multiple_of(BLOCK_BYTES)
        );
        let stored = cache
            .disk
            .file
            .read_at(allocated.start, (allocated.end - allocated.start) as usize)
            .await
            .unwrap();
        assert_eq!(&stored[..length], source.bytes().as_ref());
        assert!(stored[length..].iter().all(|&byte| byte == 0));
        assert_eq!(
            cache.get(&key, source.downloaded_range()).await.unwrap(),
            source.bytes()
        );
        previous_end = entry_metadata.last().unwrap().end;
    }
}

#[tokio::test]
async fn a_small_entry_read_does_not_need_the_rest_of_its_chunk_or_its_entry_metadata() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "small entry".to_owned();
    let source = download(3, 1024);
    cache.insert(key.clone(), source.clone()).await.unwrap();
    cache.insert("neighbor".to_owned(), download(0, 8192)).await.unwrap();
    let (payload, _) = entry_disk_ranges(&cache, &key);
    assert_eq!(payload[0].end - payload[0].start, BLOCK_BYTES);
    // A read reaching beyond this entry's alignment boundary would now fail with short I/O.
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(payload[0].end)
        .unwrap();
    assert_eq!(
        cache.get(&key, source.downloaded_range()).await.unwrap(),
        source.bytes()
    );
}

#[tokio::test]
async fn alignment_padding_is_not_part_of_the_entry_checksum() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "small entry".to_owned();
    let source = download(3, 1024);
    cache.insert(key.clone(), source.clone()).await.unwrap();
    let (payload, _) = entry_disk_ranges(&cache, &key);
    let address = payload[0].start;
    let mut bytes = cache
        .disk
        .file
        .read_at(address, BLOCK_BYTES as usize)
        .await
        .unwrap()
        .to_vec();
    bytes[1024..].fill(0xff);
    cache.disk.file.write_at(address, &Bytes::from(bytes)).await.unwrap();
    assert_eq!(
        cache.get(&key, source.downloaded_range()).await.unwrap(),
        source.bytes()
    );
}

#[tokio::test]
async fn reusing_an_entry_does_not_overwrite_its_neighbors() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let first = "first".to_owned();
    let neighbor = "neighbor".to_owned();
    cache.insert(first.clone(), download(3, 1024)).await.unwrap();
    cache.insert(neighbor.clone(), download(7, 4097)).await.unwrap();
    let old = entry_disk_ranges(&cache, &first).0;
    cache.disk.arenas[0]
        .entry_index
        .lock()
        .unwrap()
        .ranges_by_key
        .remove(&first);
    cache.insert(first.clone(), download(100, 1024)).await.unwrap();
    assert_eq!(entry_disk_ranges(&cache, &first).0, old);
    assert_eq!(
        cache.get(&first, range(100, 1124)).await.unwrap(),
        download(100, 1024).bytes()
    );
    assert_eq!(
        cache.get(&neighbor, range(7, 4104)).await.unwrap(),
        download(7, 4097).bytes()
    );
}

#[tokio::test]
async fn whole_entry_checksum_follows_fragmented_payload_regions() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    for key in ["a", "b", "c", "d"] {
        cache.insert(key.to_owned(), download(0, 1024)).await.unwrap();
    }
    {
        let mut index = cache.disk.arenas[0].entry_index.lock().unwrap();
        index.ranges_by_key.remove("a");
        index.ranges_by_key.remove("c");
    }
    let key = "fragmented".to_owned();
    let source = download(100, 3 * BLOCK_BYTES as usize - 17);
    cache.insert(key.clone(), source.clone()).await.unwrap();
    let (payload, _) = entry_disk_ranges(&cache, &key);
    assert_eq!(payload.len(), 2);
    assert!(payload[0].end < payload[1].start);
    assert_eq!(
        cache.get(&key, source.downloaded_range()).await.unwrap(),
        source.bytes()
    );
    let boundary = 100 + payload[0].end - payload[0].start;
    assert_eq!(
        cache.get(&key, range(boundary - 3, boundary + 7)).await.unwrap(),
        download(boundary - 3, 10).bytes()
    );
    for key in ["b", "d"] {
        assert_eq!(
            cache.get(&key.to_owned(), range(0, 1024)).await.unwrap(),
            download(0, 1024).bytes()
        );
    }
}

#[tokio::test]
async fn partial_overlaps_coexist_without_assembly_and_a_covering_range_replaces_them() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "object".to_owned();
    for (start, length) in [(10, 10), (30, 10), (15, 20)] {
        assert!(cache.insert(key.clone(), download(start, length)).await.unwrap());
    }
    assert!(cache.get(&key, range(10, 40)).await.is_none());
    assert_eq!(cache.get(&key, range(16, 34)).await.unwrap(), download(16, 18).bytes());
    let returned = cache.get(&key, range(11, 19)).await.unwrap();
    assert!(cache.insert(key.clone(), download(5, 50)).await.unwrap());
    assert_eq!(
        cache.disk.arenas[0].entry_index.lock().unwrap().ranges_by_key[&key].len(),
        1
    );
    assert_eq!(returned, download(11, 8).bytes());
    assert_eq!(cache.get(&key, range(10, 40)).await.unwrap(), download(10, 30).bytes());
}

#[test]
fn entry_metadata_starts_preserve_bitmap_encoding() {
    let mut starts = EntryMetadataStarts::default();
    assert_eq!(starts.bitmap, [0; 32]);
    for slot in [1, 7, 8, 63, 64, 255, 1] {
        starts.insert(slot * BLOCK_BYTES);
    }
    let mut expected = [0; 32];
    expected[0] = 0b1000_0010;
    expected[1] = 1;
    expected[7] = 0b1000_0000;
    expected[8] = 1;
    expected[31] = 0b1000_0000;
    assert_eq!(starts.bitmap, expected);
}

#[tokio::test]
async fn writes_full_key_range_and_payload_mappings_in_linked_entry_metadata() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = "long immutable key/".repeat(700);
    let source = download(17, (CHUNK_BYTES + 7) as usize);
    assert!(cache.insert(key.clone(), source.clone()).await.unwrap());
    let (payload, entry_metadata) = entry_disk_ranges(&cache, &key);
    let entry_metadata_start_address = entry_metadata[0].start;
    let chunk_address = entry_metadata_start_address / CHUNK_BYTES * CHUNK_BYTES;
    let index_bytes = cache.disk.file.read_at(chunk_address, PAGE_BYTES).await.unwrap();
    let index_checksum: [u8; 32] = index_bytes[32..64].try_into().unwrap();
    let (next_entry_metadata_page_address, entry_metadata_starts) = page_format::validate_page(
        &index_bytes,
        page_format::ENTRY_METADATA_INDEX_PAGE_TAG,
        &index_checksum,
        chunk_address,
        chunk_address / CHUNK_BYTES,
    )
    .unwrap();
    assert_eq!(next_entry_metadata_page_address, 0);
    let slot = ((entry_metadata_start_address - chunk_address) / BLOCK_BYTES) as usize;
    assert_eq!(blake3::hash(&entry_metadata_starts[..32]).as_bytes(), &index_checksum);
    assert_ne!(entry_metadata_starts[slot / 8] & (1 << (slot % 8)), 0);
    assert_eq!(entry_metadata_starts[0] & 1, 0);
    let head = cache
        .disk
        .file
        .read_at(entry_metadata_start_address, PAGE_BYTES)
        .await
        .unwrap();
    let entry_metadata_checksum: [u8; 32] = head[32..64].try_into().unwrap();
    let expected_addresses: Vec<_> = entry_metadata
        .iter()
        .flat_map(|range| range.clone().step_by(PAGE_BYTES))
        .collect();
    assert!(expected_addresses.len() > 1);
    let mut entry_metadata_bytes = Vec::new();
    let mut address = entry_metadata_start_address;
    for (ordinal, expected) in expected_addresses.iter().enumerate() {
        assert_eq!(address, *expected);
        let page = cache.disk.file.read_at(address, PAGE_BYTES).await.unwrap();
        let (next_entry_metadata_page_address, contents) = page_format::validate_page(
            &page,
            page_format::ENTRY_METADATA_PAGE_TAG,
            &entry_metadata_checksum,
            address,
            ordinal as u64,
        )
        .unwrap();
        entry_metadata_bytes.extend_from_slice(contents);
        address = next_entry_metadata_page_address;
    }
    assert_eq!(address, 0);
    let integer = |offset| u64::from_le_bytes(entry_metadata_bytes[offset..offset + 8].try_into().unwrap());
    assert_eq!(integer(0) as usize, 72 + payload.len() * 16 + key.len());
    assert_eq!(
        blake3::hash(&entry_metadata_bytes[..integer(0) as usize]).as_bytes(),
        &entry_metadata_checksum
    );
    assert_eq!(integer(8) as usize, key.len());
    assert_eq!(integer(16), 17);
    assert_eq!(integer(24), source.downloaded_range().end());
    assert_eq!(integer(32) as usize, payload.len());
    assert_eq!(&entry_metadata_bytes[40..72], blake3::hash(source.bytes()).as_bytes());
    for (index, region) in payload.iter().enumerate() {
        assert_eq!(integer(72 + index * 16), region.start);
        assert_eq!(integer(80 + index * 16), region.end);
    }
    assert_eq!(
        &entry_metadata_bytes[72 + payload.len() * 16..integer(0) as usize],
        key.as_bytes()
    );
}

#[tokio::test]
async fn entry_metadata_space_is_charged_and_failed_reservations_roll_back() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    // All 255 data blocks fit the payload, leaving no room for its entry metadata.
    assert!(
        !cache
            .insert("too large".to_owned(), download(0, BLOCK_BYTES as usize * 255))
            .await
            .unwrap()
    );
    // 254 aligned payload blocks + one entry metadata page fill this arena exactly.
    let key = "fits exactly".to_owned();
    assert!(
        cache
            .insert(key.clone(), download(7, BLOCK_BYTES as usize * 254))
            .await
            .unwrap()
    );
    assert!(cache.disk.arenas[0].allocator.reserve(1).is_none());
    assert!(!cache.insert("full".to_owned(), download(0, 1)).await.unwrap());
    assert_eq!(cache.get(&key, range(8, 12)).await.unwrap(), download(8, 4).bytes());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_equal_and_containing_writes_revalidate_publication() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let index_pages = cache.disk.arenas[0].entry_metadata_index.lock().await;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let cache = cache.clone();
        tasks.push(tokio::spawn(async move {
            cache.insert("object".to_owned(), download(7, 100)).await.unwrap()
        }));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            // Every writer must reserve both payload and entry metadata before we release publication.
            if cache.disk.arenas[0].allocator.available_bytes() == CHUNK_BYTES - (1 + 16 * 2) * BLOCK_BYTES {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        cache.disk.arenas[0]
            .entry_index
            .lock()
            .unwrap()
            .ranges_by_key
            .is_empty()
    );
    drop(index_pages);
    let mut published = 0;
    for task in tasks {
        published += usize::from(task.await.unwrap());
    }
    assert_eq!(published, 1);
    let mut tasks = Vec::new();
    for (start, length) in [(3, 120), (0, 200), (5, 150), (300, 50)] {
        let cache = cache.clone();
        tasks.push(tokio::spawn(async move {
            cache
                .insert("object".to_owned(), download(start, length))
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let key = "object".to_owned();
    let ranges: Vec<_> = cache.disk.arenas[0].entry_index.lock().unwrap().ranges_by_key[&key]
        .values()
        .map(|storage| storage.object_range)
        .collect();
    assert_eq!(ranges, [range(0, 200), range(300, 350)]);
    assert_eq!(cache.get(&key, range(0, 200)).await.unwrap(), download(0, 200).bytes());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canceled_requester_does_not_abort_the_reservation_owner() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    // Block the final metadata write. The actual background writer must keep owning its reservations.
    let index_pages = cache.disk.arenas[0].entry_metadata_index.lock().await;
    let requester = {
        let cache = cache.clone();
        tokio::spawn(async move { cache.insert("object".to_owned(), download(0, 100)).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if cache.disk.arenas[0].allocator.available_bytes() == CHUNK_BYTES - 3 * BLOCK_BYTES {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    requester.abort();
    assert!(requester.await.unwrap_err().is_cancelled());
    assert!(
        cache.disk.arenas[0]
            .allocator
            .reserve(CHUNK_BYTES - BLOCK_BYTES)
            .is_none()
    );
    assert!(cache.get(&"object".to_owned(), range(0, 1)).await.is_none());
    drop(index_pages);
    let bytes = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(bytes) = cache.get(&"object".to_owned(), range(0, 100)).await {
                break bytes;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(bytes, download(0, 100).bytes());
}

#[tokio::test]
async fn readers_keep_replaced_payload_reserved_but_results_do_not() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "object".to_owned();
    cache.insert(key.clone(), download(5, 100)).await.unwrap();
    let (guard, payload_checksum) = {
        let index = cache.disk.arenas[0].entry_index.lock().unwrap();
        let storage = index.covering_range(&key, range(6, 7)).unwrap();
        (storage.payload_regions[0].read_guard(), storage.payload_checksum)
    };
    let result = cache.get(&key, range(6, 7)).await.unwrap();
    cache.insert(key.clone(), download(0, 200)).await.unwrap();
    let bytes = cache.disk.file.read_at(guard.range().start, PAGE_BYTES).await.unwrap();
    assert_eq!(blake3::hash(&bytes[..100]), payload_checksum);
    let reserved = cache.disk.arenas[0].allocator.reserve(BLOCK_BYTES).unwrap();
    assert_ne!(reserved[0].range(), guard.range());
    drop(reserved);
    let old = guard.range();
    drop(guard);
    let reused = cache.disk.arenas[0].allocator.reserve(BLOCK_BYTES).unwrap();
    assert_eq!(reused[0].range(), old);
    assert_eq!(result, download(6, 1).bytes());
}

#[tokio::test]
async fn corrupted_and_reused_payload_miss_and_invalidate_the_entry() {
    for stale in [false, true] {
        let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
        let key = "object".to_owned();
        cache.insert(key.clone(), download(3, 50)).await.unwrap();
        let (payload, _) = entry_disk_ranges(&cache, &key);
        let address = payload[0].start;
        let mut page = cache.disk.file.read_at(address, PAGE_BYTES).await.unwrap().to_vec();
        if stale {
            // Another valid entry's payload must not match this entry metadata's expected checksum.
            let other = "other object".to_owned();
            cache.insert(other.clone(), download(100, 50)).await.unwrap();
            let (other_payload, _) = entry_disk_ranges(&cache, &other);
            page = cache
                .disk
                .file
                .read_at(other_payload[0].start, PAGE_BYTES)
                .await
                .unwrap()
                .to_vec();
        } else {
            page[49] ^= 1;
        }
        cache.disk.file.write_at(address, &Bytes::from(page)).await.unwrap();
        assert!(cache.get(&key, range(3, 4)).await.is_none());
        assert!(
            !cache.disk.arenas[0]
                .entry_index
                .lock()
                .unwrap()
                .ranges_by_key
                .contains_key(&key)
        );
        assert!(cache.insert(key.clone(), download(3, 50)).await.unwrap());
        assert_eq!(cache.get(&key, range(3, 4)).await.unwrap(), download(3, 1).bytes());
    }
}

#[tokio::test]
async fn a_short_read_is_a_miss_not_unverified_bytes() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "object".to_owned();
    cache.insert(key.clone(), download(0, 100)).await.unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(BLOCK_BYTES + 100)
        .unwrap();
    assert!(cache.get(&key, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn failed_payload_or_entry_metadata_writes_quarantine_storage_without_publication() {
    for fail_entry_metadata in [false, true] {
        let capacity = if fail_entry_metadata {
            CHUNK_BYTES
        } else {
            2 * CHUNK_BYTES
        };
        let (_directory, mut cache) = open_test_cache(capacity).await;
        // Inject an allocator/file bounds mismatch. In both cases real writes complete before a
        // later write fails: a second payload region, or the entry metadata after a complete payload.
        let start = if fail_entry_metadata { 0 } else { CHUNK_BYTES };
        Arc::get_mut(&mut cache.disk).unwrap().arenas[0].allocator =
            DiskAllocator::for_disk_range(start..start + 2 * CHUNK_BYTES).unwrap();
        let length = BLOCK_BYTES as usize * if fail_entry_metadata { 255 } else { 256 };
        assert!(matches!(
            cache.insert("object".to_owned(), download(0, length)).await,
            Err(DiskRangeCacheError::DataFile(DataFileError::OutOfBounds { .. }))
        ));
        assert!(cache.get(&"object".to_owned(), range(0, 1)).await.is_none());
        assert!(
            cache.disk.arenas[0]
                .allocator
                .reserve(2 * (CHUNK_BYTES - BLOCK_BYTES))
                .is_none()
        );
        let first_page = cache.disk.file.read_at(start + BLOCK_BYTES, PAGE_BYTES).await.unwrap();
        assert_eq!(first_page, download(0, BLOCK_BYTES as usize).bytes());
    }
}

#[tokio::test]
async fn disabled_entry_metadata_index_writes_prevent_publication() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    cache.disk.arenas[0].entry_metadata_index.lock().await.writes_enabled = false;
    assert!(matches!(
        cache.insert("object".to_owned(), download(0, 100)).await,
        Err(DiskRangeCacheError::EntryMetadataIndexWritesDisabled)
    ));
    assert!(cache.get(&"object".to_owned(), range(0, 1)).await.is_none());
    assert!(
        cache.disk.arenas[0]
            .allocator
            .reserve(CHUNK_BYTES - BLOCK_BYTES)
            .is_none()
    );
}

#[tokio::test]
async fn serves_100_mib_entry_subranges_only_after_checking_the_whole_entry() {
    let (_directory, cache) = open_test_cache(104 * CHUNK_BYTES).await;
    let key = "large object".to_owned();
    let source = download(3, 100 * CHUNK_BYTES as usize + 17);
    assert!(cache.insert(key.clone(), source.clone()).await.unwrap());
    let boundary = 3 + CHUNK_BYTES - BLOCK_BYTES;
    for request in [
        range(7, 33),
        range(boundary - 3, boundary + 13),
        range(source.downloaded_range().end() - 17, source.downloaded_range().end()),
    ] {
        assert_eq!(
            cache.get(&key, request).await.unwrap(),
            source
                .bytes()
                .slice((request.start() - 3) as usize..(request.end() - 3) as usize)
        );
    }
    let (payload, _) = entry_disk_ranges(&cache, &key);
    let last_page = payload.last().unwrap().end - BLOCK_BYTES;
    cache
        .disk
        .file
        .write_at(last_page, &Bytes::from(vec![0; PAGE_BYTES]))
        .await
        .unwrap();
    // Even a request at the beginning must detect corruption at the end of this entry.
    assert!(cache.get(&key, range(3, 13)).await.is_none());
}

#[tokio::test]
async fn arenas_are_disjoint_and_reopening_intentionally_starts_empty() {
    let (directory, cache) = open_test_cache(256 * CHUNK_BYTES).await;
    assert_eq!(cache.disk.arenas.len(), 2);
    let keys: Vec<_> = (0..2)
        .map(|arena_index| {
            (0..100)
                .map(|i| format!("object-{i}"))
                .find(|key| cache.disk.arena_index_for_key(key) == arena_index)
                .unwrap()
        })
        .collect();
    for key in &keys {
        assert!(cache.insert(key.clone(), download(0, 100)).await.unwrap());
    }
    let first = entry_disk_ranges(&cache, &keys[0]).0;
    let second = entry_disk_ranges(&cache, &keys[1]).0;
    assert!(first.last().unwrap().end <= 128 * CHUNK_BYTES);
    assert!(second[0].start >= 128 * CHUNK_BYTES);
    let returned = cache.get(&keys[0], range(0, 100)).await.unwrap();
    drop(cache);
    let reopened = DiskRangeCache::open(directory.path(), 256 * CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    for key in &keys {
        assert!(reopened.get(key, range(0, 1)).await.is_none());
    }
    assert!(reopened.insert(keys[0].clone(), download(0, 100)).await.unwrap());
    assert_eq!(returned, download(0, 100).bytes());
}

#[test]
fn invalidation_preserves_different_contents_but_may_discard_an_identical_replacement() {
    let read = GuardedObjectRangeRead {
        object_range: range(0, 3),
        payload_checksum: blake3::hash(b"old"),
        payload_regions: Vec::new(),
    };
    for replacement in [b"old", b"new"] {
        let mut index = DiskEntryIndex {
            ranges_by_key: HashMap::from([(
                "object".to_owned(),
                BTreeMap::from([(
                    0,
                    ObjectRangeDiskStorage {
                        object_range: range(0, 3),
                        payload_checksum: blake3::hash(replacement),
                        payload_regions: Vec::new(),
                        entry_metadata_regions: Vec::new(),
                    },
                )]),
            )]),
        };
        index.invalidate("object", &read);
        assert_eq!(
            index.covering_range("object", range(0, 3)).is_some(),
            replacement != b"old"
        );
    }
}

#[test]
fn metadata_page_checks_bind_content_checksum_address_ordinal_tag_and_entire_contents() {
    let mut page = vec![0; PAGE_BYTES];
    page_format::encode_page(
        &mut page,
        page_format::ENTRY_METADATA_PAGE_TAG,
        &[7; 32],
        BLOCK_BYTES,
        2,
        8192,
        b"entry metadata contents",
    );
    let validate = |bytes: &[u8]| {
        page_format::validate_page(bytes, page_format::ENTRY_METADATA_PAGE_TAG, &[7; 32], BLOCK_BYTES, 2).is_some()
    };
    assert!(validate(&page));
    for offset in [0, 32, 64, 72, 80, 88, 96, PAGE_BYTES - 1] {
        let mut torn = page.clone();
        torn[offset] ^= 1;
        assert!(!validate(&torn));
    }
    assert!(!validate(&page[..PAGE_BYTES - 1]));
    assert!(
        page_format::validate_page(
            &page,
            page_format::ENTRY_METADATA_INDEX_PAGE_TAG,
            &[7; 32],
            BLOCK_BYTES,
            2
        )
        .is_none()
    );
    assert!(
        page_format::validate_page(&page, page_format::ENTRY_METADATA_PAGE_TAG, &[8; 32], BLOCK_BYTES, 2).is_none()
    );
    assert!(page_format::validate_page(&page, page_format::ENTRY_METADATA_PAGE_TAG, &[7; 32], 8192, 2).is_none());
    assert!(
        page_format::validate_page(&page, page_format::ENTRY_METADATA_PAGE_TAG, &[7; 32], BLOCK_BYTES, 3).is_none()
    );

    // A valid page of different entry metadata at the same address must not join this chain.
    page_format::encode_page(
        &mut page,
        page_format::ENTRY_METADATA_PAGE_TAG,
        &[8; 32],
        BLOCK_BYTES,
        2,
        8192,
        b"different entry metadata contents",
    );
    assert!(!validate(&page));
}
