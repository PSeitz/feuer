//! Exercise request recording without a disk tier on platforms where storage is unavailable.

use super::*;
use std::task::Poll;

#[tokio::test]
async fn records_each_started_request_before_any_result() {
    let (metrics_registry, registry) = crate::test_metrics::registry();
    let cache = TieredMemoryDiskCache {
        inner: Arc::new(TieredMemoryDiskCacheInner {
            config: CacheConfig::new("cache", 4 << 20, 1024).unwrap(),
            memory: MemoryCache::with_metrics(1024, feuer_memory::MemoryMetrics::new(&registry)),
            metrics: LookupMetrics::new(&registry),
        }),
    };
    let history = cache.inner.memory.access_histories();
    let key = "object".to_owned();
    let requested = ByteRange::new(0, 1).unwrap();
    let result = cache
        .get_or_fetch(key.clone(), requested, || async {
            assert_eq!(history.request_count(), 1);
            Err::<Download, _>("source failed")
        })
        .await;
    assert_eq!(result, Err(GetOrFetchError::Callback("source failed")));

    let result = cache
        .get_or_fetch(key.clone(), requested, || async {
            assert_eq!(history.request_count(), 2);
            Ok::<_, &str>(Download::new(10, Bytes::from_static(b"x")).unwrap())
        })
        .await;
    assert!(matches!(result, Err(GetOrFetchError::DownloadDoesNotCover { .. })));

    let mut pending = Box::pin(cache.get_or_fetch(key.clone(), requested, || {
        std::future::pending::<Result<Download, &str>>()
    }));
    assert_eq!(
        history.request_count(),
        2,
        "an unpolled future has not started a request"
    );
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(
        history.request_count(),
        3,
        "recorded while the request is still pending"
    );
    drop(pending);
    assert_eq!(history.request_count(), 3, "cancellation does not erase demand");

    cache
        .get_or_fetch(key.clone(), requested, || async {
            assert_eq!(history.request_count(), 4);
            Ok::<_, &str>(Download::new(0, Bytes::from_static(b"x")).unwrap())
        })
        .await
        .unwrap();
    cache
        .get_or_fetch(key.clone(), requested, || async {
            Err::<Download, &str>("memory hit must not fetch")
        })
        .await
        .unwrap();
    assert_eq!(history.request_count(), 5, "success paths do not double-count");
    assert_eq!(
        crate::test_metrics::value(&metrics_registry, "feuer_lookup_total", &[("outcome", "memory_hit")]),
        1.0
    );
    assert_eq!(
        crate::test_metrics::value(&metrics_registry, "feuer_lookup_bytes_total", &[("source", "memory")]),
        1.0
    );
    assert_eq!(
        history.recent_requested_ranges(&ObjectKeyHash::from(key)),
        vec![requested; 5]
    );
}
