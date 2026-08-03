//! Integration tests for armature-metrics

use armature_metrics::*;

#[test]
fn test_register_counter() {
    let counter = register_counter("test_counter_1", "Test counter").unwrap();
    counter.inc();
    assert_eq!(counter.get(), 1.0);

    counter.inc_by(5.0);
    assert_eq!(counter.get(), 6.0);
}

#[test]
fn test_register_counter_vec() {
    let counter = register_counter_vec(
        "test_counter_vec_1",
        "Test counter vec",
        &["label1", "label2"],
    )
    .unwrap();

    counter.with_label_values(&["value1", "value2"]).inc();
    counter.with_label_values(&["value1", "value2"]).inc();
    counter.with_label_values(&["value3", "value4"]).inc();

    assert_eq!(counter.with_label_values(&["value1", "value2"]).get(), 2.0);
    assert_eq!(counter.with_label_values(&["value3", "value4"]).get(), 1.0);
}

#[test]
fn test_register_gauge() {
    let gauge = register_gauge("test_gauge_1", "Test gauge").unwrap();

    gauge.set(42.0);
    assert_eq!(gauge.get(), 42.0);

    gauge.inc();
    assert_eq!(gauge.get(), 43.0);

    gauge.dec();
    assert_eq!(gauge.get(), 42.0);

    gauge.add(10.0);
    assert_eq!(gauge.get(), 52.0);

    gauge.sub(2.0);
    assert_eq!(gauge.get(), 50.0);
}

#[test]
fn test_register_gauge_vec() {
    let gauge = register_gauge_vec("test_gauge_vec_1", "Test gauge vec", &["label1"]).unwrap();

    gauge.with_label_values(&["value1"]).set(10.0);
    gauge.with_label_values(&["value2"]).set(20.0);

    assert_eq!(gauge.with_label_values(&["value1"]).get(), 10.0);
    assert_eq!(gauge.with_label_values(&["value2"]).get(), 20.0);
}

#[test]
fn test_register_histogram() {
    let histogram = register_histogram("test_histogram_1", "Test histogram").unwrap();

    histogram.observe(0.5);
    histogram.observe(1.0);
    histogram.observe(2.0);

    assert_eq!(histogram.get_sample_count(), 3);
    assert_eq!(histogram.get_sample_sum(), 3.5);
}

#[test]
fn test_register_histogram_with_buckets() {
    let histogram = register_histogram_with_buckets(
        "test_histogram_buckets_1",
        "Test histogram with buckets",
        vec![0.1, 0.5, 1.0, 5.0],
    )
    .unwrap();

    histogram.observe(0.05); // < 0.1
    histogram.observe(0.25); // < 0.5
    histogram.observe(0.75); // < 1.0
    histogram.observe(3.0); // < 5.0

    assert_eq!(histogram.get_sample_count(), 4);
}

#[test]
fn test_register_histogram_vec() {
    let histogram =
        register_histogram_vec("test_histogram_vec_1", "Test histogram vec", &["operation"])
            .unwrap();

    histogram.with_label_values(&["read"]).observe(0.1);
    histogram.with_label_values(&["read"]).observe(0.2);
    histogram.with_label_values(&["write"]).observe(0.5);

    assert_eq!(histogram.with_label_values(&["read"]).get_sample_count(), 2);
    assert_eq!(
        histogram.with_label_values(&["write"]).get_sample_count(),
        1
    );
}

#[test]
fn test_counter_builder() {
    let counter = CounterBuilder::new("test_builder_counter", "Test builder counter")
        .register()
        .unwrap();

    counter.inc();
    assert_eq!(counter.get(), 1.0);
}

#[test]
fn test_counter_vec_builder() {
    let counter = CounterVecBuilder::new("test_builder_counter_vec", "Test builder counter vec")
        .labels(&["method", "status"])
        .register()
        .unwrap();

    counter.with_label_values(&["GET", "200"]).inc();
    assert_eq!(counter.with_label_values(&["GET", "200"]).get(), 1.0);
}

#[test]
fn test_gauge_builder() {
    let gauge = GaugeBuilder::new("test_builder_gauge", "Test builder gauge")
        .register()
        .unwrap();

    gauge.set(100.0);
    assert_eq!(gauge.get(), 100.0);
}

#[test]
fn test_gauge_vec_builder() {
    let gauge = GaugeVecBuilder::new("test_builder_gauge_vec", "Test builder gauge vec")
        .labels(&["pool"])
        .register()
        .unwrap();

    gauge.with_label_values(&["default"]).set(5.0);
    assert_eq!(gauge.with_label_values(&["default"]).get(), 5.0);
}

#[test]
fn test_histogram_builder() {
    let histogram = HistogramBuilder::new("test_builder_histogram", "Test builder histogram")
        .register()
        .unwrap();

    histogram.observe(1.0);
    assert_eq!(histogram.get_sample_count(), 1);
}

#[test]
fn test_histogram_builder_with_buckets() {
    let histogram = HistogramBuilder::new(
        "test_builder_histogram_buckets",
        "Test builder histogram buckets",
    )
    .buckets(vec![1.0, 5.0, 10.0])
    .register()
    .unwrap();

    histogram.observe(2.0);
    assert_eq!(histogram.get_sample_count(), 1);
}

#[test]
fn test_histogram_builder_latency_buckets() {
    let histogram = HistogramBuilder::new(
        "test_builder_histogram_latency",
        "Test builder histogram latency",
    )
    .latency_buckets()
    .register()
    .unwrap();

    histogram.observe(0.05);
    assert_eq!(histogram.get_sample_count(), 1);
}

#[test]
fn test_histogram_builder_size_buckets() {
    let histogram =
        HistogramBuilder::new("test_builder_histogram_size", "Test builder histogram size")
            .size_buckets()
            .register()
            .unwrap();

    histogram.observe(5000.0);
    assert_eq!(histogram.get_sample_count(), 1);
}

#[test]
fn test_histogram_vec_builder() {
    let histogram =
        HistogramVecBuilder::new("test_builder_histogram_vec", "Test builder histogram vec")
            .labels(&["endpoint"])
            .register()
            .unwrap();

    histogram.with_label_values(&["/api/users"]).observe(0.1);
    assert_eq!(
        histogram
            .with_label_values(&["/api/users"])
            .get_sample_count(),
        1
    );
}

#[test]
fn test_histogram_vec_builder_with_buckets() {
    let histogram = HistogramVecBuilder::new("test_builder_histogram_vec_buckets", "Test")
        .labels(&["method"])
        .buckets(vec![0.1, 0.5, 1.0])
        .register()
        .unwrap();

    histogram.with_label_values(&["GET"]).observe(0.25);
    assert_eq!(histogram.with_label_values(&["GET"]).get_sample_count(), 1);
}

#[test]
fn test_export_metrics() {
    let _counter = register_counter("test_export_counter", "Test export counter").unwrap();

    let metrics = export_metrics();
    assert!(
        metrics.contains("# HELP")
            || metrics.contains("test_export_counter")
            || !metrics.is_empty()
    );
}

#[test]
fn test_default_registry() {
    let registry = default_registry();
    let initial_count = registry.gather().len();

    let _counter = register_counter("test_registry_counter", "Test registry counter").unwrap();

    let final_count = registry.gather().len();
    assert!(final_count >= initial_count);
}

#[tokio::test]
async fn test_metrics_handler() {
    use armature_core::HttpRequest;

    let request = HttpRequest::new("GET", "/metrics".to_string());
    let response = metrics_handler(request).await.unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("Content-Type"),
        Some(&"text/plain; version=0.0.4".to_string())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn test_export_contains_process_metrics() {
    // The `ProcessCollector` must be registered into the *local* default
    // registry (the one `export_metrics` reads), not the global prometheus
    // registry, or process cpu/memory series never appear in the output.
    let metrics = export_metrics();
    assert!(
        metrics.contains("process_cpu_seconds_total")
            || metrics.contains("process_resident_memory_bytes")
            || metrics.contains("process_start_time_seconds"),
        "expected process metrics in export, got:\n{metrics}"
    );
}

#[test]
fn test_register_summary_exports() {
    let summary = register_summary("test_summary_integration", "Integration summary").unwrap();
    summary.observe(0.5);
    summary.observe(1.0);
    summary.observe(1.5);

    assert_eq!(summary.get_sample_count(), 3);
    assert_eq!(summary.quantile(0.5), Some(1.0));

    let metrics = export_metrics();
    assert!(metrics.contains("test_summary_integration"));
}

#[test]
fn test_request_metrics_middleware_new() {
    let _middleware = RequestMetricsMiddleware::new();
    // Just ensure it can be created
}

#[test]
fn test_request_metrics_middleware_without_path() {
    let _middleware = RequestMetricsMiddleware::without_path();
    // Just ensure it can be created
}
