use super::*;
use std::{future::Future, task::Poll, time::Duration};

// Keep single-entry scenarios concise while exercising the explicit batch API.
impl DiskRangeCache {
    async fn insert(&self, key: ObjectKey, download: Download) -> Result<bool, DiskRangeCacheError> {
        self.insert_batch(vec![(key, download)]).await.map(|count| count == 1)
    }
}

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
    // Deliberately unordered input: small entries should be packed ahead of larger ones.
    let lengths = [4097, 1024, 3 * 4096 + 9, 1, 4096, 4016];
    assert_eq!(
        cache
            .insert_batch(
                lengths
                    .into_iter()
                    .map(|length| (format!("entry-{length}"), download(17, length)))
                    .collect()
            )
            .await
            .unwrap(),
        lengths.len()
    );
    let mut previous_end = METADATA_PAGE_BYTES as u64;
    for length in [1, 1024, 4016, 4096, 4097, 3 * 4096 + 9] {
        let key = format!("entry-{length}");
        let source = download(17, length);
        let (payload, entry_metadata) = entry_disk_ranges(&cache, &key);
        assert_eq!(payload.len(), 1);
        let allocated = &payload[0];
        assert!(allocated.start >= previous_end);
        assert!(allocated.start.is_multiple_of(PAYLOAD_ALIGNMENT_BYTES));
        assert!(allocated.end <= CHUNK_BYTES);
        assert_eq!(
            allocated.end - allocated.start,
            (length as u64).next_multiple_of(PAYLOAD_ALIGNMENT_BYTES)
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
async fn mixed_batch_groups_small_entries_and_records_every_metadata_start() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    assert_eq!(cache.insert_batch(Vec::new()).await.unwrap(), 0);
    let mut inputs = vec![("large".to_owned(), download(0, CHUNK_BYTES as usize))];
    inputs.extend((0..130).map(|i| (format!("small-{i}"), download(i, 1024))));
    assert_eq!(cache.insert_batch(inputs).await.unwrap(), 131);
    let large_start = entry_disk_ranges(&cache, "large").0[0].start;
    let mut expected = BTreeMap::<u64, EntryMetadataStarts>::new();
    for i in 0..130 {
        let key = format!("small-{i}");
        assert_eq!(
            cache.get(&key, range(i, i + 1024)).await.unwrap(),
            download(i, 1024).bytes()
        );
        assert!(entry_disk_ranges(&cache, &key).0[0].start < large_start);
    }
    {
        let index = cache.disk.arenas[0].entry_index.lock().unwrap();
        for entries in index.ranges_by_key.values() {
            for entry in entries.values() {
                let address = entry.entry_metadata_regions[0].range().start;
                expected
                    .entry(address / CHUNK_BYTES * CHUNK_BYTES)
                    .or_default()
                    .insert(address % CHUNK_BYTES);
            }
        }
    }
    for address in (0..4 * CHUNK_BYTES).step_by(CHUNK_BYTES as usize) {
        let page = cache.disk.file.read_at(address, METADATA_PAGE_BYTES).await.unwrap();
        let starts = expected.remove(&address).unwrap_or_default();
        let (_, contents) = page_format::validate_page(
            &page,
            page_format::CHUNK_METADATA_PAGE_TAG,
            blake3::hash(&starts.bitmap).as_bytes(),
            address,
            address / CHUNK_BYTES,
        )
        .unwrap();
        assert_eq!(&contents[..32], &starts.bitmap);
    }
    assert!(expected.is_empty());
    assert_eq!(cache.disk.arenas[0].allocator.available_bytes(), 0);
    assert_eq!(
        cache.get(&"large".to_owned(), range(0, CHUNK_BYTES)).await.unwrap(),
        download(0, CHUNK_BYTES as usize).bytes()
    );
}

#[test]
fn shared_chunks_contain_complete_entries_and_multi_chunk_entries_have_no_neighbors() {
    let allocator = DiskAllocator::for_disk_range(0..8 * CHUNK_BYTES).unwrap();
    let mut batch = UnwrittenBatch::default();
    let histories = ObjectAccessHistories::new(1);
    // Exercise boundaries in input order, including small entries after exclusive tails.
    for (key, length) in [
        ("first".to_owned(), 600_000),
        ("does not fit the tail".to_owned(), 600_000),
        ("large".to_owned(), CHUNK_BYTES as usize),
        ("small after large".to_owned(), 1),
        ("k".repeat(CHUNK_BYTES as usize), 1),
        ("small after metadata-only chunk".to_owned(), 1),
    ] {
        batch
            .push(&allocator, &key, &download(0, length), &histories.for_key(&key))
            .unwrap();
    }
    let mut owners = BTreeMap::<u64, Vec<usize>>::new();
    for (entry_number, (_, entry)) in batch.entries.iter().enumerate() {
        let mut chunks: Vec<_> = entry
            .payload_regions
            .iter()
            .chain(&entry.entry_metadata_regions)
            .map(|region| region.range().start / CHUNK_BYTES)
            .collect();
        chunks.sort_unstable();
        chunks.dedup();
        assert_eq!(entry.shared_chunk().is_some(), chunks.len() == 1);
        for chunk in chunks {
            owners.entry(chunk).or_default().push(entry_number);
        }
    }
    for entries in owners.values() {
        if entries.len() > 1 {
            assert!(entries.iter().all(|&i| batch.entries[i].1.shared_chunk().is_some()));
        }
    }
    assert_eq!(batch.chunks.len(), 8);
}

#[test]
fn metadata_must_fit_before_an_entry_can_share_a_chunk() {
    let allocator = DiskAllocator::for_disk_range(0..2 * CHUNK_BYTES).unwrap();
    let mut batch = UnwrittenBatch::default();
    let histories = ObjectAccessHistories::new(1);
    batch
        .push(
            &allocator,
            &"first".to_owned(),
            &download(0, CHUNK_BYTES as usize - 3 * METADATA_PAGE_BYTES),
            &histories.for_key(&"first".to_owned()),
        )
        .unwrap();
    // The remaining page fits payload, but not its metadata. Both must go in a new chunk.
    batch
        .push(
            &allocator,
            &"second".to_owned(),
            &download(0, 1),
            &histories.for_key(&"second".to_owned()),
        )
        .unwrap();
    assert_eq!(batch.entries[0].1.shared_chunk(), Some(0));
    assert_eq!(batch.entries[1].1.shared_chunk(), Some(CHUNK_BYTES));
    assert_eq!(batch.chunks[0].used_bytes, CHUNK_BYTES - METADATA_PAGE_BYTES as u64);
}

#[tokio::test]
async fn pressure_reclaims_shared_and_exclusive_chunks_during_mixed_size_churn() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    for cycle in 0..20 {
        let inputs: Vec<_> = [1, 4097, CHUNK_BYTES as usize + 17]
            .into_iter()
            .enumerate()
            .map(|(i, length)| (format!("{cycle}-{i}"), download(cycle * 7, length)))
            .collect();
        assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), inputs.len());
        for (key, source) in inputs {
            assert_eq!(
                cache.get(&key, source.downloaded_range()).await.unwrap(),
                source.bytes()
            );
        }
        let index = cache.disk.arenas[0].entry_index.lock().unwrap();
        assert_eq!(
            index.eviction_candidates.len(),
            index.ranges_by_key.values().map(BTreeMap::len).sum::<usize>()
        );
        for (position, (key, start)) in index.eviction_candidates.iter().enumerate() {
            assert_eq!(index.ranges_by_key[key][start].eviction_position, position);
        }
    }
}

#[tokio::test]
async fn eviction_preserves_a_newer_replacement() {
    let (_directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let key = "replaced".to_owned();
    cache
        .insert_batch(vec![
            (key.clone(), download(5, 200)),
            ("neighbor".to_owned(), download(0, 100)),
        ])
        .await
        .unwrap();
    cache.insert(key.clone(), download(0, 300)).await.unwrap();
    // One free chunk is insufficient; evict the neighbor in the old shared chunk.
    assert!(
        cache
            .insert("large".to_owned(), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
    assert!(cache.get(&"neighbor".to_owned(), range(0, 1)).await.is_none());
    assert_eq!(cache.get(&key, range(0, 300)).await.unwrap(), download(0, 300).bytes());
}

#[tokio::test]
async fn multi_chunk_eviction_preserves_an_in_progress_read_until_its_guards_drop() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = "large".to_owned();
    let source = download(3, CHUNK_BYTES as usize + 17);
    cache.insert(key.clone(), source.clone()).await.unwrap();
    let read = {
        let index = cache.disk.arenas[0].entry_index.lock().unwrap();
        let entry = index.covering_range(&key, source.downloaded_range()).unwrap();
        GuardedObjectRangeRead {
            object_range: entry.object_range,
            payload_checksum: entry.payload_checksum,
            payload_regions: entry.payload_regions.iter().map(DiskRegion::read_guard).collect(),
        }
    };
    assert!(
        !cache
            .insert("new".to_owned(), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
    assert!(cache.get(&key, range(3, 4)).await.is_none());
    assert_eq!(
        read.read(&cache.disk.file, range(3, 20)).await.unwrap().unwrap(),
        source.bytes().slice(..17)
    );
    drop(read);
    assert!(
        cache
            .insert("new".to_owned(), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn eviction_budgets_and_active_reservations_bound_reclamation() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = "large".to_owned();
    cache
        .insert(key.clone(), download(0, CHUNK_BYTES as usize))
        .await
        .unwrap();
    let arena = &cache.disk.arenas[0];
    let mut candidates = 0;
    let mut regions = MAX_EVICTION_REGIONS;
    assert!(!arena.evict_candidate(&mut candidates, &mut regions));
    candidates = 1;
    regions = 1;
    assert!(arena.evict_candidate(&mut candidates, &mut regions));
    assert_eq!(candidates, 0);
    assert!(cache.get(&key, range(0, 1)).await.is_some());
    candidates = 1;
    regions = MAX_EVICTION_REGIONS;
    assert!(arena.evict_candidate(&mut candidates, &mut regions));
    let active = arena.allocator.reserve_chunks(2).unwrap();
    assert!(!cache.insert("blocked".to_owned(), download(0, 1)).await.unwrap());
    drop(active);
    assert!(cache.insert("unblocked".to_owned(), download(0, 1)).await.unwrap());
}

#[tokio::test]
async fn value_aware_eviction_preserves_hot_neighbors_and_needs_no_metadata_reads() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let hot = "hot".to_owned();
    let cold = "cold".to_owned();
    let other = "other".to_owned();
    cache
        .insert_batch(vec![(hot.clone(), download(0, 100)), (cold.clone(), download(0, 100))])
        .await
        .unwrap();
    cache.insert(other.clone(), download(0, 100)).await.unwrap();
    let histories = cache.access_histories();
    for _ in 0..3 {
        histories.record_access(&hot, range(0, 1));
    }
    histories.record_access(&cold, range(0, 1));
    histories.record_access(&other, range(0, 1));
    // Corrupt discovery metadata: entry-wise eviction must not read it at all.
    cache
        .disk
        .file
        .write_at(0, &Bytes::from(vec![0; METADATA_PAGE_BYTES]))
        .await
        .unwrap();
    let arena = &cache.disk.arenas[0];
    let mut attempts = 1;
    let mut regions = MAX_EVICTION_REGIONS;
    assert!(arena.evict_candidate(&mut attempts, &mut regions));
    assert!(cache.get(&cold, range(0, 1)).await.is_none());
    assert!(cache.get(&hot, range(0, 1)).await.is_some());
    assert_eq!(arena.allocator.available_bytes(), 0); // Hot neighbor still owns the chunk.
    assert!(cache.insert("new".to_owned(), download(0, 100)).await.unwrap());
    assert!(cache.get(&other, range(0, 1)).await.is_none());
    assert!(cache.get(&hot, range(0, 1)).await.is_some());
}

#[tokio::test]
async fn disk_value_uses_payload_size_not_aligned_allocations_or_chunk_size() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let small = "small".to_owned();
    let large = "large".to_owned();
    // Both entries use identical physical capacity, but different payload lengths.
    cache.insert(small.clone(), download(0, 1)).await.unwrap();
    cache.insert(large.clone(), download(0, 3000)).await.unwrap();
    cache.access_histories().record_access(&small, range(0, 1));
    cache.access_histories().record_access(&large, range(0, 1));
    assert!(cache.insert("new".to_owned(), download(0, 1)).await.unwrap());
    assert!(cache.get(&small, range(0, 1)).await.is_some());
    assert!(cache.get(&large, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn payload_value_ignores_metadata_size_and_breaks_ties_by_publication() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let older = "small metadata".to_owned();
    let newer = "long key/".repeat(1000);
    cache.insert(older.clone(), download(0, 100)).await.unwrap();
    cache.insert(newer.clone(), download(0, 100)).await.unwrap();
    cache.access_histories().record_access(&older, range(0, 1));
    cache.access_histories().record_access(&newer, range(0, 1));
    assert!(cache.insert("incoming".to_owned(), download(0, 1)).await.unwrap());
    assert!(cache.get(&older, range(0, 1)).await.is_none());
    assert!(cache.get(&newer, range(0, 1)).await.is_some());
}

#[tokio::test]
async fn pressure_replacement_preserves_evidence_while_the_last_old_entry_is_removed() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "object".to_owned();
    cache.insert(key.clone(), download(5, 10)).await.unwrap();
    cache.access_histories().record_access(&key, range(6, 7));
    let history = Arc::downgrade(&cache.access_histories().for_key(&key));
    assert!(cache.insert(key.clone(), download(0, 20)).await.unwrap());
    assert_eq!(history.upgrade().unwrap().lock().generation(), 1);
    assert!(cache.get(&key, range(0, 20)).await.is_some());
}

#[tokio::test]
async fn disk_access_evidence_ages_and_credits_only_covering_ranges() {
    use feuer_types::retention::MAX_ACCESS_AGE_ACCESSES;
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = "object".to_owned();
    cache.insert(key.clone(), download(0, 100)).await.unwrap();
    cache.insert(key.clone(), download(200, 100)).await.unwrap();
    let histories = cache.access_histories();
    let evidence = histories.for_key(&key);
    for _ in 0..3 {
        histories.record_access(&key, range(0, 1));
    }
    assert!(evidence.covered_retrieval_cost(range(0, 100)) > 0);
    assert_eq!(evidence.covered_retrieval_cost(range(200, 300)), 0);
    // Successful lookups are recorded explicitly; raw storage reads/population do not double-count.
    assert!(cache.get(&key, range(0, 1)).await.is_some());
    assert_eq!(evidence.lock().generation(), 3);
    for _ in 0..=MAX_ACCESS_AGE_ACCESSES {
        histories.record_access(&key, range(200, 201));
    }
    assert_eq!(evidence.covered_retrieval_cost(range(0, 100)), 0);
    assert!(cache.insert("new".to_owned(), download(0, 1)).await.unwrap());
    assert!(cache.get(&key, range(0, 1)).await.is_none());
    assert!(cache.get(&key, range(200, 201)).await.is_some());
}

#[tokio::test]
async fn memory_and_disk_use_the_same_evidence_through_memory_eviction() {
    let memory = feuer_memory::MemoryCache::new(4096);
    let directory = tempfile::tempdir().unwrap();
    let cache = DiskRangeCache::open_with_access_histories(
        directory.path(),
        2 * CHUNK_BYTES,
        IoMetrics::noop(),
        memory.access_histories(),
    )
    .await
    .unwrap();
    let hot = "hot".to_owned();
    let cold = "cold".to_owned();
    memory.insert_and_record(hot.clone(), download(0, 100), range(0, 1));
    cache.insert(hot.clone(), download(0, 100)).await.unwrap();
    cache.insert(cold.clone(), download(0, 100)).await.unwrap();
    memory.record_access(&cold, range(0, 1)); // A disk-only key.
    for _ in 0..3 {
        assert!(memory.get(&hot, range(0, 1)).is_some());
    }
    let history = cache.access_histories().for_key(&hot);
    assert_eq!(history.lock().generation(), 4);
    assert!(cache.insert("new".to_owned(), download(0, 100)).await.unwrap());
    assert!(cache.get(&cold, range(0, 1)).await.is_none());
    assert!(memory.remove(&hot, range(0, 100)));
    assert!(memory.get(&hot, range(0, 1)).is_none());
    assert!(cache.get(&hot, range(50, 51)).await.is_some());
    assert_eq!(history.lock().generation(), 4); // Raw storage reads do not record a second event.
    memory.record_access(&hot, range(50, 51));
    assert_eq!(history.lock().generation(), 5);
    let released = Arc::downgrade(&history);
    drop(history);
    cache.disk.arenas[0].entry_index.lock().unwrap().remove(&hot, 0);
    assert!(released.upgrade().is_none());
    assert_eq!(cache.access_histories().for_key(&hot).lock().generation(), 0);
}

#[tokio::test]
async fn per_batch_eviction_limit_does_not_force_out_a_shared_chunks_last_entry() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let hot = "hot".to_owned();
    let mut inputs: Vec<_> = (0..MAX_EVICTION_ATTEMPTS)
        .map(|i| (format!("cold-{i}"), download(0, 1)))
        .collect();
    inputs.push((hot.clone(), download(0, 1)));
    cache.insert_batch(inputs).await.unwrap();
    cache.access_histories().record_access(&hot, range(0, 1));
    // Removing 64 individually chosen cold entries still cannot release the chunk with its hot neighbor.
    assert!(!cache.insert("incoming".to_owned(), download(0, 1)).await.unwrap());
    assert!(cache.get(&hot, range(0, 1)).await.is_some());
    assert_eq!(
        cache.disk.arenas[0]
            .entry_index
            .lock()
            .unwrap()
            .eviction_candidates
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_pressure_and_reads_never_return_reused_bytes() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let mut tasks = Vec::new();
    for worker in 0..4 {
        let cache = cache.clone();
        tasks.push(tokio::spawn(async move {
            for cycle in 0..20 {
                let key = format!("{worker}-{cycle}");
                let source = download(worker * 53 + cycle, 8193);
                cache.insert(key.clone(), source.clone()).await.unwrap();
                if let Some(bytes) = cache.get(&key, source.downloaded_range()).await {
                    assert_eq!(bytes, source.bytes());
                }
            }
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::test]
async fn a_small_entry_read_does_not_need_the_rest_of_its_chunk_or_its_entry_metadata() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = "small entry".to_owned();
    let source = download(3, 1024);
    assert_eq!(
        cache
            .insert_batch(vec![
                (key.clone(), source.clone()),
                ("neighbor".to_owned(), download(0, 8192))
            ])
            .await
            .unwrap(),
        2
    );
    let (payload, _) = entry_disk_ranges(&cache, &key);
    assert_eq!(payload[0].end - payload[0].start, PAYLOAD_ALIGNMENT_BYTES);
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
        .read_at(address, PAYLOAD_ALIGNMENT_BYTES as usize)
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
async fn a_written_chunk_is_immutable_until_all_entries_and_readers_release_it() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let first = "first".to_owned();
    let neighbor = "neighbor".to_owned();
    assert_eq!(
        cache
            .insert_batch(vec![
                (first.clone(), download(3, 1024)),
                (neighbor.clone(), download(7, 4097))
            ])
            .await
            .unwrap(),
        2
    );
    let original = cache.disk.file.read_at(0, CHUNK_BYTES as usize).await.unwrap();
    let reader = {
        let mut index = cache.disk.arenas[0].entry_index.lock().unwrap();
        let entry = index.remove(&first, 3).unwrap();
        entry.payload_regions[0].read_guard()
    };
    // Neither the removed entry's bytes nor the unwritten tail can accept a later batch.
    assert!(!cache.insert(first.clone(), download(100, 1024)).await.unwrap());
    assert_eq!(
        cache.disk.file.read_at(0, CHUNK_BYTES as usize).await.unwrap(),
        original
    );
    // Pressure removed the neighbor from lookup, but the reader still prevents reuse.
    assert!(cache.get(&neighbor, range(7, 4104)).await.is_none());
    assert!(!cache.insert(first.clone(), download(100, 1024)).await.unwrap());
    drop(reader);
    assert!(cache.insert(first.clone(), download(100, 1024)).await.unwrap());
    assert_eq!(
        cache.get(&first, range(100, 1124)).await.unwrap(),
        download(100, 1024).bytes()
    );
}

#[tokio::test]
async fn whole_entry_checksum_follows_fragmented_chunks() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    for key in ["a", "b", "c", "d"] {
        assert!(cache.insert(key.to_owned(), download(0, 1024)).await.unwrap());
    }
    {
        let mut index = cache.disk.arenas[0].entry_index.lock().unwrap();
        index.remove("a", 0);
        index.remove("c", 0);
    }
    let key = "fragmented".to_owned();
    let source = download(100, CHUNK_BYTES as usize + 17);
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
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
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
        starts.insert(slot * METADATA_PAGE_BYTES as u64);
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
    let chunk_metadata_page = cache
        .disk
        .file
        .read_at(chunk_address, METADATA_PAGE_BYTES)
        .await
        .unwrap();
    let bitmap_checksum: [u8; 32] = chunk_metadata_page[32..64].try_into().unwrap();
    let (next_entry_metadata_page_address, entry_metadata_starts) = page_format::validate_page(
        &chunk_metadata_page,
        page_format::CHUNK_METADATA_PAGE_TAG,
        &bitmap_checksum,
        chunk_address,
        chunk_address / CHUNK_BYTES,
    )
    .unwrap();
    assert_eq!(next_entry_metadata_page_address, 0);
    let slot = ((entry_metadata_start_address - chunk_address) / METADATA_PAGE_BYTES as u64) as usize;
    assert_eq!(blake3::hash(&entry_metadata_starts[..32]).as_bytes(), &bitmap_checksum);
    assert_ne!(entry_metadata_starts[slot / 8] & (1 << (slot % 8)), 0);
    assert_eq!(entry_metadata_starts[0] & 1, 0);
    let head = cache
        .disk
        .file
        .read_at(entry_metadata_start_address, METADATA_PAGE_BYTES)
        .await
        .unwrap();
    let entry_metadata_checksum: [u8; 32] = head[32..64].try_into().unwrap();
    let expected_addresses: Vec<_> = entry_metadata
        .iter()
        .flat_map(|range| range.clone().step_by(METADATA_PAGE_BYTES))
        .collect();
    assert!(expected_addresses.len() > 1);
    let mut entry_metadata_bytes = Vec::new();
    let mut address = entry_metadata_start_address;
    for (ordinal, expected) in expected_addresses.iter().enumerate() {
        assert_eq!(address, *expected);
        let page = cache.disk.file.read_at(address, METADATA_PAGE_BYTES).await.unwrap();
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
async fn batch_skips_oversized_and_contained_entries_without_losing_accepted_entries() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    assert_eq!(
        cache
            .insert_batch(vec![
                ("object".to_owned(), download(5, 100)),
                ("too large".to_owned(), download(0, CHUNK_BYTES as usize)),
                ("object".to_owned(), download(0, 200)),
                ("object".to_owned(), download(0, 200)),
            ])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        cache.get(&"object".to_owned(), range(0, 200)).await.unwrap(),
        download(0, 200).bytes()
    );
    assert!(cache.get(&"too large".to_owned(), range(0, 1)).await.is_none());
    assert_eq!(
        cache.disk.arenas[0].entry_index.lock().unwrap().ranges_by_key["object"].len(),
        1
    );
}

#[tokio::test]
async fn metadata_only_chunks_remain_owned_until_the_entry_is_removed() {
    let (_directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let key = "k".repeat(CHUNK_BYTES as usize);
    assert!(cache.insert(key.clone(), download(0, 1)).await.unwrap());
    let (payload, metadata) = entry_disk_ranges(&cache, &key);
    assert_eq!(payload.len(), 1);
    assert_eq!(metadata.len(), 2);
    assert_eq!(cache.disk.arenas[0].allocator.available_bytes(), CHUNK_BYTES);
    assert!(cache.insert("other".to_owned(), download(0, 1)).await.unwrap());
    assert_eq!(cache.disk.arenas[0].allocator.available_bytes(), 0);
    assert_eq!(cache.get(&key, range(0, 1)).await.unwrap(), download(0, 1).bytes());
    cache.disk.arenas[0].entry_index.lock().unwrap().remove(&key, 0);
    assert_eq!(cache.disk.arenas[0].allocator.available_bytes(), 2 * CHUNK_BYTES);
}

#[tokio::test]
async fn entry_metadata_space_is_charged_and_failed_reservations_roll_back() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    // Fill all space after the discovery page, leaving no room for entry metadata.
    assert!(
        !cache
            .insert(
                "too large".to_owned(),
                download(0, CHUNK_BYTES as usize - METADATA_PAGE_BYTES)
            )
            .await
            .unwrap()
    );
    // Leave exactly one page for entry metadata after the payload.
    let key = "fits exactly".to_owned();
    assert!(
        cache
            .insert(key.clone(), download(7, CHUNK_BYTES as usize - 2 * METADATA_PAGE_BYTES))
            .await
            .unwrap()
    );
    assert!(cache.disk.arenas[0].allocator.reserve_chunks(1).is_none());
    // A two-chunk entry cannot fit this arena; do not evict the existing entry in vain.
    assert!(
        !cache
            .insert("oversized".to_owned(), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
    assert!(cache.get(&key, range(8, 12)).await.is_some());
    assert!(cache.insert("full".to_owned(), download(0, 1)).await.unwrap());
    assert!(cache.get(&key, range(8, 12)).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_equal_and_containing_writes_revalidate_publication() {
    let (_directory, cache) = open_test_cache(16 * CHUNK_BYTES).await;
    let start = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let cache = cache.clone();
        let start = start.clone();
        tasks.push(tokio::spawn(async move {
            start.wait().await;
            cache.insert("object".to_owned(), download(7, 100)).await.unwrap()
        }));
    }
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

#[tokio::test]
async fn canceled_requester_does_not_abort_the_reservation_owner() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let mut requester = Box::pin(cache.insert("object".to_owned(), download(0, 100)));
    // On the current-thread runtime, one poll spawns the owner without letting it run yet.
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(requester.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(requester);
    assert!(cache.get(&"object".to_owned(), range(0, 1)).await.is_none());
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
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = "object".to_owned();
    cache.insert(key.clone(), download(5, 100)).await.unwrap();
    let (guard, payload_checksum) = {
        let index = cache.disk.arenas[0].entry_index.lock().unwrap();
        let storage = index.covering_range(&key, range(6, 7)).unwrap();
        (storage.payload_regions[0].read_guard(), storage.payload_checksum)
    };
    let result = cache.get(&key, range(6, 7)).await.unwrap();
    cache.insert(key.clone(), download(0, 200)).await.unwrap();
    let bytes = cache
        .disk
        .file
        .read_at(guard.range().start, PAYLOAD_ALIGNMENT_BYTES as usize)
        .await
        .unwrap();
    assert_eq!(blake3::hash(&bytes[..100]), payload_checksum);
    assert!(cache.disk.arenas[0].allocator.reserve_chunks(1).is_none());
    drop(guard);
    let reused = cache.disk.arenas[0].allocator.reserve_chunks(1).unwrap();
    assert_eq!(reused[0].range(), 0..CHUNK_BYTES);
    assert_eq!(result, download(6, 1).bytes());
}

#[tokio::test]
async fn corrupted_and_reused_payload_miss_and_invalidate_the_entry() {
    for stale in [false, true] {
        let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
        let key = "object".to_owned();
        cache.insert(key.clone(), download(3, 50)).await.unwrap();
        let (payload, _) = entry_disk_ranges(&cache, &key);
        let address = payload[0].start;
        let mut bytes = cache
            .disk
            .file
            .read_at(address, PAYLOAD_ALIGNMENT_BYTES as usize)
            .await
            .unwrap()
            .to_vec();
        if stale {
            // Another valid entry's payload must not match this entry metadata's expected checksum.
            let other = "other object".to_owned();
            cache.insert(other.clone(), download(100, 50)).await.unwrap();
            let (other_payload, _) = entry_disk_ranges(&cache, &other);
            bytes = cache
                .disk
                .file
                .read_at(other_payload[0].start, PAYLOAD_ALIGNMENT_BYTES as usize)
                .await
                .unwrap()
                .to_vec();
        } else {
            bytes[49] ^= 1;
        }
        cache.disk.file.write_at(address, &Bytes::from(bytes)).await.unwrap();
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
        .set_len(METADATA_PAGE_BYTES as u64 + 100)
        .unwrap();
    assert!(cache.get(&key, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn failed_chunk_write_quarantines_the_batch_without_publication() {
    let (_directory, mut cache) = open_test_cache(CHUNK_BYTES).await;
    // First chunk succeeds, second is outside the real file. Even the complete small entry
    // in the first chunk must remain unpublished when its shard's batch fails.
    Arc::get_mut(&mut cache.disk).unwrap().arenas[0].allocator =
        DiskAllocator::for_disk_range(0..3 * CHUNK_BYTES).unwrap();
    assert!(matches!(
        cache
            .insert_batch(vec![
                ("small".to_owned(), download(0, 1)),
                ("large".to_owned(), download(0, CHUNK_BYTES as usize)),
            ])
            .await,
        Err(DiskRangeCacheError::DataFile(DataFileError::OutOfBounds { .. }))
    ));
    assert!(cache.get(&"small".to_owned(), range(0, 1)).await.is_none());
    assert!(cache.get(&"large".to_owned(), range(0, 1)).await.is_none());
    assert_eq!(cache.disk.arenas[0].allocator.available_bytes(), 0);
    let chunk_metadata_page = cache.disk.file.read_at(0, METADATA_PAGE_BYTES).await.unwrap();
    assert_eq!(&chunk_metadata_page[88..96], page_format::CHUNK_METADATA_PAGE_TAG);
}

#[test]
fn abandoned_chunk_write_owner_quarantines_shared_storage() {
    let allocator = DiskAllocator::for_disk_range(0..CHUNK_BYTES).unwrap();
    let mut batch = UnwrittenBatch::default();
    let histories = ObjectAccessHistories::new(1);
    batch
        .push(
            &allocator,
            &"a".to_owned(),
            &download(0, 10),
            &histories.for_key(&"a".to_owned()),
        )
        .unwrap();
    batch
        .push(
            &allocator,
            &"b".to_owned(),
            &download(0, 10),
            &histories.for_key(&"b".to_owned()),
        )
        .unwrap();
    let unfinished = UnfinishedChunkWrites(
        batch
            .chunks
            .iter()
            .map(|chunk| chunk.region.slice(chunk.region.range()))
            .collect(),
    );
    drop(unfinished);
    drop(batch);
    assert_eq!(allocator.available_bytes(), 0);
}

#[tokio::test]
async fn serves_100_mib_entry_subranges_only_after_checking_the_whole_entry() {
    let (_directory, cache) = open_test_cache(104 * CHUNK_BYTES).await;
    let key = "large object".to_owned();
    let source = download(3, 100 * CHUNK_BYTES as usize + 17);
    assert!(cache.insert(key.clone(), source.clone()).await.unwrap());
    let boundary = 3 + CHUNK_BYTES - METADATA_PAGE_BYTES as u64;
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
    let last_aligned_offset = payload.last().unwrap().end - PAYLOAD_ALIGNMENT_BYTES;
    cache
        .disk
        .file
        .write_at(
            last_aligned_offset,
            &Bytes::from(vec![0; PAYLOAD_ALIGNMENT_BYTES as usize]),
        )
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
    assert_eq!(
        cache
            .insert_batch(keys.iter().map(|key| (key.clone(), download(0, 100))).collect())
            .await
            .unwrap(),
        2
    );
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
        let mut index = DiskEntryIndex::default();
        index.insert(
            "object".to_owned(),
            ObjectRangeDiskStorage {
                eviction_position: 0,
                publication_id: 0,
                accesses: ObjectAccessHistories::new(1).for_key(&"object".to_owned()),
                object_range: range(0, 3),
                payload_checksum: blake3::hash(replacement),
                payload_regions: Vec::new(),
                entry_metadata_regions: Vec::new(),
            },
        );
        index.invalidate("object", &read);
        assert_eq!(
            index.covering_range("object", range(0, 3)).is_some(),
            replacement != b"old"
        );
    }
}

#[test]
fn metadata_page_checks_bind_content_checksum_address_ordinal_tag_and_entire_contents() {
    let mut page = vec![0; METADATA_PAGE_BYTES];
    page_format::encode_page(
        &mut page,
        page_format::ENTRY_METADATA_PAGE_TAG,
        &[7; 32],
        METADATA_PAGE_BYTES as u64,
        2,
        8192,
        b"entry metadata contents",
    );
    let validate = |bytes: &[u8]| {
        page_format::validate_page(
            bytes,
            page_format::ENTRY_METADATA_PAGE_TAG,
            &[7; 32],
            METADATA_PAGE_BYTES as u64,
            2,
        )
        .is_some()
    };
    assert!(validate(&page));
    for offset in [0, 32, 64, 72, 80, 88, 96, METADATA_PAGE_BYTES - 1] {
        let mut torn = page.clone();
        torn[offset] ^= 1;
        assert!(!validate(&torn));
    }
    assert!(!validate(&page[..METADATA_PAGE_BYTES - 1]));
    assert!(
        page_format::validate_page(
            &page,
            page_format::CHUNK_METADATA_PAGE_TAG,
            &[7; 32],
            METADATA_PAGE_BYTES as u64,
            2
        )
        .is_none()
    );
    assert!(
        page_format::validate_page(
            &page,
            page_format::ENTRY_METADATA_PAGE_TAG,
            &[8; 32],
            METADATA_PAGE_BYTES as u64,
            2
        )
        .is_none()
    );
    assert!(page_format::validate_page(&page, page_format::ENTRY_METADATA_PAGE_TAG, &[7; 32], 8192, 2).is_none());
    assert!(
        page_format::validate_page(
            &page,
            page_format::ENTRY_METADATA_PAGE_TAG,
            &[7; 32],
            METADATA_PAGE_BYTES as u64,
            3
        )
        .is_none()
    );

    // A valid page of different entry metadata at the same address must not join this chain.
    page_format::encode_page(
        &mut page,
        page_format::ENTRY_METADATA_PAGE_TAG,
        &[8; 32],
        METADATA_PAGE_BYTES as u64,
        2,
        8192,
        b"different entry metadata contents",
    );
    assert!(!validate(&page));
}
