use super::*;

#[tokio::test]
async fn download_and_memory_hit_share_the_buffer_and_charge_its_capacity() {
    let (registry, backend) = crate::test_metrics::registry();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let cache = TieredMemoryDiskCache {
        inner: Arc::new(TieredMemoryDiskCacheInner {
            config: CacheConfig::new("cache", 0, 8 << 20).unwrap(),
            memory: MemoryCache::with_metrics(8 << 20, feuer_memory::MemoryMetrics::new(&backend), 7),
            metrics: LookupMetrics::new(&backend),
            pending_fetches: PendingFetches::default(),
            #[cfg(target_os = "linux")]
            disk: None,
        }),
        access_trace: None,
    }
    .with_trace(move |package| std::future::ready(sender.send(package).map_err(std::io::Error::other)));
    // Seed an idle disk-read buffer to verify downloads acquire from that same pool.
    let pool = cache.inner.memory.buffer_pool();
    let idle = pool.allocate(4).unwrap();
    let address = idle.as_ref().as_ptr();
    let capacity = idle.capacity();
    drop(idle);
    let mut buffer = cache.allocate_buffer(4).unwrap();
    assert_eq!(buffer.as_ref().as_ptr(), address);
    buffer.as_mut_slice().copy_from_slice(b"abcd");
    let download = buffer.into_download();
    let result = cache
        .get_or_fetch("object".to_owned(), || async { Ok::<_, &str>(download) })
        .await
        .unwrap();
    assert_eq!(result.as_ref(), b"abcd");
    assert_eq!(result.as_ptr(), address);
    assert_eq!(pool.used_bytes(), capacity as u64);
    assert_eq!(
        crate::test_metrics::value(&registry, "feuer_memory_used_bytes", &[]),
        capacity as f64
    );
    let hit = cache
        .get_or_fetch("object".to_owned(), || async {
            Err::<Download, _>("memory hit must not download")
        })
        .await
        .unwrap();
    assert_eq!(hit.as_ref(), b"abcd");
    assert_eq!(hit.as_ptr(), address);
    for payload in [Bytes::new(), Bytes::from_static(b"abcdef")] {
        let key = format!("whole-object-{}", payload.len());
        let bytes = cache
            .get_or_fetch_object(key.clone(), || async { Ok::<_, &str>(payload.clone()) })
            .await
            .unwrap();
        assert_eq!(bytes, payload);
        let hit = cache
            .get_or_fetch_object(key.clone(), || async { Err::<Bytes, _>("must hit memory") })
            .await
            .unwrap();
        assert_eq!(hit, payload);
        assert_eq!(hit.as_ptr(), payload.as_ptr());
        assert!(cache.inner.memory.contains_entry(&ObjectKeyHash::from(key)));
    }
    drop(cache);
    let package = receiver.recv().await.unwrap();
    assert_eq!(&package[..8], b"FETR\x02\0\0\0");
    assert_eq!(package.len(), 8 + 12 * 41);
    let (events, _) = package[8..].as_chunks::<41>();
    assert_eq!(
        events.iter().map(|event| event[0]).collect::<Vec<_>>(),
        [0, 3, 0, 1].repeat(3)
    );
    for (object_events, payload_len) in events.as_chunks::<4>().0.iter().zip([4, 0, 6]) {
        for (event, expected_length) in object_events.iter().zip([0, payload_len, 0, payload_len]) {
            assert_eq!(u64::from_le_bytes(event[33..41].try_into().unwrap()), expected_length);
        }
    }
}
