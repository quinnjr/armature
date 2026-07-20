# armature-push

Push notifications for the Armature framework.

## Features

- **Web Push** - Browser push notifications (VAPID)
- **FCM** - Firebase Cloud Messaging (OAuth2 service account, HTTP v1 API)
- **APNS** - Apple Push Notification Service (JWT-based provider auth)
- **Multi-Platform** - Register one provider per platform and send through a single `PushService`

## Installation

```toml
[dependencies]
armature-push = { version = "0.2", features = ["all"] }
```

Enable only the providers you need instead of `all`: `web-push`, `fcm`, `apns`.

## Quick Start

```rust,no_run
# #[cfg(feature = "web-push")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_push::{Notification, PushService, WebPushConfig};

let config = WebPushConfig::new("your-vapid-private-key", "mailto:admin@example.com");
let service = PushService::web_push(config)?;

let notification = Notification::new("Hello!", "You have a new message")
    .data("message_id", "123");

// Note the argument order: token first, notification second.
service.send("endpoint|p256dh|auth", notification).await?;
# Ok(())
# }
```

The config types are `#[non_exhaustive]`, so construct them through `new` and
the builder methods rather than a struct literal. All three reject non-`https`
base URLs at provider construction; `allow_insecure_loopback(true)` relaxes
that for a local stub server in tests, and is `false` by default.

## Sending to multiple providers

`PushService::web_push`, `PushService::fcm`, and `PushService::apns` each build a
service around a single provider. To handle more than one platform, add
providers to a shared service with `add_provider`:

```rust,no_run
# #[cfg(all(feature = "web-push", feature = "fcm", feature = "apns"))]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_push::{
    ApnsConfig, ApnsProvider, FcmConfig, FcmProvider, PushService, WebPushConfig,
    WebPushProvider,
};

let web_push = WebPushProvider::new(WebPushConfig::new(
    "your-vapid-private-key",
    "mailto:admin@example.com",
))?;
let fcm = FcmProvider::new(FcmConfig::from_service_account("service-account.json")?).await?;
let apns = ApnsProvider::new(ApnsConfig::new(
    "team-id",
    "key-id",
    "-----BEGIN PRIVATE KEY-----...",
    "com.example.app",
))
.await?;

let service = PushService::new()
    .add_provider(web_push)
    .add_provider(fcm)
    .add_provider(apns);
# Ok(())
# }
```

Send with `send_to_device` and a platform-tagged `DeviceToken`. This is the only
method that routes by platform, so each token goes straight to the provider that
issued it:

```rust,no_run
# async fn example(service: armature_push::PushService) -> Result<(), Box<dyn std::error::Error>> {
use armature_push::{DeviceToken, Notification};

let notification = Notification::new("Hi", "there");

service
    .send_to_device(&DeviceToken::web("endpoint|p256dh|auth"), notification.clone())
    .await?;
service
    .send_to_device(&DeviceToken::android("fcm-token"), notification.clone())
    .await?;
service
    .send_to_device(&DeviceToken::ios("apns-token"), notification)
    .await?;
# Ok(())
# }
```

`PushService::send` takes a bare token, which carries no platform information, so
it can only try each provider in insertion order until one succeeds — with the
three-provider setup above an APNS token costs a wasted authenticated round-trip
to `fcm.googleapis.com` before it reaches APNS. Use it for a single-provider
service; prefer `send_to_device` whenever the platform is known.

For many devices at once, `send_batch` takes a `&[DeviceToken]` and fans out with
bounded concurrency, routing each device by platform and returning one result per
input device in order:

```rust,no_run
# async fn example(service: armature_push::PushService, devices: Vec<armature_push::DeviceToken>) {
use armature_push::Notification;

let results = service
    .send_batch(&devices, Notification::new("Hi", "there"))
    .await;
assert_eq!(results.len(), devices.len());
# }
```

## Building a notification

`Notification::new(title, body)` plus chained setters covers the common case;
each field documents which providers honour it.

```rust
use armature_push::{Notification, Priority};

let notification = Notification::new("New Message", "You have a new message!")
    .icon("https://example.com/icon.png")
    .badge(1)
    .data("message_id", "12345")
    .priority(Priority::High);

assert_eq!(notification.badge, Some(1));
```

Use `Notification::data_only()` for a payload with no user-visible alert.

## Web Push

Web Push subscriptions can be sent either as a `Subscription` (endpoint + keys)
or as a pipe-separated token in the form `endpoint|p256dh|auth`:

```rust,no_run
# async fn example(service: armature_push::PushService) -> Result<(), Box<dyn std::error::Error>> {
use armature_push::{Notification, Subscription};

let subscription = Subscription::new(
    "https://push.example.com/...",
    "p256dh-key",
    "auth-secret",
);

let notification = Notification::new("Hello!", "You have a new message");
service.send_to_subscription(&subscription, notification).await?;
# Ok(())
# }
```

## FCM

FCM uses a Google service account (OAuth2), not a legacy server key. Building
the provider performs a token exchange over the network, so it fails if
Google's token endpoint is unreachable.

```rust,no_run
# #[cfg(feature = "fcm")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_push::{FcmConfig, Notification, PushService};

let config = FcmConfig::from_service_account("service-account.json")?;
let service = PushService::fcm(config).await?;

service
    .send("fcm-device-token", Notification::new("Hi", "there"))
    .await?;
# Ok(())
# }
```

## APNS

```rust,no_run
# #[cfg(feature = "apns")]
# async fn example(private_key_pem: &str) -> Result<(), Box<dyn std::error::Error>> {
use armature_push::{ApnsConfig, Notification, PushService};

let config = ApnsConfig::new("team-id", "key-id", private_key_pem, "com.example.app")
    // Defaults to production; `.development()` targets the sandbox.
    .development();
let service = PushService::apns(config).await?;

service
    .send("apns-device-token", Notification::new("Hi", "there"))
    .await?;
# Ok(())
# }
```

A per-notification `Notification::topic(...)` overrides the configured bundle
ID for that send.

## License

MIT OR Apache-2.0
