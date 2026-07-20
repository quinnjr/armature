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
//! ```rust,ignore
//! use armature_push::{PushService, WebPushConfig, Notification};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure Web Push
//!     let config = WebPushConfig::new(
//!         "your-vapid-private-key",
//!         "mailto:admin@example.com",
//!     );
//!
//!     let service = PushService::web_push(config)?;
//!
//!     // Send a notification
//!     let notification = Notification::new("Hello!", "This is a push notification");
//!
//!     service.send("subscription-endpoint", notification).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## With Firebase Cloud Messaging
//!
//! ```rust,ignore
//! use armature_push::{PushService, FcmConfig, Notification};
//!
//! let config = FcmConfig::from_service_account("path/to/service-account.json")?;
//! let service = PushService::fcm(config).await?;
//!
//! let notification = Notification::new("New Message", "You have a new message!")
//!     .data("message_id", "12345")
//!     .badge(1);
//!
//! service.send("device-token", notification).await?;
//! ```

mod error;
mod notification;
mod provider;
mod subscription;

#[cfg(feature = "web-push")]
mod web_push;

#[cfg(feature = "fcm")]
mod fcm;

#[cfg(feature = "apns")]
mod apns;

pub use error::{PushError, Result};
pub use notification::{Notification, NotificationBuilder, Priority, Urgency};
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
    pub use crate::notification::{Notification, NotificationBuilder, Priority, Urgency};
    pub use crate::provider::{PushProvider, PushService};
    pub use crate::subscription::{DeviceToken, Platform, Subscription};

    #[cfg(feature = "web-push")]
    pub use crate::web_push::{WebPushConfig, WebPushProvider, WebPushSubscription};

    #[cfg(feature = "fcm")]
    pub use crate::fcm::{FcmConfig, FcmCredentials, FcmProvider};

    #[cfg(feature = "apns")]
    pub use crate::apns::{ApnsConfig, ApnsEnvironment, ApnsProvider};
}
