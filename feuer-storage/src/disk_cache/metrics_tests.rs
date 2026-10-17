use std::{future::Future, pin::Pin, task::Poll};

use super::*;
use crate::test_metrics::{registry, value};

fn read_byte_count(registry: &prometheus::Registry) -> f64 {
    value(registry, "feuer_disk_io_bytes_total", &[("operation", "read")])
}

async fn assert_pending(mut future: Pin<&mut impl Future>) {
    let result = std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await;
    assert!(result.is_pending());
}

fn download(length: usize) -> Download {
    Download::new(0, Bytes::from(vec![7; length])).unwrap()
}

async fn measured_cache(capacity: u64) -> (tempfile::TempDir, DiskCache, prometheus::Registry) {
    let directory = tempfile::tempdir().unwrap();
    let (registry, backend) = registry();
    let cache = DiskCache::open(
        directory.path(),
        capacity + CHUNK_BYTES,
        DiskCacheOptions {
            io_metrics: IoMetrics::new(&backend),
            metrics: DiskMetrics::new(&backend),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (directory, tests::with_manual_metadata_writes(cache), registry)
}

#[tokio::test]
async fn periodic_writer_combines_updates_into_page_writes() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    for key in [ObjectKeyHash(1), ObjectKeyHash(2)] {
        cache.insert(key, download(1)).await.unwrap();
    }
    let written_bytes = || value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]);
    let payload_bytes = 2 * CHUNK_BYTES;
    let header_bytes = 4096;
    assert_eq!(written_bytes(), (header_bytes + CHUNK_BYTES + payload_bytes) as f64);
    assert_eq!(read_byte_count(&registry), (header_bytes + CHUNK_BYTES) as f64);
    cache.write_dirty_metadata_pages().await;
    assert_eq!(written_bytes(), (header_bytes + 2 * CHUNK_BYTES + payload_bytes) as f64);
    for key in [ObjectKeyHash(1), ObjectKeyHash(2)] {
        cache.disk.shards[0].remove_entry(&key, 0);
    }
    let before = written_bytes();
    let weak = Arc::downgrade(&cache.disk);
    tokio::spawn(DiskCacheInner::write_metadata_periodically(weak.clone()));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        // Both removals share one page in an initialized chunk.
        while written_bytes() < before + METADATA_PAGE_BYTES as f64 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(written_bytes(), before + METADATA_PAGE_BYTES as f64);
    drop(cache);
    assert!(
        weak.upgrade().is_none(),
        "the sleeping writer must not retain the cache"
    );
}

#[tokio::test]
async fn recovery_reads_one_full_metadata_chunk_independent_of_payload_size() {
    for (entries, payload_bytes, chunks) in [
        (1, 1024, 1),
        (84, 1024, 1),
        (85, 1024, 1),
        (168, 1024, 1),
        (169, 1024, 1),
        (252, 1024, 1),
        (1, CHUNK_BYTES as usize - METADATA_PAGE_BYTES, 1),
        (1, 2 * CHUNK_BYTES as usize, 1),
        (page_format::ENTRIES_PER_METADATA_CHUNK as u128 + 1, 1, 2),
    ] {
        let capacity = if chunks == 2 { 100 } else { 4 } * CHUNK_BYTES;
        let (directory, cache) = tests::open_test_cache(capacity).await;
        let source = download(payload_bytes);
        let inputs: Vec<_> = (0..entries).map(|key| (ObjectKeyHash(key), source.clone())).collect();
        assert_eq!(cache.insert_batch(inputs.clone()).await.unwrap(), entries as usize);
        cache.write_dirty_metadata_pages().await;
        let capacity = cache.disk.file.capacity();
        drop(cache);

        let (registry, backend) = registry();
        let cache = DiskCache::open(
            directory.path(),
            capacity,
            DiskCacheOptions {
                io_metrics: IoMetrics::new(&backend),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            value(
                &registry,
                "feuer_disk_io_total",
                &[("operation", "read"), ("outcome", "success")]
            ),
            (chunks + 1) as f64
        );
        assert_eq!(
            value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]),
            (4096 + chunks * CHUNK_BYTES) as f64
        );
        assert!(
            cache
                .disk
                .shards
                .iter()
                .all(|shard| shard.metadata_pages.lock().unwrap().dirty_pages.is_empty())
        );
        for (key, source) in inputs {
            assert!(cache.covers_range(&key, source.downloaded_range()));
        }
    }
}

#[tokio::test]
async fn recovered_chunk_gauge_counts_shared_and_multi_chunk_ownership() {
    let capacity = 8 * CHUNK_BYTES;
    let (directory, cache, registry) = measured_cache(capacity).await;
    let inputs = vec![
        (ObjectKeyHash::from("small-a"), download(4)),
        (ObjectKeyHash::from("small-b"), download(8)),
        (ObjectKeyHash::from("large"), download(2 * CHUNK_BYTES as usize + 17)),
    ];
    cache.insert_batch(inputs.clone()).await.unwrap();
    assert_eq!(
        value(
            &registry,
            "feuer_disk_io_total",
            &[("operation", "write"), ("outcome", "success")]
        ),
        4.0 // Chain clearing, global header, one shared payload run, and one three-chunk payload run.
    );
    assert_eq!(
        value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]),
        (4096 + 5 * CHUNK_BYTES) as f64
    );
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), 0.0);
    let written_chunks = value(&registry, "feuer_disk_chunks", &[("state", "allocated")]);
    assert_eq!(written_chunks, 5.0); // Metadata, one shared payload chunk, and three large-entry chunks.
    let metrics = cache.disk.metrics.clone();
    cache.write_dirty_metadata_pages().await;
    drop(cache);

    let cache = DiskCache::open(
        directory.path(),
        capacity + CHUNK_BYTES,
        DiskCacheOptions {
            metrics,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), written_chunks);
    for (key, source) in inputs {
        assert_eq!(
            cache.get(&key, source.downloaded_range()).await.unwrap(),
            source.bytes()
        );
    }
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), written_chunks);
    drop(cache);
    assert_eq!(value(&registry, "feuer_disk_recovered_chunks", &[]), 0.0);
}

#[tokio::test]
async fn subrange_checksums_reduce_read_bytes_without_writes_and_reset_on_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let key = ObjectKeyHash::from("partial");
    let source = tests::download(u64::MAX - 3 * CHUNK_BYTES, 2 * CHUNK_BYTES as usize + 17);
    let start = source.downloaded_range().start();
    let request = tests::range(start + 4093, start + 4093 + 65_543);
    let nested_request = tests::range(request.start() + 7, request.start() + 40_007);
    // Alignment padding pushes this read into the 64-KiB buffer bucket before copying shrinks it.
    let unaligned_32_kib_request = tests::range(start + 1, start + 1 + 32 * 1024);
    for recovered in [false, true] {
        let (registry, backend) = registry();
        let cache = DiskCache::open(
            directory.path(),
            5 * CHUNK_BYTES,
            DiskCacheOptions {
                io_metrics: IoMetrics::new(&backend),
                metrics: DiskMetrics::new(&backend),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cache = tests::with_manual_metadata_writes(cache);
        if !recovered {
            cache.insert(key, source.clone()).await.unwrap();
        }
        let written_bytes = value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]);
        for (range, expected_read_bytes) in [
            (request, source.downloaded_range().len()),
            (request, request.len()),
            (nested_request, request.len()),
            (nested_request, nested_request.len()),
            (unaligned_32_kib_request, source.downloaded_range().len()),
            (unaligned_32_kib_request, unaligned_32_kib_request.len()),
            (source.downloaded_range(), source.downloaded_range().len()),
        ] {
            let before = read_byte_count(&registry);
            let (bytes, capacity) = cache.fetch_from_disk(&key, range).await.unwrap();
            assert_eq!(bytes, source.bytes_in_range(range));
            assert_eq!(capacity, BufferPool::allocation_capacity(bytes.len()));
            assert_eq!(read_byte_count(&registry) - before, expected_read_bytes as f64);
        }
        assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
        assert_eq!(
            value(&registry, "feuer_disk_payload_bytes", &[]),
            source.bytes().len() as f64
        );
        assert_eq!(
            value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]),
            written_bytes
        );
        cache.write_dirty_metadata_pages().await;
    }
}

#[tokio::test]
async fn writer_subrange_checksums_cover_recent_requests_until_restart() {
    // Exercise a packed payload and a request crossing a payload chunk boundary.
    for (payload_len, request_offset, payload_chunks) in [
        (96 * 1024 + 17, 4093, 1),
        (2 * CHUNK_BYTES as usize + 17, CHUNK_BYTES - 4093, 4),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let key = ObjectKeyHash::from("writer checksums");
        let neighbor = ObjectKeyHash::from("packed neighbor");
        let source = tests::download(u64::MAX - 4 * CHUNK_BYTES, payload_len);
        let start = source.downloaded_range().start();
        let end = source.downloaded_range().end();
        let request = tests::range(start + request_offset, start + request_offset + 65_543);
        let nested_request = tests::range(request.start() + 7, request.start() + 7 + 3006);
        // Newer requests remove this range from the key's history. Another key still has it.
        let range_removed_from_history = tests::range(start + 101, start + 103);
        let access_histories = Arc::new(ObjectAccessHistories::new());
        access_histories.record_access(&key, range_removed_from_history);
        for _ in 0..*feuer_historian::MAX_ACCESS_EVENTS_PER_KEY {
            access_histories.record_access(&key, source.downloaded_range());
        }
        access_histories.record_access(&neighbor, range_removed_from_history);
        for range in [
            request,
            nested_request,
            request,
            tests::range(end, end),
            tests::range(start - 1, start + 7),
            tests::range(end - 7, end + 1),
        ] {
            access_histories.record_access(&key, range);
        }
        for recovered in [false, true] {
            let (registry, backend) = registry();
            let cache = DiskCache::open(
                directory.path(),
                6 * CHUNK_BYTES,
                DiskCacheOptions {
                    access_histories: access_histories.clone(),
                    io_metrics: IoMetrics::new(&backend),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let cache = tests::with_manual_metadata_writes(cache);
            if !recovered {
                let read_bytes_before = read_byte_count(&registry);
                let written_bytes_before = value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]);
                assert_eq!(
                    cache
                        .insert_batch(vec![(key, source.clone()), (neighbor, tests::download(23, 8192))])
                        .await
                        .unwrap(),
                    2
                );
                assert_eq!(read_byte_count(&registry), read_bytes_before);
                assert_eq!(
                    value(&registry, "feuer_disk_io_bytes_total", &[("operation", "write")]) - written_bytes_before,
                    (payload_chunks * CHUNK_BYTES) as f64
                );
            }
            {
                let index = cache.disk.shards[0].entry_index.lock().unwrap();
                let checksums = &index.entries_by_key[&key][&start].subrange_checksums;
                assert_eq!(checksums.len(), if recovered { 0 } else { 2 });
            }
            for range in [nested_request, request, range_removed_from_history] {
                let expected_read_bytes = if recovered || range == range_removed_from_history {
                    payload_len as u64
                } else {
                    range.len()
                };
                let read_bytes_before = read_byte_count(&registry);
                assert_eq!(cache.get(&key, range).await.unwrap(), source.bytes_in_range(range));
                let read_bytes = read_byte_count(&registry) - read_bytes_before;
                assert_eq!(read_bytes, expected_read_bytes as f64);
            }
            cache.write_dirty_metadata_pages().await;
        }
    }
}

#[tokio::test]
async fn discarded_writes_leave_covering_entry_checksums_unchanged() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("discarded writer checksums");
    let request_before_publication = tests::range(117, 217);
    let request_after_publication = tests::range(317, 417);
    cache.access_histories().record_access(&key, request_before_publication);
    assert_eq!(cache.write(key, tests::download(17, 8192), ()).await.unwrap(), 0);
    let source = tests::download(17, CHUNK_BYTES as usize);
    assert_eq!(cache.write(key, source.clone(), ()).await.unwrap(), 1);
    // The pending small entry loses publication to the larger entry. Its newer checksums must be discarded.
    cache.access_histories().record_access(&key, request_after_publication);
    assert_eq!(cache.flush().await.unwrap(), 0);
    for (range, expected_read_bytes) in [
        (request_before_publication, request_before_publication.len()),
        (request_after_publication, CHUNK_BYTES),
    ] {
        let read_bytes_before = read_byte_count(&registry);
        assert_eq!(cache.get(&key, range).await.unwrap(), source.bytes_in_range(range));
        let read_bytes = read_byte_count(&registry) - read_bytes_before;
        assert_eq!(read_bytes, expected_read_bytes as f64);
    }
}

#[tokio::test]
async fn partial_reads_reject_corruption_with_whole_entry_or_subrange_checksums() {
    for subrange_checksum_source in ["none", "read", "write"] {
        let has_subrange_checksum = subrange_checksum_source != "none";
        let (_directory, cache, registry) = measured_cache(3 * CHUNK_BYTES).await;
        let key = ObjectKeyHash::from("corrupt partial");
        let source = tests::download(17, CHUNK_BYTES as usize);
        let request = tests::range(17 + 4093, 17 + 8197);
        if subrange_checksum_source == "write" {
            cache.access_histories().record_access(&key, request);
        }
        cache.insert(key, source.clone()).await.unwrap();
        if subrange_checksum_source == "read" {
            assert_eq!(cache.get(&key, request).await.unwrap(), source.bytes_in_range(request));
        }
        let address = cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key][&17].payload_address;
        // Without a subrange checksum, corruption outside the request still fails whole-entry verification.
        let corrupt_offset = if has_subrange_checksum { 4096 } else { 12_288 };
        cache
            .disk
            .file
            .write_at(address + corrupt_offset, &Bytes::from(vec![0; 4096]))
            .await
            .unwrap();
        let before = read_byte_count(&registry);
        assert!(cache.get(&key, request).await.is_none());
        let expected_read_bytes = if has_subrange_checksum {
            request.len()
        } else {
            CHUNK_BYTES
        };
        assert_eq!(read_byte_count(&registry) - before, expected_read_bytes as f64);
        assert_eq!(
            value(&registry, "feuer_disk_lookup_total", &[("outcome", "checksum_failed")]),
            1.0
        );
        assert!(!cache.covers_range(&key, source.downloaded_range()));
        assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 0.0);
        assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 1.0);
        assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 2.0);
    }
}

#[tokio::test]
async fn shared_reads_survive_cancellation_and_learn_each_subrange_once() {
    for read_subrange in [false, true] {
        let (_directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
        let key = ObjectKeyHash::from("concurrent reads");
        let source = tests::download(17, CHUNK_BYTES as usize);
        let learned_range = tests::range(4110, 69_653);
        let request = tests::range(4111, 69_652);
        let suffix_request = tests::range(69_636, 69_652);
        cache.insert(key, source.clone()).await.unwrap();
        assert!(cache.get(&key, learned_range).await.is_some());
        // Hold initialization pending so all lookups join the same read.
        // A whole-entry read takes priority even when a smaller checksum is available.
        let read = cache.disk.shards[0]
            .entry_index
            .lock()
            .unwrap()
            .covering_entry(&key, request)
            .unwrap()
            .share_covering_read(read_subrange.then_some(learned_range));
        let expected_read_bytes = if read_subrange {
            learned_range.len()
        } else {
            CHUNK_BYTES
        };
        let mut initializer = Box::pin(read.result.get_or_init(std::future::pending));
        assert_pending(initializer.as_mut()).await;
        let mut first = Box::pin(cache.get(&key, request));
        let mut second = Box::pin(cache.get(&key, request));
        let mut suffix = Box::pin(cache.get(&key, suffix_request));
        let mut canceled = Box::pin(cache.get(&key, learned_range));
        for waiter in [&mut first, &mut second, &mut suffix, &mut canceled] {
            assert_pending(waiter.as_mut()).await;
        }
        drop(canceled);
        let weak = Arc::downgrade(&read);
        drop(initializer);
        drop(read);
        let before = read_byte_count(&registry);
        let (first, second, suffix) = tokio::join!(first, second, suffix);
        assert_eq!(first.unwrap(), source.bytes_in_range(request));
        assert_eq!(second.unwrap(), source.bytes_in_range(request));
        assert_eq!(suffix.unwrap(), source.bytes_in_range(suffix_request));
        assert!(weak.upgrade().is_none(), "completed reads must not retain the payload");
        assert_eq!(read_byte_count(&registry) - before, expected_read_bytes as f64);
        assert_eq!(
            cache.disk.shards[0].entry_index.lock().unwrap().entries_by_key[&key][&17]
                .subrange_checksums
                .len(),
            3
        );
        // Later lookups must read each learned range from disk rather than retain payload buffers.
        for request in [request, suffix_request] {
            let before = read_byte_count(&registry);
            assert_eq!(cache.get(&key, request).await.unwrap(), source.bytes_in_range(request));
            assert_eq!(read_byte_count(&registry) - before, request.len() as f64);
        }
    }
}

#[tokio::test]
async fn late_partial_reads_do_not_modify_replacement_entries() {
    for (checksum_from_write, replacement_bytes_differ) in [(false, false), (false, true), (true, false), (true, true)]
    {
        let (_directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
        let key = ObjectKeyHash::from("replacement");
        let source = tests::download(17, CHUNK_BYTES as usize);
        let request = tests::range(4110, 69_653);
        let nested_request = tests::range(4111, 4112);
        if checksum_from_write {
            cache.access_histories().record_access(&key, request);
        }
        cache.insert(key, source.clone()).await.unwrap();
        if !checksum_from_write {
            assert!(cache.get(&key, request).await.is_some());
        }
        let shard = &cache.disk.shards[0];
        let read = shard
            .entry_index
            .lock()
            .unwrap()
            .covering_entry(&key, request)
            .unwrap()
            .share_covering_read(Some(request));
        let mut initializer = Box::pin(read.result.get_or_init(std::future::pending));
        assert_pending(initializer.as_mut()).await;
        let mut waiter = Box::pin(cache.get(&key, nested_request));
        assert_pending(waiter.as_mut()).await;
        shard.remove_entry(&key, 17);
        let replacement = if replacement_bytes_differ {
            Download::new(17, Bytes::from(vec![0; CHUNK_BYTES as usize])).unwrap()
        } else {
            source.clone()
        };
        cache.insert(key, replacement.clone()).await.unwrap();
        let address = shard.entry_index.lock().unwrap().entries_by_key[&key][&17].payload_address;
        assert_eq!(read.payload_address, address + request.start() - 17);
        drop(initializer);
        assert_eq!(
            waiter.await,
            (!replacement_bytes_differ).then(|| source.bytes_in_range(nested_request))
        );
        let before = read_byte_count(&registry);
        assert_eq!(
            cache.get(&key, nested_request).await.unwrap(),
            replacement.bytes_in_range(nested_request)
        );
        assert_eq!(
            read_byte_count(&registry) - before,
            if checksum_from_write {
                request.len()
            } else {
                CHUNK_BYTES
            } as f64
        );
    }
}

#[tokio::test]
async fn records_write_outcomes_packing_and_index_usage() {
    let (_directory, cache, registry) = measured_cache(4 * CHUNK_BYTES).await;
    let key = ObjectKeyHash::from("object");
    let request = ByteRange::new(0, 1).unwrap();
    let outcome = |label| value(&registry, "feuer_disk_write_entries_total", &[("outcome", label)]);
    assert_eq!(
        cache
            .insert_batch(vec![(key, download(4)), ("neighbor".into(), download(8))])
            .await
            .unwrap(),
        2
    );
    assert_eq!(outcome("published"), 2.0);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 12.0);
    assert_eq!(
        value(&registry, "feuer_disk_batch_bytes_total", &[("kind", "payload")]),
        12.0
    );
    assert_eq!(
        value(&registry, "feuer_disk_batch_bytes_total", &[("kind", "chunk")]),
        CHUNK_BYTES as f64
    );
    assert_eq!(cache.insert_batch(vec![(key, download(4))]).await.unwrap(), 0);
    assert_eq!(outcome("already_covered"), 1.0);
    assert_eq!(value(&registry, "feuer_disk_written_entries_total", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 2.0);
    assert_eq!(
        cache
            .insert_batch(vec![("oversized".into(), download(4 * CHUNK_BYTES as usize))])
            .await
            .unwrap(),
        0
    );
    assert_eq!(outcome("no_capacity"), 1.0);
    assert!(cache.get(&key, request).await.is_some());
    assert!(cache.get(&"absent".into(), request).await.is_none());
    for label in ["hit", "absent"] {
        assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", label)]), 1.0);
    }
    assert_eq!(value(&registry, "feuer_disk_lookup_duration_seconds", &[]), 1.0);
    drop(cache);
    for name in ["feuer_disk_entries", "feuer_disk_payload_bytes"] {
        assert_eq!(value(&registry, name, &[]), 0.0);
    }
    for state in ["free", "allocated"] {
        assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", state)]), 0.0);
    }
}

#[tokio::test]
async fn eviction_triggering_insertions_count_entries_not_victims_or_batches() {
    let (_directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
    let triggering = || value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]);
    cache
        .insert_batch(vec![("first".into(), download(4)), ("second".into(), download(4))])
        .await
        .unwrap();
    assert_eq!(triggering(), 0.0);
    cache
        .insert_batch(vec![("third".into(), download(4)), ("fourth".into(), download(4))])
        .await
        .unwrap();
    // The first incoming entry evicts both old chunk owners. Its batch neighbor evicts nothing.
    assert!(!cache.covers_range(&"first".into(), ByteRange::new(0, 1).unwrap()));
    assert!(!cache.covers_range(&"second".into(), ByteRange::new(0, 1).unwrap()));
    assert_eq!(triggering(), 1.0);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
        4.0
    );
    cache.insert_batch(vec![("third".into(), download(4))]).await.unwrap();
    assert_eq!(triggering(), 1.0);

    assert_eq!(cache.insert_batch(vec![("next".into(), download(4))]).await.unwrap(), 1);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    assert_eq!(triggering(), 2.0);
}

#[tokio::test]
async fn io_errors_preserve_entries() {
    let (directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
    let request = ByteRange::new(0, 1).unwrap();
    let key = ObjectKeyHash::from("unreadable");
    cache.insert_batch(vec![(key, download(4))]).await.unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(directory.path().join("data"))
        .unwrap()
        .set_len(0)
        .unwrap();
    assert!(cache.get(&key, request).await.is_none());
    assert_eq!(
        value(&registry, "feuer_disk_lookup_total", &[("outcome", "io_error")]),
        1.0
    );
    assert!(cache.covers_range(&key, request));
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    assert_eq!(value(&registry, "feuer_disk_payload_bytes", &[]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 0.0);
}

#[tokio::test]
async fn pressure_eviction_and_write_failure_record_each_entry_outcome() {
    let (_directory, cache, registry) = measured_cache(2 * CHUNK_BYTES).await;
    let outcome = |label| value(&registry, "feuer_disk_write_entries_total", &[("outcome", label)]);
    cache.insert_batch(vec![("first".into(), download(4))]).await.unwrap();
    cache.insert_batch(vec![("second".into(), download(4))]).await.unwrap();
    assert_eq!(
        value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]),
        1.0
    );
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 1.0);
    // As in the existing write-failure test, let allocation exceed the actual file.
    let mut disk = Arc::try_unwrap(cache.disk).ok().unwrap();
    let shard = &disk.shards[0];
    shard.remove_entry(&ObjectKeyHash::from("second"), 0);
    disk.shards[0].allocator = DiskChunkAllocator::with_metrics(2 * CHUNK_BYTES..5 * CHUNK_BYTES, disk.metrics.clone());
    let cache = DiskCache {
        disk: Arc::new(disk),
        write_sender: cache.write_sender,
    };
    assert!(
        cache
            .insert_batch(vec![
                ("small".into(), download(1)),
                ("large".into(), download(2 * CHUNK_BYTES as usize))
            ])
            .await
            .is_err()
    );
    assert_eq!(outcome("failed"), 1.0);
    assert_eq!(value(&registry, "feuer_disk_written_entries_total", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_disk_entries", &[]), 0.0);
    assert_eq!(outcome("published"), 2.0);
    assert!(cache.covers_range(&"small".into(), ByteRange::new(0, 1).unwrap()));
    cache.discard_pending().await;
    assert_eq!(outcome("canceled"), 1.0);
    // The small entry never reserved disk space. Discarding it only releases its buffered bytes and accounting.
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "free")]), 4.0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 1.0);
}

#[tokio::test]
async fn buffered_entries_only_attempt_disk_admission_when_flushed() {
    let (_directory, cache, registry) = measured_cache(CHUNK_BYTES).await;
    let key = ObjectKeyHash(1);
    let range = ByteRange::new(0, 1).unwrap();
    assert_eq!(cache.flush().await.unwrap(), 0);
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
    cache.write(key, download(1), ()).await.unwrap();
    assert_eq!(value(&registry, "feuer_disk_chunks", &[("state", "allocated")]), 0.0);
    assert_eq!(cache.get(&key, range).await.unwrap(), download(1).bytes());
    assert_eq!(cache.flush().await.unwrap(), 0); // Only metadata fits on disk.
    assert!(!cache.covers_range(&key, range));
    assert_eq!(
        value(
            &registry,
            "feuer_disk_write_entries_total",
            &[("outcome", "no_capacity")]
        ),
        1.0
    );
    cache.write(key, download(1), ()).await.unwrap();
    cache.discard_pending().await;
    assert!(!cache.covers_range(&key, range));
    assert_eq!(cache.flush().await.unwrap(), 0);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "canceled")]),
        1.0
    );
}

#[test]
fn abandoned_write_attempts_count_once_as_canceled() {
    let (registry, backend) = registry();
    let metrics = DiskMetrics::new(&backend);
    let mut canceled = DiskWriteAttempt::new(metrics.clone());
    canceled.evicted = true;
    drop(canceled);
    assert_eq!(
        value(&registry, "feuer_disk_eviction_triggering_insertions_total", &[]),
        1.0
    );
    let mut finished = DiskWriteAttempt::new(metrics);
    finished.outcome = DiskWriteOutcome::Published;
    drop(finished);
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "canceled")]),
        1.0
    );
    assert_eq!(
        value(&registry, "feuer_disk_write_entries_total", &[("outcome", "published")]),
        1.0
    );
}
