use super::*;
use crate::test_metrics::{registry, value};

#[test]
fn lookup_outcomes_have_matching_histograms_and_only_successes_serve_bytes() {
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
        assert_eq!(
            value(&registry, "feuer_lookup_duration_seconds", &[("outcome", outcome)]),
            1.0
        );
    }
    for source in ["memory", "callback"] {
        assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", source)]), 7.0);
    }
    assert_eq!(value(&registry, "feuer_lookup_bytes_total", &[("source", "disk")]), 0.0);
    metrics.callbacks.increase(1);
    metrics.download_bytes.increase(123);
    metrics.callback_success_duration.record(0.01);
    assert_eq!(value(&registry, "feuer_callback_total", &[]), 1.0);
    assert_eq!(value(&registry, "feuer_callback_download_bytes_total", &[]), 123.0);
    assert_eq!(
        value(&registry, "feuer_callback_duration_seconds", &[("outcome", "success")]),
        1.0
    );
}
