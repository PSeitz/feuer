use mixtrics::{metrics::BoxedRegistry, registry::prometheus::PrometheusMetricsRegistry};

pub fn registry() -> (prometheus::Registry, BoxedRegistry) {
    let registry = prometheus::Registry::new();
    let metrics = Box::new(PrometheusMetricsRegistry::new(registry.clone()));
    (registry, metrics)
}

/// Returns counter/gauge values or a histogram's sample count. Missing series fail the test.
pub fn value(registry: &prometheus::Registry, metric_name: &str, labels: &[(&str, &str)]) -> f64 {
    let family = registry
        .gather()
        .into_iter()
        .find(|family| family.name() == metric_name)
        .unwrap_or_else(|| panic!("missing metric {metric_name}"));
    let metric = family
        .get_metric()
        .iter()
        .find(|metric| {
            metric.get_label().len() == labels.len()
                && labels.iter().all(|(label_name, label_value)| {
                    metric
                        .get_label()
                        .iter()
                        .any(|label| label.name() == *label_name && label.value() == *label_value)
                })
        })
        .unwrap_or_else(|| panic!("missing labels {labels:?} for {metric_name}"));
    match family.get_field_type() {
        prometheus::proto::MetricType::COUNTER => metric.get_counter().get_value(),
        prometheus::proto::MetricType::GAUGE => metric.get_gauge().get_value(),
        prometheus::proto::MetricType::HISTOGRAM => metric.get_histogram().get_sample_count() as f64,
        kind => panic!("unexpected metric type {kind:?}"),
    }
}
