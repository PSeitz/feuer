use std::{fmt, sync::Arc};

use bytesize::ByteSize;
use mixtrics::metrics::{BoxedCounter, BoxedGauge, BoxedRegistry};

use crate::buffer::BUFFER_SIZES;

/// Internal metric handles for Feuer's in-memory range tier.
///
/// Operations use a fixed set of labels. Object identities and caller-defined
/// cache names are never metric labels.
pub struct MemoryMetrics {
    insert: BoxedCounter,
    replace: BoxedCounter,
    redundant: BoxedCounter,
    remove: BoxedCounter,
    pub(crate) eviction_triggering_insertions: BoxedCounter,
    trim: BoxedCounter,
    trimmed_payload_bytes: BoxedCounter,
    used_bytes: BoxedGauge,
    pub(crate) capacity_bytes: BoxedGauge,
    pub(crate) idle_buffer_bytes: [BoxedGauge; BUFFER_SIZES.len()],
    pub(crate) used_buffer_bytes: [BoxedGauge; BUFFER_SIZES.len()],
    entries: BoxedGauge,
}

impl fmt::Debug for MemoryMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryMetrics").finish_non_exhaustive()
    }
}

impl MemoryMetrics {
    /// Registers memory-tier metrics using only bounded labels.
    pub fn new(registry: &BoxedRegistry) -> Arc<Self> {
        let operations = registry.register_counter_vec(
            "feuer_memory_operations_total".into(),
            "Operations completed by Feuer's in-memory range tier".into(),
            &["operation"],
        );
        let eviction_triggering_insertions = registry.register_counter_vec(
            "feuer_memory_eviction_triggering_insertions_total".into(),
            "Completed insertion attempts that evicted at least one entry under capacity pressure".into(),
            &[],
        );
        let trimmed_payload_bytes = registry.register_counter_vec(
            "feuer_memory_compacted_payload_bytes_total".into(),
            "Cached allocation bytes released by in-memory compaction".into(),
            &[],
        );
        let used_bytes = registry.register_gauge_vec(
            "feuer_memory_used_bytes".into(),
            "Allocation bytes retained by cached entries and idle buffers".into(),
            &[],
        );
        let capacity_bytes = registry.register_gauge_vec(
            "feuer_memory_capacity_bytes".into(),
            "Shared capacity of live memory caches and their buffer pools".into(),
            &[],
        );
        let buffer_bytes = registry.register_gauge_vec(
            "feuer_io_buffer_pool_bytes".into(),
            "Aligned buffer allocation bytes by size and status".into(),
            &["bucket", "status"],
        );
        let entries = registry.register_gauge_vec(
            "feuer_memory_entries".into(),
            "Downloaded range entries retained in Feuer's memory tier".into(),
            &[],
        );
        let operation_counter = |label: &'static str| operations.counter(&[label.into()]);
        let buffer_gauges = |status: &'static str| {
            BUFFER_SIZES.map(|size| {
                let bucket = format!("{:.0}", ByteSize(size as u64));
                buffer_bytes.gauge(&[bucket.into(), status.into()])
            })
        };

        Arc::new(Self {
            insert: operation_counter("insert"),
            replace: operation_counter("replace"),
            redundant: operation_counter("redundant"),
            remove: operation_counter("remove"),
            eviction_triggering_insertions: eviction_triggering_insertions.counter(&[]),
            trim: operation_counter("compact"),
            trimmed_payload_bytes: trimmed_payload_bytes.counter(&[]),
            used_bytes: used_bytes.gauge(&[]),
            capacity_bytes: capacity_bytes.gauge(&[]),
            idle_buffer_bytes: buffer_gauges("idle"),
            used_buffer_bytes: buffer_gauges("used"),
            entries: entries.gauge(&[]),
        })
    }

    pub(crate) fn record_insert(&self, replaced: bool) {
        if replaced {
            self.replace.increase(1);
        } else {
            self.insert.increase(1);
        }
    }

    pub(crate) fn record_redundant(&self) {
        self.redundant.increase(1);
    }

    pub(crate) fn record_remove(&self) {
        self.remove.increase(1);
    }

    pub(crate) fn record_range_trim(&self, reclaimed_bytes: u64) {
        self.trim.increase(1);
        self.trimmed_payload_bytes.increase(reclaimed_bytes);
    }

    pub(crate) fn increase_usage(&self, allocation_bytes: u64, entry_count: u64) {
        self.used_bytes.increase(allocation_bytes);
        self.entries.increase(entry_count);
    }

    pub(crate) fn decrease_usage(&self, allocation_bytes: u64, entry_count: u64) {
        self.used_bytes.decrease(allocation_bytes);
        self.entries.decrease(entry_count);
    }

    pub(crate) fn noop() -> Arc<Self> {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::new(&registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_and_updates_through_the_normal_registry_boundary() {
        let metrics = MemoryMetrics::noop();

        metrics.record_insert(false);
        metrics.record_redundant();
        metrics.increase_usage(17, 1);
        metrics.record_range_trim(3);
        metrics.decrease_usage(17, 1);
    }
}
