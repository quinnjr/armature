//! Health check utilities for Cloud Run.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};

/// Health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Service is healthy.
    Healthy,
    /// Service is degraded but operational.
    Degraded,
    /// Service is unhealthy.
    Unhealthy,
}

impl HealthStatus {
    /// Check if the service is healthy or degraded (can accept traffic).
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }

    /// Check if the service is fully healthy.
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }
}

/// Health check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    /// Overall status.
    pub status: HealthStatus,
    /// Service name.
    pub service: Option<String>,
    /// Revision name.
    pub revision: Option<String>,
    /// Uptime in seconds.
    pub uptime_seconds: u64,
    /// Individual check results.
    #[serde(default)]
    pub checks: Vec<CheckResult>,
}

/// Individual check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Check name.
    pub name: String,
    /// Check status.
    pub status: HealthStatus,
    /// Optional message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Check duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Health check manager.
#[derive(Clone)]
pub struct HealthCheck {
    start_time: std::time::Instant,
    checks: Arc<RwLock<Vec<Box<dyn HealthChecker>>>>,
    status_override: Arc<RwLock<Option<HealthStatus>>>,
}

impl Default for HealthCheck {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthCheck {
    /// Create a new health check manager.
    pub fn new() -> Self {
        Self {
            start_time: std::time::Instant::now(),
            checks: Arc::new(RwLock::new(Vec::new())),
            status_override: Arc::new(RwLock::new(None)),
        }
    }

    /// Register a health checker.
    pub async fn register(&self, checker: impl HealthChecker + 'static) {
        let mut checks = self.checks.write().await;
        checks.push(Box::new(checker));
    }

    /// Override the health status (useful during shutdown).
    pub async fn set_status(&self, status: HealthStatus) {
        let mut override_status = self.status_override.write().await;
        *override_status = Some(status);
    }

    /// Clear the status override.
    pub async fn clear_status(&self) {
        let mut override_status = self.status_override.write().await;
        *override_status = None;
    }

    /// Mark as unhealthy (for graceful shutdown).
    pub async fn mark_unhealthy(&self) {
        self.set_status(HealthStatus::Unhealthy).await;
    }

    /// Run all health checks.
    pub async fn check(&self) -> HealthCheckResult {
        // Check for status override
        if let Some(status) = *self.status_override.read().await {
            return HealthCheckResult {
                status,
                service: std::env::var("K_SERVICE").ok(),
                revision: std::env::var("K_REVISION").ok(),
                uptime_seconds: self.start_time.elapsed().as_secs(),
                checks: vec![],
            };
        }

        let checks = self.checks.read().await;
        let mut results = Vec::with_capacity(checks.len());
        let mut overall_status = HealthStatus::Healthy;

        for checker in checks.iter() {
            let start = std::time::Instant::now();
            let result = checker.check().await;
            let duration_ms = start.elapsed().as_millis() as u64;

            // Update overall status
            match result.status {
                HealthStatus::Unhealthy => overall_status = HealthStatus::Unhealthy,
                HealthStatus::Degraded if overall_status == HealthStatus::Healthy => {
                    overall_status = HealthStatus::Degraded;
                }
                _ => {}
            }

            results.push(CheckResult {
                name: result.name,
                status: result.status,
                message: result.message,
                duration_ms: Some(duration_ms),
            });
        }

        HealthCheckResult {
            status: overall_status,
            service: std::env::var("K_SERVICE").ok(),
            revision: std::env::var("K_REVISION").ok(),
            uptime_seconds: self.start_time.elapsed().as_secs(),
            checks: results,
        }
    }

    /// Simple liveness check (always healthy unless overridden).
    pub async fn liveness(&self) -> bool {
        if let Some(status) = *self.status_override.read().await {
            return status.is_ready();
        }
        true
    }

    /// Readiness check (runs all checks).
    pub async fn readiness(&self) -> bool {
        self.check().await.status.is_ready()
    }

    /// Handle a single health-check HTTP request.
    ///
    /// This is the routing core behind [`HealthCheck::serve`]. It is generic
    /// over the request body (which it never reads) so it can be driven
    /// directly in tests or wired into an existing hyper server:
    ///
    /// - `GET /health`, `/healthz`, `/readyz` — full readiness: runs every
    ///   registered checker, returns `200` when healthy/degraded and `503`
    ///   when unhealthy, with the [`HealthCheckResult`] as a JSON body.
    /// - `GET /livez` — liveness: `200` unless the status has been overridden
    ///   to unhealthy (e.g. during graceful shutdown).
    /// - anything else — `404`.
    pub async fn handle_request<B>(&self, req: &Request<B>) -> Response<Full<Bytes>> {
        match req.uri().path() {
            "/livez" => {
                let alive = self.liveness().await;
                let status = if alive {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                json_response(
                    status,
                    &serde_json::json!({
                        "status": if alive { "healthy" } else { "unhealthy" },
                    }),
                )
            }
            "/health" | "/healthz" | "/readyz" => {
                let result = self.check().await;
                let status = if result.status.is_ready() {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                json_response(status, &result)
            }
            _ => json_response(
                StatusCode::NOT_FOUND,
                &serde_json::json!({ "error": "not found" }),
            ),
        }
    }

    /// Serve the health-check endpoints over HTTP on `addr`.
    ///
    /// Runs until the process exits; typically spawned alongside the main
    /// application (e.g. on a sidecar port) so Cloud Run / load-balancer
    /// probes hit [`HealthCheck::handle_request`]. Returns an error only if
    /// binding or accepting a connection fails.
    pub async fn serve(self, addr: SocketAddr) -> crate::Result<()> {
        use hyper::service::service_fn;
        use hyper_util::rt::{TokioExecutor, TokioIo};
        use hyper_util::server::conn::auto::Builder as AutoBuilder;
        use std::convert::Infallible;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| crate::CloudRunError::Server(format!("failed to bind {addr}: {e}")))?;

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|e| crate::CloudRunError::Server(format!("accept failed: {e}")))?;
            let io = TokioIo::new(stream);
            let health = self.clone();
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                    let health = health.clone();
                    async move { Ok::<_, Infallible>(health.handle_request(&req).await) }
                });
                let _ = AutoBuilder::new(TokioExecutor::new())
                    .serve_connection(io, service)
                    .await;
            });
        }
    }
}

/// Build a JSON HTTP response with the given status code.
fn json_response<T: Serialize>(status: StatusCode, body: &T) -> Response<Full<Bytes>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}

/// Trait for health checkers.
#[async_trait::async_trait]
pub trait HealthChecker: Send + Sync {
    /// Run the health check.
    async fn check(&self) -> CheckResult;
}

/// Simple function-based health checker.
pub struct FnHealthChecker<F> {
    name: String,
    check_fn: F,
}

impl<F, Fut> FnHealthChecker<F>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<(), String>> + Send,
{
    /// Create a new function-based health checker.
    pub fn new(name: impl Into<String>, check_fn: F) -> Self {
        Self {
            name: name.into(),
            check_fn,
        }
    }
}

#[async_trait::async_trait]
impl<F, Fut> HealthChecker for FnHealthChecker<F>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<(), String>> + Send,
{
    async fn check(&self) -> CheckResult {
        match (self.check_fn)().await {
            Ok(()) => CheckResult {
                name: self.name.clone(),
                status: HealthStatus::Healthy,
                message: None,
                duration_ms: None,
            },
            Err(msg) => CheckResult {
                name: self.name.clone(),
                status: HealthStatus::Unhealthy,
                message: Some(msg),
                duration_ms: None,
            },
        }
    }
}
