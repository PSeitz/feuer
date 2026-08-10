use std::{fmt, sync::Arc};

use mixtrics::metrics::{BoxedCounter, BoxedGauge, BoxedRegistry};

/// Internal metric handles for Feuer's in-memory range tier.
///
/// Operations use a fixed set of labels. Object identities and caller-defined
/// cache names are never metric labels.
pub struct MemoryMetrics {
    insert: BoxedCounter,
    replace: BoxedCounter,
    redundant: BoxedCounter,
    access: BoxedCounter,
    hit: BoxedCounter,
    miss: BoxedCounter,
    remove: BoxedCounter,
    evict: BoxedCounter,
    compact: BoxedCounter,
    compacted_payload_bytes: BoxedCounter,
    payload_bytes: BoxedGauge,
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
        let compacted_payload_bytes = registry.register_counter_vec(
            "feuer_memory_compacted_payload_bytes_total".into(),
            "Downloaded payload bytes released by in-memory compaction".into(),
            &[],
        );
        let payload_bytes = registry.register_gauge_vec(
            "feuer_memory_payload_bytes".into(),
            "Downloaded payload bytes retained in Feuer's memory tier".into(),
            &[],
        );
        let entries = registry.register_gauge_vec(
            "feuer_memory_entries".into(),
            "Downloaded range entries retained in Feuer's memory tier".into(),
            &[],
        );
        let operation = |label: &'static str| operations.counter(&[label.into()]);

        Arc::new(Self {
            insert: operation("insert"),
            replace: operation("replace"),
            redundant: operation("redundant"),
            access: operation("access"),
            hit: operation("hit"),
            miss: operation("miss"),
            remove: operation("remove"),
            evict: operation("evict"),
            compact: operation("compact"),
            compacted_payload_bytes: compacted_payload_bytes.counter(&[]),
            payload_bytes: payload_bytes.gauge(&[]),
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

    pub(crate) fn record_access(&self) {
        self.access.increase(1);
    }

    pub(crate) fn record_lookup(&self, hit: bool) {
        if hit {
            self.hit.increase(1);
        } else {
            self.miss.increase(1);
        }
    }

    pub(crate) fn record_remove(&self) {
        self.remove.increase(1);
    }

    pub(crate) fn record_evictions(&self, count: u64) {
        self.evict.increase(count);
    }

    pub(crate) fn record_compaction(&self, reclaimed_bytes: u64) {
        self.compact.increase(1);
        self.compacted_payload_bytes.increase(reclaimed_bytes);
    }

    pub(crate) fn increase_usage(&self, bytes: u64, entries: u64) {
        self.payload_bytes.increase(bytes);
        self.entries.increase(entries);
    }

    pub(crate) fn decrease_usage(&self, bytes: u64, entries: u64) {
        self.payload_bytes.decrease(bytes);
        self.entries.decrease(entries);
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
        metrics.record_access();
        metrics.record_lookup(true);
        metrics.increase_usage(17, 1);
        metrics.record_evictions(1);
        metrics.record_compaction(3);
        metrics.decrease_usage(17, 1);
    }
}
