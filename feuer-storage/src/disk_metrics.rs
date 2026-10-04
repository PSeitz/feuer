use std::{sync::Arc, time::Duration};

use mixtrics::metrics::{BoxedCounter, BoxedGauge, BoxedHistogram, BoxedRegistry, Buckets};

#[derive(Clone, Copy)]
pub(crate) enum DiskLookupOutcome {
    Hit,
    Absent,
    IoError,
    ChecksumFailed,
}

#[derive(Clone, Copy)]
pub(crate) enum DiskWriteOutcome {
    Published,
    AlreadyCovered,
    NoCapacity,
    Failed,
    Canceled,
}

/// Disk range lookup, payload writes and whole-chunk capacity metrics.
/// All labels have fixed values; gauges aggregate caches sharing a registry.
#[derive(Debug)]
pub struct DiskMetrics {
    lookup_count: [BoxedCounter; 4],
    hit_duration: BoxedHistogram,
    write_entries: [BoxedCounter; 5],
    eviction_triggering_insertions: BoxedCounter,
    pub(crate) written_entries: BoxedCounter,
    pub(crate) recovered_chunks: BoxedGauge,
    pub(crate) free_chunks: BoxedGauge,
    pub(crate) allocated_chunks: BoxedGauge,
    pub(crate) payload_bytes: BoxedGauge,
    pub(crate) entries: BoxedGauge,
    pub(crate) packed_payload_bytes: BoxedCounter,
    pub(crate) packed_chunk_bytes: BoxedCounter,
}

impl DiskMetrics {
    /// Registers disk-cache metrics independently of raw file I/O metrics.
    pub fn new(registry: &BoxedRegistry) -> Arc<Self> {
        let lookups = registry.register_counter_vec(
            "feuer_disk_lookup_total".into(),
            "Completed disk-tier lookups, including buffered entries; errors become misses".into(),
            &["outcome"],
        );
        let duration = registry.register_histogram_vec_with_buckets(
            "feuer_disk_lookup_duration_seconds".into(),
            "Completed disk-tier hit duration, including buffered copies or verified reads".into(),
            &["outcome"],
            Buckets::exponential(0.000_001, 2.0, 25),
        );
        let write_entries = registry.register_counter_vec(
            "feuer_disk_write_entries_total".into(),
            "Terminal outcomes of entries handled by the disk writer".into(),
            &["outcome"],
        );
        let eviction_triggering_insertions = registry.register_counter_vec(
            "feuer_disk_eviction_triggering_insertions_total".into(),
            "Terminal insertion attempts that evicted at least one entry under capacity pressure".into(),
            &[],
        );
        let written = registry.register_counter_vec(
            "feuer_disk_written_entries_total".into(),
            "Entries in successfully written payload regions, whether published or discarded".into(),
            &[],
        );
        let recovered = registry.register_gauge_vec(
            "feuer_disk_recovered_chunks".into(),
            "Currently allocated chunks retained by recovery, excluding temporary scan reservations".into(),
            &[],
        );
        let chunks = registry.register_gauge_vec(
            "feuer_disk_chunks".into(),
            "Live 1-MiB chunks; allocated includes payloads, metadata, and unfinished writes".into(),
            &["state"],
        );
        let payload = registry.register_gauge_vec(
            "feuer_disk_payload_bytes".into(),
            "Payload bytes in indexed disk entries, excluding metadata and padding".into(),
            &[],
        );
        let entries =
            registry.register_gauge_vec("feuer_disk_entries".into(), "Indexed disk range entries".into(), &[]);
        let packed = registry.register_counter_vec(
            "feuer_disk_batch_bytes_total".into(),
            "Payload and payload-chunk bytes in successfully written regions before publication".into(),
            &["kind"],
        );
        let outcomes = ["hit", "absent", "io_error", "checksum_failed"];
        Arc::new(Self {
            lookup_count: outcomes.map(|label| lookups.counter(&[label.into()])),
            hit_duration: duration.histogram(&["hit".into()]),
            write_entries: ["published", "already_covered", "no_capacity", "failed", "canceled"]
                .map(|label| write_entries.counter(&[label.into()])),
            eviction_triggering_insertions: eviction_triggering_insertions.counter(&[]),
            written_entries: written.counter(&[]),
            recovered_chunks: recovered.gauge(&[]),
            free_chunks: chunks.gauge(&["free".into()]),
            allocated_chunks: chunks.gauge(&["allocated".into()]),
            payload_bytes: payload.gauge(&[]),
            entries: entries.gauge(&[]),
            packed_payload_bytes: packed.counter(&["payload".into()]),
            packed_chunk_bytes: packed.counter(&["chunk".into()]),
        })
    }

    /// Creates unregistered disk-cache metrics.
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

/// Records exactly one terminal outcome even if the writer is dropped.
pub(crate) struct DiskWriteAttempt {
    metrics: Arc<DiskMetrics>,
    outcome: DiskWriteOutcome,
    pub(crate) evicted: bool,
}

impl DiskWriteAttempt {
    pub(crate) fn new(metrics: Arc<DiskMetrics>) -> Self {
        Self {
            metrics,
            outcome: DiskWriteOutcome::Canceled,
            evicted: false,
        }
    }

    pub(crate) fn set_outcome(&mut self, outcome: DiskWriteOutcome) {
        self.outcome = outcome;
    }
}

impl Drop for DiskWriteAttempt {
    fn drop(&mut self) {
        self.metrics.write_entries[self.outcome as usize].increase(1);
        if self.evicted {
            self.metrics.eviction_triggering_insertions.increase(1);
        }
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
            (DiskLookupOutcome::ChecksumFailed, "checksum_failed"),
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
