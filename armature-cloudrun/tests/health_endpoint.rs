//! Integration tests for the built-in health-check HTTP endpoint.

use armature_cloudrun::{FnHealthChecker, HealthCheck};
use hyper::{Request, StatusCode};

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

#[tokio::test]
async fn unknown_path_is_not_found() {
    let hc = HealthCheck::new();
    let resp = hc.handle_request(&req("/does-not-exist")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
