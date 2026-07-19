//! Web Push (VAPID) provider.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use async_trait::async_trait;
use tracing::debug;
use url::Host;
use web_push::{
    ContentEncoding, PartialVapidSignatureBuilder, SubscriptionInfo, VapidSignatureBuilder,
    WebPushMessageBuilder,
};

use crate::{Notification, Platform, PushError, PushProvider, Result, Subscription};

/// Maximum encrypted Web Push payload size (bytes) accepted by push services.
pub(crate) const WEB_PUSH_MAX_PAYLOAD: usize = 4096;

/// Overall request timeout for a single push (connect + send + response).
const WEB_PUSH_TIMEOUT_SECS: u64 = 30;

/// Connect-phase timeout for a single push.
const WEB_PUSH_CONNECT_TIMEOUT_SECS: u64 = 10;

/// True for hosts that are exempt from the https + internal-target SSRF checks
/// so local stub/integration tests (which speak plain http over loopback) work.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

fn is_internal_v4(ip: &Ipv4Addr) -> bool {
    // Loopback is intentionally exempt (see `is_loopback_host`); block the rest of
    // the internal ranges: private (10/8, 172.16/12, 192.168/16), link-local
    // (169.254/16) and the unspecified address.
    ip.is_private() || ip.is_link_local() || ip.is_unspecified()
}

fn is_internal_v6(ip: &Ipv6Addr) -> bool {
    let seg0 = ip.segments()[0];
    // fe80::/10 link-local or fc00::/7 unique-local, plus the unspecified address.
    ip.is_unspecified() || (seg0 & 0xffc0) == 0xfe80 || (seg0 & 0xfe00) == 0xfc00
}

/// SSRF guard for the client-supplied subscription endpoint: enforce https and
/// refuse IP-literal internal targets and `.internal`/`.local` hosts before we
/// open a connection. This is a cheap literal/suffix check, not DNS resolution —
/// enough to block the obvious pivots a malicious push subscription could aim at.
/// Loopback is exempt so local http stub tests keep working.
fn validate_endpoint(endpoint: &str) -> Result<()> {
    let url = url::Url::parse(endpoint)
        .map_err(|e| PushError::Config(format!("invalid web push endpoint URL: {e}")))?;
    let host_str = url.host_str().unwrap_or("");
    let loopback = is_loopback_host(host_str);

    if url.scheme() != "https" && !loopback {
        return Err(PushError::Config(format!(
            "web push endpoint '{endpoint}' must use https (loopback hosts are exempt for local \
             testing)"
        )));
    }

    match url.host() {
        Some(Host::Ipv4(ip)) if !ip.is_loopback() && is_internal_v4(&ip) => {
            return Err(PushError::Config(format!(
                "web push endpoint '{endpoint}' targets an internal IPv4 address"
            )));
        }
        Some(Host::Ipv6(ip)) if !ip.is_loopback() && is_internal_v6(&ip) => {
            return Err(PushError::Config(format!(
                "web push endpoint '{endpoint}' targets an internal IPv6 address"
            )));
        }
        _ => {}
    }

    let host_lower = host_str.to_ascii_lowercase();
    if host_lower.ends_with(".internal") || host_lower.ends_with(".local") {
        return Err(PushError::Config(format!(
            "web push endpoint '{endpoint}' targets an internal host"
        )));
    }

    Ok(())
}

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
    client: reqwest::Client,
    /// VAPID signing key, decoded once at construction; per-send we clone this
    /// and attach the subscription info rather than re-decoding the base64 key
    /// and re-deriving the EC key on every notification.
    vapid: PartialVapidSignatureBuilder,
}

impl WebPushProvider {
    /// Create a new Web Push provider.
    pub fn new(config: WebPushConfig) -> Result<Self> {
        // The subscription endpoint is attacker-influenced (it comes from a push
        // subscription), so the client refuses redirects and carries hard
        // connect/overall timeouts. Host/scheme validation happens per send.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(WEB_PUSH_TIMEOUT_SECS))
            .connect_timeout(Duration::from_secs(WEB_PUSH_CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| PushError::Config(e.to_string()))?;

        let vapid = VapidSignatureBuilder::from_base64_no_sub(&config.private_key)
            .map_err(|e: web_push::WebPushError| PushError::Config(e.to_string()))?;

        Ok(Self {
            config,
            client,
            vapid,
        })
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
        // SSRF hardening: reject non-https / internal-target endpoints before we
        // sign anything or open a connection.
        validate_endpoint(&subscription.endpoint)?;

        let subscription_info = SubscriptionInfo::new(
            &subscription.endpoint,
            &subscription.p256dh,
            &subscription.auth,
        );

        // Build the VAPID signature from the pre-decoded key (cloned per send).
        let mut sig_builder = self.vapid.clone().add_sub_info(&subscription_info);

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

        // Send the built message over our own reqwest (rustls) client. This
        // hand-rolls the same request web-push 0.11's
        // `request_builder::build_request` produces (POST, TTL header,
        // Content-Encoding / Content-Type / crypto headers, encrypted body) so we
        // don't depend on its isahc/libcurl client. If you bump web-push, re-diff
        // this against that function to keep the headers in sync.
        let mut request = self
            .client
            .post(message.endpoint.to_string())
            .header("TTL", message.ttl.to_string());

        // Encrypted payload length, captured before `body(...)` moves the content,
        // so a 413 can report the real size instead of 0.
        let mut payload_size = 0usize;
        if let Some(payload) = message.payload {
            payload_size = payload.content.len();
            request = request
                .header(
                    reqwest::header::CONTENT_ENCODING,
                    payload.content_encoding.to_str(),
                )
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream");
            for (key, value) in payload.crypto_headers.into_iter() {
                request = request.header(key, value);
            }
            request = request.body(payload.content);
        }

        let response = request
            .send()
            .await
            .map_err(|e| PushError::Provider(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            // Read Retry-After from the headers. We deliberately do NOT read or
            // propagate the upstream response body into the returned error: it is
            // attacker-influenced content and echoing it risks log/error injection.
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            return Err(match status {
                // 404/410: the push subscription is gone — callers should prune it.
                reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE => {
                    PushError::Unregistered(format!("web push endpoint returned {status}"))
                }
                reqwest::StatusCode::TOO_MANY_REQUESTS => {
                    PushError::RateLimited(retry_after.unwrap_or(60))
                }
                reqwest::StatusCode::PAYLOAD_TOO_LARGE => PushError::PayloadTooLarge {
                    size: payload_size,
                    limit: WEB_PUSH_MAX_PAYLOAD,
                },
                _ => PushError::Provider(format!("web push endpoint returned {status}")),
            });
        }

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
