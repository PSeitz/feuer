#[cfg(test)]
mod tests;

use std::time::Duration;

use mixtrics::metrics::{BoxedCounter, BoxedHistogram, BoxedRegistry, Buckets};

#[derive(Clone, Copy)]
pub(crate) enum LookupOutcome {
    MemoryHit,
    #[cfg(all(target_os = "linux", feature = "disk"))]
    DiskHit,
    Callback = 2,
    CallbackError,
    Miss,
}

/// Completed public lookups, never keyed by object identity.
pub(crate) struct LookupMetrics {
    outcome_counts: [BoxedCounter; 5],
    success_durations: [[BoxedHistogram; 5]; 2],
    served_bytes: [BoxedCounter; 3],
    pub(crate) memory_admission_filter: [BoxedCounter; 2],
    #[cfg(all(target_os = "linux", feature = "disk"))]
    pub(crate) disk_admission_filter: [BoxedCounter; 2],
    #[cfg(all(target_os = "linux", feature = "disk"))]
    pub(crate) disk_write_already_covered: BoxedCounter,
    #[cfg(all(target_os = "linux", feature = "disk"))]
    pub(crate) disk_write_redundant: BoxedCounter,
}

impl LookupMetrics {
    pub(crate) fn new(registry: &BoxedRegistry) -> Self {
        let count = registry.register_counter_vec(
            "feuer_lookup_total".into(),
            "Completed Feuer lookups".into(),
            &["outcome"],
        );
        let duration = registry.register_histogram_vec_with_buckets(
            "feuer_lookup_duration_seconds".into(),
            "Successful Feuer lookup duration after a memory miss, including callback work".into(),
            &["outcome", "size_range"],
            Buckets::exponential(0.000_001, 2.0, 30),
        );
        let served_bytes = registry.register_counter_vec(
            "feuer_lookup_bytes_total".into(),
            "Object payload bytes returned by successful Feuer lookups".into(),
            &["source"],
        );
        let admission_filter = registry.register_counter_vec(
            "feuer_admission_filter_total".into(),
            "Completed caller-supplied admission filter decisions".into(),
            &["tier", "outcome"],
        );
        #[cfg(all(target_os = "linux", feature = "disk"))]
        let writes = registry.register_counter_vec(
            "feuer_disk_write_queue_total".into(),
            "Disk-write queue admissions, rejections and pre-write discards. The queued outcome measures admission"
                .into(),
            &["outcome"],
        );
        Self {
            memory_admission_filter: ["rejected", "accepted"]
                .map(|outcome| admission_filter.counter(&["memory".into(), outcome.into()])),
            #[cfg(all(target_os = "linux", feature = "disk"))]
            disk_admission_filter: ["rejected", "accepted"]
                .map(|outcome| admission_filter.counter(&["disk".into(), outcome.into()])),
            #[cfg(all(target_os = "linux", feature = "disk"))]
            disk_write_already_covered: writes.counter(&["already_covered".into()]),
            #[cfg(all(target_os = "linux", feature = "disk"))]
            disk_write_redundant: writes.counter(&["redundant".into()]),
            outcome_counts: ["memory_hit", "disk_hit", "callback", "callback_error", "miss"]
                .map(|label| count.counter(&[label.into()])),
            success_durations: ["disk_hit", "callback"].map(|outcome| {
                ["lt_100kib", "lt_1mib", "lt_10mib", "lt_32mib", "gte_32mib"]
                    .map(|size_range| duration.histogram(&[outcome.into(), size_range.into()]))
            }),
            served_bytes: ["memory", "disk", "callback"].map(|label| served_bytes.counter(&[label.into()])),
        }
    }

    /// Records `elapsed` only for successful disk hits and callbacks.
    pub(crate) fn record(&self, outcome: LookupOutcome, elapsed: Duration, bytes: u64) {
        self.outcome_counts[outcome as usize].increase(1);
        if let Some(served_bytes) = self.served_bytes.get(outcome as usize) {
            served_bytes.increase(bytes);
        }
        // Disk hits and callbacks immediately follow memory hits in the outcome order.
        if let Some(durations) = (outcome as usize)
            .checked_sub(1)
            .and_then(|index| self.success_durations.get(index))
        {
            let size_range = [100 * 1024, 1 << 20, 10 << 20, 32 << 20].partition_point(|&limit| bytes >= limit);
            durations[size_range].record(elapsed.as_secs_f64());
        }
    }
}
