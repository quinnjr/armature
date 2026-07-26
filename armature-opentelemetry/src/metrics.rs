//! Metrics collection and export

use crate::{
    config::{MetricsExporter, TelemetryConfig},
    error::{TelemetryError, TelemetryResult},
};
use opentelemetry::{KeyValue, global};
use opentelemetry_sdk::metrics::SdkMeterProvider;

/// Initialize metrics based on configuration
pub async fn init_metrics(config: &TelemetryConfig) -> TelemetryResult<SdkMeterProvider> {
    if !config.enable_metrics {
        return Err(TelemetryError::Config(
            "Metrics are not enabled".to_string(),
        ));
    }

    let resource = config.create_resource()?;

    let provider = match config.metrics.exporter {
        #[cfg(feature = "otlp")]
        MetricsExporter::Otlp => {
            use opentelemetry_otlp::{MetricExporter, WithExportConfig};

            let endpoint = config.metrics.otlp_endpoint.as_ref().ok_or_else(|| {
                TelemetryError::Config("OTLP endpoint not configured for metrics".to_string())
            })?;

            let exporter = MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.clone())
                .build()
                .map_err(|e| TelemetryError::Exporter(e.to_string()))?;

            let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exporter)
                .with_interval(std::time::Duration::from_secs(
                    config.metrics.collection_interval_secs,
                ))
                .build();

            SdkMeterProvider::builder()
                .with_resource(resource)
                .with_reader(reader)
                .build()
        }

        // Note: opentelemetry-prometheus is discontinued and not compatible with opentelemetry 0.31
        // Use OTLP with a Prometheus collector/remote-write endpoint instead
        MetricsExporter::Prometheus => {
            return Err(TelemetryError::Config(
                "Prometheus exporter is discontinued. Use OTLP with a Prometheus remote-write \
                endpoint or an OpenTelemetry Collector with Prometheus exporter instead."
                    .to_string(),
            ));
        }

        MetricsExporter::None => SdkMeterProvider::builder().with_resource(resource).build(),

        #[allow(unreachable_patterns)]
        _ => {
            return Err(TelemetryError::Config(format!(
                "Metrics exporter {:?} not available (feature not enabled)",
                config.metrics.exporter
            )));
        }
    };

    // Set as global provider
    global::set_meter_provider(provider.clone());

    Ok(provider)
}

/// Shutdown metrics gracefully
pub async fn shutdown_metrics(provider: SdkMeterProvider) -> TelemetryResult<()> {
    provider
        .shutdown()
        .map_err(|e| TelemetryError::Shutdown(e.to_string()))?;
    Ok(())
}

/// Get a meter for the current service
pub fn get_meter(name: &'static str) -> opentelemetry::metrics::Meter {
    global::meter(name)
}

/// Common HTTP metrics
pub struct HttpMetrics {
    pub request_count: opentelemetry::metrics::Counter<u64>,
    pub request_duration: opentelemetry::metrics::Histogram<f64>,
    pub active_requests: opentelemetry::metrics::UpDownCounter<i64>,
}

impl HttpMetrics {
    /// Create HTTP metrics
    pub fn new(meter: &opentelemetry::metrics::Meter) -> TelemetryResult<Self> {
        let request_count = meter
            .u64_counter("http.server.request.count")
            .with_description("Total number of HTTP requests")
            .build();

        let request_duration = meter
            .f64_histogram("http.server.request.duration")
            .with_description("HTTP request duration in seconds")
            .with_unit("s")
            .build();

        let active_requests = meter
            .i64_up_down_counter("http.server.active_requests")
            .with_description("Number of active HTTP requests")
            .build();

        Ok(Self {
            request_count,
            request_duration,
            active_requests,
        })
    }

    /// Record a request
    pub fn record_request(&self, method: &str, path: &str, status: u16, duration: f64) {
        let attributes = vec![
            KeyValue::new("http.method", method.to_string()),
            KeyValue::new("http.route", path.to_string()),
            KeyValue::new("http.status_code", status.to_string()),
        ];

        self.request_count.add(1, &attributes);
        self.request_duration.record(duration, &attributes);
    }

    /// Increment active requests
    pub fn increment_active(&self) {
        self.active_requests.add(1, &[]);
    }

    /// Decrement active requests
    pub fn decrement_active(&self) {
        self.active_requests.add(-1, &[]);
    }
}
