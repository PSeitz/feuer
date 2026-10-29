use super::*;
use crate::test_metrics::{registry, value};
use prometheus::Registry;

/// Create a memory cache with metrics and idle-buffer retention disabled.
fn memory_cache(memory_capacity: u64, metrics_registry: &BoxedRegistry) -> Cache {
    Cache {
        inner: Arc::new(CacheInner {
            config: CacheConfig::new("unused", 0, memory_capacity)
                .unwrap()
                .with_idle_buffer_pool_percent(0)
                .unwrap(),
            memory: MemoryCache::with_metrics(memory_capacity, MemoryMetrics::new(metrics_registry), 0),
            metrics: LookupMetrics::new(metrics_registry),
            pending_fetches: PendingFetches::default(),
            #[cfg(target_os = "linux")]
            disk: None,
        }),
        access_trace: None,
    }
}

/// Read the capacity bytes of buffers checked out from one pool bucket.
fn checked_out_buffer_capacity(registry: &Registry, bucket: &str) -> f64 {
    value(
        registry,
        "feuer_io_buffer_pool_bytes",
        &[("bucket", bucket), ("status", "used")],
    )
}

#[tokio::test]
async fn shared_bytes_slices_charge_payload_length() {
    let (registry, metrics_registry) = registry();
    // Both entries fit even when their keys share a shard.
    let cache = memory_cache(256 << 20, &metrics_registry);
    let mut buffer = cache.allocate_buffer((1 << 20) + 2048).unwrap();
    buffer.as_mut_slice().fill(17);
    let allocation_capacity = buffer.capacity();
    let source = buffer.into_bytes();
    let source_ptr = source.as_ptr();
    let object_keys = ["first", "second"];
    for (object_key, slice_offset) in object_keys.into_iter().zip([0, (1 << 20) + 1024]) {
        let slice_bytes = source.slice(slice_offset..slice_offset + 1024);
        let bytes = cache
            .get_or_fetch(object_key.to_owned(), || async { Ok::<_, &str>(slice_bytes) })
            .await
            .unwrap();
        assert_eq!(bytes.len(), 1024);
        assert!(bytes.iter().all(|byte| *byte == 17));
        assert_eq!(bytes.as_ptr(), source_ptr.wrapping_add(slice_offset));
        let memory_hit_bytes = cache
            .get_or_fetch(object_key.to_owned(), || async { Err::<Bytes, _>("must hit memory") })
            .await
            .unwrap();
        assert_eq!(memory_hit_bytes.as_ptr(), bytes.as_ptr());
    }
    drop(source);
    assert_eq!(value(&registry, "feuer_memory_entries", &[]), 2.0);
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 2048.0);
    assert_eq!(
        checked_out_buffer_capacity(&registry, "2 MiB"),
        allocation_capacity as f64
    );
    // Each source slice is an object in the cache.
    assert!(cache.inner.memory.remove(&ObjectKeyHash::from(object_keys[0])));
    assert_eq!(value(&registry, "feuer_memory_used_bytes", &[]), 1024.0);
    assert_eq!(
        checked_out_buffer_capacity(&registry, "2 MiB"),
        allocation_capacity as f64
    );
    assert!(cache.inner.memory.remove(&ObjectKeyHash::from(object_keys[1])));
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
        cache
            .get_or_fetch(format!("object-{index}"), || async { Ok::<_, &str>(buffer) })
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
