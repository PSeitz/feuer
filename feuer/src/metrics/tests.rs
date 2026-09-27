use super::*;
use crate::test_metrics::{registry, value};

#[test]
fn lookup_latency_only_records_hits_and_only_successes_serve_bytes() {
    let (registry, backend) = registry();
    let metrics = LookupMetrics::new(&backend);
    for outcome in [
        LookupOutcome::MemoryHit,
        LookupOutcome::Callback,
        LookupOutcome::CallbackError,
        LookupOutcome::InvalidDownload,
    ] {
        metrics.record(outcome, Duration::from_micros(10), 7);
    }
    for outcome in ["memory_hit", "callback", "callback_error", "invalid_download"] {
        assert_eq!(value(&registry, "feuer_lookup_total", &[("outcome", outcome)]), 1.0);
    }
    assert_eq!(
        value(&registry, "feuer_lookup_duration_seconds", &[("outcome", "memory_hit")]),
        1.0
    );
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
            .all(|label| label.name() != "outcome" || matches!(label.value(), "memory_hit" | "disk_hit"))
    }));
    for source in ["memory", "callback"] {
        assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 7.0);
    }
    assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", "disk")]), 0.0);
    assert!(registry.gather().iter().all(|family| {
        !matches!(
            family.name(),
            "feuer_callback_total" | "feuer_callback_duration_seconds" | "feuer_callback_download_bytes_total"
        )
    }));
}
