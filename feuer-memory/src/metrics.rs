use std::{array::from_fn, sync::Arc};

use bytesize::ByteSize;
use mixtrics::{
    metrics::{BoxedCounter, BoxedGauge, BoxedRegistry},
    registry::noop::NoopMetricsRegistry,
};

use crate::buffer::{BUCKET_COUNT, BUFFER_SIZES};

/// Internal metric handles for Feuer's in-memory object tier.
///
/// Operations use a fixed set of labels. Object identities and caller-defined
/// cache names are never metric labels.
#[derive(Debug)]
pub struct MemoryMetrics {
    pub(crate) insertions: BoxedCounter,
    pub(crate) redundant_insertions: BoxedCounter,
    pub(crate) removals: BoxedCounter,
    pub(crate) eviction_triggering_insertions: BoxedCounter,
    pub(crate) used_bytes: BoxedGauge,
    pub(crate) capacity_bytes: BoxedGauge,
    pub(crate) idle_buffer_bytes: [BoxedGauge; BUCKET_COUNT],
    pub(crate) used_buffer_bytes: [BoxedGauge; BUCKET_COUNT],
    pub(crate) entries: BoxedGauge,
}

impl MemoryMetrics {
    /// Registers memory-tier metrics with predefined operation, buffer-size, and buffer-status labels.
    pub fn new(registry: &BoxedRegistry) -> Arc<Self> {
        let operations = registry.register_counter_vec(
            "feuer_memory_operations_total".into(),
            "Operations completed by Feuer's in-memory object tier".into(),
            &["operation"],
        );
        let eviction_triggering_insertions = registry.register_counter_vec(
            "feuer_memory_eviction_triggering_insertions_total".into(),
            "Completed insertion attempts that evicted at least one entry under capacity pressure".into(),
            &[],
        );
        let used_bytes = registry.register_gauge_vec(
            "feuer_memory_used_bytes".into(),
            "Cached allocation charges plus idle buffer capacity".into(),
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
            "Object entries in Feuer's memory tier".into(),
            &[],
        );
        let operation_counter = |label: &'static str| operations.counter(&[label.into()]);
        let buffer_gauges = |status: &'static str| {
            from_fn(|index| {
                let bucket = match BUFFER_SIZES.get(index) {
                    Some(&size) => format!("{:.0}", ByteSize(size as u64)),
                    None => format!(">{:.0}", ByteSize(*BUFFER_SIZES.last().unwrap() as u64)),
                };
                buffer_bytes.gauge(&[bucket.into(), status.into()])
            })
        };

        Arc::new(Self {
            insertions: operation_counter("insert"),
            redundant_insertions: operation_counter("redundant"),
            removals: operation_counter("remove"),
            eviction_triggering_insertions: eviction_triggering_insertions.counter(&[]),
            used_bytes: used_bytes.gauge(&[]),
            capacity_bytes: capacity_bytes.gauge(&[]),
            idle_buffer_bytes: buffer_gauges("idle"),
            used_buffer_bytes: buffer_gauges("used"),
            entries: entries.gauge(&[]),
        })
    }

    pub(crate) fn noop() -> Arc<Self> {
        let registry: BoxedRegistry = Box::new(NoopMetricsRegistry);
        Self::new(&registry)
    }
}
