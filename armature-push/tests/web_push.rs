//! Integration tests for the Web Push send path.
//!
//! Each test points a `WebPushSubscription.endpoint` at an in-process
//! `StubServer` and asserts how a given upstream status maps onto `PushError`.
//! The stub speaks plain http over loopback, which the provider's SSRF guard
//! exempts, so these exercise the real signing + request-building code.
#![cfg(feature = "web-push")]

use std::time::Duration;

use armature_push::{
    Notification, PushError, Urgency, WebPushConfig, WebPushProvider, WebPushSubscription,
};
use armature_testkit::{StubResponse, StubServer};

// A valid VAPID private key + subscription keys borrowed from web-push's own
// test vectors, so signing and payload encryption actually succeed.
const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
const P256DH: &str =
    "BH1HTeKM7-NwaLGHEqxeu2IamQaVVLkcsFHPIHmsCnqxcBHPQBprF41bEMOr3O1hUQ2jU1opNEm1F_lZV_sxMP8";
const AUTH: &str = "sBXU5_tIYz-5w7G2B25BEw";

/// Provider whose SSRF guard permits loopback endpoints, as the stub servers
/// below require. Production configs leave `allow_insecure_loopback` at its
/// `false` default.
fn provider() -> WebPushProvider {
    let config =
        WebPushConfig::new(VAPID_PRIVATE, "mailto:test@example.com").allow_insecure_loopback(true);
    WebPushProvider::new(config).expect("build provider")
}

/// Provider with production defaults — loopback is *not* exempt.
fn strict_provider() -> WebPushProvider {
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
async fn urgency_high_sends_high_urgency_header() {
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider();
    let sub = subscription(server.url());
    let notification = Notification::new("Hello", "World").urgency(Urgency::High);

    let result = provider.send_to_web_subscription(&sub, &notification).await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");

    let req = server.assert_received("POST", "/");
    assert_eq!(
        req.header("Urgency"),
        Some("high"),
        "expected Urgency: high header derived from Notification::urgency, headers: {:?}",
        req.headers
    );
}

#[tokio::test]
async fn urgency_default_sends_normal_urgency_header() {
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider();
    let sub = subscription(server.url());

    let result = provider
        .send_to_web_subscription(&sub, &notification())
        .await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");

    let req = server.assert_received("POST", "/");
    assert_eq!(
        req.header("Urgency"),
        Some("normal"),
        "expected Urgency: normal header for default urgency, headers: {:?}",
        req.headers
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

/// The IPv4-mapped IPv6 forms of internal addresses. These are the cases that
/// previously slipped through: `is_internal_v6` only inspected segment 0, which
/// is `0` for a mapped address, and the `Host::Ipv4` arm was never consulted
/// because the URL parses as `Host::Ipv6`.
#[tokio::test]
async fn ipv4_mapped_internal_endpoints_are_rejected() {
    // Production defaults: loopback is not exempt either, so the mapped
    // loopback form below must be refused along with the rest.
    let provider = strict_provider();
    for endpoint in [
        "https://[::ffff:169.254.169.254]/latest/meta-data",
        "https://[::ffff:10.0.0.1]/",
        "https://[::ffff:127.0.0.1]/",
        "https://[::ffff:192.168.1.1]/",
    ] {
        let sub = subscription(endpoint);
        let err = provider
            .send_to_web_subscription(&sub, &notification())
            .await
            .unwrap_err_or_panic(endpoint);
        assert!(
            matches!(err, PushError::Config(_)),
            "expected Config error for {endpoint}, got {err:?}"
        );
    }
}

#[tokio::test]
async fn cgnat_endpoint_is_rejected() {
    // 100.64.0.0/10 is routable to infrastructure inside many cloud VPCs.
    let provider = provider();
    let sub = subscription("https://100.64.0.1/internal");
    let err = provider
        .send_to_web_subscription(&sub, &notification())
        .await
        .expect_err("CGNAT endpoint must be rejected");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config error, got {err:?}"
    );
}

#[tokio::test]
async fn loopback_endpoint_is_rejected_without_opt_in() {
    // The loopback exemption exists for local stub tests. Without the explicit
    // opt-in it must not apply, or an untrusted subscription endpoint could
    // drive requests at services bound to localhost on the app host.
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = strict_provider();
    let sub = subscription(server.url());

    let err = provider
        .send_to_web_subscription(&sub, &notification())
        .await
        .expect_err("loopback must be refused with production defaults");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config error, got {err:?}"
    );
    assert!(
        server.requests().is_empty(),
        "no connection should have been opened"
    );
}

#[tokio::test]
async fn timeout_is_reported_as_retryable_timeout() {
    // A stub that accepts the connection and never responds. The transport
    // failure must surface as `PushError::Timeout` (retryable), not collapse
    // into `Provider` (which reports `is_retryable() == false`, silently
    // stopping a caller's retry loop on exactly the transient failure it was
    // written for).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    let _accept = tokio::spawn(async move {
        // Hold accepted connections open, reading nothing, until the test ends.
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });

    // A short configured timeout, so this exercises the timeout path without
    // waiting out the 30s default.
    let config = WebPushConfig::new(VAPID_PRIVATE, "mailto:test@example.com")
        .allow_insecure_loopback(true)
        .timeout(Duration::from_millis(200));
    let provider = WebPushProvider::new(config).expect("build provider");
    let sub = subscription(&format!("http://{addr}/"));

    let err = tokio::time::timeout(
        Duration::from_secs(10),
        provider.send_to_web_subscription(&sub, &notification()),
    )
    .await
    .expect("the configured 200ms timeout should fire first")
    .expect_err("expected a timeout error");

    assert!(
        matches!(err, PushError::Timeout),
        "expected Timeout, got {err:?}"
    );
    assert!(err.is_retryable(), "a timeout must be retryable: {err:?}");
}

#[tokio::test]
async fn oversized_payload_is_rejected_before_sending() {
    // The 4096-byte limit applies to the encrypted body. Catching it locally
    // saves a round-trip that is guaranteed to come back 413.
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider();
    let sub = subscription(server.url());

    let big = Notification::new("Title", "x".repeat(8192));
    let err = provider
        .send_to_web_subscription(&sub, &big)
        .await
        .expect_err("an oversized payload must be refused locally");

    match err {
        PushError::PayloadTooLarge { size, limit } => {
            assert!(
                size > limit,
                "reported size {size} must exceed the reported limit {limit}"
            );
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
    assert!(
        server.requests().is_empty(),
        "no request should have been sent"
    );
}

#[tokio::test]
async fn send_to_subscription_works_through_the_trait_object() {
    // The trait impl delegates to the inherent method of the same name; this
    // pins that the dyn dispatch path actually reaches the real send rather
    // than recursing.
    use armature_push::{PushProvider, Subscription};

    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider: Box<dyn PushProvider> = Box::new(provider());
    let sub = Subscription::new(server.url(), P256DH, AUTH);

    let result = provider.send_to_subscription(&sub, &notification()).await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");
    server.assert_received("POST", "/");
}

/// Small helper so the loop above can report which endpoint failed.
trait UnwrapErrOrPanic<E> {
    fn unwrap_err_or_panic(self, context: &str) -> E;
}

impl<E> UnwrapErrOrPanic<E> for Result<(), E> {
    fn unwrap_err_or_panic(self, context: &str) -> E {
        match self {
            Err(e) => e,
            Ok(()) => panic!("expected an error for {context}, got Ok"),
        }
    }
}
