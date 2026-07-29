//! Armature Microservice Template
//!
//! A queue-connected microservice for background job processing.
//!
//! Run with: cargo run

mod config;
mod handlers;
mod jobs;

use armature::armature_queue::{Job, Queue, QueueConfig, QueueError, QueueResult, Worker, WorkerConfig};
use armature::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::signal;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::ServiceConfig;
use crate::handlers::JobHandlers;

// =============================================================================
// Service State
// =============================================================================

#[derive(Default)]
pub struct ServiceState {
    pub jobs_processed: AtomicU64,
    pub jobs_failed: AtomicU64,
    pub is_healthy: AtomicBool,
    pub start_time: Option<Instant>,
}

impl ServiceState {
    pub fn new() -> Self {
        Self {
            jobs_processed: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            is_healthy: AtomicBool::new(true),
            start_time: Some(Instant::now()),
        }
    }

    pub fn record_success(&self) {
        self.jobs_processed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        self.jobs_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_unhealthy(&self) {
        self.is_healthy.store(false, Ordering::Relaxed);
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    pub fn stats(&self) -> ServiceStats {
        ServiceStats {
            jobs_processed: self.jobs_processed.load(Ordering::Relaxed),
            jobs_failed: self.jobs_failed.load(Ordering::Relaxed),
            is_healthy: self.is_healthy.load(Ordering::Relaxed),
            uptime_seconds: self.uptime_seconds(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceStats {
    pub jobs_processed: u64,
    pub jobs_failed: u64,
    pub is_healthy: bool,
    pub uptime_seconds: u64,
}

/// Shared service state, published once at startup so both the HTTP health
/// controllers and the background job worker observe the same counters.
///
/// Note: this `OnceLock<Arc<T>>` + clone-returning accessor is one of three
/// slightly different reinventions of the same singleton bootstrap pattern
/// across the `api-full`/`api-minimal`/`microservice` templates; consider
/// unifying them.
static SERVICE_STATE: OnceLock<Arc<ServiceState>> = OnceLock::new();

fn service_state() -> Arc<ServiceState> {
    SERVICE_STATE
        .get()
        .expect("ServiceState must be initialized before routes are served")
        .clone()
}

// =============================================================================
// Health Controller
// =============================================================================

#[controller("/health")]
#[derive(Default)]
struct HealthController;

#[routes]
impl HealthController {
    #[get("")]
    async fn health() -> Result<HttpResponse, Error> {
        let stats = service_state().stats();
        HttpResponse::json(&serde_json::json!({
            "status": if stats.is_healthy { "healthy" } else { "unhealthy" },
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_seconds": stats.uptime_seconds,
            "jobs_processed": stats.jobs_processed,
            "jobs_failed": stats.jobs_failed,
        }))
    }

    #[get("/live")]
    async fn liveness() -> Result<HttpResponse, Error> {
        HttpResponse::json(&serde_json::json!({ "status": "alive" }))
    }

    #[get("/ready")]
    async fn readiness() -> Result<HttpResponse, Error> {
        let stats = service_state().stats();
        if stats.is_healthy {
            HttpResponse::json(&serde_json::json!({ "status": "ready" }))
        } else {
            HttpResponse::service_unavailable().with_json(&serde_json::json!({ "status": "not_ready" }))
        }
    }
}

// =============================================================================
// Metrics Controller
// =============================================================================

#[controller("/metrics")]
#[derive(Default)]
struct MetricsController;

#[routes]
impl MetricsController {
    #[get("")]
    async fn metrics() -> Result<HttpResponse, Error> {
        let stats = service_state().stats();
        let body = format!(
            "# HELP jobs_processed_total Total number of jobs processed\n\
             # TYPE jobs_processed_total counter\n\
             jobs_processed_total {}\n\
             # HELP jobs_failed_total Total number of jobs that failed\n\
             # TYPE jobs_failed_total counter\n\
             jobs_failed_total {}\n\
             # HELP uptime_seconds Service uptime in seconds\n\
             # TYPE uptime_seconds gauge\n\
             uptime_seconds {}\n",
            stats.jobs_processed, stats.jobs_failed, stats.uptime_seconds
        );
        Ok(HttpResponse::text(body))
    }
}

// =============================================================================
// Service Module
// =============================================================================

#[module(controllers: [HealthController, MetricsController])]
#[derive(Default)]
struct ServiceModule;

// =============================================================================
// Job Worker
// =============================================================================

/// Job types this service knows how to process, delegating to
/// [`JobHandlers`] (see `src/handlers.rs`).
const JOB_TYPES: [&str; 3] = ["send_email", "send_notification", "process_data"];

/// Run a single dequeued job through the real handler registry, recording
/// the outcome in [`ServiceState`] so `/health` and `/metrics` reflect real
/// processing rather than a simulation.
async fn process_job(
    job_type: &str,
    payload: serde_json::Value,
    state: Arc<ServiceState>,
) -> QueueResult<()> {
    match JobHandlers::handle(job_type, payload).await {
        Ok(()) => {
            state.record_success();
            Ok(())
        }
        Err(e) => {
            state.record_failure();
            warn!(job_type, error = %e, "Job handler failed");
            Err(QueueError::ExecutionFailed(e))
        }
    }
}

/// Connect to Redis, start a real `armature-queue` worker wired to the job
/// handlers in `src/handlers.rs`, and kick off a demo producer so the
/// service has real work to do out of the box.
///
/// Runs as its own background task (see [`main`]) rather than being awaited
/// inline: connecting to Redis must never block the HTTP listener from
/// coming up, since the whole point of `/health` is to stay reachable (and
/// honestly report "unhealthy") even when the queue backend is down or slow.
/// Exits gracefully once `shutdown` fires, stopping the worker and demo
/// producer first.
async fn run_queue_worker(
    config: ServiceConfig,
    state: Arc<ServiceState>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let queue_config = QueueConfig::new(config.redis_url.clone(), config.queue_name.clone());

    // Bound the connection attempt so an unreachable Redis marks the service
    // unhealthy instead of hanging startup indefinitely.
    let queue = match tokio::time::timeout(
        Duration::from_secs(10),
        Queue::with_config(queue_config),
    )
    .await
    {
        Ok(Ok(queue)) => queue,
        Ok(Err(e)) => {
            error!(error = %e, "Failed to connect to job queue; continuing with health endpoints only");
            state.set_unhealthy();
            return;
        }
        Err(_) => {
            error!("Timed out connecting to job queue; continuing with health endpoints only");
            state.set_unhealthy();
            return;
        }
    };
    info!(redis_url = %config.redis_url, queue = %config.queue_name, "Connected to job queue");

    let worker_config = WorkerConfig {
        concurrency: config.concurrency,
        // armature-queue's retry backoff is a fixed exponential curve based on
        // attempt count with no independent delay knob, so RETRY_DELAY_MS is
        // reused here as the poll interval: how eagerly each worker task
        // rechecks the queue for new or now-due retried jobs.
        poll_interval: Duration::from_millis(config.retry_delay_ms.max(50)),
        job_timeout: Duration::from_secs(30),
        log_execution: true,
    };
    let mut worker = Worker::with_config(queue.clone(), worker_config);

    for job_type in JOB_TYPES {
        let state = state.clone();
        worker
            .register_handler(job_type, move |job| {
                let state = state.clone();
                async move { process_job(&job.job_type, job.data.clone(), state).await }
            })
            .await;
    }

    if let Err(e) = worker.start().await {
        error!(error = %e, "Failed to start job queue worker");
        state.set_unhealthy();
        return;
    }
    info!(concurrency = config.concurrency, "Job queue worker started");

    let seeder = tokio::spawn(seed_demo_jobs(
        queue,
        config.queue_name.clone(),
        config.retry_attempts,
    ));

    // Keep the worker running until the main task asks us to shut down.
    let _ = shutdown.await;

    seeder.abort();
    if let Err(e) = worker.stop().await {
        warn!(error = %e, "Error stopping worker");
    }
}

/// Build a sample payload for one of the demo job types.
fn sample_payload(job_type: &str) -> serde_json::Value {
    match job_type {
        "send_email" => serde_json::json!({
            "to": "demo@example.com",
            "subject": "Welcome to Armature",
            "body": "This message was generated by the microservice template's demo job producer.",
            "html": false
        }),
        "send_notification" => serde_json::json!({
            "user_id": "demo-user",
            "message": "A background job just completed.",
            "channel": "push"
        }),
        "process_data" => serde_json::json!({
            "data_id": Uuid::new_v4().to_string(),
            "operation": "normalize"
        }),
        other => serde_json::json!({ "job_type": other }),
    }
}

/// Periodically enqueues one sample job of each registered type so the
/// service has real work to process the moment it starts.
///
/// Real deployments enqueue jobs from their own producers (HTTP handlers,
/// other services, scheduled tasks) rather than from a loop like this one --
/// it exists purely so `cargo run` demonstrates a working end-to-end queue
/// pipeline out of the box.
async fn seed_demo_jobs(queue: Queue, queue_name: String, max_attempts: u32) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut next = 0usize;

    loop {
        interval.tick().await;

        let job_type = JOB_TYPES[next % JOB_TYPES.len()];
        next += 1;

        let job =
            Job::new(&queue_name, job_type, sample_payload(job_type)).with_max_attempts(max_attempts);

        match queue.enqueue_job(job).await {
            Ok(job_id) => info!(job_id = %job_id, job_type, "Enqueued demo job"),
            Err(e) => error!(error = %e, job_type, "Failed to enqueue demo job"),
        }
    }
}

// =============================================================================
// Graceful Shutdown
// =============================================================================

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, starting graceful shutdown...");
}

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_target(true)
        .init();

    info!("Starting Armature Microservice");

    // Load configuration
    let config = ServiceConfig::from_env();
    info!(service_name = %config.service_name, "Configuration loaded");

    // Publish shared service state for the health/metrics controllers and
    // the job worker to share.
    let state = Arc::new(ServiceState::new());
    let _ = SERVICE_STATE.set(state.clone());

    // Create application for health/metrics endpoints
    let app = Application::create::<ServiceModule>().await;

    // Connect to Redis and run the real job queue worker in the background.
    // This must not block the HTTP listener below: if the queue is
    // unreachable or slow, health endpoints still need to come up (and
    // honestly report unhealthy) rather than the whole process hanging.
    let (worker_shutdown_tx, worker_shutdown_rx) = tokio::sync::oneshot::channel();
    let worker_task = tokio::spawn(run_queue_worker(
        config.clone(),
        state.clone(),
        worker_shutdown_rx,
    ));

    info!(host = %config.host, port = config.port, "Health endpoint starting");

    println!();
    println!("Available endpoints:");
    println!("  GET /health       - Service health");
    println!("  GET /health/live  - Liveness probe");
    println!("  GET /health/ready - Readiness probe");
    println!("  GET /metrics      - Prometheus metrics");
    println!();

    // Run server with graceful shutdown.
    // `Application::listen` always binds all interfaces on the given port;
    // HOST is logged above for operator visibility but isn't a bind address
    // in this version of armature-core.
    tokio::select! {
        result = app.listen(config.port) => {
            if let Err(e) = result {
                error!(error = %e, "Server error");
            }
        }
        _ = shutdown_signal() => {
            info!("Shutting down...");
        }
    }

    // Ask the queue worker task to stop (it stops the worker and demo
    // producer internally) and wait for it to finish.
    let _ = worker_shutdown_tx.send(());
    let _ = worker_task.await;

    // Final stats
    let final_stats = state.stats();
    info!(
        processed = final_stats.jobs_processed,
        failed = final_stats.jobs_failed,
        uptime = final_stats.uptime_seconds,
        "Final stats"
    );

    info!("Shutdown complete");
    Ok(())
}
