use std::{fmt, sync::Arc, time::Duration};

use mixtrics::metrics::{BoxedCounter, BoxedGauge, BoxedHistogram, BoxedRegistry, Buckets};

use crate::IoOperation;

/// Metrics for one size class of idle aligned I/O buffers.
#[derive(Debug)]
pub(crate) struct IoBufferPoolMetrics {
    pub(crate) idle_bytes: BoxedGauge,
    pub(crate) capacity_bytes: BoxedGauge,
    pub(crate) returned: BoxedCounter,
    pub(crate) dropped: BoxedCounter,
}

struct IoOperationMetrics {
    success: BoxedCounter,
    error: BoxedCounter,
    bytes: BoxedCounter,
    success_duration: BoxedHistogram,
}

impl fmt::Debug for IoOperationMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IoOperationMetrics").finish_non_exhaustive()
    }
}

/// Internal metric handles for fixed-file positional I/O.
///
/// Feuer's public API does not expose this as a statistics snapshot. The
/// handles emit counters, gauges and histograms through the configured
/// `mixtrics` registry using only bounded labels.
#[derive(Debug)]
pub struct IoMetrics {
    read: IoOperationMetrics,
    write: IoOperationMetrics,
    read_size: BoxedHistogram,
    pub(crate) read_buffer_pools: [Arc<IoBufferPoolMetrics>; 3],
}

impl IoMetrics {
    /// Registers fixed-file I/O metrics using only bounded labels.
    pub fn new(registry: &BoxedRegistry) -> Arc<Self> {
        let operations = registry.register_counter_vec(
            "feuer_disk_io_total".into(),
            "Completed Feuer data-file operations".into(),
            &["operation", "outcome"],
        );
        let bytes = registry.register_counter_vec(
            "feuer_disk_io_bytes_total".into(),
            "Bytes completed by Feuer data-file operations".into(),
            &["operation"],
        );
        let duration = registry.register_histogram_vec_with_buckets(
            "feuer_disk_io_duration_seconds".into(),
            "Successful Feuer data-file operation duration in seconds".into(),
            &["operation", "outcome"],
            Buckets::exponential(0.000_001, 2.0, 25),
        );

        let read_size = registry.register_histogram_vec_with_buckets(
            "feuer_disk_read_size_bytes".into(),
            "Requested bytes per successful Feuer data-file read, excluding alignment padding".into(),
            &[],
            Buckets::exponential(1024.0, 2.0, 21),
        );

        let idle_bytes = registry.register_gauge_vec(
            "feuer_io_buffer_pool_idle_bytes".into(),
            "Idle aligned read buffer bytes available for reuse".into(),
            &["pool"],
        );
        let capacity_bytes = registry.register_gauge_vec(
            "feuer_io_buffer_pool_capacity_bytes".into(),
            "Live read buffer pools' configured idle byte capacity".into(),
            &["pool"],
        );
        let returns = registry.register_counter_vec(
            "feuer_io_buffer_pool_returns_total".into(),
            "Released buffers returned to live read pools or dropped due to insufficient idle capacity".into(),
            &["pool", "outcome"],
        );

        let operation_metrics = |label: &'static str| IoOperationMetrics {
            success: operations.counter(&[label.into(), "success".into()]),
            error: operations.counter(&[label.into(), "error".into()]),
            bytes: bytes.counter(&[label.into()]),
            success_duration: duration.histogram(&[label.into(), "success".into()]),
        };

        Arc::new(Self {
            read: operation_metrics(IoOperation::Read.as_str()),
            write: operation_metrics(IoOperation::Write.as_str()),
            read_size: read_size.histogram(&[]),
            read_buffer_pools: ["small", "medium", "large"].map(|pool| {
                Arc::new(IoBufferPoolMetrics {
                    idle_bytes: idle_bytes.gauge(&[pool.into()]),
                    capacity_bytes: capacity_bytes.gauge(&[pool.into()]),
                    returned: returns.counter(&[pool.into(), "returned".into()]),
                    dropped: returns.counter(&[pool.into(), "dropped".into()]),
                })
            }),
        })
    }

    pub(crate) fn record(&self, operation: IoOperation, bytes: u64, elapsed: Duration, success: bool) {
        let metrics = match operation {
            IoOperation::Read => &self.read,
            IoOperation::Write => &self.write,
            _ => return,
        };

        if success {
            metrics.success.increase(1);
            metrics.bytes.increase(bytes);
            metrics.success_duration.record(elapsed.as_secs_f64());
            if operation == IoOperation::Read {
                self.read_size.record(bytes as f64);
            }
        } else {
            metrics.error.increase(1);
        }
    }

    /// Creates unregistered no-op metrics for caches without a metrics registry.
    pub fn noop() -> Arc<Self> {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::new(&registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_size_records_only_successful_reads() {
        let (registry, backend) = crate::test_metrics::registry();
        let metrics = IoMetrics::new(&backend);
        let elapsed = Duration::from_micros(2);

        metrics.record(IoOperation::Read, 17, elapsed, true);
        metrics.record(IoOperation::Read, 4096, elapsed, true);
        metrics.record(IoOperation::Read, 8192, elapsed, false);
        metrics.record(IoOperation::Write, 16384, elapsed, true);

        let family = registry
            .gather()
            .into_iter()
            .find(|family| family.name() == "feuer_disk_read_size_bytes")
            .unwrap();
        let histogram = family.get_metric()[0].get_histogram();
        assert_eq!(histogram.get_sample_count(), 2);
        assert_eq!(histogram.get_sample_sum(), 4113.0);
        for (bound, count) in [(1024.0, 1), (2048.0, 1), (4096.0, 2)] {
            let bucket = histogram
                .get_bucket()
                .iter()
                .find(|bucket| bucket.upper_bound() == bound)
                .unwrap();
            assert_eq!(bucket.cumulative_count(), count);
        }
    }

    #[test]
    fn durations_only_record_successes_but_counters_include_errors() {
        let (registry, backend) = crate::test_metrics::registry();
        let metrics = IoMetrics::new(&backend);
        for operation in [IoOperation::Read, IoOperation::Write] {
            metrics.record(operation, 17, Duration::from_micros(2), true);
            metrics.record(operation, 0, Duration::from_micros(3), false);
            for outcome in ["success", "error"] {
                assert_eq!(
                    crate::test_metrics::value(
                        &registry,
                        "feuer_disk_io_total",
                        &[("operation", operation.as_str()), ("outcome", outcome)]
                    ),
                    1.0
                );
            }
        }
        let family = registry
            .gather()
            .into_iter()
            .find(|family| family.name() == "feuer_disk_io_duration_seconds")
            .unwrap();
        assert_eq!(family.get_metric().len(), 2);
        for metric in family.get_metric() {
            assert_eq!(metric.get_histogram().get_sample_count(), 1);
            assert!(
                metric
                    .get_label()
                    .iter()
                    .any(|label| { label.name() == "outcome" && label.value() == "success" })
            );
        }
    }
}
