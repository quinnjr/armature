//! Tests for `PushService`'s dispatch and error-selection behavior.
//!
//! These drive fake providers rather than any real backend, so they cover the
//! routing and aggregation logic in `src/provider.rs` directly: which provider
//! a given call reaches, and which error survives when several fail.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use armature_push::{
    DeviceToken, Notification, Platform, PushError, PushProvider, PushService, Result, Subscription,
};
use async_trait::async_trait;

/// A provider that records how many sends it received and returns a scripted
/// outcome.
struct FakeProvider {
    platform: Platform,
    calls: Arc<AtomicUsize>,
    outcome: Box<dyn Fn() -> Result<()> + Send + Sync>,
}

impl FakeProvider {
    fn ok(platform: Platform) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                platform,
                calls: calls.clone(),
                outcome: Box::new(|| Ok(())),
            },
            calls,
        )
    }

    fn failing(
        platform: Platform,
        make_err: impl Fn() -> PushError + Send + Sync + 'static,
    ) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                platform,
                calls: calls.clone(),
                outcome: Box::new(move || Err(make_err())),
            },
            calls,
        )
    }
}

#[async_trait]
impl PushProvider for FakeProvider {
    async fn send(&self, _token: &str, _notification: &Notification) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.outcome)()
    }

    fn platform(&self) -> Platform {
        self.platform
    }
}

fn notification() -> Notification {
    Notification::new("Hi", "there")
}

#[tokio::test]
async fn send_to_device_routes_by_platform_and_skips_others() {
    let (fcm, fcm_calls) = FakeProvider::ok(Platform::Android);
    let (apns, apns_calls) = FakeProvider::ok(Platform::Ios);
    let service = PushService::new().add_provider(fcm).add_provider(apns);

    service
        .send_to_device(&DeviceToken::ios("apns-token"), notification())
        .await
        .expect("iOS send should succeed");

    assert_eq!(
        fcm_calls.load(Ordering::SeqCst),
        0,
        "an iOS token must not cost a round-trip to FCM"
    );
    assert_eq!(apns_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn send_to_device_without_a_matching_provider_is_a_config_error() {
    let (fcm, _) = FakeProvider::ok(Platform::Android);
    let service = PushService::new().add_provider(fcm);

    let err = service
        .send_to_device(&DeviceToken::ios("apns-token"), notification())
        .await
        .expect_err("no iOS provider is configured");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config, got {err:?}"
    );
}

#[tokio::test]
async fn send_tries_every_provider_until_one_succeeds() {
    let (failing, failing_calls) =
        FakeProvider::failing(Platform::Android, || PushError::Network("down".into()));
    let (working, working_calls) = FakeProvider::ok(Platform::Ios);
    let service = PushService::new()
        .add_provider(failing)
        .add_provider(working);

    service
        .send("some-token", notification())
        .await
        .expect("the second provider should succeed");

    assert_eq!(failing_calls.load(Ordering::SeqCst), 1);
    assert_eq!(working_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn send_stops_at_the_first_success() {
    let (working, working_calls) = FakeProvider::ok(Platform::Android);
    let (second, second_calls) = FakeProvider::ok(Platform::Ios);
    let service = PushService::new()
        .add_provider(working)
        .add_provider(second);

    service.send("t", notification()).await.expect("ok");

    assert_eq!(working_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        second_calls.load(Ordering::SeqCst),
        0,
        "must not keep going after a success"
    );
}

/// The core of the fan-out bug: a transient failure followed by a spurious
/// removal verdict from a provider the token was never issued by. Keeping only
/// the *last* error handed the caller a `should_remove_device()` error and got
/// a valid registration deleted.
#[tokio::test]
async fn send_does_not_surface_a_spurious_removal_verdict() {
    let (transient, _) =
        FakeProvider::failing(Platform::Android, || PushError::Network("blip".into()));
    let (mismatched, _) = FakeProvider::failing(Platform::Ios, || {
        PushError::InvalidSubscription("not an APNS token".into())
    });
    let service = PushService::new()
        .add_provider(transient)
        .add_provider(mismatched);

    let err = service
        .send("android-token", notification())
        .await
        .expect_err("both providers failed");

    assert!(
        !err.should_remove_device(),
        "a removal verdict from a mismatched provider must not win: {err:?}"
    );
    assert!(
        matches!(err, PushError::Network(_)),
        "expected the transient error to survive, got {err:?}"
    );
}

#[tokio::test]
async fn send_passes_through_a_unanimous_removal_verdict() {
    // When every provider agrees the device is gone, that verdict is
    // consistent and should reach the caller so the token gets pruned.
    let (a, _) =
        FakeProvider::failing(Platform::Android, || PushError::Unregistered("gone".into()));
    let (b, _) = FakeProvider::failing(Platform::Ios, || PushError::Unregistered("gone".into()));
    let service = PushService::new().add_provider(a).add_provider(b);

    let err = service
        .send("dead-token", notification())
        .await
        .expect_err("both providers failed");
    assert!(
        err.should_remove_device(),
        "a unanimous removal verdict must survive: {err:?}"
    );
}

#[tokio::test]
async fn send_with_no_providers_is_a_config_error() {
    let service = PushService::new();
    let err = service
        .send("t", notification())
        .await
        .expect_err("no providers configured");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config, got {err:?}"
    );
}

#[tokio::test]
async fn send_to_subscription_selects_the_web_provider() {
    let (fcm, fcm_calls) = FakeProvider::ok(Platform::Android);
    let (web, web_calls) = FakeProvider::ok(Platform::Web);
    let service = PushService::new().add_provider(fcm).add_provider(web);

    let sub = Subscription::new("https://push.example.com/ep", "p256dh", "auth");
    service
        .send_to_subscription(&sub, notification())
        .await
        .expect("web send should succeed");

    assert_eq!(fcm_calls.load(Ordering::SeqCst), 0);
    assert_eq!(web_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn send_to_subscription_without_a_web_provider_is_a_config_error() {
    let (fcm, _) = FakeProvider::ok(Platform::Android);
    let service = PushService::new().add_provider(fcm);

    let sub = Subscription::new("https://push.example.com/ep", "p256dh", "auth");
    let err = service
        .send_to_subscription(&sub, notification())
        .await
        .expect_err("no web provider configured");
    assert!(
        matches!(err, PushError::Config(_)),
        "expected Config, got {err:?}"
    );
}

#[tokio::test]
async fn send_batch_routes_each_device_to_its_platform() {
    let (fcm, fcm_calls) = FakeProvider::ok(Platform::Android);
    let (apns, apns_calls) = FakeProvider::ok(Platform::Ios);
    let service = PushService::new().add_provider(fcm).add_provider(apns);

    let devices = vec![
        DeviceToken::android("a1"),
        DeviceToken::ios("i1"),
        DeviceToken::android("a2"),
    ];
    let results = service.send_batch(&devices, notification()).await;

    assert_eq!(results.len(), 3, "one result per device");
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert_eq!(fcm_calls.load(Ordering::SeqCst), 2);
    assert_eq!(apns_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn send_batch_reports_per_device_failures_without_aborting() {
    let (fcm, _) = FakeProvider::ok(Platform::Android);
    let service = PushService::new().add_provider(fcm);

    // The iOS device has no provider; the Android ones still go through.
    let devices = vec![
        DeviceToken::android("a1"),
        DeviceToken::ios("i1"),
        DeviceToken::android("a2"),
    ];
    let results = service.send_batch(&devices, notification()).await;

    assert!(results[0].is_ok());
    assert!(
        results[1].is_err(),
        "unroutable device should report an error"
    );
    assert!(results[2].is_ok());
}

#[tokio::test]
async fn send_to_tokens_uses_the_platform_provider() {
    let (fcm, fcm_calls) = FakeProvider::ok(Platform::Android);
    let (apns, apns_calls) = FakeProvider::ok(Platform::Ios);
    let service = PushService::new().add_provider(fcm).add_provider(apns);

    let tokens: Vec<String> = (0..5).map(|i| format!("t{i}")).collect();
    let results = service
        .send_to_tokens(Platform::Android, &tokens, notification())
        .await;

    assert_eq!(results.len(), 5);
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert_eq!(fcm_calls.load(Ordering::SeqCst), 5);
    assert_eq!(apns_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn send_to_tokens_without_a_provider_errors_per_token() {
    let service = PushService::new();
    let tokens: Vec<String> = (0..3).map(|i| format!("t{i}")).collect();
    let results = service
        .send_to_tokens(Platform::Web, &tokens, notification())
        .await;

    assert_eq!(
        results.len(),
        3,
        "one result per token even with no provider"
    );
    assert!(
        results
            .iter()
            .all(|r| matches!(r, Err(PushError::Config(_)))),
        "{results:?}"
    );
}
