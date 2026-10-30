use std::{
    pin::pin,
    task::{Context, Poll, Waker},
};
use tempfile::tempdir;
use tokio::runtime::Handle;

use super::*;
use crate::test_metrics::{registry, value};

/// Memory-only operations with immediately ready callbacks complete without an executor.
fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("memory-only operation unexpectedly yielded"),
    }
}

#[test]
fn memory_only_builder_and_lookups_need_no_runtime() {
    assert!(Handle::try_current().is_err());
    for scorer_overrides in 0..3 {
        let directory = tempdir().unwrap();
        let unused_directory = directory.path().join("unused");
        let config = CacheConfig::new(&unused_directory, 0, 1 << 20).unwrap();
        let (registry, backend) = registry();
        let scorer = Arc::new(AvoidedRetrievalCostScorer::default());
        let builder = Cache::builder(config).with_metrics(&backend);
        let builder = match scorer_overrides {
            0 => builder,
            1 => builder.with_memory_retention_scorer(scorer),
            _ => builder
                .with_memory_retention_scorer(scorer.clone())
                .with_disk_retention_scorer(scorer),
        };
        let cache = ready(builder.open()).unwrap();
        assert_eq!(
            ready(cache.get_or_fetch("failed".into(), || async { Err::<Bytes, _>("source failed") })),
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
fn memory_only_history_cleanup_preserves_cached_payloads_without_a_runtime() {
    assert!(Handle::try_current().is_err());
    let config = CacheConfig::new("unused", 0, 1 << 20)
        .unwrap()
        .with_history_entry_limit(1)
        .unwrap();
    let cache = ready(Cache::builder(config).open()).unwrap();
    let histories = cache.inner.memory.access_histories();
    for key in ["old", "new"] {
        ready(cache.get_or_fetch(key.into(), || async { Ok::<_, &str>(Bytes::from_static(b"payload")) })).unwrap();
    }
    let old = ObjectKeyHash::from("old");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while histories.decayed_request_count(&old) != 0.0 {
        assert!(std::time::Instant::now() < deadline, "history cleanup did not finish");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(histories.decayed_request_count(&ObjectKeyHash::from("new")), 1.0);
    assert_eq!(cache.inner.memory.get(&old), Some(Bytes::from_static(b"payload")));
}

#[test]
fn memory_only_builder_preserves_retention_scorer() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct ScoredEntries(AtomicUsize);

    impl RetentionScorer for ScoredEntries {
        fn score(&self, _: &ObjectKeyHash, _: u64, _: u64, _: &ObjectAccessHistories) -> f64 {
            self.0.fetch_add(1, Ordering::Relaxed);
            1.0
        }
    }

    let scorer = Arc::new(ScoredEntries::default());
    let cache = ready(
        Cache::builder(CacheConfig::new("unused", 0, 1).unwrap())
            .with_memory_retention_scorer(scorer.clone())
            .with_disk_retention_scorer(scorer.clone())
            .open(),
    )
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
    let directory = tempdir().unwrap();
    let unused_directory = directory.path().join("unused");
    for capacity in [1, 4 << 20] {
        let error =
            ready(Cache::builder(CacheConfig::new(&unused_directory, capacity, 1 << 20).unwrap()).open()).unwrap_err();
        assert!(matches!(error, DiskCacheError::UnsupportedPlatform));
        assert_eq!(
            error.to_string(),
            "disk caching requires Linux; set disk capacity to zero for memory-only operation"
        );
    }
    assert!(!unused_directory.exists());
}
