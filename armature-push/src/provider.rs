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

    /// Send a notification to a device token, trying every provider in turn.
    ///
    /// # This is best-effort fallthrough, not routing
    ///
    /// A bare token carries no platform information, so this method has
    /// nothing to dispatch on: it tries each provider in insertion order until
    /// one succeeds. With several providers configured that means an iOS token
    /// costs a wasted authenticated round-trip to FCM before it reaches APNS.
    ///
    /// **Prefer [`PushService::send_to_device`]** with a platform-tagged
    /// [`DeviceToken`] whenever you know the platform — which is essentially
    /// always, since the platform is recorded when the token is registered.
    /// This method is appropriate for a single-provider service.
    ///
    /// On total failure the returned error is chosen deliberately: the first
    /// error that does *not* claim the device should be removed wins. A
    /// removal claim from a provider the token was never issued by is
    /// meaningless — FCM failing transiently followed by APNS reporting
    /// `InvalidSubscription` for a token that was never an APNS token would
    /// otherwise hand the caller a `should_remove_device()` error and get a
    /// perfectly valid Android registration deleted.
    pub async fn send(&self, token: &str, notification: Notification) -> Result<()> {
        let mut errors: Vec<PushError> = Vec::new();

        for provider in &self.providers {
            match provider.send(token, &notification).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    debug!(
                        platform = ?provider.platform(),
                        error = %e,
                        "Provider failed, trying next"
                    );
                    errors.push(e);
                }
            }
        }

        Err(Self::select_error(errors))
    }

    /// Pick the error to surface when every provider failed.
    ///
    /// Prefers the first error that does not assert the device should be
    /// removed, so a spurious removal verdict from a mismatched provider
    /// cannot cause a caller to prune a valid token. If every provider
    /// asserted removal, that verdict is consistent and is passed through.
    fn select_error(errors: Vec<PushError>) -> PushError {
        if errors.is_empty() {
            return PushError::Config("No push providers configured".to_string());
        }

        let all_claim_removal = errors.iter().all(|e| e.should_remove_device());
        if all_claim_removal {
            return errors.into_iter().next().expect("checked non-empty");
        }

        errors
            .into_iter()
            .find(|e| !e.should_remove_device())
            .expect("not all errors claim removal")
    }

    /// Send to a device token with platform hint.
    ///
    /// This is the only method that routes by platform; prefer it over
    /// [`PushService::send`] whenever the platform is known.
    pub async fn send_to_device(
        &self,
        device: &DeviceToken,
        notification: Notification,
    ) -> Result<()> {
        self.send_to_device_ref(device, &notification).await
    }

    /// Borrowing form of [`PushService::send_to_device`], so batch fan-out does
    /// not clone the (recipient-invariant) notification once per device.
    async fn send_to_device_ref(
        &self,
        device: &DeviceToken,
        notification: &Notification,
    ) -> Result<()> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.platform() == device.platform)
            .ok_or_else(|| {
                PushError::Config(format!("No provider for platform {:?}", device.platform))
            })?;

        provider.send(&device.token, notification).await
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
    ///
    /// The notification does not vary per recipient, so it is borrowed by the
    /// fan-out rather than cloned once per device.
    pub async fn send_batch(
        &self,
        devices: &[DeviceToken],
        notification: Notification,
    ) -> Vec<Result<()>> {
        let notification = &notification;
        stream::iter(0..devices.len())
            .map(|i| self.send_to_device_ref(&devices[i], notification))
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
