//! Integration tests for the Web Push send path.
//!
//! Each test points a `WebPushSubscription.endpoint` at an in-process
//! `StubServer` and asserts how a given upstream status maps onto `PushError`.
//! The stub speaks plain http over loopback, which the provider's SSRF guard
//! exempts, so these exercise the real signing + request-building code.
#![cfg(feature = "web-push")]

use armature_push::{Notification, PushError, WebPushConfig, WebPushProvider, WebPushSubscription};
use armature_testkit::{StubResponse, StubServer};

// A valid VAPID private key + subscription keys borrowed from web-push's own
// test vectors, so signing and payload encryption actually succeed.
const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
const P256DH: &str =
    "BH1HTeKM7-NwaLGHEqxeu2IamQaVVLkcsFHPIHmsCnqxcBHPQBprF41bEMOr3O1hUQ2jU1opNEm1F_lZV_sxMP8";
const AUTH: &str = "sBXU5_tIYz-5w7G2B25BEw";

fn provider() -> WebPushProvider {
    let config = WebPushConfig::new(VAPID_PRIVATE, "mailto:test@example.com");
    WebPushProvider::new(config).expect("build provider")
}

fn subscription(endpoint: &str) -> WebPushSubscription {
    WebPushSubscription::new(endpoint, P256DH, AUTH)
}

fn notification() -> Notification {
    Notification::new("Hello", "World")
}

async fn send_against(resp: StubResponse) -> (Result<(), PushError>, StubServer) {
    let server = StubServer::start_single(resp).await;
    let provider = provider();
    let sub = subscription(server.url());
    let result = provider
        .send_to_web_subscription(&sub, &notification())
        .await;
    (result, server)
}

#[tokio::test]
async fn status_200_is_ok_and_carries_ttl_and_content_encoding() {
    let (result, server) = send_against(StubResponse::new(200, "")).await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");

    let req = server.assert_received("POST", "/");
    assert!(
        req.header("TTL").is_some(),
        "outgoing request missing TTL header; headers: {:?}",
        req.headers
    );
    assert!(
        req.header("Content-Encoding").is_some(),
        "outgoing request missing Content-Encoding header; headers: {:?}",
        req.headers
    );
}

#[tokio::test]
async fn status_404_maps_to_unregistered() {
    let (result, _server) = send_against(StubResponse::new(404, "")).await;
    let err = result.expect_err("expected error for 404");
    assert!(
        matches!(err, PushError::Unregistered(_)),
        "expected Unregistered, got {err:?}"
    );
}

#[tokio::test]
async fn status_410_maps_to_unregistered() {
    let (result, _server) = send_against(StubResponse::new(410, "")).await;
    let err = result.expect_err("expected error for 410");
    assert!(
        matches!(err, PushError::Unregistered(_)),
        "expected Unregistered, got {err:?}"
    );
}

#[tokio::test]
async fn status_413_maps_to_payload_too_large() {
    let (result, _server) = send_against(StubResponse::new(413, "")).await;
    let err = result.expect_err("expected error for 413");
    match err {
        PushError::PayloadTooLarge { size, limit } => {
            assert_eq!(limit, 4096, "limit should be the max payload const");
            assert!(size > 0, "size should be the real encrypted payload length");
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn status_429_with_retry_after_uses_header_value() {
    let resp = StubResponse::new(429, "").with_header("Retry-After", "30");
    let (result, _server) = send_against(resp).await;
    let err = result.expect_err("expected error for 429");
    assert!(
        matches!(err, PushError::RateLimited(30)),
        "expected RateLimited(30), got {err:?}"
    );
}

#[tokio::test]
async fn status_429_without_retry_after_defaults_to_60() {
    let (result, _server) = send_against(StubResponse::new(429, "")).await;
    let err = result.expect_err("expected error for 429");
    assert!(
        matches!(err, PushError::RateLimited(60)),
        "expected RateLimited(60), got {err:?}"
    );
}

#[tokio::test]
async fn error_does_not_echo_upstream_response_body() {
    let secret = "SECRET-UPSTREAM-BODY";
    let (result, _server) = send_against(StubResponse::new(500, secret)).await;
    let err = result.expect_err("expected error for 500");
    assert!(
        !err.to_string().contains(secret),
        "error must not propagate the upstream body: {err}"
    );
}

#[tokio::test]
async fn non_https_non_loopback_endpoint_is_rejected() {
    let provider = provider();
    let sub = subscription("http://push.example.com/endpoint");
    let err = provider
        .send_to_web_subscription(&sub, &notification())
        .await
        .expect_err("non-https public endpoint must be rejected");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config error, got {err:?}"
    );
}

#[tokio::test]
async fn internal_ip_endpoint_is_rejected() {
    let provider = provider();
    let sub = subscription("https://169.254.169.254/latest/meta-data");
    let err = provider
        .send_to_web_subscription(&sub, &notification())
        .await
        .expect_err("link-local metadata endpoint must be rejected");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config error, got {err:?}"
    );
}
