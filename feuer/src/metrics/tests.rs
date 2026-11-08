use super::*;
use crate::test_metrics::{registry, value};
use feuer_memory::MemoryMetrics;

#[test]
fn memory_operations_exclude_access_hit_miss_and_victim_counters() {
    let (registry, backend) = registry();
    let _metrics = MemoryMetrics::new(&backend);
    let family = registry
        .gather()
        .into_iter()
        .find(|family| family.name() == "feuer_memory_operations_total")
        .unwrap();
    assert_eq!(family.get_metric().len(), 3);
    assert!(family.get_metric().iter().all(|metric| {
        metric
            .get_label()
            .iter()
            .any(|label| label.name() == "operation" && matches!(label.value(), "insert" | "redundant" | "remove"))
    }));
}

#[test]
fn lookup_duration_size_ranges() {
    for (bytes, expected_range) in [
        (0, "lt_100kib"),
        (100 * 1024 - 1, "lt_100kib"),
        (100 * 1024, "lt_1mib"),
        ((1 << 20) - 1, "lt_1mib"),
        (1 << 20, "lt_10mib"),
        ((10 << 20) - 1, "lt_10mib"),
        (10 << 20, "lt_32mib"),
        ((32 << 20) - 1, "lt_32mib"),
        (32 << 20, "gte_32mib"),
        (u64::MAX, "gte_32mib"),
    ] {
        let (registry, backend) = registry();
        let metrics = LookupMetrics::new(&backend);
        for (outcome, label) in [
            (LookupOutcome::Callback, "callback"),
            #[cfg(all(target_os = "linux", feature = "disk"))]
            (LookupOutcome::DiskHit, "disk_hit"),
        ] {
            metrics.record(outcome, Duration::from_micros(10), bytes);
            for size_range in ["lt_100kib", "lt_1mib", "lt_10mib", "lt_32mib", "gte_32mib"] {
                assert_eq!(
                    value(
                        &registry,
                        "feuer_lookup_duration_seconds",
                        &[("outcome", label), ("size_range", size_range)],
                    ),
                    if size_range == expected_range { 1.0 } else { 0.0 },
                    "{label}, {bytes} bytes, {size_range}",
                );
            }
        }
    }
}

#[test]
fn lookup_bytes_record_all_successes_but_latency_excludes_memory_hits() {
    let (registry, backend) = registry();
    let metrics = LookupMetrics::new(&backend);
    for (outcome, label) in [
        (LookupOutcome::MemoryHit, "memory_hit"),
        #[cfg(all(target_os = "linux", feature = "disk"))]
        (LookupOutcome::DiskHit, "disk_hit"),
        (LookupOutcome::Callback, "callback"),
        (LookupOutcome::CallbackError, "callback_error"),
        (LookupOutcome::Miss, "miss"),
    ] {
        metrics.record(outcome, Duration::from_micros(10), 7);
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", label)]), 1.0);
    }
    for source in ["memory", "callback"] {
        assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 7.0);
    }
    assert_eq!(
        value(
            &registry,
            "feuer_lookup_duration_seconds",
            &[("outcome", "callback"), ("size_range", "lt_100kib")],
        ),
        1.0
    );
    #[cfg(all(target_os = "linux", feature = "disk"))]
    {
        assert_eq!(
            value(
                &registry,
                "feuer_lookup_duration_seconds",
                &[("outcome", "disk_hit"), ("size_range", "lt_100kib")],
            ),
            1.0
        );
        assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", "disk")]), 7.0);
    }
    let families = registry.gather();
    let bytes = families
        .iter()
        .find(|family| family.name() == "feuer_lookup_bytes_total")
        .unwrap();
    assert_eq!(bytes.get_metric().len(), 3);
    let durations = families
        .iter()
        .find(|family| family.name() == "feuer_lookup_duration_seconds")
        .unwrap();
    assert_eq!(durations.get_metric().len(), 10);
    assert!(durations.get_metric().iter().all(|metric| {
        metric
            .get_label()
            .iter()
            .all(|label| label.name() != "outcome" || matches!(label.value(), "disk_hit" | "callback"))
    }));
    #[cfg(not(all(target_os = "linux", feature = "disk")))]
    assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", "disk")]), 0.0);
    assert!(registry.gather().iter().all(|family| {
        !matches!(
            family.name(),
            "feuer_callback_total" | "feuer_callback_duration_seconds" | "feuer_callback_download_bytes_total"
        )
    }));
}
