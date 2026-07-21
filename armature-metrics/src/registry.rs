//! Metric registry and registration helpers

use prometheus::{Counter, CounterVec, Gauge, GaugeVec, Histogram, HistogramVec, Registry};

/// Register a counter metric
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let counter = register_counter("requests_total", "Total number of requests").unwrap();
/// counter.inc();
/// ```
pub fn register_counter(name: &str, help: &str) -> Result<Counter, prometheus::Error> {
    let counter = Counter::new(name, help)?;
    crate::default_registry().register(Box::new(counter.clone()))?;
    Ok(counter)
}

/// Register a counter with labels
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let counter = register_counter_vec(
///     "http_requests_total",
///     "Total HTTP requests",
///     &["method", "status"]
/// ).unwrap();
///
/// counter.with_label_values(&["GET", "200"]).inc();
/// ```
pub fn register_counter_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<CounterVec, prometheus::Error> {
    let counter = CounterVec::new(prometheus::Opts::new(name, help), label_names)?;
    crate::default_registry().register(Box::new(counter.clone()))?;
    Ok(counter)
}

/// Register a gauge metric
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let gauge = register_gauge("active_connections", "Number of active connections").unwrap();
/// gauge.set(42.0);
/// ```
pub fn register_gauge(name: &str, help: &str) -> Result<Gauge, prometheus::Error> {
    let gauge = Gauge::new(name, help)?;
    crate::default_registry().register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

/// Register a gauge with labels
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let gauge = register_gauge_vec(
///     "queue_size",
///     "Size of the queue",
///     &["queue_name"]
/// ).unwrap();
///
/// gauge.with_label_values(&["default"]).set(10.0);
/// ```
pub fn register_gauge_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<GaugeVec, prometheus::Error> {
    let gauge = GaugeVec::new(prometheus::Opts::new(name, help), label_names)?;
    crate::default_registry().register(Box::new(gauge.clone()))?;
    Ok(gauge)
}

/// Register a histogram metric
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let histogram = register_histogram(
///     "request_duration_seconds",
///     "Request duration in seconds"
/// ).unwrap();
///
/// histogram.observe(0.5);
/// ```
pub fn register_histogram(name: &str, help: &str) -> Result<Histogram, prometheus::Error> {
    let histogram = Histogram::with_opts(prometheus::HistogramOpts::new(name, help))?;
    crate::default_registry().register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

/// Register a histogram with custom buckets
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let histogram = register_histogram_with_buckets(
///     "api_latency_seconds",
///     "API latency in seconds",
///     vec![0.001, 0.01, 0.1, 0.5, 1.0, 5.0]
/// ).unwrap();
///
/// histogram.observe(0.25);
/// ```
pub fn register_histogram_with_buckets(
    name: &str,
    help: &str,
    buckets: Vec<f64>,
) -> Result<Histogram, prometheus::Error> {
    let opts = prometheus::HistogramOpts::new(name, help).buckets(buckets);
    let histogram = Histogram::with_opts(opts)?;
    crate::default_registry().register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

/// Register a histogram with labels
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let histogram = register_histogram_vec(
///     "http_request_duration_seconds",
///     "HTTP request duration",
///     &["method", "endpoint"]
/// ).unwrap();
///
/// histogram.with_label_values(&["GET", "/api/users"]).observe(0.123);
/// ```
pub fn register_histogram_vec(
    name: &str,
    help: &str,
    label_names: &[&str],
) -> Result<HistogramVec, prometheus::Error> {
    let histogram = HistogramVec::new(prometheus::HistogramOpts::new(name, help), label_names)?;
    crate::default_registry().register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

/// Register a histogram with labels and custom buckets
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let histogram = register_histogram_vec_with_buckets(
///     "db_query_duration_seconds",
///     "Database query duration",
///     &["operation"],
///     vec![0.001, 0.01, 0.1, 0.5, 1.0]
/// ).unwrap();
///
/// histogram.with_label_values(&["SELECT"]).observe(0.05);
/// ```
pub fn register_histogram_vec_with_buckets(
    name: &str,
    help: &str,
    label_names: &[&str],
    buckets: Vec<f64>,
) -> Result<HistogramVec, prometheus::Error> {
    let opts = prometheus::HistogramOpts::new(name, help).buckets(buckets);
    let histogram = HistogramVec::new(opts, label_names)?;
    crate::default_registry().register(Box::new(histogram.clone()))?;
    Ok(histogram)
}

/// Register a summary metric
///
/// Summaries expose configurable φ-quantiles over a sliding window plus a
/// cumulative sample count and sum.
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let summary = register_summary(
///     "request_latency_seconds",
///     "Request latency in seconds"
/// ).unwrap();
///
/// summary.observe(0.25);
/// ```
pub fn register_summary(name: &str, help: &str) -> Result<crate::Summary, prometheus::Error> {
    let summary = crate::Summary::new(name, help)?;
    crate::default_registry().register(Box::new(summary.clone()))?;
    Ok(summary)
}

/// Register a summary metric with explicit options (quantiles, window, size)
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
/// use std::time::Duration;
///
/// let summary = register_summary_with_opts(
///     SummaryOpts::new("db_query_seconds", "DB query latency")
///         .quantiles(vec![0.5, 0.95, 0.99])
///         .max_age(Duration::from_secs(60)),
/// )
/// .unwrap();
///
/// summary.observe(0.01);
/// ```
pub fn register_summary_with_opts(
    opts: crate::SummaryOpts,
) -> Result<crate::Summary, prometheus::Error> {
    let summary = crate::Summary::with_opts(opts)?;
    crate::default_registry().register(Box::new(summary.clone()))?;
    Ok(summary)
}

/// Register metrics with a custom registry
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
/// use prometheus::Registry;
///
/// let registry = Registry::new();
/// let counter = register_counter_with_registry(
///     &registry,
///     "custom_counter",
///     "A custom counter"
/// ).unwrap();
/// ```
pub fn register_counter_with_registry(
    registry: &Registry,
    name: &str,
    help: &str,
) -> Result<Counter, prometheus::Error> {
    let counter = Counter::new(name, help)?;
    registry.register(Box::new(counter.clone()))?;
    Ok(counter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_counter() {
        let result = register_counter("test_counter", "Test counter");
        assert!(result.is_ok());
    }

    #[test]
    fn test_register_gauge() {
        let result = register_gauge("test_gauge", "Test gauge");
        assert!(result.is_ok());
    }

    #[test]
    fn test_register_histogram() {
        let result = register_histogram("test_histogram", "Test histogram");
        assert!(result.is_ok());
    }
}
