use super::*;
use feuer_memory::BUFFER_ALIGNMENT;
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

/// Detach a freshly opened cache from periodic writers so tests can drive writes themselves.
pub(super) fn with_manual_metadata_writes(cache: DiskCache) -> DiskCache {
    DiskCache {
        disk: Arc::new(Arc::try_unwrap(cache.disk).ok().unwrap()),
        write_sender: cache.write_sender,
    }
}

impl DiskEntry {
    fn single_chunk_start(&self) -> Option<u64> {
        (self.chunk_count() == 1).then_some(self.payload_address / CHUNK_BYTES * CHUNK_BYTES)
    }
}

pub(super) async fn entry_disk_ranges(
    cache: &DiskCache,
    key: &ObjectKeyHash,
) -> (std::ops::Range<u64>, std::ops::Range<u64>) {
    let shard = &cache.disk.shards[cache.disk.shard_index_for_key(key)];
    let disk_index = shard.entry_index.lock().unwrap();
    let entry = disk_index.entries_by_key[key].first_key_value().unwrap().1;
    let location = entry.metadata;
    let pages = shard.metadata_pages.lock().unwrap();
    let address = pages.chunks[location.chunk_index as usize]
        .reserved_chunk
        .disk_byte_range()
        .start
        + (location.entry_index as usize / page_format::ENTRIES_PER_METADATA_PAGE * METADATA_PAGE_BYTES) as u64;
    let metadata = address..address + METADATA_PAGE_BYTES as u64;
    let payload = entry.payload_address..entry.payload_address + payload_disk_bytes(entry.object_range.len());
    (payload, metadata)
}

#[tokio::test]
async fn rejects_invalid_capacity() {
    let directory = tempfile::tempdir().unwrap();
    for capacity in [0, 1, CHUNK_BYTES - 1, 1 << 63, u64::MAX] {
        let result = DiskCache::open(directory.path(), capacity, IoMetrics::noop()).await;
        assert!(matches!(result, Err(DiskCacheError::InvalidCapacity)));
    }
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
            cache.fetch_from_disk(&key, request).await.unwrap(),
            (source.bytes_in_range(request), 32 * 1024)
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
    let disk_index = cache.disk.shards[0].entry_index.lock().unwrap();
    assert_eq!(disk_index.entries_by_key[&key].len(), 1);
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
        let (allocated, entry_metadata) = entry_disk_ranges(&cache, &key).await;
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
        assert_eq!(entry_metadata, 0..METADATA_PAGE_BYTES as u64);
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
    let large_start = entry_disk_ranges(&cache, &ObjectKeyHash::from("large")).await.0.start;
    let metadata_end = METADATA_PAGE_BYTES as u64;
    for i in 0..130 {
        let key = ObjectKeyHash::from(format!("small-{i}"));
        assert_eq!(
            cache.get(&key, range(i, i + 1024)).await.unwrap(),
            download(i, 1024).bytes()
        );
        let (payload, metadata) = entry_disk_ranges(&cache, &key).await;
        assert!((2 * CHUNK_BYTES..3 * CHUNK_BYTES).contains(&payload.start));
        assert!(metadata.end <= 2 * metadata_end);
    }
    assert_eq!(large_start, CHUNK_BYTES);
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
    let inputs: Vec<_> = [
        600_000,
        600_000,
        CHUNK_BYTES as usize,
        0,
        CHUNK_BYTES as usize + 1,
        1,
        512 * 1024 - BUFFER_ALIGNMENT,
        512 * 1024 - 1,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, length)| (ObjectKeyHash(i as u128), download(0, length)))
    .collect();
    assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), inputs.len());
    let disk_index = cache.disk.shards[0].entry_index.lock().unwrap();
    let mut owners = BTreeMap::<u64, Vec<&DiskEntry>>::new();
    for entries in disk_index.entries_by_key.values() {
        for entry in entries.values() {
            let first_chunk = entry.payload_address / CHUNK_BYTES;
            let end_chunk =
                (entry.payload_address + payload_disk_bytes(entry.object_range.len())).div_ceil(CHUNK_BYTES);
            assert_eq!(entry.chunk_count(), end_chunk - first_chunk);
            for chunk in first_chunk..end_chunk {
                owners.entry(chunk).or_default().push(entry);
            }
        }
    }
    for entries in owners.values() {
        if entries.len() > 1 {
            assert!(entries.iter().all(|entry| entry.single_chunk_start().is_some()
                && payload_disk_bytes(entry.object_range.len()) < 512 * 1024));
        }
    }
    assert_eq!(owners.len(), 7);
}

#[tokio::test]
async fn full_and_no_fit_chunks_flush_across_metadata_pages() {
    for length in [4096, 12 * 1024] {
        let (_directory, cache) = open_test_cache(3 * CHUNK_BYTES).await;
        let count = CHUNK_BYTES as usize / length;
        let source = download(0, length);
        let mut published = 0;
        for i in 0..count {
            assert_eq!(cache.covers_range(&ObjectKeyHash(0), range(0, 1)), i != 0);
            let key = ObjectKeyHash(i as u128);
            published += cache.write(key, source.clone(), ()).await.unwrap();
        }
        assert_eq!(published, if length == 4096 { count } else { 0 });
        let next = ObjectKeyHash(count as u128);
        published += cache.write(next, source.clone(), ()).await.unwrap();
        assert_eq!(published, count);
        // A larger entry publishes without flushing the new partial chunk.
        let large = download(0, 512 * 1024 - 1);
        assert_eq!(cache.write(ObjectKeyHash(999), large, ()).await.unwrap(), 1);
        let buffered = cache.get(&next, range(1, 17)).await.unwrap();
        for i in 0..count {
            let key = ObjectKeyHash(i as u128);
            let (payload, _) = entry_disk_ranges(&cache, &key).await;
            assert_eq!(payload.start, CHUNK_BYTES + (i * length) as u64);
            assert_eq!(
                cache.get(&key, source.downloaded_range()).await.unwrap(),
                source.bytes()
            );
        }
        cache.discard_pending().await;
        assert!(!cache.covers_range(&next, range(0, 1)));
        assert_eq!(buffered, source.bytes().slice(1..17));
        assert_eq!(cache.disk.shards[0].allocator.available_bytes(), CHUNK_BYTES);
    }
}

#[tokio::test]
async fn failed_large_reservation_leaves_buffered_entries_and_metadata_positions_untouched() {
    let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
    let shard = &cache.disk.shards[0];
    let source = download(0, 4097);
    let bytes = source.bytes().clone();
    assert_eq!(cache.write(ObjectKeyHash(1), source, ()).await.unwrap(), 0);
    assert!(bytes.is_unique(), "the writer must release the incoming buffer");
    assert_eq!(shard.allocator.available_bytes(), 2 * CHUNK_BYTES);
    assert!(shard.metadata_pages.lock().unwrap().chunks.is_empty());
    assert!(cache.covers_range(&ObjectKeyHash(1), range(1, 4097)));
    assert!(!cache.covers_range(&ObjectKeyHash(1), range(0, 4098)));
    assert!(!cache.covers_range(&ObjectKeyHash(2), range(0, 1)));
    assert_eq!(
        cache
            .write(ObjectKeyHash(2), download(0, 2 * CHUNK_BYTES as usize), ())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        shard.metadata_pages.lock().unwrap().free_position_count(),
        page_format::ENTRIES_PER_METADATA_CHUNK
    );
    assert_eq!(cache.get(&ObjectKeyHash(1), range(0, 4097)).await.unwrap(), bytes);
    let start = u64::MAX - 1;
    cache.write(ObjectKeyHash(3), download(start, 1), ()).await.unwrap();
    assert_eq!(
        cache.get(&ObjectKeyHash(3), range(start, start + 1)).await.unwrap(),
        download(start, 1).bytes()
    );
    assert!(
        cache
            .get(&ObjectKeyHash(3), range(start - 1, start + 1))
            .await
            .is_none()
    );
    assert_eq!(cache.flush().await.unwrap(), 2);
    assert_eq!(
        entry_disk_ranges(&cache, &ObjectKeyHash(1)).await.0,
        CHUNK_BYTES..CHUNK_BYTES + 8192
    );
    assert_eq!(
        entry_disk_ranges(&cache, &ObjectKeyHash(3)).await.0.start,
        CHUNK_BYTES + 8192
    );
    assert_eq!(cache.get(&ObjectKeyHash(1), range(0, 4097)).await.unwrap(), bytes);
}

#[tokio::test]
async fn cancellation_or_lost_metadata_capacity_discards_the_whole_flush() {
    for cancel in [true, false] {
        let (_directory, cache) = open_test_cache(CHUNK_BYTES).await;
        for key in [ObjectKeyHash(1), ObjectKeyHash(2)] {
            cache.write(key, download(0, 1), ()).await.unwrap();
        }
        let mut flush = Box::pin(cache.flush());
        if let Poll::Ready(result) = std::future::poll_fn(|cx| Poll::Ready(flush.as_mut().poll(cx))).await {
            assert_eq!(result.unwrap(), 2);
            continue;
        }
        let shard = &cache.disk.shards[0];
        assert_eq!(
            shard.metadata_pages.lock().unwrap().free_position_count(),
            page_format::ENTRIES_PER_METADATA_CHUNK
        );
        if cancel {
            drop(flush);
        } else {
            // Simulate another publisher consuming positions during I/O. One slot cannot admit both entries.
            {
                let mut pages = shard.metadata_pages.lock().unwrap();
                for _ in 1..page_format::ENTRIES_PER_METADATA_CHUNK {
                    pages.get_free_metadata_location().unwrap();
                }
            }
            assert_eq!(flush.await.unwrap(), 0);
        }
        assert_eq!(shard.allocator.available_bytes(), CHUNK_BYTES);
        assert_eq!(
            shard.metadata_pages.lock().unwrap().free_position_count(),
            if cancel {
                page_format::ENTRIES_PER_METADATA_CHUNK
            } else {
                1
            }
        );
        assert!(shard.entry_index.lock().unwrap().entries_by_key.is_empty());
        assert_eq!(cache.flush().await.unwrap(), 0);
    }
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
        let disk_index = cache.disk.shards[0].entry_index.lock().unwrap();
        assert_eq!(
            disk_index.eviction_candidates.len(),
            disk_index.entries_by_key.values().map(BTreeMap::len).sum::<usize>()
        );
        for (position, (key, start)) in disk_index.eviction_candidates.iter().enumerate() {
            assert_eq!(disk_index.entries_by_key[key][start].eviction_position, position);
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
    let read = cache.disk.shards[0]
        .entry_index
        .lock()
        .unwrap()
        .covering_entry(&key, source.downloaded_range())
        .unwrap()
        .share_payload_read();
    assert!(
        cache
            .insert(ObjectKeyHash::from("new"), download(0, CHUNK_BYTES as usize))
            .await
            .unwrap()
    );
    assert!(cache.get(&key, range(3, 4)).await.is_none());
    let result = read.read_and_verify_payload(&cache.disk.file).await;
    assert!(matches!(result, Err(DiskLookupOutcome::ChecksumFailed)));
}

#[tokio::test]
async fn eviction_budgets_and_active_reservations_bound_reclamation() {
    let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("large");
    cache.insert(key, download(0, CHUNK_BYTES as usize + 1)).await.unwrap();
    let shard = &cache.disk.shards[0];
    let mut remaining_eviction_attempts = 0;
    let mut remaining_eviction_chunk_budget = MAX_EVICTION_CHUNKS;
    assert!(!shard.sample_and_evict_entry(
        &cache.disk,
        &mut remaining_eviction_attempts,
        &mut remaining_eviction_chunk_budget
    ));
    remaining_eviction_attempts = 1;
    remaining_eviction_chunk_budget = 1;
    assert!(shard.sample_and_evict_entry(
        &cache.disk,
        &mut remaining_eviction_attempts,
        &mut remaining_eviction_chunk_budget
    ));
    assert_eq!(remaining_eviction_attempts, 0);
    assert!(cache.get(&key, range(0, 1)).await.is_some());
    remaining_eviction_attempts = 1;
    remaining_eviction_chunk_budget = MAX_EVICTION_CHUNKS;
    assert!(shard.sample_and_evict_entry(
        &cache.disk,
        &mut remaining_eviction_attempts,
        &mut remaining_eviction_chunk_budget
    ));
    assert_eq!(remaining_eviction_chunk_budget, MAX_EVICTION_CHUNKS - 2);
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
    let mut remaining_eviction_attempts = 1;
    let mut remaining_eviction_chunk_budget = MAX_EVICTION_CHUNKS;
    assert!(shard.sample_and_evict_entry(
        &cache.disk,
        &mut remaining_eviction_attempts,
        &mut remaining_eviction_chunk_budget
    ));
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
async fn eviction_respects_sample_size_and_keeps_first_sampled_on_ties() {
    for sample_size in [1, RECLAIM_SAMPLE_SIZE] {
        let (_directory, mut cache) = open_test_cache(2 * CHUNK_BYTES).await;
        Arc::get_mut(&mut cache.disk).unwrap().reclaim_sample_size = sample_size;
        let older = ObjectKeyHash::from("small metadata");
        let newer = ObjectKeyHash::from("long key/".repeat(1000));
        cache.insert(older, download(0, 100)).await.unwrap();
        cache.insert(newer, download(0, 100)).await.unwrap();
        if sample_size == 1 {
            cache.access_histories().record_access(&newer, range(0, 1));
        }
        // Sample the newer entry first; a one-entry sample must not consider the cheaper older entry.
        cache.disk.shards[0].entry_index.lock().unwrap().next_sample_start = usize::MAX;
        assert!(
            cache
                .insert(ObjectKeyHash::from("incoming"), download(0, 1))
                .await
                .unwrap()
        );
        assert!(cache.get(&older, range(0, 1)).await.is_some());
        assert!(cache.get(&newer, range(0, 1)).await.is_none());
    }
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
    shard.remove_entry(&hot, 0);
    drop(memory);
    drop(cache);
    assert_eq!(history.recent_requested_ranges(&hot).len(), 5);
    assert!(history.decayed_retrieval_cost(&hot, range(0, 100)) > 0.0);
}

#[tokio::test]
async fn admission_eviction_limit_does_not_force_out_a_shared_chunks_last_entry() {
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
    assert_eq!(payload.end - payload.start, BUFFER_ALIGNMENT as u64);
    // A read reaching beyond this entry's alignment boundary would now fail with short I/O.
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(payload.end)
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
    let address = payload.start;
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
    let free_metadata_slots = shard.metadata_pages.lock().unwrap().free_position_count();
    shard.remove_entry(&first, 3);
    assert!(!cache.covers_range(&first, range(3, 1027)));
    assert_eq!(
        shard.metadata_pages.lock().unwrap().free_position_count(),
        free_metadata_slots + 1
    );
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
    assert!(metadata.end <= payload.start);
    assert!(payload.start.is_multiple_of(CHUNK_BYTES));
    // Read the disk bytes directly across physical chunk boundaries: no assembly or skipped headers.
    assert_eq!(
        cache
            .disk
            .file
            .read_at(payload.start, source.bytes().len())
            .await
            .unwrap(),
        source.bytes()
    );
    let boundary = 100 + CHUNK_BYTES - payload.start % CHUNK_BYTES;
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
    shard
        .metadata_pages
        .lock()
        .unwrap()
        .ensure_free_positions(1, &shard.allocator)
        .unwrap();
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
    shard.remove_entry(&key, 0);
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
    assert_eq!(entry_metadata, 0..METADATA_PAGE_BYTES as u64);
    assert_eq!(payload.start, CHUNK_BYTES);
    cache.write_dirty_metadata_pages().await;
    let page = cache.disk.file.read_at(0, METADATA_PAGE_BYTES).await.unwrap();
    let entry_metadata_bytes = page_format::validate_page(&page, page_format::ENTRY_METADATA_PAGE_TAG).unwrap();
    let read_u64 = |offset| u64::from_le_bytes(entry_metadata_bytes[offset..offset + 8].try_into().unwrap());
    assert_eq!(&entry_metadata_bytes[..16], &key.0.to_le_bytes());
    assert_eq!(read_u64(16), 17);
    assert_eq!(read_u64(24), source.downloaded_range().len());
    assert_eq!(read_u64(32), payload.start);
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
    let (_, metadata) = entry_disk_ranges(&cache, &key).await;
    assert_eq!(metadata.end - metadata.start, METADATA_PAGE_BYTES as u64);
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), 2 * CHUNK_BYTES);
    assert!(
        cache
            .insert(ObjectKeyHash::from("other"), download(0, 1))
            .await
            .unwrap()
    );
    assert_eq!(cache.disk.shards[0].allocator.available_bytes(), CHUNK_BYTES);
    assert_eq!(cache.get(&key, range(0, 1)).await.unwrap(), download(0, 1).bytes());
    let shard = &cache.disk.shards[0];
    shard.remove_entry(&key, 0);
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
            (small, download(0, 512 * 1024)),
            (large, download(0, CHUNK_BYTES as usize + 1)),
        ])
        .await
        .unwrap();
    // Exclusive entries publish immediately, so the next entry can evict them and reuse their chunks.
    assert_eq!(published, 2);
    assert!(!cache.covers_range(&small, range(0, 1)));
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
    assert_eq!(cache.write_sender.strong_count(), 1);
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
    let (address, payload_checksum) = {
        let mut disk_index = cache.disk.shards[0].entry_index.lock().unwrap();
        let entry = disk_index.covering_entry(&key, range(6, 7)).unwrap();
        (entry.payload_address, entry.payload_checksum)
    };
    let result = cache.get(&key, range(6, 7)).await.unwrap();
    cache.insert(key, download(0, 200)).await.unwrap();
    let bytes = cache.disk.file.read_at(address, BUFFER_ALIGNMENT).await.unwrap();
    assert_eq!(XxHash64::oneshot(0, &bytes[..100]), payload_checksum);
    let reused = cache.disk.shards[0].allocator.reserve_chunks(1).unwrap();
    assert_eq!(reused.disk_byte_range(), CHUNK_BYTES..2 * CHUNK_BYTES);
    assert_eq!(result, download(6, 1).bytes());
}

#[tokio::test]
async fn corrupted_and_reused_payload_miss_and_invalidate_the_entry() {
    for stale in [false, true] {
        let (_directory, cache) = open_test_cache(2 * CHUNK_BYTES).await;
        let key = ObjectKeyHash::from("object");
        cache.insert(key, download(3, 50)).await.unwrap();
        let (payload, _) = entry_disk_ranges(&cache, &key).await;
        let address = payload.start;
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
                .read_at(other_payload.start, BUFFER_ALIGNMENT)
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
    // The first exclusive entry succeeds; the large entry's reserved range exceeds the file.
    let mut disk = Arc::try_unwrap(cache.disk).ok().unwrap();
    disk.shards[0].allocator = DiskChunkAllocator::for_disk_range(0..4 * CHUNK_BYTES);
    let cache = DiskCache {
        disk: Arc::new(disk),
        write_sender: cache.write_sender,
    };
    assert!(matches!(
        cache
            .insert_batch(vec![
                (ObjectKeyHash::from("small"), download(0, 512 * 1024)),
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
        shard.metadata_pages.lock().unwrap().free_position_count(),
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
        let (bytes, capacity) = cache.fetch_from_disk(&key, source.downloaded_range()).await.unwrap();
        assert_eq!(capacity, 32 * CHUNK_BYTES as usize);
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
    let boundary = 3 + CHUNK_BYTES - entry_disk_ranges(&cache, &key).await.0.start % CHUNK_BYTES;
    for (request, capacity) in [
        (source.downloaded_range(), 100 * CHUNK_BYTES as usize + BUFFER_ALIGNMENT),
        (
            range(3, 3 + 80 * CHUNK_BYTES),
            100 * CHUNK_BYTES as usize + BUFFER_ALIGNMENT,
        ),
        (range(3, 3 + 75 * CHUNK_BYTES), 75 * CHUNK_BYTES as usize),
        (range(7, 33), 32 * 1024),
        (range(boundary - 3, boundary + 13), 32 * 1024),
        (
            range(source.downloaded_range().end() - 17, source.downloaded_range().end()),
            32 * 1024,
        ),
    ] {
        assert_eq!(
            cache.fetch_from_disk(&key, request).await.unwrap(),
            (source.bytes_in_range(request), capacity)
        );
    }
    let (payload, _) = entry_disk_ranges(&cache, &key).await;
    let last_aligned_offset = payload.end - BUFFER_ALIGNMENT as u64;
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
    let (directory, cache) = open_test_cache(256 * CHUNK_BYTES + 1).await;
    assert_eq!(cache.disk.shards.len(), 2);
    let keys = [ObjectKeyHash(0), ObjectKeyHash(1)];
    for key in keys {
        assert_eq!(cache.write(key, download(0, 100), ()).await.unwrap(), 0);
        assert!(cache.covers_range(&key, range(0, 100)));
    }
    // An explicit batch shares and flushes the chunks already buffered on both shards.
    assert_eq!(cache.insert_batch(vec![(keys[0], download(0, 100))]).await.unwrap(), 2);
    let first = entry_disk_ranges(&cache, &keys[0]).await.0;
    let second = entry_disk_ranges(&cache, &keys[1]).await.0;
    assert!(first.end <= 128 * CHUNK_BYTES);
    assert!(second.start >= 128 * CHUNK_BYTES);
    let returned = cache.get(&keys[0], range(0, 100)).await.unwrap();
    cache.write_dirty_metadata_pages().await;
    let capacity = cache.disk.file.capacity();
    drop(cache);
    let reopened = DiskCache::open(directory.path(), capacity, IoMetrics::noop())
        .await
        .unwrap();
    assert!(keys.iter().all(|key| reopened.covers_range(key, range(0, 100))));
    for key in &keys {
        assert_eq!(
            reopened.get(key, range(0, 100)).await.unwrap(),
            download(0, 100).bytes()
        );
    }
    assert!(!reopened.insert(keys[0], download(0, 100)).await.unwrap());
    assert_eq!(returned, download(0, 100).bytes());
}

#[tokio::test]
async fn insertion_releases_contained_entries_and_preserves_covering_entries() {
    let key = ObjectKeyHash::from("object");
    let (registry, backend) = crate::test_metrics::registry();
    let directory = tempfile::tempdir().unwrap();
    let cache = DiskCache::open_with_metrics(
        directory.path(),
        3 * CHUNK_BYTES,
        IoMetrics::noop(),
        Arc::new(ObjectAccessHistories::new()),
        DiskMetrics::new(&backend),
        RECLAIM_SAMPLE_SIZE,
    )
    .await
    .unwrap();
    let cache = with_manual_metadata_writes(cache);
    for (object_range, inserted) in [
        (range(10, 20), true),
        (range(0, 30), true),
        (range(15, 16), false),
        (range(0, 30), false),
        (range(0, 40), true),
    ] {
        assert_eq!(
            cache
                .insert(key, download(object_range.start(), object_range.len() as usize))
                .await
                .unwrap(),
            inserted
        );
        let shard = &cache.disk.shards[0];
        assert_eq!(shard.allocator.available_bytes(), CHUNK_BYTES);
        assert_eq!(
            shard.metadata_pages.lock().unwrap().free_position_count(),
            page_format::ENTRIES_PER_METADATA_CHUNK - 1
        );
    }
    let disk_index = cache.disk.shards[0].entry_index.lock().unwrap();
    assert_eq!(crate::test_metrics::value(&registry, "feuer_disk_entries", &[]), 1.0);
    assert_eq!(
        crate::test_metrics::value(&registry, "feuer_disk_payload_bytes", &[]),
        40.0
    );
    assert_eq!(disk_index.entries_by_key[&key].len(), 1);
    assert_eq!(disk_index.entries_by_key[&key][&0].object_range, range(0, 40));
    assert_eq!(disk_index.eviction_candidates.len(), 1);
    for (position, (key, start)) in disk_index.eviction_candidates.iter().enumerate() {
        assert_eq!(disk_index.entries_by_key[key][start].eviction_position, position);
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
