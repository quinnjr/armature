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
    let config = ApnsConfig::new("team-id", "key-id", EC_PRIVATE_KEY, "com.example.app")
        .environment(ApnsEnvironment::Custom(server.url().to_string()));
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
async fn status_429_maps_to_rate_limited() {
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
