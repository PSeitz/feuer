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
    duration: Option<BoxedHistogram>,
}

/// Completed public lookups, never keyed by object identity.
pub(crate) struct LookupMetrics {
    outcomes: [LookupOutcomeMetrics; 5],
    served_bytes: [BoxedCounter; 3],
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
            "Completed Feuer memory and disk hit lookup duration".into(),
            &["outcome"],
            Buckets::exponential(0.000_001, 2.0, 30),
        );
        let served_bytes = registry.register_counter_vec(
            "feuer_lookup_bytes_total".into(),
            "Requested bytes returned by successful Feuer lookups".into(),
            &["source"],
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
                duration: matches!(label, "memory_hit" | "disk_hit").then(|| duration.histogram(&[label.into()])),
            }),
            served_bytes: ["memory", "disk", "callback"].map(|label| served_bytes.counter(&[label.into()])),
        }
    }

    pub(crate) fn record(&self, outcome: LookupOutcome, elapsed: Duration, bytes: u64) {
        let metrics = &self.outcomes[outcome as usize];
        metrics.count.increase(1);
        if let Some(duration) = &metrics.duration {
            duration.record(elapsed.as_secs_f64());
        }
        if let Some(counter) = self.served_bytes.get(outcome as usize) {
            counter.increase(bytes);
        }
    }
}
