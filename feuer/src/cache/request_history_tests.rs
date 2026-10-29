//! Exercise request recording without a disk tier on platforms where storage is unavailable.

use super::*;
use std::task::Poll;

#[tokio::test]
async fn records_each_started_request_before_any_result() {
    let (metrics_registry, registry) = crate::test_metrics::registry();
    let cache = TieredMemoryDiskCache {
        inner: Arc::new(TieredMemoryDiskCacheInner {
            config: CacheConfig::new("cache", 4 << 20, 1024).unwrap(),
            memory: MemoryCache::with_metrics(1024, feuer_memory::MemoryMetrics::new(&registry), 7),
            metrics: LookupMetrics::new(&registry),
            pending_fetches: PendingFetches::default(),
        }),
        access_trace: None,
    };
    let history = cache.inner.memory.access_histories();
    let key = "object".to_owned();
    let result = cache
        .get_or_fetch(key.clone(), || async {
            assert_eq!(history.request_count(), 1);
            Err::<Bytes, _>("source failed")
        })
        .await;
    assert_eq!(result, Err(GetOrFetchError::Callback("source failed")));

    let mut pending = Box::pin(cache.get_or_fetch(key.clone(), std::future::pending::<Result<Bytes, &str>>));
    assert_eq!(
        history.request_count(),
        1,
        "an unpolled future has not started a request"
    );
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(
        history.request_count(),
        2,
        "recorded while the request is still pending"
    );
    drop(pending);
    assert_eq!(history.request_count(), 2, "cancellation does not erase demand");

    cache
        .get_or_fetch(key.clone(), || async {
            assert_eq!(history.request_count(), 3);
            Ok::<_, &str>(Bytes::from_static(b"x"))
        })
        .await
        .unwrap();
    cache
        .get_or_fetch(key.clone(), || async {
            Err::<Bytes, &str>("memory hit must not fetch")
        })
        .await
        .unwrap();
    assert_eq!(history.request_count(), 4, "success paths do not double-count");
    assert_eq!(
        crate::test_metrics::value(&metrics_registry, "feuer_lookup_total", &[("outcome", "memory_hit")]),
        1.0
    );
    assert_eq!(
        crate::test_metrics::value(&metrics_registry, "feuer_lookup_bytes_total", &[("source", "memory")]),
        1.0
    );
    assert!(history.decayed_request_count(&ObjectKeyHash::from(key)) > 3.0);
}
