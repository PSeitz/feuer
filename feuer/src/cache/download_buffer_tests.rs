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
            #[cfg(target_os = "linux")]
            disk: None,
        }),
        trace: None,
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
    let download = buffer.into_download(10).unwrap();
    let result = cache
        .get_or_fetch("object".to_owned(), ByteRange::new(11, 13).unwrap(), || async {
            Ok::<_, &str>(download)
        })
        .await
        .unwrap();
    assert_eq!(result.as_ref(), b"bc");
    assert_eq!(result.as_ptr(), address.wrapping_add(1));
    assert_eq!(pool.used_bytes(), capacity as u64);
    assert_eq!(
        crate::test_metrics::value(&registry, "feuer_memory_used_bytes", &[]),
        capacity as f64
    );
    let hit = cache
        .get_or_fetch("object".to_owned(), ByteRange::new(10, 14).unwrap(), || async {
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
        assert!(cache.inner.memory.contains_entry(
            &ObjectKeyHash::from(key),
            ByteRange::new(0, payload.len() as u64).unwrap(),
        ));
    }
    drop(cache);
    let bytes = receiver.recv().await.unwrap();
    assert_eq!(&bytes[..8], b"FETR\x01\0\0\0");
    assert_eq!(bytes.len(), 8 + 12 * 65);
    let (events, _) = bytes[8..].as_chunks::<65>();
    assert_eq!(
        events.iter().map(|event| event[0]).collect::<Vec<_>>(),
        [0, 3, 0, 1].repeat(3)
    );
    for event in &events[4..] {
        assert_eq!(u64::from_le_bytes(event[41..49].try_into().unwrap()), u64::MAX);
    }
}
