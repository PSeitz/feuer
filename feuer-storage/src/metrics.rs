use std::{fmt, sync::Arc, time::Duration};

use mixtrics::metrics::{BoxedCounter, BoxedHistogram, BoxedRegistry, Buckets};

use crate::IoOperation;

struct OperationMetrics {
    success: BoxedCounter,
    error: BoxedCounter,
    bytes: BoxedCounter,
    success_duration: BoxedHistogram,
    error_duration: BoxedHistogram,
}

impl fmt::Debug for OperationMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationMetrics").finish_non_exhaustive()
    }
}

/// Internal metric handles for fixed-file positional I/O.
///
/// Feuer's public API does not expose this as a statistics snapshot. The
/// handles emit monotonic counters and histograms through the configured
/// `mixtrics` registry using only bounded operation and outcome labels.
#[derive(Debug)]
pub struct IoMetrics {
    read: OperationMetrics,
    write: OperationMetrics,
    sync_data: OperationMetrics,
    sync_all: OperationMetrics,
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
            "Feuer data-file operation duration in seconds".into(),
            &["operation", "outcome"],
            Buckets::exponential(0.000_001, 2.0, 25),
        );

        let operation = |label: &'static str| OperationMetrics {
            success: operations.counter(&[label.into(), "success".into()]),
            error: operations.counter(&[label.into(), "error".into()]),
            bytes: bytes.counter(&[label.into()]),
            success_duration: duration.histogram(&[label.into(), "success".into()]),
            error_duration: duration.histogram(&[label.into(), "error".into()]),
        };

        Arc::new(Self {
            read: operation(IoOperation::Read.as_str()),
            write: operation(IoOperation::Write.as_str()),
            sync_data: operation(IoOperation::SyncData.as_str()),
            sync_all: operation(IoOperation::SyncAll.as_str()),
        })
    }

    pub(crate) fn record(&self, operation: IoOperation, bytes: u64, elapsed: Duration, success: bool) {
        let metrics = match operation {
            IoOperation::Read => &self.read,
            IoOperation::Write => &self.write,
            IoOperation::SyncData => &self.sync_data,
            IoOperation::SyncAll => &self.sync_all,
            _ => return,
        };

        if success {
            metrics.success.increase(1);
            metrics.bytes.increase(bytes);
            metrics.success_duration.record(elapsed.as_secs_f64());
        } else {
            metrics.error.increase(1);
            metrics.error_duration.record(elapsed.as_secs_f64());
        }
    }

    #[cfg(test)]
    pub(crate) fn noop() -> Arc<Self> {
        let registry: BoxedRegistry = Box::new(mixtrics::registry::noop::NoopMetricsRegistry);
        Self::new(&registry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_with_the_normal_registry_boundary() {
        let metrics = IoMetrics::noop();

        metrics.record(IoOperation::Read, 17, Duration::from_micros(2), true);
        metrics.record(IoOperation::Write, 0, Duration::from_micros(3), false);
    }
}
