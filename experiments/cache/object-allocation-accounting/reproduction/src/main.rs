use std::ops::Range;

use feuer::{ByteRange, CacheConfig, TieredMemoryDiskCache};
use mixtrics::{metrics::BoxedRegistry, registry::prometheus::PrometheusMetricsRegistry};

/// The cache read method: an object byte range, or an object charged by payload length or allocation capacity.
#[derive(Debug)]
enum CacheReadMethod {
    ByteRange,
    ObjectPayloadCharge,
    ObjectAllocationCharge,
}

/// Sum a metric's gauge values, optionally selecting one buffer-pool status.
fn gauge_sum(registry: &prometheus::Registry, metric_name: &str, status: Option<&str>) -> u64 {
    let metric_family = registry
        .gather()
        .into_iter()
        .find(|metric_family| metric_family.name() == metric_name)
        .expect("metric exists");
    metric_family
        .get_metric()
        .iter()
        .filter(|metric| {
            status.is_none_or(|status| {
                metric
                    .get_label()
                    .iter()
                    .any(|label| label.name() == "status" && label.value() == status)
            })
        })
        .map(|metric| metric.get_gauge().get_value() as u64)
        .sum()
}

/// Open a memory cache with metrics and idle-buffer retention disabled.
async fn open_memory_cache(memory_capacity: u64) -> (TieredMemoryDiskCache, prometheus::Registry) {
    let registry = prometheus::Registry::new();
    let metrics_registry: BoxedRegistry = Box::new(PrometheusMetricsRegistry::new(registry.clone()));
    let config = CacheConfig::new("/tmp/unused-disk-cache", 0, memory_capacity)
        .unwrap()
        .with_idle_buffer_pool_percent(0)
        .unwrap();
    let cache = TieredMemoryDiskCache::open(config, Some(&metrics_registry), None)
        .await
        .unwrap();
    (cache, registry)
}

/// Check memory charges and retained buffer capacity for slices of one download.
async fn check_slice_charges(read_method: CacheReadMethod, request_ranges: &[Range<usize>]) {
    // Even with 64 shards, two 2 MiB charges fit in any one shard.
    let (cache, registry) = open_memory_cache(256 << 20).await;
    let mut buffer = cache.allocate_buffer(request_ranges.last().unwrap().end).unwrap();
    buffer.as_mut_slice().fill(17);
    let download = buffer.into_download(0).unwrap();
    let allocation_capacity = download.allocation_charge() as u64;
    for request_range in request_ranges {
        let requested_range = ByteRange::new(request_range.start as u64, request_range.end as u64).unwrap();
        let object_key = format!("split-slice-v1:{}:{}:split", request_range.start, request_range.end);
        let bytes = match read_method {
            CacheReadMethod::ObjectPayloadCharge => {
                cache
                    .get_or_fetch_object(object_key, || async {
                        Ok::<_, &str>(download.bytes_in_range(requested_range))
                    })
                    .await
            }
            CacheReadMethod::ObjectAllocationCharge => {
                cache
                    .get_or_fetch_object_with_allocation_charge(object_key, || async {
                        Ok::<_, &str>((download.bytes_in_range(requested_range), download.allocation_charge()))
                    })
                    .await
            }
            CacheReadMethod::ByteRange => {
                cache
                    .get_or_fetch("split".to_string(), requested_range, || async {
                        Ok::<_, &str>(download.clone())
                    })
                    .await
            }
        }
        .unwrap();
        assert_eq!(bytes.len(), request_range.len());
        assert!(bytes.iter().all(|byte| *byte == 17));
        assert_eq!(bytes.as_ptr(), download.bytes()[request_range.start..].as_ptr());
    }
    drop(download);

    let charged_bytes = gauge_sum(&registry, "feuer_memory_used_bytes", None);
    let retained_buffer_bytes = gauge_sum(&registry, "feuer_io_buffer_pool_bytes", Some("used"));
    let entry_count = gauge_sum(&registry, "feuer_memory_entries", None);
    assert_eq!(retained_buffer_bytes, allocation_capacity);
    let (expected_entry_count, expected_charged_bytes) = match read_method {
        CacheReadMethod::ObjectPayloadCharge => (
            request_ranges.len() as u64,
            request_ranges
                .iter()
                .map(|request_range| request_range.len() as u64)
                .sum(),
        ),
        CacheReadMethod::ObjectAllocationCharge => (
            request_ranges.len() as u64,
            request_ranges.len() as u64 * allocation_capacity,
        ),
        CacheReadMethod::ByteRange => (1, allocation_capacity),
    };
    assert_eq!(entry_count, expected_entry_count);
    assert_eq!(charged_bytes, expected_charged_bytes);
    println!(
        "{read_method:?} / {} slices: entries={entry_count}, charged={charged_bytes}, retained={retained_buffer_bytes} bytes",
        request_ranges.len(),
    );
}

/// Check whether object eviction uses payload length or backing capacity.
async fn check_object_eviction(charge_allocation_capacity: bool) {
    let memory_capacity = 2 << 20;
    let (cache, registry) = open_memory_cache(memory_capacity).await;
    for index in 0..65 {
        let buffer = cache.allocate_buffer(1024).unwrap();
        let allocation_capacity = buffer.capacity();
        let object_key = format!("split-slice-v1:0:1024:split-{index}");
        if charge_allocation_capacity {
            cache
                .get_or_fetch_object_with_allocation_charge(object_key, || async {
                    Ok::<_, &str>((buffer.into_bytes(), allocation_capacity))
                })
                .await
        } else {
            cache
                .get_or_fetch_object(object_key, || async { Ok::<_, &str>(buffer.into_bytes()) })
                .await
        }
        .unwrap();
    }
    let charged_bytes = gauge_sum(&registry, "feuer_memory_used_bytes", None);
    let retained_buffer_bytes = gauge_sum(&registry, "feuer_io_buffer_pool_bytes", Some("used"));
    let entry_count = gauge_sum(&registry, "feuer_memory_entries", None);
    assert_eq!(retained_buffer_bytes, entry_count * 32 * 1024);
    if charge_allocation_capacity {
        assert!(entry_count > 0 && entry_count < 65);
        assert_eq!(charged_bytes, retained_buffer_bytes);
        assert!(charged_bytes <= memory_capacity);
    } else {
        assert_eq!(entry_count, 65);
        assert_eq!(charged_bytes, entry_count * 1024);
        assert!(retained_buffer_bytes > memory_capacity);
    }
    println!(
        "Object / charge_allocation_capacity={charge_allocation_capacity}: target={memory_capacity}, entries={entry_count}, evicted={}, charged={charged_bytes}, retained={retained_buffer_bytes} bytes",
        65 - entry_count,
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let slice_ranges = [0..1024, ((1 << 20) + 1024)..((1 << 20) + 2048)];
    for request_ranges in [&slice_ranges[..1], &slice_ranges[..]] {
        for read_method in [
            CacheReadMethod::ByteRange,
            CacheReadMethod::ObjectPayloadCharge,
            CacheReadMethod::ObjectAllocationCharge,
        ] {
            check_slice_charges(read_method, request_ranges).await;
        }
    }
    for charge_allocation_capacity in [false, true] {
        check_object_eviction(charge_allocation_capacity).await;
    }
}
