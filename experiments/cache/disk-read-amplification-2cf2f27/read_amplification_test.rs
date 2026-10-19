use super::{tests::*, *};

// Every stored payload is an exact request. No expanded downloads are involved.
#[tokio::test]
async fn exact_requests_still_amplify_covering_disk_reads() {
    let large_bytes = 4 * CHUNK_BYTES;
    let small_bytes = 16 * 1024;
    let requests = [
        range(0, small_bytes),
        range(2 * CHUNK_BYTES, 2 * CHUNK_BYTES + small_bytes),
    ];
    for exact_keys in [false, true] {
        let (registry, backend) = crate::test_metrics::registry();
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskCache::open(
            directory.path(),
            16 * CHUNK_BYTES,
            DiskCacheOptions {
                io_metrics: IoMetrics::new(&backend),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cache = with_manual_metadata_writes(cache);
        let key = ObjectKeyHash::from("object");
        let keys = if exact_keys {
            [ObjectKeyHash::from("slice-one"), ObjectKeyHash::from("slice-two")]
        } else {
            [key, key]
        };
        for (slice_key, request) in keys.into_iter().zip(requests) {
            let start = if exact_keys { 0 } else { request.start() };
            assert!(
                cache
                    .insert(slice_key, download(start, small_bytes as usize))
                    .await
                    .unwrap()
            );
        }
        assert!(cache.insert(key, download(0, large_bytes as usize)).await.unwrap());
        // Range mode discards both small entries and refuses to restore them.
        if !exact_keys {
            for request in requests {
                assert!(
                    !cache
                        .insert(key, download(request.start(), small_bytes as usize))
                        .await
                        .unwrap()
                );
            }
        }
        let read_bytes =
            || crate::test_metrics::value(&registry, "feuer_disk_io_bytes_total", &[("operation", "read")]);
        let before = read_bytes();
        for (slice_key, request) in keys.into_iter().zip(requests) {
            let requested = if exact_keys { None } else { Some(request) };
            let (bytes, _) = cache.fetch_from_disk(&slice_key, requested).await.unwrap();
            assert_eq!(bytes.len(), small_bytes as usize);
        }
        let actual = read_bytes() - before;
        let expected = if exact_keys { 2 * small_bytes } else { 2 * large_bytes };
        println!(
            "exact_keys={exact_keys} returned_bytes={} disk_read_bytes={actual}",
            2 * small_bytes
        );
        assert_eq!(actual, expected as f64);
    }
}
