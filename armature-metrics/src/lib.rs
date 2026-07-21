//! Prometheus metrics and monitoring for Armature
//!
//! This crate provides comprehensive metrics collection and export using Prometheus.
//!
//! # Features
//!
//! - **Prometheus Integration** - Native Prometheus metrics
//! - **Request Metrics** - Automatic HTTP request instrumentation
//! - **Business Metrics** - Custom metric registration
//! - **Multiple Metric Types** - Counter, Gauge, Histogram, Summary
//! - **Labels** - Support for metric labels
//! - **/metrics Endpoint** - Automatic metrics endpoint
//!
//! # Quick Start
//!
//! ```no_run
//! use armature_metrics::*;
//!
//! // Get default metrics registry
//! let registry = default_registry();
//!
//! // Create a counter
//! let counter = register_counter("my_counter", "My counter help").unwrap();
//! counter.inc();
//!
//! // Export metrics
//! let metrics_text = export_metrics();
//! ```

pub mod counter;
pub mod endpoint;
pub mod gauge;
pub mod histogram;
pub mod middleware;
pub mod registry;
pub mod summary;

pub use counter::*;
pub use endpoint::*;
pub use gauge::*;
pub use histogram::*;
pub use middleware::*;
pub use prometheus;
pub use registry::*;
pub use summary::*;

use once_cell::sync::Lazy;
use prometheus::{Encoder, Registry, TextEncoder};

/// Global default registry
static DEFAULT_REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let registry = Registry::new();

    // Register default process metrics into *this* registry (Linux only -
    // process_collector requires procfs). Registering into the global
    // `prometheus::default_registry()` would mean `export_metrics`, which reads
    // this local registry, never emits process cpu/memory series.
    #[cfg(target_os = "linux")]
    {
        if let Err(e) = registry.register(Box::new(
            prometheus::process_collector::ProcessCollector::for_self(),
        )) {
            tracing::warn!("Failed to register process collector: {}", e);
        }
    }

    registry
});

/// Get the default metrics registry
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let registry = default_registry();
/// ```
pub fn default_registry() -> &'static Registry {
    &DEFAULT_REGISTRY
}

/// Export all metrics as Prometheus text format
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
///
/// let metrics = export_metrics();
/// println!("{}", metrics);
/// ```
pub fn export_metrics() -> String {
    export_metrics_from_registry(&DEFAULT_REGISTRY)
}

/// Export metrics from a specific registry
///
/// # Examples
///
/// ```
/// use armature_metrics::*;
/// use prometheus::Registry;
///
/// let registry = Registry::new();
/// let metrics = export_metrics_from_registry(&registry);
/// ```
pub fn export_metrics_from_registry(registry: &Registry) -> String {
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();

    let mut buffer = Vec::new();
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!("Failed to encode metrics: {}", e);
        return String::from("# Error encoding metrics\n");
    }

    String::from_utf8(buffer)
        .unwrap_or_else(|_| String::from("# Error converting metrics to UTF-8\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_registry() {
        let registry = default_registry();
        // Registry may have metrics from other tests running in parallel,
        // so we just verify it's accessible
        let _ = registry.gather();
    }

    #[test]
    fn test_export_metrics() {
        let metrics = export_metrics();
        assert!(metrics.contains("# HELP") || metrics.is_empty());
    }
}
