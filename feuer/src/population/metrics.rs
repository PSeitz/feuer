use std::sync::Arc;

use mixtrics::metrics::{BoxedCounter, BoxedGauge, BoxedHistogram, BoxedRegistry, Buckets};

/// An admission or discard outcome from the disk-population queue.
#[derive(Clone, Copy)]
pub(super) enum PopulationQueueOutcome {
    Queued,
    Full,
    Closed,
    Stale,
    Canceled,
    AlreadyCovered,
    Redundant,
}

/// Metrics for population-queue admission, waiting entries, and pending bytes.
pub(super) struct PopulationQueueMetrics {
    outcomes: [BoxedCounter; 7],
    pub(super) queued_entries: BoxedGauge,
    pub(super) pending_bytes: BoxedGauge,
    pub(super) queue_duration: BoxedHistogram,
}

impl PopulationQueueMetrics {
    pub(super) fn new(registry: &BoxedRegistry) -> Arc<Self> {
        let outcomes = registry.register_counter_vec(
            "feuer_disk_population_queue_total".into(),
            "Population queue admissions, rejections and pre-write discards; queued is not a terminal outcome".into(),
            &["outcome"],
        );
        let queued = registry.register_gauge_vec(
            "feuer_disk_population_queued_entries".into(),
            "Entries waiting to start disk population".into(),
            &[],
        );
        let bytes = registry.register_gauge_vec(
            "feuer_disk_population_pending_bytes".into(),
            "Queued plus active population payload bytes".into(),
            &[],
        );
        let duration = registry.register_histogram_vec_with_buckets(
            "feuer_disk_population_queue_duration_seconds".into(),
            "Time from queue admission to dequeue, including stale entries".into(),
            &[],
            Buckets::exponential(0.000_001, 2.0, 25),
        );
        Arc::new(Self {
            outcomes: [
                "queued",
                "queue_full",
                "queue_closed",
                "stale",
                "canceled",
                "already_covered",
                "redundant",
            ]
            .map(|label| outcomes.counter(&[label.into()])),
            queued_entries: queued.gauge(&[]),
            pending_bytes: bytes.gauge(&[]),
            queue_duration: duration.histogram(&[]),
        })
    }

    pub(super) fn record(&self, outcome: PopulationQueueOutcome) {
        self.outcomes[outcome as usize].increase(1);
    }
}
