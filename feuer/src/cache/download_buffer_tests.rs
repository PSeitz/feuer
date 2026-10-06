use super::*;

#[tokio::test]
async fn download_and_memory_hit_share_the_buffer_and_charge_its_capacity() {
    let (registry, backend) = crate::test_metrics::registry();
    let cache = TieredMemoryDiskCache {
        inner: Arc::new(TieredMemoryDiskCacheInner {
            config: CacheConfig::new("cache", 0, 8 << 20).unwrap(),
            memory: MemoryCache::with_metrics(8 << 20, feuer_memory::MemoryMetrics::new(&backend)),
            metrics: LookupMetrics::new(&backend),
            #[cfg(target_os = "linux")]
            disk: None,
        }),
    };
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
}
