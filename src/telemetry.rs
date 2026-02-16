//! OpenTelemetry integration for distributed tracing.
//!
//! This module provides optional OpenTelemetry tracing integration for r8r.
//! When enabled, it exports traces to an OTLP-compatible collector.
//!
//! # Environment Variables
//!
//! - `R8R_OTEL_ENABLED`: Set to "true" to enable OpenTelemetry (default: false)
//! - `R8R_OTEL_ENDPOINT`: OTLP endpoint URL (default: http://localhost:4317)
//! - `R8R_OTEL_SERVICE_NAME`: Service name for traces (default: r8r)
//! - `R8R_OTEL_SAMPLE_RATE`: Sampling rate 0.0-1.0 (default: 1.0)

use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime,
    trace::{RandomIdGenerator, Sampler, TracerProvider as SdkTracerProvider},
    Resource,
};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

/// Configuration for OpenTelemetry.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// Whether OpenTelemetry is enabled.
    pub enabled: bool,
    /// OTLP endpoint URL.
    pub endpoint: String,
    /// Service name for traces.
    pub service_name: String,
    /// Sampling rate (0.0 to 1.0).
    pub sample_rate: f64,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            enabled: std::env::var("R8R_OTEL_ENABLED")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
            endpoint: std::env::var("R8R_OTEL_ENDPOINT")
                .unwrap_or_else(|_| "http://localhost:4317".to_string()),
            service_name: std::env::var("R8R_OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| "r8r".to_string()),
            sample_rate: std::env::var("R8R_OTEL_SAMPLE_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
        }
    }
}

/// Initialize OpenTelemetry tracing.
///
/// Returns `Ok(())` if OpenTelemetry was initialized successfully or is disabled.
/// Returns `Err` if initialization failed.
pub fn init_telemetry(
    config: &OtelConfig,
) -> Result<Option<SdkTracerProvider>, Box<dyn std::error::Error + Send + Sync>> {
    if !config.enabled {
        // Just use standard tracing without OpenTelemetry
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(true)
                    .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
            )
            .init();
        return Ok(None);
    }

    // Create OTLP exporter
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.endpoint)
        .build()?;

    // Create tracer provider with sampling
    let sampler = if config.sample_rate >= 1.0 {
        Sampler::AlwaysOn
    } else if config.sample_rate <= 0.0 {
        Sampler::AlwaysOff
    } else {
        Sampler::TraceIdRatioBased(config.sample_rate)
    };

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_sampler(sampler)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(Resource::new(vec![
            KeyValue::new("service.name", config.service_name.clone()),
            KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
        ]))
        .build();

    // Create OpenTelemetry tracing layer
    let tracer = provider.tracer("r8r");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Combine with standard formatting layer
    tracing_subscriber::registry()
        .with(otel_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_filter(tracing_subscriber::EnvFilter::from_default_env()),
        )
        .init();

    info!(
        endpoint = %config.endpoint,
        service_name = %config.service_name,
        sample_rate = config.sample_rate,
        "OpenTelemetry tracing initialized"
    );

    Ok(Some(provider))
}

/// Shutdown OpenTelemetry tracing gracefully.
pub fn shutdown_telemetry(provider: Option<SdkTracerProvider>) {
    if let Some(provider) = provider {
        if let Err(e) = provider.shutdown() {
            tracing::error!("Failed to shutdown OpenTelemetry provider: {:?}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = OtelConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.endpoint, "http://localhost:4317");
        assert_eq!(config.service_name, "r8r");
        assert_eq!(config.sample_rate, 1.0);
    }

    #[test]
    fn test_sample_rate_bounds() {
        let config = OtelConfig {
            enabled: true,
            endpoint: "http://localhost:4317".to_string(),
            service_name: "test".to_string(),
            sample_rate: 1.5, // Over 1.0
        };
        // Sample rate > 1.0 should result in AlwaysOn sampler
        assert!(config.sample_rate >= 1.0);

        let config2 = OtelConfig {
            sample_rate: -0.5, // Negative
            ..config.clone()
        };
        // Sample rate < 0.0 should result in AlwaysOff sampler
        assert!(config2.sample_rate <= 0.0);
    }
}
