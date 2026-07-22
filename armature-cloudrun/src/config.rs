//! Cloud Run configuration.

use std::net::SocketAddr;

/// Cloud Run configuration.
///
/// Reads configuration from the process environment. Cloud Run itself only
/// sets a handful of these variables (`PORT`, `K_SERVICE`, `K_REVISION`,
/// `K_CONFIGURATION`); the remaining fields are **user-supplied overrides**
/// that you must set yourself (they are not provided by the Cloud Run
/// runtime) — see the individual field docs.
#[derive(Debug, Clone)]
pub struct CloudRunConfig {
    /// Port to listen on (from the `PORT` env var set by Cloud Run).
    pub port: u16,
    /// Host to bind to.
    pub host: String,
    /// Service name (from the `K_SERVICE` env var set by Cloud Run).
    pub service: Option<String>,
    /// Revision name (from the `K_REVISION` env var set by Cloud Run).
    pub revision: Option<String>,
    /// Configuration name (from the `K_CONFIGURATION` env var set by Cloud Run).
    pub configuration: Option<String>,
    /// Project ID. User-supplied via `GOOGLE_CLOUD_PROJECT` — Cloud Run does
    /// not set this automatically; fetch it from the metadata server if needed
    /// (see [`crate::InstanceMetadata`]).
    pub project_id: Option<String>,
    /// Region. User-supplied via `GOOGLE_CLOUD_REGION` — Cloud Run does not set
    /// this automatically.
    pub region: Option<String>,
    /// Service URL. User-supplied via `CLOUD_RUN_URL` — Cloud Run does not set
    /// this automatically. See [`CloudRunConfig::service_url`].
    pub url: Option<String>,
    /// Memory limit in MB. **User-supplied override** via `MEMORY_LIMIT_MB` —
    /// Cloud Run does not expose the container memory limit as an env var, so
    /// this is `None` unless you set it yourself.
    pub memory_limit_mb: Option<u32>,
    /// CPU limit. **User-supplied override** via `CPU_LIMIT` — Cloud Run does
    /// not expose the CPU allocation as an env var, so this is `None` unless
    /// you set it yourself.
    pub cpu_limit: Option<f32>,
    /// Request timeout in seconds. **User-supplied override** via
    /// `CLOUD_RUN_TIMEOUT_SECONDS` — Cloud Run does not expose the request
    /// timeout as an env var, so this falls back to the default (300) unless
    /// you set it yourself.
    pub timeout_seconds: u32,
    /// Maximum concurrent requests per instance. **User-supplied override** via
    /// `CLOUD_RUN_CONCURRENCY` — Cloud Run does not expose the concurrency
    /// setting as an env var, so this falls back to the default (80) unless you
    /// set it yourself.
    pub max_concurrent_requests: u32,
}

impl Default for CloudRunConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            host: "0.0.0.0".to_string(),
            service: None,
            revision: None,
            configuration: None,
            project_id: None,
            region: None,
            url: None,
            memory_limit_mb: None,
            cpu_limit: None,
            timeout_seconds: 300,
            max_concurrent_requests: 80,
        }
    }
}

impl CloudRunConfig {
    /// Create configuration from environment variables.
    ///
    /// `PORT`, `K_SERVICE`, `K_REVISION` and `K_CONFIGURATION` are populated by
    /// the Cloud Run runtime. The remaining fields (`project_id`, `region`,
    /// `url`, `memory_limit_mb`, `cpu_limit`, `timeout_seconds`,
    /// `max_concurrent_requests`) are read from env vars that Cloud Run does
    /// **not** set — they only take effect if you supply them yourself, and
    /// otherwise stay `None`/at their defaults.
    pub fn from_env() -> Self {
        let port = std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);

        Self {
            port,
            host: "0.0.0.0".to_string(),
            service: std::env::var("K_SERVICE").ok(),
            revision: std::env::var("K_REVISION").ok(),
            configuration: std::env::var("K_CONFIGURATION").ok(),
            project_id: std::env::var("GOOGLE_CLOUD_PROJECT").ok(),
            region: std::env::var("GOOGLE_CLOUD_REGION").ok(),
            url: std::env::var("CLOUD_RUN_URL").ok(),
            memory_limit_mb: std::env::var("MEMORY_LIMIT_MB")
                .ok()
                .and_then(|m| m.parse().ok()),
            cpu_limit: std::env::var("CPU_LIMIT").ok().and_then(|c| c.parse().ok()),
            timeout_seconds: std::env::var("CLOUD_RUN_TIMEOUT_SECONDS")
                .ok()
                .and_then(|t| t.parse().ok())
                .unwrap_or(300),
            max_concurrent_requests: std::env::var("CLOUD_RUN_CONCURRENCY")
                .ok()
                .and_then(|c| c.parse().ok())
                .unwrap_or(80),
        }
    }

    /// Set the port.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the host.
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Get the bind address.
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get the socket address.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(
            self.host
                .parse()
                .unwrap_or_else(|_| "0.0.0.0".parse().unwrap()),
            self.port,
        )
    }

    /// Check if running on Cloud Run.
    pub fn is_cloud_run(&self) -> bool {
        self.service.is_some()
    }

    /// Get the service URL, if known.
    ///
    /// Cloud Run's public URL is derived from an opaque project hash and cannot
    /// be reconstructed from the service name, project ID and region alone.
    /// This returns the URL only when it was provided out-of-band via the
    /// `CLOUD_RUN_URL` env var (read into [`CloudRunConfig::url`] by
    /// [`CloudRunConfig::from_env`]); otherwise it returns `None` rather than a
    /// fabricated, non-resolving URL.
    pub fn service_url(&self) -> Option<String> {
        self.url.clone()
    }
}
