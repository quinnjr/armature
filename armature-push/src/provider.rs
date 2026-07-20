//! Push notification provider trait and service.

use async_trait::async_trait;
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use tracing::debug;

use crate::{DeviceToken, Notification, Platform, PushError, Result, Subscription};

/// Maximum number of in-flight requests when fanning a batch send out
/// concurrently. Bounded so a large token slice can't open an unbounded
/// number of simultaneous connections to the provider.
const BATCH_CONCURRENCY: usize = 32;

/// Push provider trait.
#[async_trait]
pub trait PushProvider: Send + Sync {
    /// Send a notification to a device token.
    async fn send(&self, token: &str, notification: &Notification) -> Result<()>;

    /// Send to multiple tokens.
    ///
    /// Requests are issued with bounded concurrency (`BATCH_CONCURRENCY` in
    /// flight at a time) so independent HTTP round-trips overlap instead of
    /// serializing one after another, while still returning one `Result` per
    /// input token in the original order.
    async fn send_batch(&self, tokens: &[String], notification: &Notification) -> Vec<Result<()>> {
        // Map over indices rather than `&String` items directly: mapping
        // over borrowed items ties the closure to a for-any-lifetime bound
        // that `self.send`'s (async-trait-generated) borrowed future can't
        // satisfy. Indices are `Copy` and lifetime-free, so the closure just
        // captures `self`/`tokens`/`notification` once at a single lifetime.
        stream::iter(0..tokens.len())
            .map(|i| self.send(&tokens[i], notification))
            .buffered(BATCH_CONCURRENCY)
            .collect()
            .await
    }

    /// Get the platform this provider handles.
    fn platform(&self) -> Platform;

    /// Send to a subscription (for Web Push).
    async fn send_to_subscription(
        &self,
        subscription: &Subscription,
        notification: &Notification,
    ) -> Result<()> {
        // Default implementation uses the endpoint as token
        self.send(&subscription.endpoint, notification).await
    }
}

/// Unified push service supporting multiple providers.
pub struct PushService {
    providers: Vec<Arc<dyn PushProvider>>,
}

impl PushService {
    /// Create a new push service.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Add a provider.
    pub fn add_provider(mut self, provider: impl PushProvider + 'static) -> Self {
        self.providers.push(Arc::new(provider));
        self
    }

    /// Create with a single Web Push provider.
    #[cfg(feature = "web-push")]
    pub fn web_push(config: crate::WebPushConfig) -> Result<Self> {
        let provider = crate::WebPushProvider::new(config)?;
        Ok(Self::new().add_provider(provider))
    }

    /// Create with a single FCM provider.
    #[cfg(feature = "fcm")]
    pub async fn fcm(config: crate::FcmConfig) -> Result<Self> {
        let provider = crate::FcmProvider::new(config).await?;
        Ok(Self::new().add_provider(provider))
    }

    /// Create with a single APNS provider.
    #[cfg(feature = "apns")]
    pub async fn apns(config: crate::ApnsConfig) -> Result<Self> {
        let provider = crate::ApnsProvider::new(config).await?;
        Ok(Self::new().add_provider(provider))
    }

    /// Send a notification to a device token.
    pub async fn send(&self, token: &str, notification: Notification) -> Result<()> {
        // Try each provider until one succeeds
        let mut last_error = None;

        for provider in &self.providers {
            match provider.send(token, &notification).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    debug!(
                        platform = ?provider.platform(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| PushError::Config("No push providers configured".to_string())))
    }

    /// Send to a device token with platform hint.
    pub async fn send_to_device(
        &self,
        device: &DeviceToken,
        notification: Notification,
    ) -> Result<()> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.platform() == device.platform)
            .ok_or_else(|| {
                PushError::Config(format!("No provider for platform {:?}", device.platform))
            })?;

        provider.send(&device.token, &notification).await
    }

    /// Send to a web push subscription.
    pub async fn send_to_subscription(
        &self,
        subscription: &Subscription,
        notification: Notification,
    ) -> Result<()> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.platform() == Platform::Web)
            .ok_or_else(|| PushError::Config("No Web Push provider configured".to_string()))?;

        provider
            .send_to_subscription(subscription, &notification)
            .await
    }

    /// Send to multiple devices.
    ///
    /// Requests are issued with bounded concurrency (see `BATCH_CONCURRENCY`)
    /// so N devices cost roughly one round-trip instead of N serial ones,
    /// while results are still returned in the original device order.
    pub async fn send_batch(
        &self,
        devices: &[DeviceToken],
        notification: Notification,
    ) -> Vec<Result<()>> {
        stream::iter(0..devices.len())
            .map(|i| self.send_to_device(&devices[i], notification.clone()))
            .buffered(BATCH_CONCURRENCY)
            .collect()
            .await
    }

    /// Send to multiple tokens with a single platform.
    ///
    /// Delegates to the platform provider's `send_batch`, which fans requests
    /// out with bounded concurrency rather than sending them one at a time.
    pub async fn send_to_tokens(
        &self,
        platform: Platform,
        tokens: &[String],
        notification: Notification,
    ) -> Vec<Result<()>> {
        let provider = match self.providers.iter().find(|p| p.platform() == platform) {
            Some(p) => p,
            None => {
                return tokens
                    .iter()
                    .map(|_| {
                        Err(PushError::Config(format!(
                            "No provider for platform {:?}",
                            platform
                        )))
                    })
                    .collect();
            }
        };

        provider.send_batch(tokens, &notification).await
    }
}

impl Default for PushService {
    fn default() -> Self {
        Self::new()
    }
}
