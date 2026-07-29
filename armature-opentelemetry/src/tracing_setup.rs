//! Tracing setup and management

use crate::{
    config::{TelemetryConfig, TracingExporter},
    error::{TelemetryError, TelemetryResult},
};
use opentelemetry::global;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};

/// Install the global W3C Trace Context text-map propagator.
///
/// Without this, the global propagator defaults to a no-op and inbound
/// `traceparent`/`tracestate` headers are never parsed, so cross-service
/// context propagation silently does nothing. This is idempotent.
pub fn install_propagator() {
    global::set_text_map_propagator(TraceContextPropagator::new());
}

/// Initialize tracing based on configuration
pub async fn init_tracing(config: &TelemetryConfig) -> TelemetryResult<SdkTracerProvider> {
    if !config.enable_tracing {
        return Err(TelemetryError::Config("Tracing is not enabled".to_string()));
    }

    // Ensure inbound distributed-trace headers are actually parsed by the
    // telemetry middleware (the default global propagator is a no-op).
    install_propagator();

    let resource = config.create_resource()?;

    let max_attributes_per_span = config.tracing.max_attributes_per_span;
    let max_events_per_span = config.tracing.max_events_per_span;

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
                .with_max_attributes_per_span(max_attributes_per_span)
                .with_max_events_per_span(max_events_per_span)
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
                .with_max_attributes_per_span(max_attributes_per_span)
                .with_max_events_per_span(max_events_per_span)
                .build()
        }

        TracingExporter::None => SdkTracerProvider::builder()
            .with_resource(resource)
            .with_sampler(sampler)
            .with_id_generator(RandomIdGenerator::default())
            .with_max_attributes_per_span(max_attributes_per_span)
            .with_max_events_per_span(max_events_per_span)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TelemetryConfig, TracingExporter};
    use opentelemetry::propagation::Extractor;
    use opentelemetry::trace::TraceContextExt;
    use std::collections::HashMap;

    struct MapExtractor(HashMap<String, String>);

    impl Extractor for MapExtractor {
        fn get(&self, key: &str) -> Option<&str> {
            self.0.get(key).map(|s| s.as_str())
        }
        fn keys(&self) -> Vec<&str> {
            self.0.keys().map(|k| k.as_str()).collect()
        }
    }

    /// Regression: before this fix the global propagator was left at the SDK
    /// default no-op, so an inbound W3C `traceparent` was never parsed into a
    /// remote parent context. `init_tracing` must install a real propagator.
    ///
    /// `#[serial]`: this test calls `global::set_tracer_provider`, which is
    /// process-global state also mutated by
    /// `middleware::tests::handle_attaches_context_so_application_spans_nest_under_request_span`;
    /// run those tests one at a time so neither observes the other's provider.
    #[tokio::test]
    #[serial_test::serial]
    async fn init_tracing_installs_w3c_propagator_that_parses_traceparent() {
        let mut config = TelemetryConfig::new("propagation-test");
        config.tracing.exporter = TracingExporter::None;
        config.enable_metrics = false;
        let _provider = init_tracing(&config)
            .await
            .expect("init_tracing should succeed");

        let mut headers = HashMap::new();
        headers.insert(
            "traceparent".to_string(),
            "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_string(),
        );

        let extractor = MapExtractor(headers);
        let cx = global::get_text_map_propagator(|p| p.extract(&extractor));
        let span_ctx = cx.span().span_context().clone();

        assert!(
            span_ctx.is_valid(),
            "propagator did not parse the traceparent header into a valid parent context"
        );
        assert_eq!(
            span_ctx.trace_id().to_string(),
            "0af7651916cd43dd8448eb211c80319c",
            "parsed trace id did not match the inbound traceparent"
        );
    }
}
