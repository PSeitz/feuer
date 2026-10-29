use std::task::{Context, Poll, Waker};

use super::*;
use crate::test_metrics::{registry, value};

/// Memory-only operations with immediately ready callbacks complete without an executor.
fn ready<T>(future: impl Future<Output = T>) -> T {
    match std::pin::pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("memory-only operation unexpectedly yielded"),
    }
}

#[test]
fn memory_only_constructors_and_lookups_need_no_runtime() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    for constructor in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let unused_directory = directory.path().join("unused");
        let config = CacheConfig::new(&unused_directory, 0, 1 << 20).unwrap();
        let (registry, backend) = registry();
        let scorer = Arc::new(AvoidedRetrievalCostScorer::default());
        let cache = ready(async {
            match constructor {
                0 => TieredMemoryDiskCache::open(config, Some(&backend), None).await,
                1 => TieredMemoryDiskCache::open_with_retention_scorer(config, Some(&backend), None, scorer).await,
                _ => {
                    TieredMemoryDiskCache::open_with_retention_scorers(
                        config,
                        Some(&backend),
                        None,
                        scorer.clone(),
                        scorer,
                    )
                    .await
                }
            }
        })
        .unwrap();
        assert_eq!(
            ready(cache.get_or_fetch("failed".into(), || async { Err("source failed") })),
            Err(GetOrFetchError::Callback("source failed"))
        );
        let bytes = ready(cache.get_or_fetch("object".into(), || async {
            Ok::<_, &str>(Bytes::from_static(b"payload"))
        }))
        .unwrap();
        assert_eq!(ready(cache.clone().get("object".into())), Some(bytes));
        assert_eq!(ready(cache.get("missing".into())), None);
        for outcome in ["callback", "callback_error", "memory_hit", "miss"] {
            assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), 1.0);
        }
        assert_eq!(cache.inner.memory.access_histories().request_count(), 4);
        assert!(!unused_directory.exists());
    }
}

#[test]
fn memory_only_constructor_preserves_retention_scorer() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct ScoredEntries(AtomicUsize);

    impl RetentionScorer for ScoredEntries {
        fn score(&self, _: &ObjectKeyHash, _: u64, _: u64, _: &crate::ObjectAccessHistories) -> f64 {
            self.0.fetch_add(1, Ordering::Relaxed);
            1.0
        }
    }

    let scorer = Arc::new(ScoredEntries::default());
    let cache = ready(TieredMemoryDiskCache::open_with_retention_scorer(
        CacheConfig::new("unused", 0, 1).unwrap(),
        None,
        None,
        scorer.clone(),
    ))
    .unwrap();
    // More entries than memory shards force eviction to consult the supplied scorer.
    for index in 0..65 {
        ready(cache.get_or_fetch(index.to_string(), || async { Ok::<_, &str>(Bytes::from_static(b"x")) })).unwrap();
    }
    assert!(scorer.0.load(Ordering::Relaxed) > 0);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn nonzero_disk_capacity_is_rejected_without_runtime_or_files() {
    let directory = tempfile::tempdir().unwrap();
    let unused_directory = directory.path().join("unused");
    for capacity in [1, 4 << 20] {
        let error = ready(TieredMemoryDiskCache::open(
            CacheConfig::new(&unused_directory, capacity, 1 << 20).unwrap(),
            None,
            None,
        ))
        .unwrap_err();
        assert!(matches!(error, DiskCacheError::UnsupportedPlatform));
        assert_eq!(
            error.to_string(),
            "disk caching requires Linux; set disk capacity to zero for memory-only operation"
        );
    }
    assert!(!unused_directory.exists());
}
