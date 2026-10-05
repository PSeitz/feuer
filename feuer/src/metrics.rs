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

/// Completed public lookups, never keyed by object identity.
pub(crate) struct LookupMetrics {
    outcome_counts: [BoxedCounter; 5],
    success_durations: [BoxedHistogram; 3],
    served_bytes: [BoxedCounter; 3],
    #[cfg(target_os = "linux")]
    pub(crate) disk_write_already_covered: BoxedCounter,
    #[cfg(target_os = "linux")]
    pub(crate) disk_write_redundant: BoxedCounter,
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
            "Completed successful Feuer lookup duration, including callback work".into(),
            &["outcome"],
            Buckets::exponential(0.000_001, 2.0, 30),
        );
        let served_bytes = registry.register_counter_vec(
            "feuer_lookup_bytes_total".into(),
            "Requested bytes returned by successful Feuer lookups".into(),
            &["source"],
        );
        #[cfg(target_os = "linux")]
        let writes = registry.register_counter_vec(
            "feuer_disk_write_queue_total".into(),
            "Disk-write queue admissions, rejections and pre-write discards; queued is not a terminal outcome".into(),
            &["outcome"],
        );
        Self {
            #[cfg(target_os = "linux")]
            disk_write_already_covered: writes.counter(&["already_covered".into()]),
            #[cfg(target_os = "linux")]
            disk_write_redundant: writes.counter(&["redundant".into()]),
            outcome_counts: [
                "memory_hit",
                "disk_hit",
                "callback",
                "callback_error",
                "invalid_download",
            ]
            .map(|label| count.counter(&[label.into()])),
            success_durations: ["memory_hit", "disk_hit", "callback"].map(|label| duration.histogram(&[label.into()])),
            served_bytes: ["memory", "disk", "callback"].map(|label| served_bytes.counter(&[label.into()])),
        }
    }

    pub(crate) fn record(&self, outcome: LookupOutcome, elapsed: Duration, bytes: u64) {
        self.outcome_counts[outcome as usize].increase(1);
        if let Some(duration) = self.success_durations.get(outcome as usize) {
            duration.record(elapsed.as_secs_f64());
            self.served_bytes[outcome as usize].increase(bytes);
        }
    }
}
