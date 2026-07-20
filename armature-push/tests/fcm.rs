//! Integration tests for the FCM send path.
//!
//! Points `FcmConfig::token_uri` (OAuth2 token exchange) and
//! `FcmConfig::api_base` (the `messages:send` call) at an in-process
//! `StubServer` and asserts how a given `messages:send` status maps onto
//! `PushError`.
#![cfg(feature = "fcm")]

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

    let config = FcmConfig {
        project_id: "test-project".to_string(),
        credentials: armature_push::FcmCredentials {
            client_email: "svc@test-project.iam.gserviceaccount.com".to_string(),
            private_key: RSA_PRIVATE_KEY.to_string(),
            token_uri: format!("{}/token", server.url()),
        },
        api_base: server.url().to_string(),
    };
    let provider = FcmProvider::new(config).await.expect("build provider");
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
    let (provider, _server) = provider_against(StubResponse::new(404, "")).await;
    let err = provider
        .send("device-token", &Notification::new("Hi", "there"))
        .await
        .expect_err("expected error for 404");
    assert!(
        matches!(err, PushError::Unregistered(_)),
        "expected Unregistered, got {err:?}"
    );
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
async fn status_429_maps_to_rate_limited() {
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
