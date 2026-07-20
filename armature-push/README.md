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

```rust,ignore
use armature_push::{PushService, WebPushConfig, Notification};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = WebPushConfig::new(
        "your-vapid-private-key",
        "mailto:admin@example.com",
    );
    let service = PushService::web_push(config)?;

    let notification = Notification::new("Hello!", "You have a new message")
        .data("message_id", "123");

    // Note the argument order: token first, notification second.
    service.send("endpoint|p256dh|auth", notification).await?;
    Ok(())
}
```

## Sending to multiple providers

`PushService::web_push`, `PushService::fcm`, and `PushService::apns` each build a
service around a single provider. To handle more than one platform, add
providers to a shared service with `add_provider`:

```rust,ignore
use armature_push::{
    PushService, WebPushConfig, WebPushProvider, FcmConfig, FcmProvider,
    ApnsConfig, ApnsProvider, Notification,
};

let web_push = WebPushProvider::new(WebPushConfig::new(
    "your-vapid-private-key",
    "mailto:admin@example.com",
))?;
let fcm = FcmProvider::new(FcmConfig::from_service_account("service-account.json")?).await?;
let apns = ApnsProvider::new(ApnsConfig::new("team-id", "key-id", "-----BEGIN EC PRIVATE KEY-----...", "com.example.app")).await?;

let service = PushService::new()
    .add_provider(web_push)
    .add_provider(fcm)
    .add_provider(apns);

service.send("device-token", Notification::new("Hi", "there")).await?;
```

## Web Push

Web Push subscriptions can be sent either as a `Subscription` (endpoint + keys)
or as a pipe-separated token in the form `endpoint|p256dh|auth`:

```rust,ignore
use armature_push::{Subscription, Notification};

let subscription = Subscription::new(
    "https://push.example.com/...",
    "p256dh-key",
    "auth-secret",
);

let notification = Notification::new("Hello!", "You have a new message");
service.send_to_subscription(&subscription, notification).await?;
```

## FCM

FCM uses a Google service account (OAuth2), not a legacy server key:

```rust,ignore
use armature_push::FcmConfig;

let config = FcmConfig::from_service_account("service-account.json")?;
let service = PushService::fcm(config).await?;

service.send("fcm-device-token", Notification::new("Hi", "there")).await?;
```

## APNS

```rust,ignore
use armature_push::ApnsConfig;

let config = ApnsConfig::new("team-id", "key-id", private_key_pem, "com.example.app");
let service = PushService::apns(config).await?;

service.send("apns-device-token", Notification::new("Hi", "there")).await?;
```

A per-notification `Notification::topic(...)` overrides the configured bundle
ID for that send.

## License

MIT OR Apache-2.0
