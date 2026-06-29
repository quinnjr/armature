//! Tracing setup and management

use crate::{
    config::{TelemetryConfig, TracingExporter},
    error::{TelemetryError, TelemetryResult},
};
use opentelemetry::global;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};

/// Initialize tracing based on configuration
pub async fn init_tracing(config: &TelemetryConfig) -> TelemetryResult<SdkTracerProvider> {
    if !config.enable_tracing {
        return Err(TelemetryError::Config("Tracing is not enabled".to_string()));
    }

    let resource = config.create_resource()?;

    let sampler = if config.tracing.sampling_ratio >= 1.0 {
        Sampler::AlwaysOn
    } else if config.tracing.sampling_ratio <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.tracing.sampling_ratio)
    };

    let provider = match config.tracing.exporter {
        #[cfg(feature = "otlp")]
        TracingExporter::Otlp => {
            use opentelemetry_otlp::{SpanExporter, WithExportConfig};

            let endpoint = config.tracing.otlp_endpoint.as_ref().ok_or_else(|| {
                TelemetryError::Config("OTLP endpoint not configured".to_string())
            })?;

            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.clone())
                .build()
                .map_err(|e| TelemetryError::Exporter(e.to_string()))?;

            SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource)
                .with_sampler(sampler)
                .with_id_generator(RandomIdGenerator::default())
                .build()
        }

        // Note: opentelemetry-jaeger is discontinued and not compatible with opentelemetry 0.31
        // Use OTLP with a Jaeger collector backend instead
        TracingExporter::Jaeger => {
            return Err(TelemetryError::Config(
                "Jaeger exporter is discontinued. Use OTLP with a Jaeger collector instead. \
                See: https://www.jaegertracing.io/docs/1.35/apis/#opentelemetry-protocol-stable"
                    .to_string(),
            ));
        }

        #[cfg(feature = "zipkin")]
        #[allow(deprecated)]
        // upstream deprecated the Zipkin exporter; kept while the feature exists
        TracingExporter::Zipkin => {
            use opentelemetry_zipkin::ZipkinExporter;

            let endpoint = config.tracing.zipkin_endpoint.as_ref().ok_or_else(|| {
                TelemetryError::Config("Zipkin endpoint not configured".to_string())
            })?;

            let exporter = ZipkinExporter::builder()
                .with_collector_endpoint(endpoint)
                .build()
                .map_err(|e| TelemetryError::Exporter(format!("{:?}", e)))?;

            SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource)
                .with_sampler(sampler)
                .with_id_generator(RandomIdGenerator::default())
                .build()
        }

        TracingExporter::None => SdkTracerProvider::builder()
            .with_resource(resource)
            .with_sampler(sampler)
            .with_id_generator(RandomIdGenerator::default())
            .build(),

        #[allow(unreachable_patterns)]
        _ => {
            return Err(TelemetryError::Config(format!(
                "Tracing exporter {:?} not available (feature not enabled)",
                config.tracing.exporter
            )));
        }
    };

    // Set as global provider
    global::set_tracer_provider(provider.clone());

    Ok(provider)
}

/// Shutdown tracing gracefully
pub async fn shutdown_tracing(provider: SdkTracerProvider) -> TelemetryResult<()> {
    provider
        .shutdown()
        .map_err(|e| TelemetryError::Shutdown(e.to_string()))?;
    Ok(())
}

/// Get a tracer for the current service
pub fn get_tracer(name: &'static str) -> impl opentelemetry::trace::Tracer {
    global::tracer(name)
}
