//! Push notification message types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Notification priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Normal priority.
    #[default]
    Normal,
    /// High priority (may wake device).
    High,
}

/// Web Push urgency level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    /// Very low urgency (device may delay significantly).
    VeryLow,
    /// Low urgency.
    Low,
    /// Normal urgency.
    #[default]
    Normal,
    /// High urgency (deliver immediately).
    High,
}

/// Push notification content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    /// Notification title.
    pub title: String,
    /// Notification body.
    pub body: String,
    /// Icon URL.
    ///
    /// Honored by **FCM** (`android.notification.icon`) and **Web Push**.
    /// APNS has no icon field — iOS uses the app icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Image URL for a rich notification.
    ///
    /// Honored by **FCM** (`notification.image`), **APNS** (delivered as an
    /// attachment URL alongside `mutable-content`, which the app's
    /// Notification Service Extension reads), and **Web Push**.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Badge count.
    ///
    /// Honored by **APNS** (`aps.badge`) and **FCM**
    /// (`android.notification.notification_count`). Ignored by Web Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<u32>,
    /// Sound to play.
    ///
    /// Honored by **APNS** (`aps.sound`) and **FCM**
    /// (`android.notification.sound`). Ignored by Web Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    /// Click action URL or intent.
    ///
    /// Honored by **FCM** (`android.notification.click_action`) and **APNS**
    /// (mapped to `aps.category`). Ignored by Web Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_action: Option<String>,
    /// Tag for notification grouping.
    ///
    /// Honored by **FCM** (`android.notification.tag`) and **APNS** (mapped to
    /// `aps.thread-id`). Ignored by Web Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Custom data payload.
    ///
    /// Honored by all three providers. On **APNS** these become top-level
    /// members of the payload, so the key `aps` is reserved — using it returns
    /// [`crate::PushError::Config`] rather than emitting a duplicate member.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub data: HashMap<String, String>,
    /// Notification priority.
    ///
    /// Honored by **FCM** (`android.priority`) and **APNS**
    /// (`apns-priority` header). Web Push uses `urgency` instead.
    #[serde(default)]
    pub priority: Priority,
    /// Web Push urgency (RFC 8030 `Urgency` header).
    ///
    /// **Web Push only.** FCM and APNS use `priority`.
    #[serde(default)]
    pub urgency: Urgency,
    /// Time to live in seconds.
    ///
    /// Honored by all three: **FCM** `android.ttl`, **APNS**
    /// `apns-expiration` header, **Web Push** `TTL` header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u32>,
    /// Topic override.
    ///
    /// **APNS only** — overrides the configured bundle ID in the `apns-topic`
    /// header. Ignored by FCM and Web Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Collapse key.
    ///
    /// Honored by **FCM** (`android.collapse_key`) and **APNS**
    /// (`apns-collapse-id` header). Ignored by Web Push.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collapse_key: Option<String>,
    /// Silent notification (data only).
    ///
    /// Honored by all three: suppresses the visible alert block and, on APNS,
    /// switches the `apns-push-type` header to `background`.
    #[serde(default)]
    pub silent: bool,
    /// Mutable content, allowing a Notification Service Extension to run.
    ///
    /// **APNS only** (`aps.mutable-content`). Set implicitly when `image` is
    /// present, since the extension cannot run without it.
    #[serde(default)]
    pub mutable_content: bool,
    /// Content available (background wake).
    ///
    /// **APNS only** (`aps.content-available`).
    #[serde(default)]
    pub content_available: bool,
    /// Action buttons.
    ///
    /// **Web Push only** — serialized into the payload the service worker
    /// receives. FCM and APNS express actions through, respectively, a
    /// click action and a registered notification category, so neither reads
    /// this field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NotificationAction>,
}

impl Notification {
    /// Create a new notification.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            icon: None,
            image: None,
            badge: None,
            sound: None,
            click_action: None,
            tag: None,
            data: HashMap::new(),
            priority: Priority::Normal,
            urgency: Urgency::Normal,
            ttl: None,
            topic: None,
            collapse_key: None,
            silent: false,
            mutable_content: false,
            content_available: false,
            actions: Vec::new(),
        }
    }

    /// Create a builder.
    #[deprecated(
        since = "0.2.0",
        note = "use `Notification::new(title, body)` and its chaining methods, which cover every \
                field; `NotificationBuilder` only covers six"
    )]
    #[allow(deprecated)]
    pub fn builder() -> NotificationBuilder {
        NotificationBuilder::new()
    }

    /// Create a silent/data-only notification.
    pub fn data_only() -> Self {
        Self {
            title: String::new(),
            body: String::new(),
            silent: true,
            content_available: true,
            ..Default::default()
        }
    }

    /// Set the icon URL.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Set the image URL.
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = Some(image.into());
        self
    }

    /// Set the badge count.
    pub fn badge(mut self, count: u32) -> Self {
        self.badge = Some(count);
        self
    }

    /// Set the sound.
    pub fn sound(mut self, sound: impl Into<String>) -> Self {
        self.sound = Some(sound.into());
        self
    }

    /// Set the click action.
    pub fn click_action(mut self, action: impl Into<String>) -> Self {
        self.click_action = Some(action.into());
        self
    }

    /// Set the tag for grouping.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Add custom data.
    pub fn data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }

    /// Set priority.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Set high priority.
    pub fn high_priority(mut self) -> Self {
        self.priority = Priority::High;
        self.urgency = Urgency::High;
        self
    }

    /// Set urgency (Web Push).
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    /// Set time to live.
    pub fn ttl(mut self, seconds: u32) -> Self {
        self.ttl = Some(seconds);
        self
    }

    /// Set topic (iOS).
    pub fn topic(mut self, topic: impl Into<String>) -> Self {
        self.topic = Some(topic.into());
        self
    }

    /// Set collapse key (Android).
    pub fn collapse_key(mut self, key: impl Into<String>) -> Self {
        self.collapse_key = Some(key.into());
        self
    }

    /// Make this a silent notification.
    pub fn silent(mut self) -> Self {
        self.silent = true;
        self.content_available = true;
        self
    }

    /// Enable mutable content (iOS).
    pub fn mutable_content(mut self) -> Self {
        self.mutable_content = true;
        self
    }

    /// Add an action button.
    pub fn action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Serialized JSON size in bytes — **not** the encrypted wire size.
    ///
    /// This is the length of this notification's plaintext JSON. It is *not*
    /// a valid pre-flight check against the Web Push 4096-byte limit, which
    /// applies to the AES128GCM-encrypted body: ECE adds an 86-byte header,
    /// padding, and a 16-byte authentication tag on top of this figure. The
    /// Web Push send path performs its own check against the real encrypted
    /// length and returns [`crate::PushError::PayloadTooLarge`] locally.
    pub fn payload_size(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }
}

impl Default for Notification {
    fn default() -> Self {
        Self::new("", "")
    }
}

/// Notification action button.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    /// Action identifier.
    pub action: String,
    /// Button title.
    pub title: String,
    /// Icon URL (Web Push).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl NotificationAction {
    /// Create a new action.
    pub fn new(action: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            title: title.into(),
            icon: None,
        }
    }

    /// Set the icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Builder for notifications.
///
/// # Deprecated
///
/// This is a strictly weaker duplicate of the chaining methods on
/// [`Notification`] itself: it exposes 6 of the ~17 fields, omitting `sound`,
/// `click_action`, `tag`, `urgency`, `ttl`, `topic`, `collapse_key`, `silent`,
/// `mutable_content`, `actions` and `content_available`. Use
/// `Notification::new(title, body).icon(..).badge(..)` instead, which reads
/// the same and can express every field.
#[deprecated(
    since = "0.2.0",
    note = "use `Notification::new(title, body)` and its chaining methods, which cover every \
            field; `NotificationBuilder` only covers six"
)]
#[derive(Default)]
pub struct NotificationBuilder {
    notification: Notification,
}

#[allow(deprecated)]
impl NotificationBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.notification.title = title.into();
        self
    }

    /// Set the body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.notification.body = body.into();
        self
    }

    /// Set the icon.
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.notification.icon = Some(icon.into());
        self
    }

    /// Set the image.
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.notification.image = Some(image.into());
        self
    }

    /// Set the badge.
    pub fn badge(mut self, badge: u32) -> Self {
        self.notification.badge = Some(badge);
        self
    }

    /// Add data.
    pub fn data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.notification.data.insert(key.into(), value.into());
        self
    }

    /// Set priority.
    pub fn priority(mut self, priority: Priority) -> Self {
        self.notification.priority = priority;
        self
    }

    /// Build the notification.
    pub fn build(self) -> Notification {
        self.notification
    }
}
