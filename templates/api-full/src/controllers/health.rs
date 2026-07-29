//! Health check controller

use crate::models::HealthResponse;
use crate::services::{get_auth_service, get_user_service};
use armature::prelude::*;
use chrono::Utc;
use std::sync::OnceLock;
use std::time::Instant;

static START_TIME: OnceLock<Instant> = OnceLock::new();

#[controller("/health")]
#[derive(Default, Clone)]
pub struct HealthController;

#[routes]
impl HealthController {
    /// GET /health - full health report
    #[get("")]
    async fn health() -> Result<HttpResponse, Error> {
        let uptime = START_TIME.get_or_init(Instant::now).elapsed().as_secs();

        let response = HealthResponse {
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Utc::now(),
            uptime_seconds: uptime,
        };

        HttpResponse::ok().with_json(&response)
    }

    /// GET /health/live - liveness probe
    #[get("/live")]
    async fn liveness() -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({ "status": "alive" }))
    }

    /// GET /health/ready - readiness probe
    ///
    /// This is wired directly into the k8s readiness probe
    /// (`templates/k8s/deployment.yaml`), so it must reflect real health
    /// rather than an unconditional "ready". This template has no direct
    /// database/cache client in scope here, so the check verifies that the
    /// process-wide services the API depends on (`AuthService`,
    /// `UserService`) actually finished initializing. `get_auth_service()`
    /// / `get_user_service()` panic if called before `init_*_service()` has
    /// run, so the check is done via `catch_unwind` rather than calling
    /// them unconditionally — a non-panicking equivalent of checking
    /// `OnceLock::get().is_some()` from outside the `services` module.
    #[get("/ready")]
    async fn readiness() -> Result<HttpResponse, Error> {
        let auth_ready = std::panic::catch_unwind(get_auth_service).is_ok();
        let user_ready = std::panic::catch_unwind(get_user_service).is_ok();

        let body = serde_json::json!({
            "status": if auth_ready && user_ready { "ready" } else { "not_ready" },
            "checks": {
                "auth_service": if auth_ready { "ok" } else { "unavailable" },
                "user_service": if user_ready { "ok" } else { "unavailable" }
            }
        });

        if auth_ready && user_ready {
            HttpResponse::ok().with_json(&body)
        } else {
            HttpResponse::service_unavailable().with_json(&body)
        }
    }
}
