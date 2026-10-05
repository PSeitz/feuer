use super::*;
use crate::test_metrics::{registry, value};

#[test]
fn memory_operations_exclude_access_hit_miss_and_victim_counters() {
    let (registry, backend) = registry();
    let _metrics = feuer_memory::MemoryMetrics::new(&backend);
    let family = registry
        .gather()
        .into_iter()
        .find(|family| family.name() == "feuer_memory_operations_total")
        .unwrap();
    assert_eq!(family.get_metric().len(), 5);
    assert!(family.get_metric().iter().all(|metric| {
        metric.get_label().iter().any(|label| {
            label.name() == "operation"
                && matches!(label.value(), "insert" | "replace" | "redundant" | "remove" | "compact")
        })
    }));
}

#[test]
fn lookup_bytes_record_all_successes_but_latency_excludes_memory_hits() {
    let (registry, backend) = registry();
    let metrics = LookupMetrics::new(&backend);
    for (outcome, label, elapsed) in [
        (LookupOutcome::MemoryHit, "memory_hit", None),
        #[cfg(target_os = "linux")]
        (LookupOutcome::DiskHit, "disk_hit", Some(Duration::from_micros(10))),
        (LookupOutcome::Callback, "callback", Some(Duration::from_micros(10))),
        (LookupOutcome::CallbackError, "callback_error", None),
        (LookupOutcome::InvalidDownload, "invalid_download", None),
    ] {
        metrics.record(outcome, elapsed, 7);
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", label)]), 1.0);
    }
    for source in ["memory", "callback"] {
        assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 7.0);
    }
    assert_eq!(
        value(&registry, "feuer_lookup_duration_seconds", &[("outcome", "callback")]),
        1.0
    );
    #[cfg(target_os = "linux")]
    {
        assert_eq!(
            value(&registry, "feuer_lookup_duration_seconds", &[("outcome", "disk_hit")]),
            1.0
        );
        assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", "disk")]), 7.0);
    }
    let families = registry.gather();
    let durations = families
        .iter()
        .find(|family| family.name() == "feuer_lookup_duration_seconds")
        .unwrap();
    assert_eq!(durations.get_metric().len(), 2);
    assert!(durations.get_metric().iter().all(|metric| {
        metric
            .get_label()
            .iter()
            .all(|label| label.name() != "outcome" || matches!(label.value(), "disk_hit" | "callback"))
    }));
    #[cfg(not(target_os = "linux"))]
    assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", "disk")]), 0.0);
    assert!(registry.gather().iter().all(|family| {
        !matches!(
            family.name(),
            "feuer_callback_total" | "feuer_callback_duration_seconds" | "feuer_callback_download_bytes_total"
        )
    }));
}
