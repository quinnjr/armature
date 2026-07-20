//! Apple Push Notification Service (APNS) provider.

use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;
use std::fmt;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::error::map_status;
use crate::{Notification, Platform, Priority, PushError, PushProvider, Result};

/// Maximum APNS payload size in bytes for a regular notification.
///
/// VoIP pushes are allowed 5120; this crate does not send VoIP pushes, so the
/// 4096 alert/background limit is the one that applies.
pub(crate) const APNS_MAX_PAYLOAD: usize = 4096;

/// The reserved top-level key in an APNS payload.
const APNS_RESERVED_KEY: &str = "aps";

/// Default overall request timeout for a single APNS call (connect + send +
/// response). Overridable per config via [`ApnsConfig::timeout`].
const APNS_TIMEOUT: Duration = Duration::from_secs(30);

/// Default connect-phase timeout for a single APNS call. Overridable per
/// config via [`ApnsConfig::connect_timeout`].
const APNS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// APNS environment.
///
/// `#[non_exhaustive]`: the `Custom` variant makes this genuinely open-ended,
/// and marking it here keeps future variants from breaking downstream
/// exhaustive `match`es. Note this type is not `Copy` — `Custom(String)` owns
/// a heap allocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ApnsEnvironment {
    /// Development/sandbox environment.
    Development,
    /// Production environment.
    #[default]
    Production,
    /// A custom base URL, e.g. an in-process stub server for tests.
    Custom(String),
}

impl ApnsEnvironment {
    fn endpoint(&self) -> &str {
        match self {
            Self::Development => "https://api.sandbox.push.apple.com",
            Self::Production => "https://api.push.apple.com",
            Self::Custom(url) => url,
        }
    }
}

/// APNS configuration.
#[non_exhaustive]
#[derive(Clone)]
pub struct ApnsConfig {
    /// Team ID.
    pub team_id: String,
    /// Key ID.
    pub key_id: String,
    /// Private key (P8 format).
    pub private_key: String,
    /// Bundle ID (topic).
    pub bundle_id: String,
    /// Environment.
    pub environment: ApnsEnvironment,
    /// Permit a plain-http [`ApnsEnvironment::Custom`] endpoint on a loopback
    /// host.
    ///
    /// Defaults to `false`. `Custom` is a test seam that accepts any URL; this
    /// flag is what keeps it from being usable to send a JWT-bearing request
    /// in the clear from a production binary.
    pub allow_insecure_loopback: bool,
    /// Overall request timeout for a single APNS call: connect, send and
    /// response.
    ///
    /// Defaults to 30 seconds. Raise it behind a slow proxy; lower it for a
    /// latency-sensitive caller that would rather fail fast and retry (a
    /// timeout surfaces as [`crate::PushError::Timeout`], which is retryable).
    pub timeout: Duration,
    /// Connect-phase timeout for a single APNS call.
    ///
    /// Defaults to 10 seconds. Bounded separately from [`Self::timeout`] so an
    /// unreachable endpoint fails quickly rather than consuming the whole
    /// request budget.
    pub connect_timeout: Duration,
}

/// Hand-written so the P8 private key is never rendered — a derived `Debug`
/// would dump the full EC key into any `{:?}` interpolation.
impl fmt::Debug for ApnsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApnsConfig")
            .field("team_id", &self.team_id)
            .field("key_id", &self.key_id)
            .field("private_key", &"<redacted>")
            .field("bundle_id", &self.bundle_id)
            .field("environment", &self.environment)
            .field("allow_insecure_loopback", &self.allow_insecure_loopback)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

impl ApnsConfig {
    /// Create a new APNS configuration.
    pub fn new(
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        private_key: impl Into<String>,
        bundle_id: impl Into<String>,
    ) -> Self {
        Self {
            team_id: team_id.into(),
            key_id: key_id.into(),
            private_key: private_key.into(),
            bundle_id: bundle_id.into(),
            environment: ApnsEnvironment::Production,
            allow_insecure_loopback: false,
            timeout: APNS_TIMEOUT,
            connect_timeout: APNS_CONNECT_TIMEOUT,
        }
    }

    /// Load private key from a P8 file.
    pub fn from_p8_file(
        team_id: impl Into<String>,
        key_id: impl Into<String>,
        p8_path: impl AsRef<std::path::Path>,
        bundle_id: impl Into<String>,
    ) -> Result<Self> {
        let private_key = std::fs::read_to_string(p8_path)?;
        Ok(Self::new(team_id, key_id, private_key, bundle_id))
    }

    /// Set the environment.
    pub fn environment(mut self, env: ApnsEnvironment) -> Self {
        self.environment = env;
        self
    }

    /// Use development environment.
    pub fn development(self) -> Self {
        self.environment(ApnsEnvironment::Development)
    }

    /// Use production environment.
    pub fn production(self) -> Self {
        self.environment(ApnsEnvironment::Production)
    }

    /// Permit a plain-http [`ApnsEnvironment::Custom`] endpoint on loopback.
    ///
    /// Off by default. Intended for local stub servers in tests.
    pub fn allow_insecure_loopback(mut self, allow: bool) -> Self {
        self.allow_insecure_loopback = allow;
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
}

/// Reject a JWT-bearing endpoint that is neither https nor an opted-in
/// loopback http URL.
fn validate_endpoint(raw: &str, allow_insecure_loopback: bool) -> Result<()> {
    let url = url::Url::parse(raw)
        .map_err(|e| PushError::Config(format!("invalid APNS endpoint URL: {e}")))?;

    if url.scheme() == "https" {
        return Ok(());
    }

    let host = url.host_str().unwrap_or("");
    if url.scheme() == "http"
        && crate::host_guard::is_loopback_host(host)
        && allow_insecure_loopback
    {
        return Ok(());
    }

    Err(PushError::Config(
        "APNS endpoint must use https (set ApnsConfig::allow_insecure_loopback to permit plain \
         http against a loopback host in tests)"
            .to_string(),
    ))
}

/// APNS provider.
pub struct ApnsProvider {
    config: ApnsConfig,
    client: Client,
    access_token: RwLock<Option<AccessToken>>,
    /// Serializes JWT re-signing so a concurrent batch that crosses the expiry
    /// boundary performs one ES256 signature instead of one per in-flight send.
    refresh_lock: std::sync::Mutex<()>,
}

struct AccessToken {
    token: String,
    expires_at: Instant,
}

impl ApnsProvider {
    /// Create a new APNS provider.
    pub async fn new(config: ApnsConfig) -> Result<Self> {
        // Apple's real APNS endpoints are HTTPS and negotiate HTTP/2 via ALPN,
        // so we don't need (and previously shouldn't have forced) HTTP/2
        // prior knowledge, which assumes a cleartext h2 preface the peer
        // understands without any negotiation — that broke interop with
        // plain HTTP/1.1 test servers and offered no benefit against the
        // real, TLS-only APNS hosts.
        validate_endpoint(
            config.environment.endpoint(),
            config.allow_insecure_loopback,
        )?;

        // A connect timeout is new here: previously only an overall timeout was
        // set, so a TCP connect that hung (a black-holed route rather than a
        // refused connection) consumed the entire 30s budget before failing.
        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|e| PushError::Config(e.to_string()))?;

        Ok(Self {
            config,
            client,
            access_token: RwLock::new(None),
            refresh_lock: std::sync::Mutex::new(()),
        })
    }

    /// Read the cached JWT if it is still valid.
    fn cached_token(&self) -> Option<String> {
        let token = self.access_token.read().unwrap();
        token
            .as_ref()
            .and_then(|t| (t.expires_at > Instant::now()).then(|| t.token.clone()))
    }

    /// Get or create the JWT token.
    fn get_token(&self) -> Result<String> {
        if let Some(token) = self.cached_token() {
            return Ok(token);
        }

        // Signing is CPU-only, so a plain std Mutex is right here (no await is
        // held across it). One waiter signs; the rest re-check and reuse.
        let _guard = self.refresh_lock.lock().unwrap();

        if let Some(token) = self.cached_token() {
            return Ok(token);
        }

        self.create_token()
    }

    /// Create a new JWT token.
    fn create_token(&self) -> Result<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        #[derive(Serialize)]
        struct Claims {
            iss: String,
            iat: i64,
        }

        let claims = Claims {
            iss: self.config.team_id.clone(),
            iat: now,
        };

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.config.key_id.clone());

        let key = EncodingKey::from_ec_pem(self.config.private_key.as_bytes())
            .map_err(|e| PushError::Config(format!("Invalid private key: {}", e)))?;

        let token = encode(&header, &claims, &key)
            .map_err(|e| PushError::Config(format!("JWT encoding failed: {}", e)))?;

        // Cache the token (valid for ~50 minutes)
        let access_token = AccessToken {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(3000),
        };

        *self.access_token.write().unwrap() = Some(access_token);

        Ok(token)
    }

    /// Build APNS payload.
    ///
    /// Returns `Err(PushError::Config)` if custom data would collide with the
    /// reserved `aps` key: `ApnsPayload` flattens `data` alongside `aps`, so a
    /// `data("aps", ..)` entry would serialize *two* `aps` members into one
    /// JSON object. RFC 8259 leaves duplicate names undefined, and APNS would
    /// either reject the request or take the last value — discarding the real
    /// `aps` dictionary (alert, badge, sound) entirely. Failing loudly beats
    /// silently dropping the notification body.
    fn build_payload(&self, notification: &Notification) -> Result<ApnsPayload> {
        if notification.data.contains_key(APNS_RESERVED_KEY) {
            warn!(
                "notification data contains the reserved APNS key '{APNS_RESERVED_KEY}'; refusing \
                 to build a payload with a duplicate member"
            );
            return Err(PushError::Config(format!(
                "notification data may not use the reserved APNS key '{APNS_RESERVED_KEY}'"
            )));
        }

        let mut aps = ApnsAps {
            alert: None,
            badge: notification.badge,
            sound: notification.sound.clone(),
            content_available: if notification.content_available {
                Some(1)
            } else {
                None
            },
            mutable_content: if notification.mutable_content {
                Some(1)
            } else {
                None
            },
            category: notification.click_action.clone(),
            thread_id: notification.tag.clone(),
        };

        // Add alert if not silent
        if !notification.silent {
            aps.alert = Some(ApnsAlert {
                title: Some(notification.title.clone()),
                body: Some(notification.body.clone()),
                subtitle: None,
            });
        }

        // A rich-media image is deliverable on iOS: the Notification Service
        // Extension reads a custom key and attaches the URL, which requires
        // `mutable-content: 1` to run at all. Set that implicitly when an
        // image is present so the extension is actually invoked.
        if notification.image.is_some() {
            aps.mutable_content = Some(1);
        }

        let mut payload = ApnsPayload {
            aps,
            image_url: notification.image.clone(),
            custom: std::collections::HashMap::new(),
        };

        // Add custom data
        for (key, value) in &notification.data {
            payload
                .custom
                .insert(key.clone(), serde_json::Value::String(value.clone()));
        }

        Ok(payload)
    }
}

#[async_trait]
impl PushProvider for ApnsProvider {
    async fn send(&self, token: &str, notification: &Notification) -> Result<()> {
        let jwt = self.get_token()?;
        let payload = self.build_payload(notification)?;

        let url = format!("{}/3/device/{}", self.config.environment.endpoint(), token);

        debug!(token = %token, "Sending APNS notification");

        let mut request = self
            .client
            .post(&url)
            .header("authorization", format!("bearer {}", jwt))
            .header(
                "apns-topic",
                notification
                    .topic
                    .as_deref()
                    .unwrap_or(&self.config.bundle_id),
            )
            .header(
                "apns-push-type",
                if notification.silent {
                    "background"
                } else {
                    "alert"
                },
            );

        // Set priority
        request = request.header(
            "apns-priority",
            match notification.priority {
                Priority::High => "10",
                Priority::Normal => "5",
            },
        );

        // Set TTL (expiration)
        if let Some(ttl) = notification.ttl {
            let expiration = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + ttl as u64;
            request = request.header("apns-expiration", expiration.to_string());
        }

        // Set collapse ID
        if let Some(collapse_key) = &notification.collapse_key {
            request = request.header("apns-collapse-id", collapse_key);
        }

        // Serialize up front so a 413 (APNS enforces a 4 KB limit) can report
        // the real body size instead of collapsing into an opaque `Provider`.
        let body_bytes = serde_json::to_vec(&payload)?;
        let payload_size = body_bytes.len();

        let response = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_bytes)
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            debug!("APNS notification sent successfully");
            return Ok(());
        }

        // APNS carries a machine-readable `reason` in the body for 4xx. Two
        // cases need it before falling through to the shared mapper.
        if status.as_u16() == 400 || status.as_u16() == 410 {
            let headers = response.headers().clone();
            let body = response.text().await.unwrap_or_default();

            if status.as_u16() == 410 || body.contains("ExpiredToken") {
                // Apple's documented reason for 410 is `ExpiredToken`; treat
                // the explicit reason as the more specific error and let a
                // bare 410 stay `Unregistered`. Both remove the device.
                return Err(if body.contains("ExpiredToken") {
                    PushError::TokenExpired(token.to_string())
                } else {
                    PushError::Unregistered(token.to_string())
                });
            }

            if body.contains("BadDeviceToken") || body.contains("DeviceTokenNotForTopic") {
                return Err(PushError::InvalidSubscription(token.to_string()));
            }

            return Err(map_status(
                "APNS",
                status,
                &headers,
                token,
                payload_size,
                APNS_MAX_PAYLOAD,
            ));
        }

        Err(map_status(
            "APNS",
            status,
            response.headers(),
            token,
            payload_size,
            APNS_MAX_PAYLOAD,
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Ios
    }
}

// APNS payload types

#[derive(Serialize)]
struct ApnsPayload {
    aps: ApnsAps,
    /// Rich-notification attachment URL, read by the app's Notification
    /// Service Extension. Not part of the reserved `aps` dictionary.
    #[serde(rename = "image-url", skip_serializing_if = "Option::is_none")]
    image_url: Option<String>,
    #[serde(flatten)]
    custom: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct ApnsAps {
    #[serde(skip_serializing_if = "Option::is_none")]
    alert: Option<ApnsAlert>,
    #[serde(skip_serializing_if = "Option::is_none")]
    badge: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<String>,
    #[serde(rename = "content-available", skip_serializing_if = "Option::is_none")]
    content_available: Option<u8>,
    #[serde(rename = "mutable-content", skip_serializing_if = "Option::is_none")]
    mutable_content: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(rename = "thread-id", skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
}

#[derive(Serialize)]
struct ApnsAlert {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NotificationAction, Priority as NPriority, Urgency};

    const FAKE_KEY: &str =
        "-----BEGIN PRIVATE KEY-----\nSUPERSECRETKEYMATERIAL\n-----END PRIVATE KEY-----";

    fn config() -> ApnsConfig {
        ApnsConfig::new("TEAM123456", "KEY7890", FAKE_KEY, "com.example.app")
    }

    fn provider() -> ApnsProvider {
        // `build_payload` needs no network or valid key, so build the struct
        // directly rather than going through the validating `new`.
        ApnsProvider {
            config: config(),
            client: Client::new(),
            access_token: RwLock::new(None),
            refresh_lock: std::sync::Mutex::new(()),
        }
    }

    #[test]
    fn timeouts_default_to_the_documented_values() {
        // Making these configurable must not change the overall-timeout
        // behavior for anyone who does not set them.
        let config = config();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
    }

    #[tokio::test]
    async fn timeout_builders_override_the_defaults() {
        let config = config()
            .timeout(Duration::from_millis(250))
            .connect_timeout(Duration::from_millis(50));
        assert_eq!(config.timeout, Duration::from_millis(250));
        assert_eq!(config.connect_timeout, Duration::from_millis(50));

        // And the provider constructor accepts them.
        assert!(ApnsProvider::new(config).await.is_ok());
    }

    #[test]
    fn debug_renders_the_configured_timeouts() {
        let rendered = format!("{:?}", config().timeout(Duration::from_millis(250)));
        assert!(rendered.contains("250ms"), "got {rendered}");
    }

    #[test]
    fn debug_does_not_leak_private_key() {
        let rendered = format!("{:?}", config());
        assert!(
            !rendered.contains("SUPERSECRETKEYMATERIAL"),
            "ApnsConfig Debug leaked the private key: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "got {rendered}");
        // Non-secret fields must survive.
        assert!(rendered.contains("TEAM123456"), "got {rendered}");
        assert!(rendered.contains("KEY7890"), "got {rendered}");
        assert!(rendered.contains("com.example.app"), "got {rendered}");
    }

    /// Golden payload: nothing else in the suite asserts an APNS request
    /// *body*, only headers and status mapping.
    #[test]
    fn build_payload_golden() {
        let notification = Notification::new("Title", "Body")
            .icon("https://example.com/icon.png")
            .image("https://example.com/image.png")
            .badge(7)
            .sound("chime.caf")
            .click_action("OPEN_ACTIVITY")
            .tag("group-a")
            .data("order_id", "12345")
            .priority(NPriority::High)
            .urgency(Urgency::High)
            .ttl(600)
            .topic("com.example.other")
            .collapse_key("collapse-me")
            .action(NotificationAction::new("view", "View"));

        let payload = provider().build_payload(&notification).unwrap();
        let json = serde_json::to_value(payload).unwrap();

        let expected = serde_json::json!({
            "aps": {
                "alert": { "title": "Title", "body": "Body" },
                "badge": 7,
                "sound": "chime.caf",
                // Implied by the presence of an image: the Notification
                // Service Extension cannot run without it.
                "mutable-content": 1,
                "category": "OPEN_ACTIVITY",
                "thread-id": "group-a"
            },
            "image-url": "https://example.com/image.png",
            "order_id": "12345"
        });

        assert_eq!(
            json, expected,
            "APNS payload drifted from the golden fixture"
        );
    }

    #[test]
    fn image_sets_mutable_content_and_attachment_url() {
        let notification = Notification::new("Hi", "there").image("https://example.com/x.png");
        let json = serde_json::to_value(provider().build_payload(&notification).unwrap()).unwrap();

        assert_eq!(
            json["image-url"],
            serde_json::json!("https://example.com/x.png"),
            "image must reach the APNS payload: {json}"
        );
        assert_eq!(
            json["aps"]["mutable-content"],
            serde_json::json!(1),
            "an image requires mutable-content for the service extension: {json}"
        );
    }

    #[test]
    fn reserved_aps_data_key_is_rejected() {
        let notification = Notification::new("Hi", "there").data("aps", "hijacked");
        match provider().build_payload(&notification) {
            Err(PushError::Config(_)) => {}
            Err(other) => panic!("expected Config, got {other:?}"),
            Ok(_) => panic!("a data key of 'aps' must be refused, not silently duplicated"),
        }
    }

    #[test]
    fn payload_has_exactly_one_aps_member() {
        let notification = Notification::new("Hi", "there")
            .badge(1)
            .data("other", "value");
        let serialized =
            serde_json::to_string(&provider().build_payload(&notification).unwrap()).unwrap();

        // Count raw `"aps":` occurrences in the serialized text: a duplicate
        // member survives serialization even though serde_json::Value would
        // silently collapse it on re-parse.
        assert_eq!(
            serialized.matches("\"aps\":").count(),
            1,
            "payload must contain exactly one aps member: {serialized}"
        );
    }

    #[test]
    fn silent_notification_omits_alert_and_sets_content_available() {
        let notification = Notification::data_only().data("k", "v");
        let json = serde_json::to_value(provider().build_payload(&notification).unwrap()).unwrap();

        assert!(json["aps"].get("alert").is_none(), "got {json}");
        assert_eq!(json["aps"]["content-available"], serde_json::json!(1));
        assert_eq!(json["k"], serde_json::json!("v"));
    }

    #[test]
    fn custom_endpoint_must_be_https_or_opted_in_loopback() {
        assert!(
            validate_endpoint("http://push.example.com", false).is_err(),
            "plain http to a public host must be refused"
        );
        assert!(
            validate_endpoint("http://127.0.0.1:8080", false).is_err(),
            "loopback http must be refused unless opted in"
        );
        assert!(validate_endpoint("http://127.0.0.1:8080", true).is_ok());
        assert!(
            validate_endpoint("http://169.254.169.254", true).is_err(),
            "opt-in must not permit http to a non-loopback host"
        );
        assert!(validate_endpoint("https://api.push.apple.com", false).is_ok());
    }

    #[tokio::test]
    async fn provider_new_rejects_insecure_custom_environment() {
        let config = config().environment(ApnsEnvironment::Custom("http://evil.example".into()));
        match ApnsProvider::new(config).await {
            Err(PushError::Config(_)) => {}
            Err(other) => panic!("expected Config, got {other:?}"),
            Ok(_) => panic!("insecure custom endpoint must be refused at construction"),
        }
    }
}
