//! Integration tests for the built-in health-check HTTP endpoint.

use armature_cloudrun::{FnHealthChecker, HealthCheck, HealthStatus};
use hyper::{Request, StatusCode};
use std::time::{Duration, Instant};

fn req(path: &str) -> Request<()> {
    Request::builder().uri(path).body(()).unwrap()
}

#[tokio::test]
async fn healthz_reports_ok_with_json() {
    let hc = HealthCheck::new();
    let resp = hc.handle_request(&req("/healthz")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn readyz_reports_200_json_when_healthy() {
    let hc = HealthCheck::new();
    hc.register(FnHealthChecker::new("dep", || async { Ok(()) }))
        .await;

    let resp = hc.handle_request(&req("/readyz")).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn readiness_fails_when_a_checker_is_unhealthy_but_liveness_stays_ok() {
    let hc = HealthCheck::new();
    hc.register(FnHealthChecker::new("dep", || async {
        Err("dependency down".to_string())
    }))
    .await;

    // Readiness runs the checkers -> 503.
    let ready = hc.handle_request(&req("/readyz")).await;
    assert_eq!(ready.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Liveness ignores checkers (no override) -> 200.
    let live = hc.handle_request(&req("/livez")).await;
    assert_eq!(live.status(), StatusCode::OK);
}

#[tokio::test]
async fn livez_reflects_shutdown_override() {
    let hc = HealthCheck::new();
    hc.mark_unhealthy().await;
    let resp = hc.handle_request(&req("/livez")).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// Regression: a checker that never returns used to stall `/readyz`
/// indefinitely, so the revision never became ready and never drained. Each
/// checker now runs under its own deadline.
#[tokio::test]
async fn a_hung_checker_times_out_instead_of_stalling_readiness() {
    let hc = HealthCheck::new().with_check_timeout(Duration::from_millis(50));
    hc.register(FnHealthChecker::new("wedged", || async {
        std::future::pending::<()>().await;
        Ok(())
    }))
    .await;
    hc.register(FnHealthChecker::new("fine", || async { Ok(()) }))
        .await;

    let result = tokio::time::timeout(Duration::from_secs(5), hc.check())
        .await
        .expect("readiness must not hang on a wedged checker");

    assert_eq!(result.status, HealthStatus::Unhealthy);

    let wedged = result
        .checks
        .iter()
        .find(|c| c.name == "wedged")
        .expect("the timed-out check must still be named in the response");
    assert_eq!(wedged.status, HealthStatus::Unhealthy);

    // One hung checker must not take the healthy ones down with it.
    let fine = result.checks.iter().find(|c| c.name == "fine").unwrap();
    assert_eq!(fine.status, HealthStatus::Healthy);
}

/// Regression: checkers ran serially, so readiness latency was the SUM of
/// every check rather than the slowest one.
#[tokio::test]
async fn checkers_run_concurrently() {
    let hc = HealthCheck::new();
    for i in 0..4 {
        hc.register(FnHealthChecker::new(format!("slow-{i}"), || async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok(())
        }))
        .await;
    }

    let started = Instant::now();
    let result = hc.check().await;
    let elapsed = started.elapsed();

    assert_eq!(result.status, HealthStatus::Healthy);
    assert_eq!(result.checks.len(), 4);
    assert!(
        elapsed < Duration::from_millis(450),
        "4 x 150ms checks took {elapsed:?}; they are still running serially"
    );
}

#[tokio::test]
async fn unknown_path_is_not_found() {
    let hc = HealthCheck::new();
    let resp = hc.handle_request(&req("/does-not-exist")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
