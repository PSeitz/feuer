use super::*;
use std::{future::Future, task::Poll, time::Duration};

// Keep single-entry scenarios concise while exercising the explicit batch API.
impl DiskCache {
    pub(super) async fn insert(&self, key: ObjectKeyHash, download: Download) -> Result<bool, DiskCacheError> {
        self.insert_batch(vec![(key, download)]).await.map(|count| count == 1)
    }

    pub(super) async fn write_dirty_metadata_pages(&self) {
        for shard in &self.disk.shards {
            shard.write_dirty_metadata_pages(&self.disk.file).await.unwrap();
        }
    }
}

pub(super) fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

pub(super) fn download(start: u64, length: usize) -> Download {
    Download::new(
        start,
        Bytes::from(
            (start..start + length as u64)
                .map(|object_offset| (object_offset % 251) as u8)
                .collect::<Vec<_>>(),
        ),
    )
    .unwrap()
}

pub(super) async fn open_test_cache(capacity: u64) -> (tempfile::TempDir, DiskCache) {
    let directory = tempfile::tempdir().unwrap();
    // Keep the requested payload capacity, plus one metadata chunk per shard.
    let shards = (capacity / (128 * CHUNK_BYTES)).clamp(1, 64);
    let cache = DiskCache::open(directory.path(), capacity + shards * CHUNK_BYTES, IoMetrics::noop())
        .await
        .unwrap();
    (directory, with_manual_metadata_writes(cache))
}

/// Detach a freshly opened cache from the writer's weak reference so tests can drive writes themselves.
pub(super) fn with_manual_metadata_writes(cache: DiskCache) -> DiskCache {
    DiskCache {
        disk: Arc::new(Arc::try_unwrap(cache.disk).ok().unwrap()),
    }
}

impl DiskEntry {
    fn single_chunk_start(&self) -> Option<u64> {
        (self.chunk_count() == 1).then_some(self.payload_range.start / CHUNK_BYTES * CHUNK_BYTES)
    }
}

pub(super) async fn entry_disk_ranges(
    cache: &DiskCache,
    key: &ObjectKeyHash,
) -> (Vec<std::ops::Range<u64>>, Vec<std::ops::Range<u64>>) {
    let shard = &cache.disk.shards[cache.disk.shard_index_for_key(key)];
    let index = shard.entry_index.lock().unwrap();
    let entry = index.entries_by_key[key].first_key_value().unwrap().1;
    let (chunk_index, entry_metadata_index) = entry.metadata;
    let pages = shard.metadata_pages.lock().unwrap();
    let address = pages.chunks[chunk_index].region.range().start
        + (entry_metadata_index / page_format::ENTRIES_PER_METADATA_PAGE * METADATA_PAGE_BYTES) as u64;
    let metadata = address..address + METADATA_PAGE_BYTES as u64;
    (vec![entry.payload_range.clone()], vec![metadata])
}

#[tokio::test]
async fn metadata_changes_after_copying_remain_dirty() {
    let (_directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let first = ObjectKeyHash(1);
    let second = ObjectKeyHash(2);
    cache.insert(first, download(0, 1)).await.unwrap();
    let shard = &cache.disk.shards[0];
    let mut writer = Box::pin(shard.write_dirty_metadata_pages(&cache.disk.file));
    let completion = std::future::poll_fn(|cx| Poll::Ready(writer.as_mut().poll(cx))).await;
    // The copy is taken on the first poll; the mutex is released even if I/O is still pending.
    assert!(shard.metadata_pages.try_lock().unwrap().dirty_pages.is_empty());
    cache.insert(second, download(0, 1)).await.unwrap();
    assert!(cache.get(&second, range(0, 1)).await.is_some());
    match completion {
        Poll::Ready(result) => result.unwrap(),
        Poll::Pending => writer.await.unwrap(),
    }
    let pages = shard.metadata_pages.lock().unwrap();
    assert_eq!(pages.dirty_pages.len(), 1);
    assert!(pages.dirty_pages.contains(&(0, 0)));
}

#[tokio::test]
async fn exact_unaligned_reads_containment_and_key_hash_identity() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("complete immutable identity");
    assert!(cache.get(&key, range(3, 4)).await.is_none());
    let source = download(3, 20_007);
    assert!(cache.insert(key, source.clone()).await.unwrap());
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
            .get(&ObjectKeyHash::from("different immutable identity"), range(3, 4))
            .await
            .is_none()
    );
    assert!(!cache.insert(key, source).await.unwrap());
    assert!(!cache.insert(key, download(7, 100)).await.unwrap());
    let index = cache.disk.shards[0].entry_index.lock().unwrap();
    assert_eq!(index.entries_by_key[&key].len(), 1);
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
                    .map(|length| (ObjectKeyHash::from(format!("entry-{length}")), download(17, length)))
                    .collect()
            )
            .await
            .unwrap(),
        lengths.len()
    );
    let mut previous_end = CHUNK_BYTES;
    for length in [1, 1024, 4016, 4096, 4097, 3 * 4096 + 9] {
        let key = ObjectKeyHash::from(format!("entry-{length}"));
        let source = download(17, length);
        let (payload, entry_metadata) = entry_disk_ranges(&cache, &key).await;
        assert_eq!(payload.len(), 1);
        let allocated = &payload[0];
        assert_eq!(allocated.start, previous_end);
        assert!(allocated.start.is_multiple_of(BUFFER_ALIGNMENT as u64));
        assert!(allocated.end <= 2 * CHUNK_BYTES);
        assert_eq!(
            allocated.end - allocated.start,
            (length as u64).next_multiple_of(BUFFER_ALIGNMENT as u64)
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
        assert_eq!(entry_metadata[0], 0..METADATA_PAGE_BYTES as u64);
        previous_end = allocated.end;
    }
}

#[tokio::test]
async fn mixed_batch_shares_metadata_separately_from_payload_chunks() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    assert_eq!(cache.insert_batch(Vec::new()).await.unwrap(), 0);
    let mut inputs = vec![(ObjectKeyHash::from("large"), download(0, CHUNK_BYTES as usize))];
    inputs.extend((0..130).map(|i| (ObjectKeyHash::from(format!("small-{i}")), download(i, 1024))));
    assert_eq!(cache.insert_batch(inputs).await.unwrap(), 131);
    let large_start = entry_disk_ranges(&cache, &ObjectKeyHash::from("large")).await.0[0].start;
    let metadata_end = METADATA_PAGE_BYTES as u64;
    for i in 0..130 {
        let key = ObjectKeyHash::from(format!("small-{i}"));
        assert_eq!(
            cache.get(&key, range(i, i + 1024)).await.unwrap(),
            download(i, 1024).bytes()
        );
        assert!(entry_disk_ranges(&cache, &key).await.0[0].start < large_start);
    }
    let (first_payload, metadata) = entry_disk_ranges(&cache, &ObjectKeyHash::from("small-0")).await;
    assert_eq!(metadata[0], 0..metadata_end);
    assert_eq!(first_payload[0].start, CHUNK_BYTES);
    assert_eq!(large_start, 2 * CHUNK_BYTES);
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 2 * CHUNK_BYTES);
    assert_eq!(
        cache
            .get(&ObjectKeyHash::from("large"), range(0, CHUNK_BYTES))
            .await
            .unwrap(),
        download(0, CHUNK_BYTES as usize).bytes()
    );
}

#[tokio::test]
async fn shared_chunks_contain_complete_entries_and_multi_chunk_entries_have_no_neighbors() {
    let (_directory, cache) = open_test_cache(8 * CHUNK_BYTES).await;
    let inputs: Vec<_> = [600_000, 600_000, CHUNK_BYTES as usize, 1, CHUNK_BYTES as usize + 1, 1]
        .into_iter()
        .enumerate()
        .map(|(i, length)| (ObjectKeyHash(i as u128), download(0, length)))
        .collect();
    assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), inputs.len());
    let index = cache.disk.shards[0].entry_index.lock().unwrap();
    let mut owners = BTreeMap::<u64, Vec<&DiskEntry>>::new();
    for entries in index.entries_by_key.values() {
        for entry in entries.values() {
            for chunk in entry.payload_range.start / CHUNK_BYTES..entry.payload_range.end.div_ceil(CHUNK_BYTES) {
                owners.entry(chunk).or_default().push(entry);
            }
        }
    }
    for entries in owners.values() {
        if entries.len() > 1 {
            assert!(entries.iter().all(|entry| entry.single_chunk_start().is_some()));
        }
    }
    assert_eq!(owners.len(), 5);
}

#[tokio::test]
async fn metadata_growth_does_not_consume_payload_chunk_space() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let entries_per_page = PAGE_CONTENT_BYTES / page_format::ENTRY_METADATA_BYTES;
    // Cross a metadata page boundary while still fitting all payloads in one chunk.
    let inputs = (0..=entries_per_page)
        .map(|i| {
            let length = if i == 0 {
                CHUNK_BYTES as usize - (entries_per_page + 1) * METADATA_PAGE_BYTES
            } else {
                1
            };
            (ObjectKeyHash(i as u128), download(0, length))
        })
        .collect();
    assert_eq!(cache.insert_batch(inputs).await.unwrap(), entries_per_page + 1);
    let index = cache.disk.shards[0].entry_index.lock().unwrap();
    let entries: Vec<_> = index.entries_by_key.values().flat_map(BTreeMap::values).collect();
    assert!(
        entries
            .iter()
            .all(|entry| entry.single_chunk_start() == Some(CHUNK_BYTES))
    );
    assert_eq!(
        entries.iter().map(|entry| entry.payload_range.end).max(),
        Some(2 * CHUNK_BYTES - METADATA_PAGE_BYTES as u64)
    );
}

#[tokio::test]
async fn failed_reservation_releases_metadata_without_moving_pending_payloads() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let shard = &cache.disk.shards[0];
    let mut region = None;
    let mut attempts_left = 0;
    let mut chunks_left = MAX_EVICTION_CHUNKS;
    let first = shard
        .reserve_entry(
            &download(0, CHUNK_BYTES as usize - METADATA_PAGE_BYTES),
            &mut region,
            None,
            &cache.disk.access_histories,
            &mut attempts_left,
            &mut chunks_left,
        )
        .unwrap();
    let free_positions = shard.metadata_pages.lock().unwrap().free_entry_positions.len();
    assert!(
        shard
            .reserve_entry(
                &download(0, METADATA_PAGE_BYTES + 1),
                &mut None,
                None,
                &cache.disk.access_histories,
                &mut attempts_left,
                &mut chunks_left,
            )
            .is_none()
    );
    assert_eq!(
        shard.metadata_pages.lock().unwrap().free_entry_positions.len(),
        free_positions
    );
    let second = shard
        .reserve_entry(
            &download(0, 1),
            &mut region,
            Some(first.payload_range.end),
            &cache.disk.access_histories,
            &mut attempts_left,
            &mut chunks_left,
        )
        .unwrap();
    assert_eq!(
        first.payload_range,
        CHUNK_BYTES..2 * CHUNK_BYTES - METADATA_PAGE_BYTES as u64
    );
    assert_eq!(second.payload_range, first.payload_range.end..2 * CHUNK_BYTES);
    assert_eq!(shard.allocator.available_bytes(), 0);
    drop(region);
    assert_eq!(shard.allocator.available_bytes(), CHUNK_BYTES);
}

#[tokio::test]
async fn pressure_reclaims_shared_and_exclusive_chunks_during_mixed_size_churn() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    for cycle in 0..20 {
        let inputs: Vec<_> = [1, 4097, CHUNK_BYTES as usize + 17]
            .into_iter()
            .enumerate()
            .map(|(i, length)| (ObjectKeyHash::from(format!("{cycle}-{i}")), download(cycle * 7, length)))
            .collect();
        assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), inputs.len());
        // The final region remains published; admitting it may evict earlier entries in the same batch.
        let (key, source) = inputs.last().unwrap();
        assert_eq!(cache.get(key, source.downloaded_range()).await.unwrap(), source.bytes());
        for (key, source) in inputs {
            if let Some(bytes) = cache.get(&key, source.downloaded_range()).await {
                assert_eq!(bytes, source.bytes());
            }
        }
        let index = cache.disk.shards[0].entry_index.lock().unwrap();
        assert_eq!(
            index.eviction_candidates.len(),
            index.entries_by_key.values().map(BTreeMap::len).sum::<usize>()
        );
        for (position, (key, start)) in index.eviction_candidates.iter().enumerate() {
            assert_eq!(index.entries_by_key[key][start].eviction_position, position);
        }
    }
}

#[tokio::test]
async fn eviction_preserves_a_newer_replacement() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("replaced");
    cache
        .insert_batch(vec![
            (key, download(5, 200)),
            (ObjectKeyHash::from("neighbor"), download(0, 100)),
        ])
        .await
        .unwrap();
    cache.insert(key, download(0, 300)).await.unwrap();
    // Both payload chunks are occupied; evict the neighbor in the old shared chunk.
    assert!(
        cache
            .insert(ObjectKeyHash::from("large"), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
    assert!(cache.get(&ObjectKeyHash::from("neighbor"), range(0, 1)).await.is_none());
    assert_eq!(cache.get(&key, range(0, 300)).await.unwrap(), download(0, 300).bytes());
}

#[tokio::test]
async fn multi_chunk_reuse_turns_an_old_read_into_a_checksum_miss() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("large");
    let source = download(3, CHUNK_BYTES as usize + 17);
    cache.insert(key, source.clone()).await.unwrap();
    let read = {
        let index = cache.disk.shards[0].entry_index.lock().unwrap();
        let entry = index.covering_entry(&key, source.downloaded_range()).unwrap();
        PayloadRead {
            object_range: entry.object_range,
            payload_checksum: entry.payload_checksum,
            payload_range: entry.payload_range.clone(),
        }
    };
    assert!(
        cache
            .insert(ObjectKeyHash::from("new"), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
    assert!(cache.get(&key, range(3, 4)).await.is_none());
    assert!(
        read.read_verified_range(&cache.disk.file, range(3, 20))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn eviction_budgets_and_active_reservations_bound_reclamation() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("large");
    cache.insert(key, download(0, CHUNK_BYTES as usize + 1)).await.unwrap();
    let shard = &cache.disk.shards[0];
    let mut attempts_left = 0;
    let mut chunks_left = MAX_EVICTION_CHUNKS;
    assert!(!shard.sample_and_evict_entry(&cache.disk.access_histories, &mut attempts_left, &mut chunks_left));
    attempts_left = 1;
    chunks_left = 1;
    assert!(shard.sample_and_evict_entry(&cache.disk.access_histories, &mut attempts_left, &mut chunks_left));
    assert_eq!(attempts_left, 0);
    assert!(cache.get(&key, range(0, 1)).await.is_some());
    attempts_left = 1;
    chunks_left = MAX_EVICTION_CHUNKS;
    assert!(shard.sample_and_evict_entry(&cache.disk.access_histories, &mut attempts_left, &mut chunks_left));
    shard.write_dirty_metadata_pages(&cache.disk.file).await.unwrap();
    let active = shard.allocator.reserve_chunks(2).unwrap();
    assert!(
        !cache
            .insert(ObjectKeyHash::from("blocked"), download(0, 1))
            .await
            .unwrap()
    );
    drop(active);
    assert!(
        cache
            .insert(ObjectKeyHash::from("unblocked"), download(0, 1))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn value_aware_eviction_preserves_hot_neighbors_and_needs_no_metadata_reads() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let hot = ObjectKeyHash::from("hot");
    let cold = ObjectKeyHash::from("cold");
    let other = ObjectKeyHash::from("other");
    cache
        .insert_batch(vec![(hot, download(0, 100)), (cold, download(0, 100))])
        .await
        .unwrap();
    cache.insert(other, download(0, 100)).await.unwrap();
    let histories = cache.access_histories();
    for _ in 0..3 {
        histories.record_access(&hot, range(0, 1));
    }
    histories.record_access(&cold, range(0, 1));
    histories.record_access(&other, range(0, 1));
    // Corrupt entry metadata: entry-wise eviction must not read it at all.
    cache
        .disk
        .file
        .write_at(0, &Bytes::from(vec![0; METADATA_PAGE_BYTES]))
        .await
        .unwrap();
    let shard = &cache.disk.shards[0];
    let mut attempts_left = 1;
    let mut chunks_left = MAX_EVICTION_CHUNKS;
    assert!(shard.sample_and_evict_entry(&cache.disk.access_histories, &mut attempts_left, &mut chunks_left));
    assert!(cache.get(&cold, range(0, 1)).await.is_none());
    assert!(cache.get(&hot, range(0, 1)).await.is_some());
    assert_eq!(shard.allocator.available_bytes(), 0); // Hot neighbor still owns the chunk.
    assert!(
        cache
            .insert(ObjectKeyHash::from("new"), download(0, 100))
            .await
            .unwrap()
    );
    assert!(cache.get(&other, range(0, 1)).await.is_none());
    assert!(cache.get(&hot, range(0, 1)).await.is_some());
}

#[tokio::test]
async fn disk_value_uses_payload_size_not_aligned_allocations_or_chunk_size() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let small = ObjectKeyHash::from("small");
    let large = ObjectKeyHash::from("large");
    // Both entries use identical physical capacity, but different payload lengths.
    cache.insert(small, download(0, 1)).await.unwrap();
    cache.insert(large, download(0, 3000)).await.unwrap();
    cache.access_histories().record_access(&small, range(0, 1));
    cache.access_histories().record_access(&large, range(0, 1));
    assert!(cache.insert(ObjectKeyHash::from("new"), download(0, 1)).await.unwrap());
    assert!(cache.get(&small, range(0, 1)).await.is_some());
    assert!(cache.get(&large, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn payload_value_ignores_original_key_length_and_keeps_first_sampled_on_ties() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let older = ObjectKeyHash::from("small metadata");
    let newer = ObjectKeyHash::from("long key/".repeat(1000));
    cache.insert(older, download(0, 100)).await.unwrap();
    cache.insert(newer, download(0, 100)).await.unwrap();
    // Without access history both scores are zero; sample the newer entry first.
    cache.disk.shards[0].entry_index.lock().unwrap().next_candidate = 1;
    assert!(
        cache
            .insert(ObjectKeyHash::from("incoming"), download(0, 1))
            .await
            .unwrap()
    );
    assert!(cache.get(&older, range(0, 1)).await.is_some());
    assert!(cache.get(&newer, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn pressure_replacement_preserves_evidence_while_the_last_old_entry_is_removed() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    cache.insert(key, download(5, 10)).await.unwrap();
    cache.access_histories().record_access(&key, range(6, 7));
    assert!(cache.insert(key, download(0, 20)).await.unwrap());
    assert_eq!(
        cache.access_histories().recent_requested_ranges(&key),
        vec![range(6, 7)]
    );
    assert!(cache.get(&key, range(0, 20)).await.is_some());
}

#[tokio::test]
async fn disk_access_evidence_ages_and_credits_only_covering_ranges() {
    use feuer_types::retention::ACCESS_COUNT_HALF_LIFE;
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    cache.insert(key, download(0, 100)).await.unwrap();
    cache.insert(key, download(200, 100)).await.unwrap();
    let histories = cache.access_histories();
    for _ in 0..3 {
        histories.record_access(&key, range(0, 1));
    }
    let original_cost = histories.decayed_retrieval_cost(&key, range(0, 100));
    assert!(original_cost > 0.0);
    assert_eq!(histories.decayed_retrieval_cost(&key, range(200, 300)), 0.0);
    // Successful lookups are recorded explicitly; raw storage reads/writes do not double-count.
    assert!(cache.get(&key, range(0, 1)).await.is_some());
    assert_eq!(histories.request_count(), 3);
    for _ in 0..*ACCESS_COUNT_HALF_LIFE {
        histories.record_access(&key, range(200, 201));
    }
    assert_eq!(
        histories.decayed_retrieval_cost(&key, range(0, 100)),
        original_cost * 0.5
    );
    assert!(cache.insert(ObjectKeyHash::from("new"), download(0, 1)).await.unwrap());
    assert!(cache.get(&key, range(0, 1)).await.is_none());
    assert!(cache.get(&key, range(200, 201)).await.is_some());
}

#[tokio::test]
async fn memory_and_disk_use_the_same_evidence_through_memory_eviction() {
    let history = Arc::new(ObjectAccessHistories::new());
    let (_, registry) = crate::test_metrics::registry();
    let memory = feuer_memory::MemoryCache::with_access_histories(
        4096,
        feuer_memory::MemoryMetrics::new(&registry),
        history.clone(),
    );
    let directory = tempfile::tempdir().unwrap();
    let cache =
        DiskCache::open_with_access_histories(directory.path(), 3 * CHUNK_BYTES, IoMetrics::noop(), history.clone())
            .await
            .unwrap();
    let hot = ObjectKeyHash::from("hot");
    let cold = ObjectKeyHash::from("cold");
    memory.insert(hot, download(0, 100));
    history.record_access(&hot, range(0, 1));
    cache.insert(hot, download(0, 100)).await.unwrap();
    cache.insert(cold, download(0, 100)).await.unwrap();
    history.record_access(&cold, range(0, 1)); // A disk-only key.
    for _ in 0..3 {
        assert!(memory.get(&hot, range(0, 1)).is_some());
        history.record_access(&hot, range(0, 1));
    }
    assert_eq!(history.request_count(), 5);
    assert!(
        cache
            .insert(ObjectKeyHash::from("new"), download(0, 100))
            .await
            .unwrap()
    );
    assert!(cache.get(&cold, range(0, 1)).await.is_none());
    assert!(memory.remove(&hot, range(0, 100)));
    assert!(memory.get(&hot, range(0, 1)).is_none());
    assert!(cache.get(&hot, range(50, 51)).await.is_some());
    assert_eq!(history.request_count(), 5); // Raw storage reads do not record a second event.
    history.record_access(&hot, range(50, 51));
    assert_eq!(history.request_count(), 6);
    let shard = &cache.disk.shards[0];
    let entry = shard.entry_index.lock().unwrap().remove(&hot, 0).unwrap();
    shard.remove_payload_and_allow_metadata_overwrite(entry);
    drop(memory);
    drop(cache);
    assert_eq!(history.recent_requested_ranges(&hot).len(), 5);
    assert!(history.decayed_retrieval_cost(&hot, range(0, 100)) > 0.0);
}

#[tokio::test]
async fn per_batch_eviction_limit_does_not_force_out_a_shared_chunks_last_entry() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let hot = ObjectKeyHash::from("hot");
    let mut inputs: Vec<_> = (0..MAX_EVICTION_ATTEMPTS)
        .map(|i| (ObjectKeyHash::from(format!("cold-{i}")), download(0, 1)))
        .collect();
    inputs.push((hot, download(0, 1)));
    cache.insert_batch(inputs).await.unwrap();
    cache.access_histories().record_access(&hot, range(0, 1));
    // Removing 64 individually chosen cold entries still cannot release the chunk with its hot neighbor.
    assert!(
        !cache
            .insert(ObjectKeyHash::from("incoming"), download(0, 1))
            .await
            .unwrap()
    );
    assert!(cache.get(&hot, range(0, 1)).await.is_some());
    assert_eq!(
        cache.disk.shards[0]
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
                let key = ObjectKeyHash::from(format!("{worker}-{cycle}"));
                let source = download(worker * 53 + cycle, 8193);
                cache.insert(key, source.clone()).await.unwrap();
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
    let key = ObjectKeyHash::from("small entry");
    let source = download(3, 1024);
    assert_eq!(
        cache
            .insert_batch(vec![
                (key, source.clone()),
                (ObjectKeyHash::from("neighbor"), download(0, 8192))
            ])
            .await
            .unwrap(),
        2
    );
    let (payload, _) = entry_disk_ranges(&cache, &key).await;
    assert_eq!(payload[0].end - payload[0].start, BUFFER_ALIGNMENT as u64);
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
    let key = ObjectKeyHash::from("small entry");
    let source = download(3, 1024);
    cache.insert(key, source.clone()).await.unwrap();
    let (payload, _) = entry_disk_ranges(&cache, &key).await;
    let address = payload[0].start;
    let mut bytes = cache
        .disk
        .file
        .read_at(address, BUFFER_ALIGNMENT)
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
async fn removing_one_shared_payload_does_not_free_its_neighbors_chunks() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let first = ObjectKeyHash::from("first");
    let neighbor = ObjectKeyHash::from("neighbor");
    assert_eq!(
        cache
            .insert_batch(vec![(first, download(3, 1024)), (neighbor, download(7, 4097))])
            .await
            .unwrap(),
        2
    );
    let original = cache
        .disk
        .file
        .read_at(CHUNK_BYTES, CHUNK_BYTES as usize)
        .await
        .unwrap();
    let shard = &cache.disk.shards[0];
    let entry = shard.entry_index.lock().unwrap().remove(&first, 3).unwrap();
    shard.remove_payload_and_allow_metadata_overwrite(entry);
    assert!(shard.allocator.reserve_chunks(1).is_none());
    assert_eq!(
        cache
            .disk
            .file
            .read_at(CHUNK_BYTES, CHUNK_BYTES as usize)
            .await
            .unwrap(),
        original
    );
    assert!(cache.get(&neighbor, range(7, 4104)).await.is_some());
    assert!(cache.insert(first, download(100, 1024)).await.unwrap());
    assert!(cache.get(&neighbor, range(7, 4104)).await.is_none());
    assert_eq!(
        cache.get(&first, range(100, 1124)).await.unwrap(),
        download(100, 1024).bytes()
    );
}

#[tokio::test]
async fn multi_chunk_payload_has_no_metadata_gaps() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("contiguous");
    let source = download(100, 2 * CHUNK_BYTES as usize + 17);
    assert!(cache.insert(key, source.clone()).await.unwrap());
    let (payload, metadata) = entry_disk_ranges(&cache, &key).await;
    assert_eq!(payload.len(), 1);
    assert!(metadata[0].end <= payload[0].start);
    assert!(payload[0].start.is_multiple_of(CHUNK_BYTES));
    // Read the disk bytes directly across physical chunk boundaries: no assembly or skipped headers.
    assert_eq!(
        cache
            .disk
            .file
            .read_at(payload[0].start, source.bytes().len())
            .await
            .unwrap(),
        source.bytes()
    );
    let boundary = 100 + CHUNK_BYTES - payload[0].start % CHUNK_BYTES;
    assert_eq!(
        cache.get(&key, range(boundary - 3, boundary + 7)).await.unwrap(),
        download(boundary - 3, 10).bytes()
    );
    assert_eq!(
        cache.get(&key, source.downloaded_range()).await.unwrap(),
        source.bytes()
    );
}

#[tokio::test]
async fn batch_rejects_fragmented_space_without_consuming_it() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let shard = &cache.disk.shards[0];
    // Reserve metadata first, leaving four payload chunks to fragment.
    let position = shard
        .metadata_pages
        .lock()
        .unwrap()
        .reserve_metadata(&shard.allocator)
        .unwrap();
    shard.metadata_pages.lock().unwrap().free_entry_positions.push(position);
    let allocator = &shard.allocator;
    let first = allocator.reserve_chunks(1).unwrap();
    let second = allocator.reserve_chunks(1).unwrap();
    let third = allocator.reserve_chunks(1).unwrap();
    let fourth = allocator.reserve_chunks(1).unwrap();
    drop((first, third));
    let key = ObjectKeyHash::from("large");
    let source = download(0, CHUNK_BYTES as usize + 1);
    assert!(!cache.insert(key, source.clone()).await.unwrap());
    assert_eq!(allocator.available_bytes(), 2 * CHUNK_BYTES);
    drop(second);
    assert!(cache.insert(key, source.clone()).await.unwrap());
    assert_eq!(
        cache.get(&key, source.downloaded_range()).await.unwrap(),
        source.bytes()
    );
    drop(fourth);
    let entry = shard.entry_index.lock().unwrap().remove(&key, 0).unwrap();
    shard.remove_payload_and_allow_metadata_overwrite(entry);
    assert_eq!(allocator.available_bytes(), 4 * CHUNK_BYTES);
}

#[tokio::test]
async fn partial_overlaps_coexist_without_assembly_and_a_covering_range_replaces_them() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    for (start, length) in [(10, 10), (30, 10), (15, 20)] {
        assert!(cache.insert(key, download(start, length)).await.unwrap());
    }
    assert!(cache.get(&key, range(10, 40)).await.is_none());
    assert_eq!(cache.get(&key, range(16, 34)).await.unwrap(), download(16, 18).bytes());
    let returned = cache.get(&key, range(11, 19)).await.unwrap();
    assert!(cache.insert(key, download(5, 50)).await.unwrap());
    assert_eq!(
        cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key].len(),
        1
    );
    assert_eq!(returned, download(11, 8).bytes());
    assert_eq!(cache.get(&key, range(10, 40)).await.unwrap(), download(10, 30).bytes());
}

#[tokio::test]
async fn writes_key_hash_range_and_payload_address_in_fixed_size_metadata() {
    let (_directory, cache) = open_test_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("long immutable key/".repeat(700));
    let source = download(17, (CHUNK_BYTES + 7) as usize);
    assert!(cache.insert(key, source.clone()).await.unwrap());
    let (payload, entry_metadata) = entry_disk_ranges(&cache, &key).await;
    assert_eq!(entry_metadata[0], 0..METADATA_PAGE_BYTES as u64);
    assert_eq!(payload[0].start, CHUNK_BYTES);
    cache.write_dirty_metadata_pages().await;
    let page = cache.disk.file.read_at(0, METADATA_PAGE_BYTES).await.unwrap();
    let entry_metadata_bytes = page_format::validate_page(&page, page_format::ENTRY_METADATA_PAGE_TAG).unwrap();
    let read_u64 = |offset| u64::from_le_bytes(entry_metadata_bytes[offset..offset + 8].try_into().unwrap());
    assert_eq!(&entry_metadata_bytes[..16], &key.0.to_le_bytes());
    assert_eq!(read_u64(16), 17);
    assert_eq!(read_u64(24), source.downloaded_range().len());
    assert_eq!(read_u64(32), payload[0].start);
    assert_eq!(read_u64(40), XxHash64::oneshot(0, source.bytes()));
    assert!(
        entry_metadata_bytes[page_format::ENTRY_METADATA_BYTES..]
            .iter()
            .all(|&byte| byte == 0)
    );
}

#[tokio::test]
async fn batch_skips_oversized_and_contained_entries_without_losing_accepted_entries() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    assert_eq!(
        cache
            .insert_batch(vec![
                (ObjectKeyHash::from("object"), download(5, 100)),
                (ObjectKeyHash::from("too large"), download(0, CHUNK_BYTES as usize + 1)),
                (ObjectKeyHash::from("object"), download(0, 200)),
                (ObjectKeyHash::from("object"), download(0, 200)),
            ])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        cache.get(&ObjectKeyHash::from("object"), range(0, 200)).await.unwrap(),
        download(0, 200).bytes()
    );
    assert!(
        cache
            .get(&ObjectKeyHash::from("too large"), range(0, 1))
            .await
            .is_none()
    );
    assert_eq!(
        cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&ObjectKeyHash::from("object")].len(),
        1
    );
}

#[tokio::test]
async fn long_keys_do_not_allocate_extra_metadata_chunks() {
    let (_directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("k".repeat(CHUNK_BYTES as usize));
    assert!(cache.insert(key, download(0, 1)).await.unwrap());
    let (payload, metadata) = entry_disk_ranges(&cache, &key).await;
    assert_eq!(payload.len(), 1);
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0].end - metadata[0].start, METADATA_PAGE_BYTES as u64);
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 2 * CHUNK_BYTES);
    assert!(
        cache
            .insert(ObjectKeyHash::from("other"), download(0, 1))
            .await
            .unwrap()
    );
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), CHUNK_BYTES);
    assert_eq!(cache.get(&key, range(0, 1)).await.unwrap(), download(0, 1).bytes());
    let entry = cache.disk.shards[0]
        .entry_index
        .lock()
        .unwrap()
        .remove(&key, 0)
        .unwrap();
    cache.disk.shards[0].remove_payload_and_allow_metadata_overwrite(entry);
    let shard = &cache.disk.shards[0];
    shard.write_dirty_metadata_pages(&cache.disk.file).await.unwrap();
    assert_eq!(shard.allocator.available_bytes(), 2 * CHUNK_BYTES);
}

#[tokio::test]
async fn entry_metadata_space_is_charged_and_failed_reservations_roll_back() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    // The separate metadata chunk is charged, leaving exactly one payload chunk.
    assert!(
        !cache
            .insert(ObjectKeyHash::from("too large"), download(0, CHUNK_BYTES as usize + 1))
            .await
            .unwrap()
    );
    // Payloads can now occupy the entire chunk.
    let key = ObjectKeyHash::from("fits exactly");
    assert!(cache.insert(key, download(7, CHUNK_BYTES as usize)).await.unwrap());
    assert!(cache.disk.shards[0].allocator.reserve_chunks(1).is_none());
    // A two-chunk entry cannot fit this shard; do not evict the existing entry in vain.
    assert!(
        !cache
            .insert(ObjectKeyHash::from("oversized"), download(0, CHUNK_BYTES as usize + 1))
            .await
            .unwrap()
    );
    assert!(cache.get(&key, range(8, 12)).await.is_some());
    assert!(cache.insert(ObjectKeyHash::from("full"), download(0, 1)).await.unwrap());
    assert!(cache.get(&key, range(8, 12)).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_equal_and_containing_writes_revalidate_publication() {
    let (_directory, cache) = open_test_cache(16 * CHUNK_BYTES).await;
    let start_barrier = Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let cache = cache.clone();
        let start_barrier = start_barrier.clone();
        tasks.push(tokio::spawn(async move {
            start_barrier.wait().await;
            cache
                .insert(ObjectKeyHash::from("object"), download(7, 100))
                .await
                .unwrap()
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
                .insert(ObjectKeyHash::from("object"), download(start, length))
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let key = ObjectKeyHash::from("object");
    let ranges: Vec<_> = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key]
        .values()
        .map(|entry| entry.object_range)
        .collect();
    assert_eq!(ranges, [range(0, 200), range(300, 350)]);
    assert_eq!(cache.get(&key, range(0, 200)).await.unwrap(), download(0, 200).bytes());
}

#[tokio::test]
async fn publishes_each_region_before_reserving_the_next() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let small = ObjectKeyHash::from("small");
    let large = ObjectKeyHash::from("large");
    let published = cache
        .insert_batch(vec![
            (small, download(0, 1)),
            (large, download(0, CHUNK_BYTES as usize + 1)),
        ])
        .await
        .unwrap();
    // The small entry must publish first so the larger entry can evict it and reuse its chunk.
    assert_eq!(published, 2);
    assert!(!cache.contains(&small, range(0, 1)));
    assert_eq!(
        cache.get(&large, range(0, 100)).await.unwrap(),
        download(0, 100).bytes()
    );
}

#[tokio::test]
async fn canceled_requester_does_not_abort_the_reservation_owner() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let mut requester = Box::pin(cache.insert(ObjectKeyHash::from("object"), download(0, 100)));
    // On the current-thread runtime, one poll spawns the owner without letting it run yet.
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(requester.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(requester);
    assert!(cache.get(&ObjectKeyHash::from("object"), range(0, 1)).await.is_none());
    let bytes = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(bytes) = cache.get(&ObjectKeyHash::from("object"), range(0, 100)).await {
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
async fn read_addresses_and_results_do_not_delay_replaced_payload_reuse() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    cache.insert(key, download(5, 100)).await.unwrap();
    let (guard, payload_checksum) = {
        let index = cache.disk.shards[0].entry_index.lock().unwrap();
        let entry = index.covering_entry(&key, range(6, 7)).unwrap();
        (entry.payload_range.clone(), entry.payload_checksum)
    };
    let result = cache.get(&key, range(6, 7)).await.unwrap();
    cache.insert(key, download(0, 200)).await.unwrap();
    let bytes = cache.disk.file.read_at(guard.start, BUFFER_ALIGNMENT).await.unwrap();
    assert_eq!(XxHash64::oneshot(0, &bytes[..100]), payload_checksum);
    let reused = cache.disk.shards[0].allocator.reserve_chunks(1).unwrap();
    assert_eq!(reused.range(), CHUNK_BYTES..2 * CHUNK_BYTES);
    assert_eq!(result, download(6, 1).bytes());
}

#[tokio::test]
async fn corrupted_and_reused_payload_miss_and_invalidate_the_entry() {
    for stale in [false, true] {
        let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
        let key = ObjectKeyHash::from("object");
        cache.insert(key, download(3, 50)).await.unwrap();
        let (payload, _) = entry_disk_ranges(&cache, &key).await;
        let address = payload[0].start;
        let mut bytes = cache
            .disk
            .file
            .read_at(address, BUFFER_ALIGNMENT)
            .await
            .unwrap()
            .to_vec();
        if stale {
            // Another valid entry's payload must not match this entry metadata's expected checksum.
            let other = ObjectKeyHash::from("other object");
            cache.insert(other, download(100, 50)).await.unwrap();
            let (other_payload, _) = entry_disk_ranges(&cache, &other).await;
            bytes = cache
                .disk
                .file
                .read_at(other_payload[0].start, BUFFER_ALIGNMENT)
                .await
                .unwrap()
                .to_vec();
        } else {
            bytes[49] ^= 1;
        }
        cache.disk.file.write_at(address, &Bytes::from(bytes)).await.unwrap();
        assert!(cache.get(&key, range(3, 4)).await.is_none());
        assert!(
            !cache.disk.shards[0]
                .entry_index
                .lock()
                .unwrap()
                .entries_by_key
                .contains_key(&key)
        );
        assert!(cache.insert(key, download(3, 50)).await.unwrap());
        assert_eq!(cache.get(&key, range(3, 4)).await.unwrap(), download(3, 1).bytes());
    }
}

#[tokio::test]
async fn a_short_read_is_a_miss_not_unverified_bytes() {
    let (directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    cache.insert(key, download(0, 100)).await.unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(METADATA_PAGE_BYTES as u64 + 100)
        .unwrap();
    assert!(cache.get(&key, range(0, 1)).await.is_none());
}

#[tokio::test]
async fn failed_multi_chunk_write_keeps_earlier_entries_and_releases_its_storage() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    // The small entry and the large entry's first chunk succeed; its second chunk is outside the file.
    let mut disk = Arc::try_unwrap(cache.disk).ok().unwrap();
    disk.shards[0].allocator = DiskChunkAllocator::for_disk_range(0..4 * CHUNK_BYTES).unwrap();
    let cache = DiskCache { disk: Arc::new(disk) };
    assert!(matches!(
        cache
            .insert_batch(vec![
                (ObjectKeyHash::from("small"), download(0, 1)),
                (ObjectKeyHash::from("large"), download(0, CHUNK_BYTES as usize + 1)),
            ])
            .await,
        Err(DiskCacheError::DataFile(DataFileError::RangeExceedsCapacity { .. }))
    ));
    assert_eq!(
        cache.get(&ObjectKeyHash::from("small"), range(0, 1)).await.unwrap(),
        download(0, 1).bytes()
    );
    assert!(cache.get(&ObjectKeyHash::from("large"), range(0, 1)).await.is_none());
    let shard = &cache.disk.shards[0];
    assert_eq!(shard.allocator.available_bytes(), 2 * CHUNK_BYTES);
    assert_eq!(
        shard.metadata_pages.lock().unwrap().free_entry_positions.len(),
        page_format::ENTRIES_PER_METADATA_CHUNK - 1
    );
}

#[tokio::test]
async fn reads_30_mib_contiguous_payloads_and_trims_final_padding() {
    for length in [30 * CHUNK_BYTES as usize, 30 * CHUNK_BYTES as usize + 17] {
        let (_directory, cache) = open_test_cache(32 * CHUNK_BYTES).await;
        let key = ObjectKeyHash::from("large read");
        let source = download(7, length);
        assert!(cache.insert(key, source.clone()).await.unwrap());
        let bytes = cache.get(&key, source.downloaded_range()).await.unwrap();
        assert_eq!(bytes, source.bytes());
        // Results own only memory, not disk regions or the cache itself.
        drop(cache);
        assert_eq!(bytes, source.bytes());
    }
}

#[tokio::test]
async fn serves_100_mib_entry_subranges_only_after_checking_the_whole_entry() {
    let (_directory, cache) = open_test_cache(104 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("large object");
    let source = download(3, 100 * CHUNK_BYTES as usize + 17);
    assert!(cache.insert(key, source.clone()).await.unwrap());
    let boundary = 3 + CHUNK_BYTES - entry_disk_ranges(&cache, &key).await.0[0].start % CHUNK_BYTES;
    for request in [
        source.downloaded_range(),
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
    let (payload, _) = entry_disk_ranges(&cache, &key).await;
    let last_aligned_offset = payload.last().unwrap().end - BUFFER_ALIGNMENT as u64;
    cache
        .disk
        .file
        .write_at(last_aligned_offset, &Bytes::from(vec![0; BUFFER_ALIGNMENT]))
        .await
        .unwrap();
    // Even a request at the beginning must detect corruption at the end of this entry.
    assert!(cache.get(&key, range(3, 13)).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shards_are_disjoint_and_recovered_before_open_returns() {
    let (directory, cache) = open_test_cache(256 * CHUNK_BYTES).await;
    assert_eq!(cache.disk.shards.len(), 2);
    let keys: Vec<_> = (0..2)
        .map(|shard_index| {
            (0..100)
                .map(|i| ObjectKeyHash::from(format!("object-{i}")))
                .find(|key| cache.disk.shard_index_for_key(key) == shard_index)
                .unwrap()
        })
        .collect();
    assert_eq!(
        cache
            .insert_batch(keys.iter().map(|key| (*key, download(0, 100))).collect())
            .await
            .unwrap(),
        2
    );
    let first = entry_disk_ranges(&cache, &keys[0]).await.0;
    let second = entry_disk_ranges(&cache, &keys[1]).await.0;
    assert!(first.last().unwrap().end <= 128 * CHUNK_BYTES);
    assert!(second[0].start >= 128 * CHUNK_BYTES);
    let returned = cache.get(&keys[0], range(0, 100)).await.unwrap();
    cache.write_dirty_metadata_pages().await;
    let capacity = cache.disk.file.capacity();
    drop(cache);
    let reopened = DiskCache::open(directory.path(), capacity, IoMetrics::noop())
        .await
        .unwrap();
    assert!(keys.iter().all(|key| reopened.contains(key, range(0, 100))));
    for key in &keys {
        assert_eq!(
            reopened.get(key, range(0, 100)).await.unwrap(),
            download(0, 100).bytes()
        );
    }
    assert!(!reopened.insert(keys[0], download(0, 100)).await.unwrap());
    assert_eq!(returned, download(0, 100).bytes());
}

#[test]
fn index_insertion_removes_covered_ranges_and_duplicate_starts() {
    let key = ObjectKeyHash::from("object");
    let (registry, backend) = crate::test_metrics::registry();
    let mut index = DiskEntryIndex::new(DiskMetrics::new(&backend));
    let entries = [range(10, 20), range(0, 30), range(15, 16), range(0, 30), range(0, 40)].map(|object_range| {
        (
            key,
            DiskEntry {
                in_flight_read: Weak::new(),
                eviction_position: 0,
                object_range,
                payload_checksum: 0,
                payload_range: 0..4096,
                metadata: (0, 0),
            },
        )
    });
    let removed: usize = entries
        .into_iter()
        .map(|(key, entry)| index.insert(key, entry).len())
        .sum();
    assert_eq!(removed, 4);
    assert_eq!(crate::test_metrics::value(&registry, "feuer_disk_entries", &[]), 1.0);
    assert_eq!(
        crate::test_metrics::value(&registry, "feuer_disk_payload_bytes", &[]),
        40.0
    );
    assert_eq!(index.entries_by_key[&key].len(), 1);
    assert_eq!(index.entries_by_key[&key][&0].object_range, range(0, 40));
    assert_eq!(index.eviction_candidates.len(), 1);
    for (position, (key, start)) in index.eviction_candidates.iter().enumerate() {
        assert_eq!(index.entries_by_key[key][start].eviction_position, position);
    }
}

#[test]
fn invalidation_preserves_different_contents_but_may_discard_an_identical_replacement() {
    let read = PayloadRead {
        object_range: range(0, 3),
        payload_checksum: XxHash64::oneshot(0, b"old"),
        payload_range: 8192..12288,
    };
    for replacement in [b"old", b"new"] {
        let mut index = DiskEntryIndex::new(DiskMetrics::noop());
        index.insert(
            ObjectKeyHash::from("object"),
            DiskEntry {
                in_flight_read: Weak::new(),
                eviction_position: 0,
                object_range: range(0, 3),
                payload_checksum: XxHash64::oneshot(0, replacement),
                payload_range: 8192..12288,
                metadata: (0, 0),
            },
        );
        index.remove_entry_matching_read(&ObjectKeyHash::from("object"), &read);
        assert_eq!(
            index
                .covering_entry(&ObjectKeyHash::from("object"), range(0, 3))
                .is_some(),
            replacement != b"old"
        );
    }
}

#[test]
fn metadata_pages_use_writer_layout_and_validate_checksum_and_tag() {
    let mut page = vec![0xff; METADATA_PAGE_BYTES];
    let tag = page_format::ENTRY_METADATA_PAGE_TAG;
    let count = page_format::ENTRIES_PER_METADATA_PAGE as u64;
    let contents = b"entry metadata contents";
    page_format::encode_page(&mut page, tag, count, contents);
    let validate = |bytes: &[u8]| page_format::validate_page(bytes, tag).is_some();
    assert!(validate(&page));
    assert_eq!(&page[8..16], &[0; 8]);
    assert_eq!(&page[16..24], &count.to_le_bytes());
    assert_eq!(&page[24..32], tag);
    assert!(page[32..].starts_with(contents));
    assert!(page[32 + contents.len()..].iter().all(|&byte| byte == 0));
    for offset in [0, 8, 16, 24, 32, 40, 48, METADATA_PAGE_BYTES - 1] {
        let mut torn = page.clone();
        torn[offset] ^= 1;
        assert!(!validate(&torn));
    }
    for other_tag in [b"FEUDES09", page_format::NEXT_CHUNK_PAGE_TAG] {
        page_format::encode_page(&mut page, other_tag, count, contents);
        assert!(!validate(&page));
    }
}
