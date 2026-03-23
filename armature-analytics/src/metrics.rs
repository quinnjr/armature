//! Metrics types and helpers

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Metric value types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Counter(u64),
    Gauge(f64),
    Histogram(HistogramValue),
}

/// Histogram metric value with buckets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramValue {
    pub count: u64,
    pub sum: f64,
    pub buckets: Vec<(f64, u64)>,
}

impl HistogramValue {
    pub fn new() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            buckets: vec![
                (0.005, 0), // 5ms
                (0.01, 0),  // 10ms
                (0.025, 0), // 25ms
                (0.05, 0),  // 50ms
                (0.1, 0),   // 100ms
                (0.25, 0),  // 250ms
                (0.5, 0),   // 500ms
                (1.0, 0),   // 1s
                (2.5, 0),   // 2.5s
                (5.0, 0),   // 5s
                (10.0, 0),  // 10s
                (f64::INFINITY, 0),
            ],
        }
    }

    pub fn observe(&mut self, value: f64) {
        self.count += 1;
        self.sum += value;

        for (bound, count) in &mut self.buckets {
            if value <= *bound {
                *count += 1;
            }
        }
    }
}

impl Default for HistogramValue {
    fn default() -> Self {
        Self::new()
    }
}

/// Named metric with labels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub help: String,
    pub metric_type: MetricType,
    pub value: MetricValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<(String, String)>>,
}

impl Metric {
    pub fn counter(name: impl Into<String>, help: impl Into<String>, value: u64) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            metric_type: MetricType::Counter,
            value: MetricValue::Counter(value),
            labels: None,
        }
    }

    pub fn gauge(name: impl Into<String>, help: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            metric_type: MetricType::Gauge,
            value: MetricValue::Gauge(value),
            labels: None,
        }
    }

    pub fn histogram(
        name: impl Into<String>,
        help: impl Into<String>,
        histogram: HistogramValue,
    ) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            metric_type: MetricType::Histogram,
            value: MetricValue::Histogram(histogram),
            labels: None,
        }
    }

    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let labels = self.labels.get_or_insert_with(Vec::new);
        labels.push((key.into(), value.into()));
        self
    }

    pub fn with_labels(mut self, labels: Vec<(String, String)>) -> Self {
        self.labels = Some(labels);
        self
    }
}

/// Metric types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Summary,
}

/// Time series data point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub timestamp: i64,
    pub value: f64,
}

/// Time series data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeries {
    pub name: String,
    pub points: Vec<TimeSeriesPoint>,
}

impl TimeSeries {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            points: Vec::new(),
        }
    }

    pub fn add_point(&mut self, timestamp: i64, value: f64) {
        self.points.push(TimeSeriesPoint { timestamp, value });
    }
}

/// Duration formatting helpers
pub trait DurationExt {
    fn as_millis_f64(&self) -> f64;
    fn as_micros_f64(&self) -> f64;
}

impl DurationExt for Duration {
    fn as_millis_f64(&self) -> f64 {
        self.as_secs_f64() * 1000.0
    }

    fn as_micros_f64(&self) -> f64 {
        self.as_secs_f64() * 1_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_histogram() {
        let mut hist = HistogramValue::new();
        hist.observe(0.001); // 1ms
        hist.observe(0.050); // 50ms
        hist.observe(0.500); // 500ms

        assert_eq!(hist.count, 3);
    }

    #[test]
    fn test_metric_with_labels() {
        let metric = Metric::counter("http_requests_total", "Total HTTP requests", 100)
            .with_label("method", "GET")
            .with_label("status", "200");

        assert_eq!(metric.labels.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_duration_ext() {
        let duration = Duration::from_millis(150);
        assert_eq!(duration.as_millis_f64(), 150.0);
    }
}
