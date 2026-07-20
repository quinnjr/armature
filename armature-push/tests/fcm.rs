//! Integration tests for the FCM send path.
//!
//! Points `FcmConfig::token_uri` (OAuth2 token exchange) and
//! `FcmConfig::api_base` (the `messages:send` call) at an in-process
//! `StubServer` and asserts how a given `messages:send` status maps onto
//! `PushError`.
#![cfg(feature = "fcm")]

use std::time::Duration;

use armature_push::{FcmConfig, FcmProvider, Notification, PushError, PushProvider};
use armature_testkit::{StubResponse, StubServer};

// A throwaway RSA private key in PKCS#1 PEM form, used only to exercise the
// RS256 JWT signing path against a stub server; it signs no real tokens.
const RSA_PRIVATE_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEowIBAAKCAQEAvjQtXEx2txLa1xq2fJlPkF10aLn1wg45Sm5y/KPZZXdrm6wB\n\
MKAc/iLkRu5beVi3p/3QsuYVTIDsAI15G4YWNw8UbQM4cLLsxbsjaO517fiBhpQZ\n\
DV2YmQerrHsSTgPquBN+nOqSR1ohX48l61izwEzlE+hKyexm/E6Qjo9NSUFaCzc5\n\
HDHsoKVA9YabDhC4vncbpANVg1UpSF06kGn5GVGHmvW8W3EnxWKDRDy8+UMD0DDr\n\
4BHiyZDwe6/8M8XotH3fHeouV3xubc32TX1qdGvu3+4nqUn0Ri6gyjsf6dtL7tLV\n\
hmpOpWkE1Fhm4Y78iJZTatDk8DCqRGRsFyuy9wIDAQABAoIBAFhc6CfllA9oMIfX\n\
LqlDFj4U1KRkpCJDtmT4W+439qLXcIQRTDo5YE7GifPT/2YoC6Z9WawLDSEOEdYN\n\
45IgYIiytkQQx3NABJS15HT2t43XMeGCQwM9FMwfTqeiQ3ZABpb+44blyRBh9Hgv\n\
CihEfLmdX504gSo+6/dSToEUXQznA/NTeGzXDkCZl/ffdFUaOgdL7PM+g8SjU4yT\n\
M6V0Ptda7W0hZf01ME09vmTqqSp0A0x1qoIApw/yner0hMLjqo64GENRW2Ngjox1\n\
l4cRGtHr+3yXdadiWvCzPziUDu8mLCHZi/ltxu2xNv19JeLNm9Q4dRuKtObCdBAM\n\
/WrMHTkCgYEA86JJpgP7C+hQJWnHC7jlCIEXyiyiDBrdf+QrpmdeLILwTKtwFbi/\n\
ohj1EhjnpLsuHBI0mwLwyKOJgROXnRFZ6riKsev7KqG2XQiR7LjWO/c1lNyLXNPE\n\
eokariX4E+tLxFUCAf8AqcEw1IOxQAyVsQ251jFvY/JBuLbo025BWt8CgYEAx9ui\n\
G+SsHMlvnjjaQiUgMX6VG4jmFv7XynyiWPlIxdwqYIP7l3Qhr/aP6QoPi6+ew4p4\n\
Cu1fYY3k2FLMO87nZTiODBNvplF8Rz3KXxP6KKY+9l38ECxslrmxKjNDCdCeDoKV\n\
F34ZNmmNGf1Vwu4vkZBKlmPOU858ty/qfxaNwukCgYEAgK14ZJy5nYJnwjrqDEDt\n\
ht5X6EpGlEokLwYeH9d8n9nQfU4W9wILBNxVo+dPgWvzYJQlALI+5lmpqGjmrOib\n\
KyOo7WwLzmp23RBHslW1oRpiTGtnl/GpVmbPlqcrLaoa7GlRlChQ+1e0KKodlgyP\n\
i2IKgxy9DnbHS34f3nvfPNUCgYB+RodmmFUm2x9rGQDOSibNHu2XOCgo31v41Ea/\n\
cMJKQZGE6d9NElM2mtLSq0inOY9WfWbbgJ+DQ+QTyjzAjTom+lTFzIH+0/1yBdiX\n\
ukeU53VgtIFOtsLleO43e6wfx3AWOut4rHPBrW85vJczUss7ba+y1dzHlu+1ztCa\n\
++UWAQKBgAoE/eVdNxyYWAMMnBQWdsdheQ6r8dQzvtmav1RaNepZTvruH3DbmLrB\n\
qW4xxRDWnLdDp+8E1lofQaHdc2I5vr13FO3RwXNjj7WIYhANzzM5KH67pZvfXBpo\n\
K/DOxADfYKnB6e0U8/C3G+2QaMUZaERL+G/l+xk7ai//bTt0F+c3\n\
-----END RSA PRIVATE KEY-----\n";

const TOKEN_RESPONSE: &str = r#"{"access_token":"test-access-token","expires_in":3600}"#;

/// Build a config pointed at `server`. `allow_insecure_loopback` is required
/// because the stub speaks plain http; the default refuses a non-https base so
/// a production binary can never send credentials in the clear.
fn config_for(server: &StubServer) -> FcmConfig {
    FcmConfig::new(
        "test-project",
        "svc@test-project.iam.gserviceaccount.com",
        RSA_PRIVATE_KEY,
    )
    .api_base(server.url())
    .token_uri(format!("{}/token", server.url()))
    .allow_insecure_loopback(true)
}

async fn provider_against(send_response: StubResponse) -> (FcmProvider, StubServer) {
    let server = StubServer::builder()
        .route("POST", "/token", StubResponse::json(200, TOKEN_RESPONSE))
        .route(
            "POST",
            "/v1/projects/test-project/messages:send",
            send_response,
        )
        .start()
        .await;

    let provider = FcmProvider::new(config_for(&server))
        .await
        .expect("build provider");
    (provider, server)
}

#[tokio::test]
async fn status_200_is_ok() {
    let (provider, _server) = provider_against(StubResponse::json(200, r#"{"name":"msg"}"#)).await;
    let result = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await;
    assert!(result.is_ok(), "expected Ok, got {result:?}");
}

#[tokio::test]
async fn status_404_maps_to_unregistered() {
    // On FCM a 404 genuinely is `UNREGISTERED`. It is mapped at the FCM call
    // site rather than centrally, because 404 on APNs means `BadPath` — see
    // `status_404_is_bad_path_and_never_removes_the_device` in tests/apns.rs.
    let (provider, _server) = provider_against(StubResponse::new(404, "")).await;
    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 404");
    assert!(
        matches!(err, PushError::Unregistered(_)),
        "expected Unregistered, got {err:?}"
    );
    assert!(err.should_remove_device());
}

#[tokio::test]
async fn status_503_is_retryable() {
    // A transient FCM outage used to produce a permanently non-retryable
    // `Provider` error, so a five-minute 503 window dropped whole batches.
    let (provider, _server) = provider_against(StubResponse::new(503, "")).await;
    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 503");
    assert!(
        matches!(
            err,
            PushError::Provider {
                status: Some(503),
                ..
            }
        ),
        "expected Provider(503), got {err:?}"
    );
    assert!(err.is_retryable(), "a 5xx from FCM must be retryable");
    assert!(!err.should_remove_device());
}

#[tokio::test]
async fn status_410_maps_to_unregistered() {
    let (provider, _server) = provider_against(StubResponse::new(410, "")).await;
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
async fn status_429_with_retry_after_uses_header_value() {
    // The stub must actually send Retry-After: the previous 429 test asserted
    // RateLimited(60) against a stub that sent no header, so it passed whether
    // or not the header was read at all.
    let resp = StubResponse::new(429, "").with_header("Retry-After", "30");
    let (provider, _server) = provider_against(resp).await;
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
    let (provider, _server) = provider_against(StubResponse::new(429, "")).await;
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
    let (provider, _server) = provider_against(StubResponse::new(413, "")).await;
    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 413");
    match err {
        PushError::PayloadTooLarge { size, limit } => {
            assert_eq!(limit, 4 * 1024 * 1024, "FCM v1 limit is 4 MB");
            assert!(size > 0, "size should be the real serialized body length");
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn status_401_maps_to_auth() {
    let (provider, _server) = provider_against(StubResponse::new(401, "")).await;
    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 401");
    assert!(
        matches!(err, PushError::Auth(_)),
        "expected Auth, got {err:?}"
    );
    assert!(
        !err.should_remove_device(),
        "auth failure is not the device's fault"
    );
}

#[tokio::test]
async fn status_403_maps_to_auth() {
    let (provider, _server) = provider_against(StubResponse::new(403, "")).await;
    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 403");
    assert!(
        matches!(err, PushError::Auth(_)),
        "expected Auth, got {err:?}"
    );
}

#[tokio::test]
async fn error_does_not_echo_upstream_response_body() {
    let secret = "SECRET-UPSTREAM-BODY";
    let (provider, _server) = provider_against(StubResponse::new(500, secret)).await;
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
async fn oauth_token_exchange_failure_maps_to_auth() {
    // A revoked/expired service-account key comes back as a 4xx with a JSON
    // error body. That used to fail deserialization and surface as an opaque
    // Serialization error.
    let server = StubServer::builder()
        .route(
            "POST",
            "/token",
            StubResponse::json(401, r#"{"error":"invalid_grant"}"#),
        )
        .start()
        .await;

    let err = FcmProvider::new(config_for(&server))
        .await
        .err()
        .expect("a rejected token exchange must fail construction");
    assert!(
        matches!(err, PushError::Auth(_)),
        "expected Auth, got {err:?}"
    );
}

#[tokio::test]
async fn concurrent_sends_share_a_single_token_refresh() {
    // Regression test for the token-refresh stampede: every concurrent send
    // that observed an expired token used to issue its own OAuth2 exchange.
    // Construction performs exactly one exchange; the batch that follows must
    // add none, because the cached token is still valid.
    let server = StubServer::builder()
        .route("POST", "/token", StubResponse::json(200, TOKEN_RESPONSE))
        .route(
            "POST",
            "/v1/projects/test-project/messages:send",
            StubResponse::json(200, r#"{"name":"msg"}"#),
        )
        .start()
        .await;

    let provider = FcmProvider::new(config_for(&server))
        .await
        .expect("build provider");

    let tokens: Vec<String> = (0..32).map(|i| format!("device-{i}")).collect();
    let results = provider
        .send_batch(&tokens, &Notification::new("Hi", "there"))
        .await;
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");

    let token_hits = server
        .requests()
        .iter()
        .filter(|r| r.path == "/token")
        .count();
    assert_eq!(
        token_hits, 1,
        "expected exactly one OAuth2 token exchange across a concurrent batch, got {token_hits}"
    );
}

#[tokio::test]
async fn request_body_carries_badge_as_notification_count() {
    let (provider, server) = provider_against(StubResponse::json(200, r#"{"name":"msg"}"#)).await;
    let notification = Notification::new("Hi", "there").badge(9);
    provider
        .send("device-token", &notification)
        .await
        .expect("send should succeed");

    let req = server.assert_received("POST", "/v1/projects/test-project/messages:send");
    let body: serde_json::Value =
        serde_json::from_str(&req.body_string()).expect("body should be JSON");
    assert_eq!(
        body["message"]["android"]["notification"]["notification_count"],
        serde_json::json!(9),
        "badge must reach the wire as notification_count: {body}"
    );
}

#[tokio::test]
async fn provider_builds_and_sends_with_custom_timeouts() {
    // `FcmProvider::new` performs a network token exchange, so this is where
    // acceptance of the configured timeouts is actually proven end-to-end.
    let server = StubServer::builder()
        .route("POST", "/token", StubResponse::json(200, TOKEN_RESPONSE))
        .route(
            "POST",
            "/v1/projects/test-project/messages:send",
            StubResponse::json(200, r#"{"name":"msg"}"#),
        )
        .start()
        .await;

    let config = config_for(&server)
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2));
    let provider = FcmProvider::new(config)
        .await
        .expect("provider should build with custom timeouts");

    provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect("send should succeed under the configured timeouts");
}

#[tokio::test]
async fn a_connection_dropped_mid_request_is_a_retryable_network_error() {
    // FCM's send path goes through `?`, i.e. `From<reqwest::Error>`. That impl
    // classified only `is_timeout()` and `is_connect()`, sending
    // `is_request()`/`is_body()` — a TCP reset after connect, a TLS
    // renegotiation failure, a broken pipe mid-payload — to a `Provider` whose
    // `status` is always `None` (`reqwest::Error::status()` is `Some` only for
    // errors from `error_for_status()`, which no provider here calls). So those
    // transient failures reported `is_retryable() == false` and callers'
    // retry loops gave up on exactly what they were written for.
    let token_server = StubServer::builder()
        .route("POST", "/token", StubResponse::json(200, TOKEN_RESPONSE))
        .start()
        .await;

    // A listener that accepts and immediately drops, so the failure happens
    // after connect rather than during it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });

    let config = FcmConfig::new(
        "test-project",
        "svc@test-project.iam.gserviceaccount.com",
        RSA_PRIVATE_KEY,
    )
    .api_base(format!("http://{addr}"))
    .token_uri(format!("{}/token", token_server.url()))
    .allow_insecure_loopback(true)
    .connect_timeout(Duration::from_millis(500))
    .timeout(Duration::from_secs(2));

    let provider = FcmProvider::new(config).await.expect("build provider");

    let err = tokio::time::timeout(
        Duration::from_secs(10),
        provider.send("device-token", &Notification::new("Hi", "there")),
    )
    .await
    .expect("send should not hang")
    .expect_err("a connection dropped mid-request must fail");

    assert!(
        matches!(err, PushError::Network(_)),
        "expected Network, got {err:?}"
    );
    assert!(
        err.is_retryable(),
        "a mid-request transport failure must be retryable: {err:?}"
    );
}

#[tokio::test]
async fn non_https_api_base_is_rejected_without_opt_in() {
    let config = FcmConfig::new(
        "test-project",
        "svc@test-project.iam.gserviceaccount.com",
        RSA_PRIVATE_KEY,
    )
    .api_base("http://127.0.0.1:1")
    .token_uri("http://127.0.0.1:1/token");

    let err = FcmProvider::new(config)
        .await
        .err()
        .expect("plain http must be refused without the explicit opt-in");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config, got {err:?}"
    );
}
