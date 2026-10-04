use std::{sync::Arc, time::Duration};

use mixtrics::metrics::{BoxedCounter, BoxedHistogram, BoxedRegistry, Buckets};

use crate::IoOperation;

#[derive(Debug)]
struct IoOperationMetrics {
    success: BoxedCounter,
    error: BoxedCounter,
    bytes: BoxedCounter,
    success_duration: BoxedHistogram,
}

/// Internal metric handles for fixed-file positional I/O.
///
/// Feuer's public API does not expose this as a statistics snapshot. The
/// handles emit counters, gauges and histograms through the configured
/// `mixtrics` registry with read/write operation and success/error outcome labels.
#[derive(Debug)]
pub struct IoMetrics {
    read: IoOperationMetrics,
    write: IoOperationMetrics,
    read_size: BoxedHistogram,
}

impl IoMetrics {
    /// Registers fixed-file I/O metrics with read/write operation and success/error outcome labels.
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
    fn read_sizes_and_durations_only_record_successes_but_counters_include_errors() {
        let (registry, backend) = crate::test_metrics::registry();
        let metrics = IoMetrics::new(&backend);
        let elapsed = Duration::from_micros(2);

        metrics.record(IoOperation::Read, 17, elapsed, true);
        metrics.record(IoOperation::Read, 4096, elapsed, true);
        metrics.record(IoOperation::Read, 8192, elapsed, false);
        metrics.record(IoOperation::Write, 16384, elapsed, true);
        metrics.record(IoOperation::Write, 0, elapsed, false);

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
        for (operation, successes) in [(IoOperation::Read, 2.0), (IoOperation::Write, 1.0)] {
            for (metric_name, outcome, count) in [
                ("feuer_disk_io_total", "success", successes),
                ("feuer_disk_io_total", "error", 1.0),
                ("feuer_disk_io_duration_seconds", "success", successes),
            ] {
                assert_eq!(
                    crate::test_metrics::value(
                        &registry,
                        metric_name,
                        &[("operation", operation.as_str()), ("outcome", outcome)]
                    ),
                    count
                );
            }
        }
        let family = registry
            .gather()
            .into_iter()
            .find(|family| family.name() == "feuer_disk_io_duration_seconds")
            .unwrap();
        assert_eq!(family.get_metric().len(), 2);
    }
}
