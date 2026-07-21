//! OpenTelemetry integration for Armature
//!
//! This crate provides observability features for Armature applications including:
//! - Distributed tracing with automatic span creation
//! - Metrics collection (counters, gauges, histograms)
//! - W3C Trace Context propagation for distributed traces
//! - OTLP (default) and Zipkin (`zipkin` feature) exporters
//! - Middleware for automatic instrumentation
//!
//! Note: the legacy Jaeger and Prometheus exporters were discontinued upstream
//! for OpenTelemetry 0.32; export to a Collector via OTLP instead.
//!
//! # Examples
//!
//! ## Telemetry Configuration
//!
//! ```
//! use armature_opentelemetry::TelemetryConfig;
//!
//! // Create configuration with service name
//! let config = TelemetryConfig::new("my-service");
//!
//! assert_eq!(config.service_name, "my-service");
//! assert!(config.enable_tracing);
//! assert!(config.enable_metrics);
//!
//! // Customize configuration with builder pattern
//! let custom = TelemetryConfig::new("api-service")
//!     .with_version("1.0.0")
//!     .with_environment("production")
//!     .with_namespace("backend")
//!     .with_tracing(true)
//!     .with_metrics(false);
//!
//! assert_eq!(custom.service_name, "api-service");
//! assert_eq!(custom.service_version, Some("1.0.0".to_string()));
//! assert_eq!(custom.environment, Some("production".to_string()));
//! assert!(custom.enable_tracing);
//! assert!(!custom.enable_metrics);
//! ```
//!
//! ## Tracing and Metrics Configuration
//!
//! ```
//! use armature_opentelemetry::TelemetryConfig;
//!
//! // Enable only tracing (disable metrics)
//! let tracing_only = TelemetryConfig::new("tracing-service")
//!     .with_tracing(true)
//!     .with_metrics(false);
//!
//! assert!(tracing_only.enable_tracing);
//! assert!(!tracing_only.enable_metrics);
//!
//! // Enable only metrics (disable tracing)
//! let metrics_only = TelemetryConfig::new("metrics-service")
//!     .with_tracing(false)
//!     .with_metrics(true);
//!
//! assert!(!metrics_only.enable_tracing);
//! assert!(metrics_only.enable_metrics);
//! ```
//!
//! ## Service Metadata Configuration
//!
//! ```
//! use armature_opentelemetry::TelemetryConfig;
//!
//! // Configure full service metadata
//! let config = TelemetryConfig::new("user-api")
//!     .with_version("2.1.0")
//!     .with_namespace("microservices")
//!     .with_environment("staging");
//!
//! assert_eq!(config.service_name, "user-api");
//! assert_eq!(config.service_version, Some("2.1.0".to_string()));
//! assert_eq!(config.service_namespace, Some("microservices".to_string()));
//! assert_eq!(config.environment, Some("staging".to_string()));
//! ```
//!
//! ## Creating KeyValue Attributes
//!
//! ```
//! use armature_opentelemetry::{KeyValue, StringValue};
//!
//! // Create key-value pairs for telemetry attributes
//! let service_attr = KeyValue::new("service.name", "my-api");
//! let version_attr = KeyValue::new("service.version", "1.0.0");
//! let env_attr = KeyValue::new("environment", "production");
//!
//! // Key-value pairs are used for span attributes and resource attributes
//! let attributes = vec![
//!     service_attr,
//!     version_attr,
//!     env_attr,
//! ];
//!
//! assert_eq!(attributes.len(), 3);
//! ```
//!
//! ## Complete Example (requires external collector)
//!
//! ```no_run
//! use armature_opentelemetry::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), TelemetryError> {
//!     // Initialize OpenTelemetry with OTLP exporter
//!     let telemetry = TelemetryBuilder::new("my-service")
//!         .with_otlp_endpoint("http://localhost:4317")
//!         .with_tracing()
//!         .with_metrics()
//!         .build()
//!         .await?;
//!
//!     // Use middleware for automatic instrumentation
//!     // let app = Application::new(container, router)
//!     //     .with_middleware(telemetry.middleware());
//!
//!     // Shutdown telemetry on exit
//!     telemetry.shutdown().await?;
//!     Ok(())
//! }
//! ```

pub mod builder;
pub mod config;
pub mod error;
pub mod metrics;
pub mod middleware;
pub mod tracing_setup;

pub use builder::*;
pub use config::*;
pub use error::{TelemetryError, TelemetryResult};
pub use metrics::*;
pub use middleware::*;
pub use tracing_setup::*;

// Re-export commonly used OpenTelemetry types
pub use opentelemetry::{Context as OtelContext, KeyValue, StringValue, Value, global};
pub use opentelemetry_sdk::{
    Resource,
    metrics::{PeriodicReader, SdkMeterProvider},
    runtime,
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider, Tracer},
};
