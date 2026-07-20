#![doc = include_str!("../README.md")]
//!
//! # Armature Push
//!
//! Push notifications for web and mobile applications.
//!
//! ## Features
//!
//! - **Web Push**: VAPID-based web push notifications
//! - **FCM**: Firebase Cloud Messaging for Android
//! - **APNS**: Apple Push Notification Service for iOS
//! - **Unified API**: Send to multiple providers with one call
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! # #[cfg(feature = "web-push")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use armature_push::{Notification, PushService, WebPushConfig};
//!
//! // Configure Web Push. `WebPushConfig` is `#[non_exhaustive]`, so build it
//! // with `new` plus the builder methods rather than a struct literal.
//! let config = WebPushConfig::new("your-vapid-private-key", "mailto:admin@example.com");
//!
//! let service = PushService::web_push(config)?;
//!
//! // Send a notification. Note the argument order: token first.
//! let notification = Notification::new("Hello!", "This is a push notification");
//!
//! service
//!     .send("endpoint|p256dh|auth", notification)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## With Firebase Cloud Messaging
//!
//! Note that `PushService::fcm` performs an OAuth2 token exchange over the
//! network, so it fails if Google's token endpoint is unreachable.
//!
//! ```rust,no_run
//! # #[cfg(feature = "fcm")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use armature_push::{FcmConfig, Notification, PushService};
//!
//! let config = FcmConfig::from_service_account("path/to/service-account.json")?;
//! let service = PushService::fcm(config).await?;
//!
//! let notification = Notification::new("New Message", "You have a new message!")
//!     .data("message_id", "12345")
//!     .badge(1);
//!
//! service.send("device-token", notification).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Routing to the right provider
//!
//! With more than one provider configured, prefer
//! [`PushService::send_to_device`] with a platform-tagged [`DeviceToken`]:
//! it dispatches on the platform, so a token only ever reaches the provider
//! that issued it.
//!
//! ```rust,no_run
//! # async fn example(service: armature_push::PushService) -> Result<(), Box<dyn std::error::Error>> {
//! use armature_push::{DeviceToken, Notification};
//!
//! let notification = Notification::new("Hello!", "Routed by platform");
//!
//! service
//!     .send_to_device(&DeviceToken::ios("apns-token"), notification.clone())
//!     .await?;
//! service
//!     .send_to_device(&DeviceToken::android("fcm-token"), notification.clone())
//!     .await?;
//! service
//!     .send_to_device(&DeviceToken::web("endpoint|p256dh|auth"), notification)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! [`PushService::send`] has no platform information, so it tries every
//! provider in turn — see its documentation for when that is appropriate.

mod error;
mod notification;
mod provider;
mod subscription;

#[cfg(any(feature = "web-push", feature = "fcm", feature = "apns"))]
mod host_guard;

#[cfg(feature = "web-push")]
mod web_push;

#[cfg(feature = "fcm")]
mod fcm;

#[cfg(feature = "apns")]
mod apns;

pub use error::{PushError, Result};
#[allow(deprecated)]
pub use notification::NotificationBuilder;
pub use notification::{Notification, NotificationAction, Priority, Urgency};
pub use provider::{PushProvider, PushService};
pub use subscription::{DeviceToken, Platform, Subscription};

#[cfg(feature = "web-push")]
pub use web_push::{WebPushConfig, WebPushProvider, WebPushSubscription};

#[cfg(feature = "fcm")]
pub use fcm::{FcmConfig, FcmCredentials, FcmProvider};

#[cfg(feature = "apns")]
pub use apns::{ApnsConfig, ApnsEnvironment, ApnsProvider};

/// Prelude for common imports.
///
/// ```
/// use armature_push::prelude::*;
/// ```
pub mod prelude {
    pub use crate::error::{PushError, Result};
    // `NotificationBuilder` is deliberately absent: it is deprecated in favour
    // of `Notification::new(..)`'s chaining methods, which cover every field.
    pub use crate::notification::{Notification, NotificationAction, Priority, Urgency};
    pub use crate::provider::{PushProvider, PushService};
    pub use crate::subscription::{DeviceToken, Platform, Subscription};

    #[cfg(feature = "web-push")]
    pub use crate::web_push::{WebPushConfig, WebPushProvider, WebPushSubscription};

    #[cfg(feature = "fcm")]
    pub use crate::fcm::{FcmConfig, FcmCredentials, FcmProvider};

    #[cfg(feature = "apns")]
    pub use crate::apns::{ApnsConfig, ApnsEnvironment, ApnsProvider};
}
