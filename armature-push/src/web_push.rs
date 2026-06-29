//! Web Push (VAPID) provider.

use async_trait::async_trait;
use tracing::debug;
use web_push::{
    ContentEncoding, SubscriptionInfo, VapidSignatureBuilder, WebPushClient, WebPushMessageBuilder,
};

use crate::{Notification, Platform, PushError, PushProvider, Result, Subscription};

/// Web Push configuration.
#[derive(Debug, Clone)]
pub struct WebPushConfig {
    /// VAPID private key (base64 URL-safe encoded).
    pub private_key: String,
    /// VAPID public key (base64 URL-safe encoded).
    pub public_key: Option<String>,
    /// Subject (mailto: or https: URL).
    pub subject: String,
    /// Default TTL in seconds.
    pub default_ttl: u32,
}

impl WebPushConfig {
    /// Create a new Web Push configuration.
    pub fn new(private_key: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            private_key: private_key.into(),
            public_key: None,
            subject: subject.into(),
            default_ttl: 86400, // 24 hours
        }
    }

    /// Set the public key.
    pub fn public_key(mut self, key: impl Into<String>) -> Self {
        self.public_key = Some(key.into());
        self
    }

    /// Set the default TTL.
    pub fn ttl(mut self, ttl: u32) -> Self {
        self.default_ttl = ttl;
        self
    }
}

/// Web Push subscription (from browser).
#[derive(Debug, Clone)]
pub struct WebPushSubscription {
    /// Endpoint URL.
    pub endpoint: String,
    /// p256dh key.
    pub p256dh: String,
    /// Auth secret.
    pub auth: String,
}

impl WebPushSubscription {
    /// Create a new subscription.
    pub fn new(
        endpoint: impl Into<String>,
        p256dh: impl Into<String>,
        auth: impl Into<String>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            p256dh: p256dh.into(),
            auth: auth.into(),
        }
    }
}

impl From<&Subscription> for WebPushSubscription {
    fn from(sub: &Subscription) -> Self {
        Self {
            endpoint: sub.endpoint.clone(),
            p256dh: sub.keys.p256dh.clone(),
            auth: sub.keys.auth.clone(),
        }
    }
}

/// Web Push provider using VAPID.
pub struct WebPushProvider {
    config: WebPushConfig,
    client: web_push::IsahcWebPushClient,
}

impl WebPushProvider {
    /// Create a new Web Push provider.
    pub fn new(config: WebPushConfig) -> Result<Self> {
        let client =
            web_push::IsahcWebPushClient::new().map_err(|e| PushError::Config(e.to_string()))?;

        Ok(Self { config, client })
    }

    /// Send to a subscription.
    pub async fn send_to_subscription(
        &self,
        subscription: &Subscription,
        notification: &Notification,
    ) -> Result<()> {
        let sub = WebPushSubscription::from(subscription);
        self.send_to_web_subscription(&sub, notification).await
    }

    /// Send to a web push subscription.
    pub async fn send_to_web_subscription(
        &self,
        subscription: &WebPushSubscription,
        notification: &Notification,
    ) -> Result<()> {
        let subscription_info = SubscriptionInfo::new(
            &subscription.endpoint,
            &subscription.p256dh,
            &subscription.auth,
        );

        // Build VAPID signature
        let mut sig_builder =
            VapidSignatureBuilder::from_base64(&self.config.private_key, &subscription_info)
                .map_err(|e: web_push::WebPushError| PushError::Config(e.to_string()))?;

        sig_builder.add_claim(
            "sub",
            serde_json::Value::String(self.config.subject.clone()),
        );

        let signature = sig_builder
            .build()
            .map_err(|e: web_push::WebPushError| PushError::Config(e.to_string()))?;

        // Build payload
        let payload = serde_json::to_string(notification)?;

        // Build message
        let mut builder = WebPushMessageBuilder::new(&subscription_info);
        builder.set_vapid_signature(signature);
        builder.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
        builder.set_ttl(notification.ttl.unwrap_or(self.config.default_ttl));

        let message = builder
            .build()
            .map_err(|e| PushError::Provider(e.to_string()))?;

        debug!(endpoint = %subscription.endpoint, "Sending web push notification");

        // Send (WebPushClient trait is in scope)
        WebPushClient::send(&self.client, message)
            .await
            .map_err(PushError::from)?;

        debug!("Web push notification sent successfully");
        Ok(())
    }
}

#[async_trait]
impl PushProvider for WebPushProvider {
    async fn send(&self, token: &str, notification: &Notification) -> Result<()> {
        // Token format: endpoint|p256dh|auth (pipe-separated)
        let parts: Vec<&str> = token.split('|').collect();
        if parts.len() != 3 {
            return Err(PushError::InvalidSubscription(
                "Invalid web push token format. Expected: endpoint|p256dh|auth".to_string(),
            ));
        }

        let subscription = WebPushSubscription::new(parts[0], parts[1], parts[2]);
        self.send_to_web_subscription(&subscription, notification)
            .await
    }

    fn platform(&self) -> Platform {
        Platform::Web
    }

    async fn send_to_subscription(
        &self,
        subscription: &Subscription,
        notification: &Notification,
    ) -> Result<()> {
        self.send_to_subscription(subscription, notification).await
    }
}
