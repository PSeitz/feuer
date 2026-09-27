#[cfg(test)]
mod tests;

use std::time::Duration;

use mixtrics::metrics::{BoxedCounter, BoxedHistogram};
#[cfg(any(target_os = "linux", test))]
use mixtrics::metrics::{BoxedRegistry, Buckets};

#[derive(Clone, Copy)]
pub(crate) enum LookupOutcome {
    MemoryHit,
    #[cfg(target_os = "linux")]
    DiskHit,
    Callback = 2,
    CallbackError,
    InvalidDownload,
}

struct LookupOutcomeMetrics {
    count: BoxedCounter,
    duration: BoxedHistogram,
}

/// Completed public lookups and callback work, never keyed by object identity.
pub(crate) struct LookupMetrics {
    outcomes: [LookupOutcomeMetrics; 5],
    served_bytes: [BoxedCounter; 3],
    pub(crate) callbacks: BoxedCounter,
    pub(crate) download_bytes: BoxedCounter,
    pub(crate) callback_success_duration: BoxedHistogram,
    pub(crate) callback_error_duration: BoxedHistogram,
}

impl LookupMetrics {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn new(registry: &BoxedRegistry) -> Self {
        let count = registry.register_counter_vec(
            "feuer_lookup_total".into(),
            "Completed Feuer lookups".into(),
            &["outcome"],
        );
        let duration = registry.register_histogram_vec_with_buckets(
            "feuer_lookup_duration_seconds".into(),
            "Completed Feuer lookup duration, including callback work".into(),
            &["outcome"],
            Buckets::exponential(0.000_001, 2.0, 30),
        );
        let served_bytes = registry.register_counter_vec(
            "feuer_lookup_bytes_total".into(),
            "Requested bytes returned by successful Feuer lookups".into(),
            &["source"],
        );
        let callbacks = registry.register_counter_vec(
            "feuer_callback_total".into(),
            "Download callback invocations, not source GETs".into(),
            &[],
        );
        let download_bytes = registry.register_counter_vec(
            "feuer_callback_download_bytes_total".into(),
            "Bytes returned by callbacks, including non-covering downloads; not network bytes".into(),
            &[],
        );
        let callback_duration = registry.register_histogram_vec_with_buckets(
            "feuer_callback_duration_seconds".into(),
            "Completed download callback duration".into(),
            &["outcome"],
            Buckets::exponential(0.000_001, 2.0, 30),
        );
        Self {
            outcomes: [
                "memory_hit",
                "disk_hit",
                "callback",
                "callback_error",
                "invalid_download",
            ]
            .map(|label| LookupOutcomeMetrics {
                count: count.counter(&[label.into()]),
                duration: duration.histogram(&[label.into()]),
            }),
            served_bytes: ["memory", "disk", "callback"].map(|label| served_bytes.counter(&[label.into()])),
            callbacks: callbacks.counter(&[]),
            download_bytes: download_bytes.counter(&[]),
            callback_success_duration: callback_duration.histogram(&["success".into()]),
            callback_error_duration: callback_duration.histogram(&["error".into()]),
        }
    }

    pub(crate) fn record(&self, outcome: LookupOutcome, elapsed: Duration, bytes: u64) {
        let metrics = &self.outcomes[outcome as usize];
        metrics.count.increase(1);
        metrics.duration.record(elapsed.as_secs_f64());
        if let Some(counter) = self.served_bytes.get(outcome as usize) {
            counter.increase(bytes);
        }
    }
}
