//! Exercises `armature-push`'s public surface from *outside* the crate.
//!
//! Every other unit test in this crate is an in-crate `#[cfg(test)]` module,
//! which can reach private items and therefore cannot see export gaps.
//! `SubscriptionKeys` was exactly that: it is the type of a `pub` field on the
//! exported `Subscription`, so callers could read `sub.keys.p256dh` but could
//! not *name* the type to hold it in a variable, take it as a parameter, or
//! build a `Subscription` literal. No test noticed, because no test compiled
//! as an external crate.
//!
//! The rule this file enforces: **every public field's type must itself be
//! nameable through the crate root.** Each assertion below names a type in a
//! position that requires the export to exist (a type annotation, a return
//! type, a turbofish) — a plain field read would compile even with the export
//! missing.

use armature_push::{
    DeviceToken, Notification, NotificationAction, Platform, Priority, PushError, PushProvider,
    PushService, Result, Subscription, SubscriptionKeys, Urgency,
};

/// The regression this file was added for. `keys` is a `pub` field of the
/// exported `Subscription`, so its type has to be nameable.
#[test]
fn subscription_keys_is_nameable_from_outside_the_crate() {
    let subscription = Subscription::new("https://push.example/ep", "p256dh-key", "auth-secret");

    // A type annotation, not a field read: this line does not compile unless
    // `SubscriptionKeys` is re-exported.
    let keys: &SubscriptionKeys = &subscription.keys;
    assert_eq!(keys.p256dh, "p256dh-key");
    assert_eq!(keys.auth, "auth-secret");

    // And it must be constructible, so a caller can build a `Subscription`
    // from stored parts rather than only through `new`.
    let built = SubscriptionKeys {
        p256dh: "a".to_string(),
        auth: "b".to_string(),
    };
    assert_eq!(built.p256dh, "a");
}

/// Every other `pub` field type on `Subscription` and `DeviceToken`.
#[test]
fn subscription_and_device_token_public_field_types_are_nameable() {
    let subscription = Subscription::new("https://push.example/ep", "p", "a").user_id("user-1");
    let _endpoint: &String = &subscription.endpoint;
    let _expiration: &Option<i64> = &subscription.expiration_time;
    let _user_id: &Option<String> = &subscription.user_id;
    assert!(!subscription.is_expired());

    let device = DeviceToken::ios("apns-token")
        .user_id("user-1")
        .device_id("device-1");
    let _token: &String = &device.token;
    let platform: Platform = device.platform;
    assert_eq!(platform, Platform::Ios);
    let _user_id: &Option<String> = &device.user_id;
    let _device_id: &Option<String> = &device.device_id;
    let _app_version: &Option<String> = &device.app_version;
    let _last_active: &Option<i64> = &device.last_active;
    let _created_at: &Option<i64> = &device.created_at;
}

/// The notification builder types and both of its enums.
#[test]
fn notification_types_are_nameable() {
    let notification: Notification = Notification::new("Title", "Body")
        .priority(Priority::High)
        .urgency(Urgency::VeryLow)
        .action(NotificationAction::new("view", "View"));

    let _priority: Priority = notification.priority;
    let _urgency: Urgency = notification.urgency;
    let _actions: &Vec<NotificationAction> = &notification.actions;

    // `Notification` must be `Clone`: `PushService::send_to_device` takes it by
    // value, so any caller sending to more than one device needs to clone it.
    let _cloned = notification.clone();
}

/// `PushError::Provider` is a struct variant, so its field names are part of
/// the public API. This restructure (from `Provider(String)`) is what makes a
/// transient 5xx retryable, and callers matching on it must be able to read
/// the status.
#[test]
fn push_error_provider_variant_exposes_its_status() {
    let err = PushError::provider(503u16, "upstream unavailable");
    match &err {
        PushError::Provider { status, message } => {
            let status: &Option<u16> = status;
            assert_eq!(*status, Some(503));
            assert!(message.contains("upstream"));
        }
        other => panic!("expected Provider, got {other:?}"),
    }
    assert!(err.is_retryable(), "a 5xx provider error must be retryable");
    assert!(!err.should_remove_device());

    // A `Provider` error with no status carries no retry information, so it is
    // deliberately not retryable.
    assert!(!PushError::provider(None, "no status").is_retryable());
}

/// `Result` is the crate's own alias; naming it proves it is exported.
#[test]
fn crate_result_alias_is_nameable() {
    fn returns_crate_result() -> Result<u8> {
        Ok(7)
    }
    assert_eq!(returns_crate_result().unwrap(), 7);
}

/// `PushProvider` must be nameable and object-safe: the documented way to hold
/// several providers is behind a trait object.
#[test]
fn push_provider_is_object_safe_and_service_is_constructible() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<PushService>();

    let _service = PushService::new();
    let _boxed: Option<Box<dyn PushProvider>> = None;
}

/// The prelude must re-export the same set, including `SubscriptionKeys`.
#[test]
fn prelude_exports_the_public_types() {
    use armature_push::prelude::*;

    let subscription = Subscription::new("https://push.example/ep", "p", "a");
    let _keys: &SubscriptionKeys = &subscription.keys;
    let _device: DeviceToken = DeviceToken::web("endpoint|p256dh|auth");
    let _platform: Platform = Platform::Web;
    let _notification: Notification = Notification::new("t", "b")
        .priority(Priority::Normal)
        .urgency(Urgency::Normal)
        .action(NotificationAction::new("a", "A"));
    let _err: PushError = PushError::Timeout;
    let _service: PushService = PushService::new();
}

/// Provider config types are `#[non_exhaustive]`, so they must be reachable
/// through their constructors and builders rather than struct literals.
#[cfg(feature = "web-push")]
#[test]
fn web_push_public_types_are_nameable() {
    use armature_push::{WebPushConfig, WebPushProvider, WebPushSubscription};

    let config: WebPushConfig = WebPushConfig::new(
        "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY",
        "mailto:a@b.c",
    );
    let _private_key: &String = &config.private_key;
    let _subject: &String = &config.subject;
    let _default_ttl: u32 = config.default_ttl;
    let _allow: bool = config.allow_insecure_loopback;
    let _timeout: std::time::Duration = config.timeout;
    let _connect_timeout: std::time::Duration = config.connect_timeout;

    let sub: WebPushSubscription = WebPushSubscription::new("https://push.example/ep", "p", "a");
    let _endpoint: &String = &sub.endpoint;
    let _p256dh: &String = &sub.p256dh;
    let _auth: &String = &sub.auth;

    let provider: WebPushProvider = WebPushProvider::new(config).expect("build provider");
    assert_eq!(provider.platform(), Platform::Web);
}

#[cfg(feature = "fcm")]
#[test]
fn fcm_public_types_are_nameable() {
    use armature_push::{FcmConfig, FcmCredentials};

    let config: FcmConfig = FcmConfig::new("project", "svc@example.com", "key")
        .api_base("https://fcm.example.com")
        .token_uri("https://oauth.example.com/token")
        .allow_insecure_loopback(false);

    let _project_id: &String = &config.project_id;
    let credentials: &FcmCredentials = &config.credentials;
    let _client_email: &String = &credentials.client_email;
    let _private_key: &String = &credentials.private_key;
    let _token_uri: &String = &credentials.token_uri;
    let _api_base: &String = &config.api_base;
    let _allow: bool = config.allow_insecure_loopback;
    let _timeout: std::time::Duration = config.timeout;
    let _connect_timeout: std::time::Duration = config.connect_timeout;

    // Secrets must not survive into `Debug` output.
    assert!(format!("{config:?}").contains("<redacted>"));
}

#[cfg(feature = "apns")]
#[test]
fn apns_public_types_are_nameable() {
    use armature_push::{ApnsConfig, ApnsEnvironment};

    let config: ApnsConfig = ApnsConfig::new("team", "key-id", "pem", "com.example.app")
        .environment(ApnsEnvironment::Custom("https://apns.example.com".into()))
        .allow_insecure_loopback(false);

    let _team_id: &String = &config.team_id;
    let _key_id: &String = &config.key_id;
    let _private_key: &String = &config.private_key;
    let _bundle_id: &String = &config.bundle_id;
    let environment: &ApnsEnvironment = &config.environment;
    assert_ne!(*environment, ApnsEnvironment::Production);
    let _allow: bool = config.allow_insecure_loopback;
    let _timeout: std::time::Duration = config.timeout;
    let _connect_timeout: std::time::Duration = config.connect_timeout;

    assert!(format!("{config:?}").contains("<redacted>"));
}
