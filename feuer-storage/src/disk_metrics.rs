use std::{sync::Arc, time::Duration};

use mixtrics::metrics::{BoxedCounter, BoxedGauge, BoxedHistogram, BoxedRegistry, Buckets};

#[derive(Clone, Copy)]
pub(crate) enum DiskLookupOutcome {
    Hit,
    Absent,
    IoError,
    IntegrityFailure,
}

#[derive(Clone, Copy)]
pub(crate) enum PopulationOutcome {
    Published,
    AlreadyCovered,
    NoCapacity,
    Stale,
    Failed,
    Canceled,
}

/// Disk range lookup, immutable population and whole-chunk capacity metrics.
/// All labels have fixed values; gauges aggregate caches sharing a registry.
#[derive(Debug)]
pub struct DiskMetrics {
    lookup_count: [BoxedCounter; 4],
    hit_duration: BoxedHistogram,
    population: [BoxedCounter; 6],
    pub(crate) written_entries: BoxedCounter,
    pub(crate) free_chunks: BoxedGauge,
    pub(crate) reserved_chunks: BoxedGauge,
    pub(crate) quarantined_chunks: BoxedGauge,
    pub(crate) payload_bytes: BoxedGauge,
    pub(crate) entries: BoxedGauge,
    pub(crate) evictions: BoxedCounter,
    pub(crate) packed_payload_bytes: BoxedCounter,
    pub(crate) packed_chunk_bytes: BoxedCounter,
}

impl DiskMetrics {
    /// Registers range-cache metrics independently of raw file I/O metrics.
    pub fn new(registry: &BoxedRegistry) -> Arc<Self> {
        let lookups = registry.register_counter_vec(
            "feuer_disk_lookup_total".into(),
            "Completed disk range lookups; errors and integrity failures are returned as misses".into(),
            &["outcome"],
        );
        let duration = registry.register_histogram_vec_with_buckets(
            "feuer_disk_lookup_duration_seconds".into(),
            "Completed disk hit duration including integrity checking and copying".into(),
            &["outcome"],
            Buckets::exponential(0.000_001, 2.0, 25),
        );
        let population = registry.register_counter_vec(
            "feuer_disk_population_total".into(),
            "Terminal outcomes of entries submitted to disk batch insertion".into(),
            &["outcome"],
        );
        let written = registry.register_counter_vec(
            "feuer_disk_population_written_entries_total".into(),
            "Entries in successfully written shard batches, whether published or discarded".into(),
            &[],
        );
        let chunks = registry.register_gauge_vec(
            "feuer_disk_chunks".into(),
            "Live allocator capacity in 1-MiB chunks; reserved includes owners and read guards".into(),
            &["state"],
        );
        let payload = registry.register_gauge_vec(
            "feuer_disk_payload_bytes".into(),
            "Payload bytes in indexed disk entries, excluding metadata and padding".into(),
            &[],
        );
        let entries =
            registry.register_gauge_vec("feuer_disk_entries".into(), "Indexed disk range entries".into(), &[]);
        let evictions = registry.register_counter_vec(
            "feuer_disk_evictions_total".into(),
            "Disk entries removed by capacity pressure, not necessarily freeing a chunk".into(),
            &[],
        );
        let packed = registry.register_counter_vec(
            "feuer_disk_batch_bytes_total".into(),
            "Payload and whole-chunk bytes in successfully written shard batches before publication".into(),
            &["kind"],
        );
        let outcomes = ["hit", "absent", "io_error", "integrity_failure"];
        Arc::new(Self {
            lookup_count: outcomes.map(|label| lookups.counter(&[label.into()])),
            hit_duration: duration.histogram(&["hit".into()]),
            population: [
                "published",
                "already_covered",
                "no_capacity",
                "stale",
                "failed",
                "canceled",
            ]
            .map(|label| population.counter(&[label.into()])),
            written_entries: written.counter(&[]),
            free_chunks: chunks.gauge(&["free".into()]),
            reserved_chunks: chunks.gauge(&["reserved".into()]),
            quarantined_chunks: chunks.gauge(&["quarantined".into()]),
            payload_bytes: payload.gauge(&[]),
            entries: entries.gauge(&[]),
            evictions: evictions.counter(&[]),
            packed_payload_bytes: packed.counter(&["payload".into()]),
            packed_chunk_bytes: packed.counter(&["chunk".into()]),
        })
    }

    /// Creates unregistered range-cache metrics.
    pub fn noop() -> Arc<Self> {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::new(&registry)
    }

    pub(crate) fn record_lookup(&self, outcome: DiskLookupOutcome, elapsed: Duration) {
        self.lookup_count[outcome as usize].increase(1);
        if matches!(outcome, DiskLookupOutcome::Hit) {
            self.hit_duration.record(elapsed.as_secs_f64());
        }
    }
}

/// Records exactly one terminal outcome even if a batch task is dropped.
pub(crate) struct PopulationAttempt {
    metrics: Arc<DiskMetrics>,
    outcome: PopulationOutcome,
}

impl PopulationAttempt {
    pub(crate) fn new(metrics: Arc<DiskMetrics>) -> Self {
        Self {
            metrics,
            outcome: PopulationOutcome::Canceled,
        }
    }

    pub(crate) fn set_outcome(&mut self, outcome: PopulationOutcome) {
        self.outcome = outcome;
    }
}

impl Drop for PopulationAttempt {
    fn drop(&mut self) {
        self.metrics.population[self.outcome as usize].increase(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_metrics::{registry, value};

    #[test]
    fn lookup_durations_only_record_hits_but_counters_include_all_outcomes() {
        let (registry, backend) = registry();
        let metrics = DiskMetrics::new(&backend);
        for (outcome, label) in [
            (DiskLookupOutcome::Hit, "hit"),
            (DiskLookupOutcome::Absent, "absent"),
            (DiskLookupOutcome::IoError, "io_error"),
            (DiskLookupOutcome::IntegrityFailure, "integrity_failure"),
        ] {
            metrics.record_lookup(outcome, Duration::from_micros(10));
            assert_eq!(value(&registry, "feuer_disk_lookup_total", &[("outcome", label)]), 1.0);
        }
        let family = registry
            .gather()
            .into_iter()
            .find(|family| family.name() == "feuer_disk_lookup_duration_seconds")
            .unwrap();
        assert_eq!(family.get_metric().len(), 1);
        assert_eq!(
            value(&registry, "feuer_disk_lookup_duration_seconds", &[("outcome", "hit")]),
            1.0
        );
    }
}
