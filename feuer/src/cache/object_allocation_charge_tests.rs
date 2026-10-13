use super::*;
use crate::test_metrics::{registry, value};

/// Create a memory cache with metrics and idle-buffer retention disabled.
fn memory_cache(memory_capacity: u64, metrics_registry: &mixtrics::metrics::BoxedRegistry) -> TieredMemoryDiskCache {
    TieredMemoryDiskCache {
        inner: Arc::new(TieredMemoryDiskCacheInner {
            config: CacheConfig::new("unused", 0, memory_capacity)
                .unwrap()
                .with_idle_buffer_pool_percent(0)
                .unwrap(),
            memory: MemoryCache::with_metrics(memory_capacity, feuer_memory::MemoryMetrics::new(metrics_registry), 0),
            metrics: LookupMetrics::new(metrics_registry),
            #[cfg(target_os = "linux")]
            disk: None,
        }),
        trace: None,
    }
}

/// Read the capacity bytes of buffers checked out from one pool bucket.
fn checked_out_buffer_capacity(registry: &prometheus::Registry, bucket: &str) -> f64 {
    value(
        registry,
        "feuer_io_buffer_pool_bytes",
        &[("bucket", bucket), ("status", "used")],
    )
}

#[tokio::test]
async fn shared_slices_charge_capacity_per_entry() {
    let (registry, metrics_registry) = registry();
    // Two 2 MiB charges fit even when both keys share one of 64 shards.
    let cache = memory_cache(256 << 20, &metrics_registry);
    let mut buffer = cache.allocate_buffer((1 << 20) + 2048).unwrap();
    buffer.as_mut_slice().fill(17);
    let download = buffer.into_download(100).unwrap();
    let allocation_capacity = download.allocation_charge();
    let download_ptr = download.bytes().as_ptr();
    let object_keys = ["first", "second"];
    for (object_key, slice_offset) in object_keys.into_iter().zip([0, (1 << 20) + 1024]) {
        let slice_bytes =
            download.bytes_in_range(ByteRange::new(100 + slice_offset, 100 + slice_offset + 1024).unwrap());
        let bytes = cache
            .get_or_fetch_object_with_allocation_charge(object_key.to_owned(), || async {
                Ok::<_, &str>((slice_bytes, allocation_capacity))
            })
            .await
            .unwrap();
        assert_eq!(bytes.len(), 1024);
        assert!(bytes.iter().all(|byte| *byte == 17));
        assert_eq!(bytes.as_ptr(), download_ptr.wrapping_add(slice_offset as usize));
        let memory_hit_bytes = cache
            .get_or_fetch_object(object_key.to_owned(), || async { Err::<Bytes, _>("must hit memory") })
            .await
            .unwrap();
        assert_eq!(memory_hit_bytes.as_ptr(), bytes.as_ptr());
    }
    drop(download);
    assert_eq!(value(&registry, "feuer_memory_entries", &[]), 2.0);
    assert_eq!(
        value(&registry, "feuer_memory_used_bytes", &[]),
        (2 * allocation_capacity) as f64
    );
    assert_eq!(
        checked_out_buffer_capacity(&registry, "2 MiB"),
        allocation_capacity as f64
    );
    // Each source slice is a complete object starting at zero in the cache.
    let object_range = ByteRange::new(0, 1024).unwrap();
    assert!(
        cache
            .inner
            .memory
            .remove(&ObjectKeyHash::from(object_keys[0]), object_range)
    );
    assert_eq!(
        value(&registry, "feuer_memory_used_bytes", &[]),
        allocation_capacity as f64
    );
    assert_eq!(
        checked_out_buffer_capacity(&registry, "2 MiB"),
        allocation_capacity as f64
    );
    assert!(
        cache
            .inner
            .memory
            .remove(&ObjectKeyHash::from(object_keys[1]), object_range)
    );
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 0.0);
    assert_eq!(checked_out_buffer_capacity(&registry, "2 MiB"), 0.0);
}

#[tokio::test]
async fn allocation_capacity_drives_eviction() {
    let (registry, metrics_registry) = registry();
    let memory_capacity = 2 << 20;
    let cache = memory_cache(memory_capacity, &metrics_registry);
    // 65 separate 32 KiB allocations exceed the target, but their 1 KiB payloads fit.
    // Every allocation fits the target share of a shard, so oversized admission is not involved.
    for index in 0..65 {
        let buffer = cache.allocate_buffer(1024).unwrap();
        let allocation_capacity = buffer.capacity();
        cache
            .get_or_fetch_object_with_allocation_charge(format!("object-{index}"), || async {
                Ok::<_, &str>((buffer.into_bytes(), allocation_capacity))
            })
            .await
            .unwrap();
    }
    let entry_count = value(&registry, "feuer_memory_entries", &[]);
    let charged_bytes = value(&registry, "feuer_memory_used_bytes", &[]);
    assert!(entry_count > 0.0 && entry_count < 65.0);
    assert_eq!(charged_bytes, entry_count * (32 * 1024) as f64);
    assert!(charged_bytes <= memory_capacity as f64);
    assert_eq!(checked_out_buffer_capacity(&registry, "32 KiB"), charged_bytes);
}
