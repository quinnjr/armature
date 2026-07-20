//! Integration tests for the APNS send path.
//!
//! Each test points `ApnsEnvironment::Custom` at an in-process `StubServer`
//! and asserts how a given upstream status maps onto `PushError`, and that a
//! per-notification `topic` overrides the configured bundle ID in the
//! `apns-topic` header.
#![cfg(feature = "apns")]

use armature_push::{
    ApnsConfig, ApnsEnvironment, ApnsProvider, Notification, PushError, PushProvider,
};
use armature_testkit::{StubResponse, StubServer};

// A throwaway EC (P-256) private key in PKCS#8 PEM form, used only to
// exercise the ES256 JWT signing path against a stub server; it signs no
// real tokens. `jsonwebtoken`'s PEM decoder only recognizes `PRIVATE KEY`
// (PKCS#8), not the SEC1 `EC PRIVATE KEY` header.
const EC_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgOyXw4GYy1pYEbh35\n\
9XgYkJb1FGjk1aL27YuyTsnamBShRANCAAQbRc4Wqq2In6Cu30VQw7vuS+Ge34qM\n\
j+ve0j9VziXnHi5UsZiy3LBy4hkjOxQctW99w9n66PflFhaNWSA1CmT3\n\
-----END PRIVATE KEY-----\n";

async fn provider_against(server: &StubServer) -> ApnsProvider {
    // `allow_insecure_loopback` is required because the stub speaks plain http;
    // the default refuses a non-https endpoint so a production binary can never
    // send a JWT-bearing request in the clear.
    let config = ApnsConfig::new("team-id", "key-id", EC_PRIVATE_KEY, "com.example.app")
        .environment(ApnsEnvironment::Custom(server.url().to_string()))
        .allow_insecure_loopback(true);
    ApnsProvider::new(config).await.expect("build provider")
}

#[tokio::test]
async fn topic_override_sets_apns_topic_header() {
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider_against(&server).await;

    let notification = Notification::new("Hi", "there").topic("com.example.other");
    let result = provider.send("device-token", &notification).await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");

    let req = server.assert_received("POST", "/3/device/device-token");
    assert_eq!(
        req.header("apns-topic"),
        Some("com.example.other"),
        "per-notification topic should override the configured bundle ID, headers: {:?}",
        req.headers
    );
}

#[tokio::test]
async fn default_topic_falls_back_to_bundle_id() {
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider_against(&server).await;

    let notification = Notification::new("Hi", "there");
    let result = provider.send("device-token", &notification).await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");

    let req = server.assert_received("POST", "/3/device/device-token");
    assert_eq!(req.header("apns-topic"), Some("com.example.app"));
}

#[tokio::test]
async fn status_410_maps_to_unregistered() {
    let server = StubServer::start_single(StubResponse::new(410, "")).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 410");
    assert!(
        matches!(err, PushError::Unregistered(_)),
        "expected Unregistered, got {err:?}"
    );
}

#[tokio::test]
async fn status_400_bad_device_token_maps_to_invalid_subscription() {
    let server = StubServer::start_single(StubResponse::new(400, "BadDeviceToken")).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 400");
    assert!(
        matches!(err, PushError::InvalidSubscription(_)),
        "expected InvalidSubscription, got {err:?}"
    );
}

#[tokio::test]
async fn status_429_with_retry_after_uses_header_value() {
    // The stub must actually send Retry-After: the previous 429 test asserted
    // RateLimited(60) against a stub that sent no header, so it passed whether
    // or not the header was read at all.
    let server =
        StubServer::start_single(StubResponse::new(429, "").with_header("Retry-After", "30")).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 429");
    assert!(
        matches!(err, PushError::RateLimited(30)),
        "expected RateLimited(30) from the Retry-After header, got {err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn status_429_without_retry_after_defaults_to_60() {
    let server = StubServer::start_single(StubResponse::new(429, "")).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 429");
    assert!(
        matches!(err, PushError::RateLimited(60)),
        "expected RateLimited(60), got {err:?}"
    );
}

#[tokio::test]
async fn status_413_maps_to_payload_too_large() {
    // APNS enforces a 4 KB limit; this used to surface as an opaque
    // `Provider`, so callers had no structured way to detect an oversize
    // payload on iOS.
    let server = StubServer::start_single(StubResponse::new(413, "")).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 413");
    match err {
        PushError::PayloadTooLarge { size, limit } => {
            assert_eq!(limit, 4096, "APNS alert/background limit is 4 KB");
            assert!(size > 0, "size should be the real serialized body length");
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn status_404_is_bad_path_and_never_removes_the_device() {
    // On APNs 404 is `BadPath`: *we* built the wrong `:path` — wrong topic,
    // wrong environment, wrong host. It says nothing about the device. This
    // was previously mapped (via the shared status mapper, which unified it
    // with FCM's genuine `UNREGISTERED`) to `PushError::Unregistered`, whose
    // `should_remove_device()` is `true` — so one misconfigured deploy made
    // every send report "this device is gone" and a caller honoring that
    // contract deleted its entire iOS token table.
    let server = StubServer::start_single(StubResponse::new(404, "")).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 404");
    assert!(
        !err.should_remove_device(),
        "an APNs 404 must never prune the device: {err:?}"
    );
    assert!(
        matches!(
            err,
            PushError::Provider {
                status: Some(404),
                ..
            }
        ),
        "expected Provider(404), got {err:?}"
    );
    assert!(
        err.to_string().contains("BadPath"),
        "the error should name the real APNs reason: {err}"
    );
}

#[tokio::test]
async fn status_403_maps_to_auth() {
    // APNS returns 403 for ExpiredProviderToken / InvalidProviderToken — a
    // problem with our signing key, not with the device.
    let server = StubServer::start_single(StubResponse::new(
        403,
        r#"{"reason":"ExpiredProviderToken"}"#,
    ))
    .await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 403");
    assert!(
        matches!(err, PushError::Auth(_)),
        "expected Auth, got {err:?}"
    );
    assert!(
        !err.should_remove_device(),
        "a provider-token problem must not prune the device"
    );
}

#[tokio::test]
async fn status_410_with_expired_token_reason_maps_to_token_expired() {
    let server =
        StubServer::start_single(StubResponse::new(410, r#"{"reason":"ExpiredToken"}"#)).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 410");
    assert!(
        matches!(err, PushError::TokenExpired(_)),
        "expected TokenExpired, got {err:?}"
    );
    assert!(err.should_remove_device());
}

#[tokio::test]
async fn error_does_not_echo_upstream_response_body() {
    let secret = "SECRET-UPSTREAM-BODY";
    let server = StubServer::start_single(StubResponse::new(500, secret)).await;
    let provider = provider_against(&server).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 500");
    assert!(
        !err.to_string().contains(secret),
        "error must not propagate the upstream body: {err}"
    );
}

#[tokio::test]
async fn request_body_carries_aps_and_image() {
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider_against(&server).await;

    let notification = Notification::new("Title", "Body")
        .badge(4)
        .image("https://example.com/pic.png")
        .data("order_id", "12345");
    provider
        .send("device-token", &notification)
        .await
        .expect("send should succeed");

    let req = server.assert_received("POST", "/3/device/device-token");
    let raw = req.body_string();
    assert_eq!(
        raw.matches("\"aps\":").count(),
        1,
        "exactly one aps member must be on the wire: {raw}"
    );

    let body: serde_json::Value = serde_json::from_str(&raw).expect("body should be JSON");
    assert_eq!(body["aps"]["badge"], serde_json::json!(4), "{body}");
    assert_eq!(body["aps"]["alert"]["title"], serde_json::json!("Title"));
    assert_eq!(
        body["image-url"],
        serde_json::json!("https://example.com/pic.png"),
        "image must reach the wire: {body}"
    );
    assert_eq!(body["aps"]["mutable-content"], serde_json::json!(1));
    assert_eq!(body["order_id"], serde_json::json!("12345"));
}

#[tokio::test]
async fn send_batch_prepares_once_and_sends_one_request_per_token() {
    // `send_batch` used to call `send` per token, rebuilding the payload
    // (cloning the data HashMap) and re-running `serde_json::to_vec` for each
    // one — 10k identical serializations for a 10k-device campaign.
    //
    // The "one payload, one JWT" property is *not* observable at the HTTP
    // layer: a single-key payload serializes byte-identically every time, and
    // the JWT is cached with a whole-second `iat`, so five sends in one second
    // share it either way. Every body/header assertion below therefore holds
    // against the old send-per-token code too, and only `prepare_count()`
    // actually pins the rewrite. The wire assertions are kept for what they do
    // verify: one request per token, at its own URL, with the right topic.
    let server = StubServer::builder()
        .default_response(StubResponse::new(200, ""))
        .start()
        .await;
    let provider = provider_against(&server).await;

    let tokens: Vec<String> = (0..5).map(|i| format!("device-{i}")).collect();
    let notification = Notification::new("Title", "Body").data("order_id", "12345");
    let results = provider.send_batch(&tokens, &notification).await;

    assert_eq!(results.len(), tokens.len(), "one result per input token");
    assert!(results.iter().all(|r| r.is_ok()), "got {results:?}");

    assert_eq!(
        provider.prepare_count(),
        1,
        "a 5-token batch must build the payload and fetch the JWT exactly once, \
         not once per token"
    );

    let requests = server.requests();
    assert_eq!(requests.len(), tokens.len());

    let first = requests.first().expect("at least one request");
    for req in &requests {
        assert_eq!(
            req.body_string(),
            first.body_string(),
            "the body is token-invariant and must be identical across the batch"
        );
        assert_eq!(
            req.header("authorization"),
            first.header("authorization"),
            "one JWT must be reused across the batch"
        );
        assert_eq!(req.header("apns-topic"), Some("com.example.app"));
    }

    let mut paths: Vec<String> = requests.iter().map(|r| r.path.clone()).collect();
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "/3/device/device-0",
            "/3/device/device-1",
            "/3/device/device-2",
            "/3/device/device-3",
            "/3/device/device-4",
        ],
        "each token must get its own request URL"
    );
}

#[tokio::test]
async fn send_batch_reports_a_preparation_failure_for_every_token() {
    // A reserved `aps` data key fails before any request is built. The batch
    // must still return one result per input token, all errors, and send
    // nothing.
    let server = StubServer::builder()
        .default_response(StubResponse::new(200, ""))
        .start()
        .await;
    let provider = provider_against(&server).await;

    let tokens: Vec<String> = (0..3).map(|i| format!("device-{i}")).collect();
    let notification = Notification::new("Hi", "there").data("aps", "hijacked");
    let results = provider.send_batch(&tokens, &notification).await;

    assert_eq!(results.len(), tokens.len());
    for result in &results {
        assert!(
            matches!(result, Err(PushError::Config(_))),
            "expected Config, got {result:?}"
        );
    }
    assert!(server.requests().is_empty(), "nothing should be sent");
}

/// A server that promises more body than it sends, then closes — so
/// `response.text()` fails partway through the read.
async fn truncated_body_server(status_line: &'static str, body: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");

    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // Drain enough of the request that the client finishes sending.
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len() + 512
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.flush().await;
                // Close early: the promised Content-Length never arrives.
                drop(stream);
            });
        }
    });

    format!("http://{addr}")
}

async fn provider_at(url: String) -> ApnsProvider {
    let config = ApnsConfig::new("team-id", "key-id", EC_PRIVATE_KEY, "com.example.app")
        .environment(ApnsEnvironment::Custom(url))
        .allow_insecure_loopback(true);
    ApnsProvider::new(config).await.expect("build provider")
}

#[tokio::test]
async fn an_unreadable_400_body_is_reported_rather_than_guessed_at() {
    // `response.text().await.unwrap_or_default()` was load-bearing, not
    // cosmetic: the body feeds `contains("ExpiredToken")` /
    // `contains("BadDeviceToken")`, so a truncated read turned a
    // device-removal verdict into a silent, generic `Provider(400)` and the
    // dead device stayed in the caller's table forever.
    let url = truncated_body_server("400 Bad Request", r#"{"reason":"BadDeviceToken"}"#).await;
    let provider = provider_at(url).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("a truncated response body must not be silently ignored");

    assert!(
        matches!(
            err,
            PushError::Provider {
                status: Some(400),
                ..
            }
        ),
        "expected Provider(400), got {err:?}"
    );
    assert!(
        err.to_string().contains("APNS response body unreadable"),
        "the error must say the body could not be read, got {err}"
    );
}

#[tokio::test]
async fn an_unreadable_410_body_still_removes_the_device() {
    // 410 is the one status in that arm whose verdict does not need the body:
    // Apple's bare 410 already means the device is gone. Downgrading it to a
    // generic `Provider` on an unreadable body would be the mirror image of the
    // bug above.
    let url = truncated_body_server("410 Gone", r#"{"reason":"ExpiredToken"}"#).await;
    let provider = provider_at(url).await;

    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 410");

    assert!(
        matches!(err, PushError::Unregistered(_)),
        "expected Unregistered, got {err:?}"
    );
    assert!(err.should_remove_device());
}

#[tokio::test]
async fn reserved_aps_data_key_is_rejected_before_sending() {
    let server = StubServer::start_single(StubResponse::new(200, "")).await;
    let provider = provider_against(&server).await;

    let notification = Notification::new("Hi", "there").data("aps", "hijacked");
    let err = provider
        .send("device-token", &notification)
        .await
        .expect_err("a data key of 'aps' must be refused");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config, got {err:?}"
    );
    assert!(
        server.requests().is_empty(),
        "a colliding payload must not be sent at all"
    );
}
