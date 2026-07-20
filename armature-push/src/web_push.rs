//! Web Push (VAPID) provider.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use tracing::debug;
use url::Host;
use web_push::{
    ContentEncoding, PartialVapidSignatureBuilder, SubscriptionInfo, VapidSignatureBuilder,
    WebPushMessageBuilder,
};

use crate::error::map_status;
use crate::host_guard::{is_internal_v4, is_internal_v6, is_loopback_host};
use crate::{Notification, Platform, PushError, PushProvider, Result, Subscription, Urgency};

/// Maximum encrypted Web Push payload size (bytes) accepted by push services.
pub(crate) const WEB_PUSH_MAX_PAYLOAD: usize = 4096;

/// Maximum *plaintext* payload size (bytes) that can be encrypted into a body
/// fitting [`WEB_PUSH_MAX_PAYLOAD`].
///
/// AES128GCM (ECE) adds an 86-byte header, padding, and a 16-byte
/// authentication tag, so the plaintext ceiling sits well below the 4096-byte
/// wire limit. This mirrors the bound `web-push`'s encryptor enforces
/// internally; checking it here lets us return a structured
/// `PayloadTooLarge` instead of the opaque build error.
pub(crate) const WEB_PUSH_MAX_PLAINTEXT: usize = 3052;

/// Default overall request timeout for a single push (connect + send +
/// response). Overridable per config via [`WebPushConfig::timeout`].
const WEB_PUSH_TIMEOUT: Duration = Duration::from_secs(30);

/// Default connect-phase timeout for a single push. Overridable per config via
/// [`WebPushConfig::connect_timeout`].
const WEB_PUSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Map `Notification::urgency` onto the Web Push `Urgency` request header
/// value (RFC 8030 section 5.3). Without this header the push service treats
/// every message as `normal`, so `Urgency::High` ("deliver immediately")
/// previously had no delivery effect even though it was documented and
/// serialized into the payload.
fn urgency_header_value(urgency: Urgency) -> &'static str {
    match urgency {
        Urgency::VeryLow => "very-low",
        Urgency::Low => "low",
        Urgency::Normal => "normal",
        Urgency::High => "high",
    }
}

/// SSRF guard for the client-supplied subscription endpoint: enforce https and
/// refuse IP-literal internal targets and `.internal`/`.local` hosts before we
/// open a connection. This is a cheap literal/suffix check, not DNS resolution —
/// enough to block the obvious pivots a malicious push subscription could aim at.
///
/// Loopback is exempt **only** when `allow_insecure_loopback` is set. That flag
/// defaults to `false`, which matters: the exemption's stated justification is
/// local stub tests, but it previously applied unconditionally, so in a
/// production binary an untrusted subscription endpoint of
/// `http://127.0.0.1:6379/...` passed validation and let a push subscription
/// drive requests at any loopback-bound service on the app host.
fn validate_endpoint(endpoint: &str, allow_insecure_loopback: bool) -> Result<()> {
    let url = url::Url::parse(endpoint)
        .map_err(|e| PushError::Config(format!("invalid web push endpoint URL: {e}")))?;
    let host_str = url.host_str().unwrap_or("");

    // Is this host loopback in any spelling — the `localhost` domain, an IPv4
    // or IPv6 literal, or an IPv4-mapped IPv6 literal?
    let is_loopback = is_loopback_host(host_str)
        || match url.host() {
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => {
                ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
            }
            _ => false,
        };

    if is_loopback {
        // Loopback is only reachable behind the explicit opt-in. Without it,
        // an untrusted endpoint could aim requests at services bound to
        // localhost on the app host.
        return if allow_insecure_loopback {
            Ok(())
        } else {
            Err(PushError::Config(
                "web push endpoint targets a loopback address (set \
                 WebPushConfig::allow_insecure_loopback to permit this in tests)"
                    .to_string(),
            ))
        };
    }

    // Error text deliberately omits the endpoint itself: it is
    // attacker-influenced and echoing it invites log/error injection.
    if url.scheme() != "https" {
        return Err(PushError::Config(
            "web push endpoint must use https".to_string(),
        ));
    }

    match url.host() {
        Some(Host::Ipv4(ip)) if is_internal_v4(&ip) => {
            return Err(PushError::Config(
                "web push endpoint targets an internal IPv4 address".to_string(),
            ));
        }
        Some(Host::Ipv6(ip)) if is_internal_v6(&ip) => {
            return Err(PushError::Config(
                "web push endpoint targets an internal IPv6 address".to_string(),
            ));
        }
        _ => {}
    }

    let host_lower = host_str.to_ascii_lowercase();
    if host_lower.ends_with(".internal") || host_lower.ends_with(".local") {
        return Err(PushError::Config(
            "web push endpoint targets an internal host".to_string(),
        ));
    }

    Ok(())
}

/// Web Push configuration.
#[non_exhaustive]
#[derive(Clone)]
pub struct WebPushConfig {
    /// VAPID private key (base64 URL-safe encoded).
    pub private_key: String,
    /// Subject (mailto: or https: URL).
    pub subject: String,
    /// Default TTL in seconds.
    pub default_ttl: u32,
    /// Permit subscription endpoints that resolve to loopback over plain http.
    ///
    /// Defaults to `false`. Subscription endpoints are attacker-influenced, so
    /// enabling this in production would let a crafted subscription drive
    /// requests at services bound to localhost on the app host.
    pub allow_insecure_loopback: bool,
    /// Overall request timeout for a single push: connect, send and response.
    ///
    /// Defaults to 30 seconds. Raise it behind a slow proxy; lower it for a
    /// latency-sensitive caller that would rather fail fast and retry (a
    /// timeout surfaces as [`crate::PushError::Timeout`], which is retryable).
    pub timeout: Duration,
    /// Connect-phase timeout for a single push.
    ///
    /// Defaults to 10 seconds. Bounded separately from [`Self::timeout`] so an
    /// unreachable endpoint fails quickly rather than consuming the whole
    /// request budget.
    pub connect_timeout: Duration,
}

/// Hand-written so the VAPID private key is never rendered.
impl fmt::Debug for WebPushConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebPushConfig")
            .field("private_key", &"<redacted>")
            .field("subject", &self.subject)
            .field("default_ttl", &self.default_ttl)
            .field("allow_insecure_loopback", &self.allow_insecure_loopback)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

impl WebPushConfig {
    /// Create a new Web Push configuration.
    ///
    /// Only the private key is needed: VAPID signing derives the matching
    /// public key from it, so there is no separate public-key input to
    /// configure here (an earlier revision stored one, but nothing in the
    /// signing path ever read it).
    pub fn new(private_key: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            private_key: private_key.into(),
            subject: subject.into(),
            default_ttl: 86400, // 24 hours
            allow_insecure_loopback: false,
            timeout: WEB_PUSH_TIMEOUT,
            connect_timeout: WEB_PUSH_CONNECT_TIMEOUT,
        }
    }

    /// Set the default TTL.
    pub fn ttl(mut self, ttl: u32) -> Self {
        self.default_ttl = ttl;
        self
    }

    /// Set the overall request timeout (default: 30 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the connect-phase timeout (default: 10 seconds).
    pub fn connect_timeout(mut self, connect_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout;
        self
    }

    /// Permit subscription endpoints on loopback over plain http.
    ///
    /// Off by default. Intended for local stub servers in tests.
    pub fn allow_insecure_loopback(mut self, allow: bool) -> Self {
        self.allow_insecure_loopback = allow;
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
        // subscription), so the client refuses redirects and always carries
        // connect/overall timeouts — configurable, but never absent. Host/scheme
        // validation happens per send.
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
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
        let payload = serde_json::to_string(notification)?;
        self.send_prepared(subscription, notification, &payload)
            .await
    }

    /// Send to a web push subscription using an already-serialized payload.
    ///
    /// The ECE encryption genuinely is per-subscription (it is keyed by the
    /// subscription's p256dh/auth), but the *plaintext* JSON is not — it is
    /// identical for every recipient. Batch sends serialize it once and pass
    /// it in here rather than re-running `serde_json::to_string` per device.
    async fn send_prepared(
        &self,
        subscription: &WebPushSubscription,
        notification: &Notification,
        payload: &str,
    ) -> Result<()> {
        // SSRF hardening: reject non-https / internal-target endpoints before we
        // sign anything or open a connection.
        validate_endpoint(&subscription.endpoint, self.config.allow_insecure_loopback)?;

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

        // Pre-flight the plaintext against the ceiling that can still encrypt
        // into a body within WEB_PUSH_MAX_PAYLOAD. Without this the oversize
        // case surfaces as an opaque build failure, giving callers no
        // structured way to detect it. `size` and `limit` are both plaintext
        // bytes here; the post-build check below reports encrypted bytes.
        if payload.len() > WEB_PUSH_MAX_PLAINTEXT {
            return Err(PushError::PayloadTooLarge {
                size: payload.len(),
                limit: WEB_PUSH_MAX_PLAINTEXT,
            });
        }

        // Build message
        let mut builder = WebPushMessageBuilder::new(&subscription_info);
        builder.set_vapid_signature(signature);
        builder.set_payload(ContentEncoding::Aes128Gcm, payload.as_bytes());
        builder.set_ttl(notification.ttl.unwrap_or(self.config.default_ttl));

        let message = builder
            .build()
            .map_err(|e| PushError::provider(None, e.to_string()))?;

        // Pre-flight size check against the *encrypted* body, which is what the
        // 4096-byte limit actually applies to (AES128GCM adds an 86-byte header,
        // padding and a 16-byte tag on top of the plaintext JSON). Catching it
        // here saves a round-trip that is guaranteed to come back 413.
        if let Some(payload) = message.payload.as_ref()
            && payload.content.len() > WEB_PUSH_MAX_PAYLOAD
        {
            return Err(PushError::PayloadTooLarge {
                size: payload.content.len(),
                limit: WEB_PUSH_MAX_PAYLOAD,
            });
        }

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
            .header("TTL", message.ttl.to_string())
            .header("Urgency", urgency_header_value(notification.urgency));

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

        // Classify the transport failure *here* rather than letting `?` run
        // `From<reqwest::Error>`.
        //
        // Two requirements have to hold at once. First, timeouts must stay
        // `PushError::Timeout` and connect failures `PushError::Network`, both
        // of which are `is_retryable()`: collapsing them into `Provider` (as
        // an earlier revision did) made a Web Push timeout report
        // `is_retryable() == false` while an identical FCM/APNS timeout
        // reported `true`, so callers' retry loops gave up on exactly the
        // transient failures they were written for.
        //
        // Second — and this is what `?` got wrong — the attacker-supplied
        // endpoint URL must not reach the error text. `From<reqwest::Error>`
        // formats the error with `to_string()`, and `reqwest::Error`'s
        // `Display` appends ` for url (…)`, so a DNS or connect failure
        // produced `Network("error sending request for url
        // (https://attacker.example/push?tok=…)")` and put it straight into
        // caller logs. That contradicted the invariant asserted a few lines
        // below, where `map_status` is deliberately handed a fixed string for
        // the same reason. Every message here is a constant.
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                PushError::Timeout
            } else if e.is_connect() {
                PushError::Network("web push endpoint unreachable".to_string())
            } else {
                PushError::provider(None, "web push request failed")
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            // RFC 8030 §7.3: a push service answers 404 when the subscription
            // has expired, so here — as on FCM, and unlike on APNs, where 404
            // is `BadPath` — it really does mean the recipient is gone. It is
            // mapped at this call site rather than in `error::map_status`
            // because the shared mapper deliberately refuses to guess what 404
            // means; see that function's documentation.
            //
            // The fixed subject string is load-bearing: this provider's
            // "token" is an attacker-supplied endpoint URL that must not be
            // echoed into error text or logs.
            if status.as_u16() == 404 {
                return Err(PushError::Unregistered(
                    "web push subscription is no longer valid".to_string(),
                ));
            }

            // The mapper reads Retry-After from the headers. We deliberately do
            // NOT read or propagate the upstream response body into the
            // returned error: it is attacker-influenced content and echoing it
            // risks log/error injection. For the same reason the "subject"
            // passed for device-removal errors is a fixed description rather
            // than the endpoint URL that serves as this provider's token.
            return Err(map_status(
                "web push endpoint",
                status,
                response.headers(),
                "web push subscription is no longer valid",
                payload_size,
                WEB_PUSH_MAX_PAYLOAD,
            ));
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

    /// Overrides the default fan-out so the recipient-invariant plaintext JSON
    /// is serialized once for the whole batch. (The ECE encryption below it is
    /// genuinely per-subscription and still runs per token.) Concurrency and
    /// result ordering match the trait default.
    async fn send_batch(&self, tokens: &[String], notification: &Notification) -> Vec<Result<()>> {
        let payload = match serde_json::to_string(notification) {
            Ok(payload) => payload,
            Err(e) => {
                let message = e.to_string();
                return tokens
                    .iter()
                    .map(|_| Err(PushError::Serialization(message.clone())))
                    .collect();
            }
        };

        let subscriptions: Vec<Result<WebPushSubscription>> = tokens
            .iter()
            .map(|token| {
                let parts: Vec<&str> = token.split('|').collect();
                if parts.len() != 3 {
                    Err(PushError::InvalidSubscription(
                        "Invalid web push token format. Expected: endpoint|p256dh|auth".to_string(),
                    ))
                } else {
                    Ok(WebPushSubscription::new(parts[0], parts[1], parts[2]))
                }
            })
            .collect();

        let (payload, subscriptions) = (payload.as_str(), &subscriptions);
        futures_util::stream::iter(0..subscriptions.len())
            .map(|i| async move {
                match &subscriptions[i] {
                    Ok(sub) => self.send_prepared(sub, notification, payload).await,
                    Err(PushError::InvalidSubscription(m)) => {
                        Err(PushError::InvalidSubscription(m.clone()))
                    }
                    Err(other) => Err(PushError::provider(None, other.to_string())),
                }
            })
            .buffered(crate::provider::BATCH_CONCURRENCY)
            .collect()
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
        // Fully-qualified on purpose. `self.send_to_subscription(..)` resolves
        // to the inherent method of the same name, which is correct but reads
        // as self-recursive — and if that inherent method were ever renamed or
        // removed, the call would silently rebind to *this* trait method,
        // compiling cleanly and hanging at runtime.
        WebPushProvider::send_to_subscription(self, subscription, notification).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_vapid_private_key() {
        let config = WebPushConfig::new("SUPERSECRETVAPIDKEY", "mailto:admin@example.com");
        let rendered = format!("{config:?}");

        assert!(
            !rendered.contains("SUPERSECRETVAPIDKEY"),
            "WebPushConfig Debug leaked the VAPID private key: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "got {rendered}");
        // Non-secret fields must survive so the output stays useful.
        assert!(
            rendered.contains("mailto:admin@example.com"),
            "got {rendered}"
        );
    }

    #[test]
    fn timeouts_default_to_the_documented_values() {
        // Making these configurable must not change behavior for anyone who
        // does not set them.
        let config = WebPushConfig::new("key", "mailto:admin@example.com");
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
    }

    #[test]
    fn timeout_builders_override_the_defaults() {
        // A valid VAPID key, so the provider actually builds and we prove the
        // configured timeouts are accepted by the reqwest client builder.
        const VAPID_PRIVATE: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";

        let config = WebPushConfig::new(VAPID_PRIVATE, "mailto:admin@example.com")
            .timeout(Duration::from_millis(250))
            .connect_timeout(Duration::from_millis(50));
        assert_eq!(config.timeout, Duration::from_millis(250));
        assert_eq!(config.connect_timeout, Duration::from_millis(50));

        assert!(WebPushProvider::new(config).is_ok());
    }

    #[test]
    fn debug_renders_the_configured_timeouts() {
        let config = WebPushConfig::new("key", "mailto:admin@example.com")
            .timeout(Duration::from_millis(250));
        let rendered = format!("{config:?}");
        assert!(rendered.contains("250ms"), "got {rendered}");
    }

    #[test]
    fn allow_insecure_loopback_defaults_to_false() {
        let config = WebPushConfig::new("key", "mailto:admin@example.com");
        assert!(
            !config.allow_insecure_loopback,
            "the loopback SSRF exemption must be opt-in"
        );
    }

    #[test]
    fn validate_endpoint_accepts_public_https() {
        assert!(validate_endpoint("https://fcm.googleapis.com/fcm/send/abc", false).is_ok());
        assert!(
            validate_endpoint(
                "https://updates.push.services.mozilla.com/wpush/v2/x",
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_endpoint_rejects_internal_hostname_suffixes() {
        for endpoint in [
            "https://metadata.internal/latest",
            "https://printer.local/",
            "https://METADATA.INTERNAL/latest",
        ] {
            assert!(
                validate_endpoint(endpoint, false).is_err(),
                "{endpoint} should be rejected"
            );
        }
    }

    #[test]
    fn validate_endpoint_error_never_echoes_the_endpoint() {
        // The endpoint is attacker-influenced; echoing it into error text
        // invites log/error injection.
        let endpoint = "https://169.254.169.254/latest/meta-data?leak=SECRETMARKER";
        let err = validate_endpoint(endpoint, false).expect_err("internal target");
        assert!(
            !err.to_string().contains("SECRETMARKER"),
            "validation error echoed the endpoint: {err}"
        );
    }

    #[test]
    fn loopback_requires_opt_in_in_every_spelling() {
        for endpoint in [
            "http://localhost:8080/ep",
            "http://127.0.0.1:8080/ep",
            "https://127.0.0.1:8080/ep",
            "http://[::1]:8080/ep",
            "http://[::ffff:127.0.0.1]:8080/ep",
        ] {
            assert!(
                validate_endpoint(endpoint, false).is_err(),
                "{endpoint} must be refused without the opt-in"
            );
            assert!(
                validate_endpoint(endpoint, true).is_ok(),
                "{endpoint} should be permitted with the opt-in"
            );
        }
    }

    #[test]
    fn urgency_header_values_match_rfc_8030() {
        assert_eq!(urgency_header_value(Urgency::VeryLow), "very-low");
        assert_eq!(urgency_header_value(Urgency::Low), "low");
        assert_eq!(urgency_header_value(Urgency::Normal), "normal");
        assert_eq!(urgency_header_value(Urgency::High), "high");
    }
}
