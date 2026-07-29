//! Firebase Cloud Messaging (FCM) provider.

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::RwLock;
use std::time::{Duration, Instant};
use tracing::debug;

use crate::error::map_status;
use crate::time::unix_now_secs;
use crate::{Notification, Platform, Priority, PushError, PushProvider, Result};

/// Maximum FCM HTTP v1 message size in bytes (4 MB).
pub(crate) const FCM_MAX_PAYLOAD: usize = 4 * 1024 * 1024;

/// Default overall request timeout for a single FCM call (connect + send +
/// response). Overridable per config via [`FcmConfig::timeout`].
const FCM_TIMEOUT: Duration = Duration::from_secs(30);

/// Default connect-phase timeout for a single FCM call. Overridable per config
/// via [`FcmConfig::connect_timeout`].
const FCM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// FCM configuration.
///
/// Marked `#[non_exhaustive]` so that adding further fields (this type has
/// already grown `api_base` and `allow_insecure_loopback` as test seams) is
/// not a breaking change for downstream exhaustive struct literals. Construct
/// via [`FcmConfig::new`], [`FcmConfig::from_service_account`], or
/// [`FcmConfig::from_service_account_json`] and refine with the builder
/// methods.
#[derive(Clone)]
#[non_exhaustive]
pub struct FcmConfig {
    /// Project ID.
    pub project_id: String,
    /// Service account credentials (JSON).
    pub credentials: FcmCredentials,
    /// Base URL for the FCM HTTP v1 API. Defaults to the real Google
    /// endpoint; overridable so tests can point it at a stub server.
    ///
    /// A non-https base is rejected at provider construction unless it is a
    /// loopback host *and* [`FcmConfig::allow_insecure_loopback`] is set.
    pub api_base: String,
    /// Permit a plain-http `api_base`/`token_uri` on a loopback host.
    ///
    /// Defaults to `false`. This exists so local stub servers can be used in
    /// tests; leaving it `false` in production means the credentials-bearing
    /// requests this provider makes can never be sent in the clear.
    pub allow_insecure_loopback: bool,
    /// Overall request timeout for a single FCM call: connect, send and
    /// response. Applies to both the OAuth2 token exchange and `messages:send`.
    ///
    /// Defaults to 30 seconds. Raise it behind a slow proxy; lower it for a
    /// latency-sensitive caller that would rather fail fast and retry (a
    /// timeout surfaces as [`crate::PushError::Timeout`], which is retryable).
    pub timeout: Duration,
    /// Connect-phase timeout for a single FCM call.
    ///
    /// Defaults to 10 seconds. Bounded separately from [`Self::timeout`] so an
    /// unreachable endpoint fails quickly rather than consuming the whole
    /// request budget.
    pub connect_timeout: Duration,
}

/// Redacts `credentials`, which carries the service-account private key.
impl fmt::Debug for FcmConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FcmConfig")
            .field("project_id", &self.project_id)
            .field("credentials", &self.credentials)
            .field("api_base", &self.api_base)
            .field("allow_insecure_loopback", &self.allow_insecure_loopback)
            .field("timeout", &self.timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

fn default_api_base() -> String {
    "https://fcm.googleapis.com".to_string()
}

/// FCM service account credentials.
#[derive(Clone, Deserialize)]
pub struct FcmCredentials {
    /// Client email.
    pub client_email: String,
    /// Private key (PEM format).
    pub private_key: String,
    /// Token URI.
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

/// Hand-written so the RSA private key is never rendered.
///
/// A derived `Debug` would put the full PEM into any `{:?}` interpolation —
/// a `tracing::debug!(config = ?config)`, an error message, a panic payload —
/// and this type is re-exported at the crate root and in the prelude, so it
/// travels far from here.
impl fmt::Debug for FcmCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FcmCredentials")
            .field("client_email", &self.client_email)
            .field("private_key", &"<redacted>")
            .field("token_uri", &self.token_uri)
            .finish()
    }
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

impl FcmConfig {
    /// Create config from a service account JSON file.
    pub fn from_service_account(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_service_account_json(&content)
    }

    /// Create config from service account JSON string.
    pub fn from_service_account_json(json: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct ServiceAccount {
            project_id: String,
            client_email: String,
            private_key: String,
            #[serde(default = "default_token_uri")]
            token_uri: String,
        }

        let sa: ServiceAccount =
            serde_json::from_str(json).map_err(|e| PushError::Config(e.to_string()))?;

        Ok(Self {
            project_id: sa.project_id,
            credentials: FcmCredentials {
                client_email: sa.client_email,
                private_key: sa.private_key,
                token_uri: sa.token_uri,
            },
            api_base: default_api_base(),
            allow_insecure_loopback: false,
            timeout: FCM_TIMEOUT,
            connect_timeout: FCM_CONNECT_TIMEOUT,
        })
    }

    /// Create with explicit credentials.
    pub fn new(
        project_id: impl Into<String>,
        client_email: impl Into<String>,
        private_key: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            credentials: FcmCredentials {
                client_email: client_email.into(),
                private_key: private_key.into(),
                token_uri: default_token_uri(),
            },
            api_base: default_api_base(),
            allow_insecure_loopback: false,
            timeout: FCM_TIMEOUT,
            connect_timeout: FCM_CONNECT_TIMEOUT,
        }
    }

    /// Override the FCM API base URL (default: the real Google endpoint).
    /// Intended for tests that need to point requests at a stub server.
    ///
    /// A non-https value is rejected by [`FcmProvider::new`] unless it names a
    /// loopback host and [`FcmConfig::allow_insecure_loopback`] is set.
    pub fn api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Override the OAuth2 token exchange URI (default: Google's).
    /// Intended for tests that need to point the token exchange at a stub.
    pub fn token_uri(mut self, token_uri: impl Into<String>) -> Self {
        self.credentials.token_uri = token_uri.into();
        self
    }

    /// Permit plain-http `api_base`/`token_uri` values on loopback hosts.
    ///
    /// Off by default. Intended for local stub servers in tests — enabling it
    /// in production would allow this provider's bearer-token-bearing requests
    /// to be sent unencrypted.
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

/// Reject a credential-bearing URL that is neither https nor an opted-in
/// loopback http URL.
///
/// This checks the *scheme* (plus, for loopback, the opt-in flag) only. It is
/// **not** an SSRF guard: it does not resolve the host, and does not vet
/// resolved addresses against internal/link-local ranges. `web_push`'s
/// `validate_endpoint` is the function that does that fuller, DNS-rebinding-
/// aware check; this one is named for exactly what it guarantees so the two
/// are not mistaken for each other.
///
/// The gap is acceptable here: unlike a Web Push subscription endpoint, which
/// is attacker-influenced input arriving on every send, the values checked
/// here are `FcmConfig::api_base` and `FcmConfig::token_uri` — set once by
/// the application developer, defaulting to Google's real endpoints.
/// Overriding them is a test seam for pointing at a local stub server, not a
/// per-notification input from an untrusted party.
fn require_https_or_loopback(label: &str, raw: &str, allow_insecure_loopback: bool) -> Result<()> {
    let url = url::Url::parse(raw)
        .map_err(|e| PushError::Config(format!("invalid FCM {label} URL: {e}")))?;

    if url.scheme() == "https" {
        return Ok(());
    }

    let host = url.host_str().unwrap_or("");
    let loopback = crate::host_guard::is_loopback_host(host);

    if url.scheme() == "http" && loopback && allow_insecure_loopback {
        return Ok(());
    }

    Err(PushError::Config(format!(
        "FCM {label} must use https (set FcmConfig::allow_insecure_loopback to permit plain http \
         against a loopback host in tests)"
    )))
}

/// FCM provider.
pub struct FcmProvider {
    config: FcmConfig,
    client: Client,
    access_token: RwLock<Option<AccessToken>>,
    /// Serializes access-token refreshes.
    ///
    /// Without this, every concurrent send that observed an expired token
    /// issued its own RS256 signature and its own POST to the OAuth2 token
    /// endpoint — up to `BATCH_CONCURRENCY` (32) redundant network token
    /// exchanges at each expiry boundary, with last-writer-wins on the cache.
    /// Waiters now queue here and re-check the cache after acquiring, so
    /// exactly one exchange happens per expiry.
    refresh_lock: tokio::sync::Mutex<()>,
}

struct AccessToken {
    token: String,
    expires_at: Instant,
}

/// Refresh once the cached token is within this window of expiring.
const TOKEN_REFRESH_SKEW: Duration = Duration::from_secs(60);

impl FcmProvider {
    /// Create a new FCM provider.
    ///
    /// **This performs a network round-trip.** The OAuth2 token exchange
    /// against `token_uri` runs eagerly here so that a bad service-account key
    /// surfaces at construction rather than at first send. As a consequence
    /// `FcmProvider::new` — and therefore [`crate::PushService::fcm`] — fails
    /// if the token endpoint is unreachable. Construct it where you can
    /// tolerate that, or retry.
    pub async fn new(config: FcmConfig) -> Result<Self> {
        require_https_or_loopback("api_base", &config.api_base, config.allow_insecure_loopback)?;
        require_https_or_loopback(
            "token_uri",
            &config.credentials.token_uri,
            config.allow_insecure_loopback,
        )?;

        // A connect timeout is new here: previously only an overall timeout was
        // set, so a TCP connect that hung (a black-holed route rather than a
        // refused connection) consumed the entire 30s budget before failing.
        let client = crate::host_guard::build_push_client(config.timeout, config.connect_timeout)?;

        let provider = Self {
            config,
            client,
            access_token: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        };

        // Get initial token
        provider.refresh_token().await?;

        Ok(provider)
    }

    /// Read the cached token if it is still comfortably valid.
    ///
    /// Poison is recovered rather than propagated: the guarded value is a
    /// plain `Option<AccessToken>` with no cross-field invariant, so a panic
    /// elsewhere cannot have left it half-updated. Unwrapping instead meant one
    /// panic anywhere in the process bricked the provider forever.
    fn cached_token(&self) -> Option<String> {
        let token = self.access_token.read().unwrap_or_else(|e| e.into_inner());
        token.as_ref().and_then(|t| {
            (t.expires_at > Instant::now() + TOKEN_REFRESH_SKEW).then(|| t.token.clone())
        })
    }

    /// Get or refresh the access token.
    async fn get_access_token(&self) -> Result<String> {
        if let Some(token) = self.cached_token() {
            return Ok(token);
        }

        // Only one task performs the refresh; the rest block here and then
        // find the freshly-cached token on the re-check below.
        let _guard = self.refresh_lock.lock().await;

        if let Some(token) = self.cached_token() {
            return Ok(token);
        }

        self.refresh_token().await
    }

    /// Refresh the access token.
    async fn refresh_token(&self) -> Result<String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

        // A clock before the UNIX epoch is a configuration problem (a VM
        // resumed from a snapshot, a container with an unsynced RTC), not a
        // reason to panic. `duration_since(UNIX_EPOCH).unwrap()` here unwound
        // straight out of `send()` and through the caller's task, while the
        // byte-identical APNS code returned a `PushError::Config`.
        let now = unix_now_secs()? as i64;

        #[derive(Serialize)]
        struct Claims {
            iss: String,
            scope: String,
            aud: String,
            iat: i64,
            exp: i64,
        }

        let claims = Claims {
            iss: self.config.credentials.client_email.clone(),
            scope: "https://www.googleapis.com/auth/firebase.messaging".to_string(),
            aud: self.config.credentials.token_uri.clone(),
            iat: now,
            exp: now + 3600,
        };

        let key = EncodingKey::from_rsa_pem(self.config.credentials.private_key.as_bytes())
            .map_err(|e| PushError::Config(format!("Invalid private key: {}", e)))?;

        let jwt = encode(&Header::new(Algorithm::RS256), &claims, &key)
            .map_err(|e| PushError::Config(format!("JWT encoding failed: {}", e)))?;

        // Exchange JWT for access token
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        let http_response = self
            .client
            .post(&self.config.credentials.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &jwt),
            ])
            .send()
            .await?;

        // A rejected service-account key (expired, revoked, wrong project)
        // comes back as a 4xx with a JSON error body. Deserializing that into
        // `TokenResponse` fails, which used to surface as an opaque
        // `Serialization` error; report it as the authentication failure it is.
        let status = http_response.status();
        if !status.is_success() {
            return Err(PushError::Auth(format!(
                "FCM OAuth2 token exchange failed with {status}"
            )));
        }

        let response: TokenResponse = http_response.json().await?;

        let token = AccessToken {
            token: response.access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(response.expires_in),
        };

        // Same rationale as `cached_token`: the guarded value has no invariant
        // a panicking holder could have broken, so poison is recovered rather
        // than turned into a permanently unusable provider.
        *self.access_token.write().unwrap_or_else(|e| e.into_inner()) = Some(token);

        Ok(response.access_token)
    }

    /// Build the token-invariant body of an FCM message.
    ///
    /// The device token is the *only* recipient-dependent field in an FCM v1
    /// message, yet this used to be rebuilt per token — cloning title, body,
    /// image and the whole data map once for every device in a batch.
    /// `send_batch` now builds it once and borrows it into each request.
    fn build_message_body(&self, notification: &Notification) -> FcmMessageBody {
        let mut message = FcmMessageBody {
            notification: None,
            data: None,
            android: None,
            apns: None,
            webpush: None,
        };

        // Add notification if not silent
        if !notification.silent {
            message.notification = Some(FcmNotification {
                title: Some(notification.title.clone()),
                body: Some(notification.body.clone()),
                image: notification.image.clone(),
            });
        }

        // Add data if present
        if !notification.data.is_empty() {
            message.data = Some(notification.data.clone());
        }

        // Android-specific config
        message.android = Some(FcmAndroidConfig {
            priority: match notification.priority {
                Priority::High => "high".to_string(),
                Priority::Normal => "normal".to_string(),
            },
            ttl: notification.ttl.map(|t| format!("{}s", t)),
            collapse_key: notification.collapse_key.clone(),
            notification: Some(FcmAndroidNotification {
                icon: notification.icon.clone(),
                color: None,
                sound: notification.sound.clone(),
                tag: notification.tag.clone(),
                click_action: notification.click_action.clone(),
                // FCM v1's AndroidNotification spells the badge
                // `notification_count`. This was previously unmapped, so
                // `Notification::badge` — set by this crate's own quick-start
                // doc example — was silently dropped on the FCM path.
                notification_count: notification.badge,
            }),
        });

        message
    }

    /// Send one already-built message body to a single token.
    ///
    /// Unlike APNS the token lives *inside* the FCM body, so the JSON is
    /// serialized per request — but from a borrowed, shared body rather than
    /// a freshly cloned one.
    async fn send_prepared(
        &self,
        token: &str,
        access_token: &str,
        body: &FcmMessageBody,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.config.api_base, self.config.project_id
        );

        debug!(token = %token, "Sending FCM notification");

        // Serialize up front so a 413 can report the real body size rather
        // than falling through to an opaque `Provider` error.
        let request_body = serde_json::to_vec(&FcmRequest {
            message: FcmMessage { token, body },
        })?;
        let payload_size = request_body.len();

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(request_body)
            .send()
            .await?;

        let status = response.status();

        if status.is_success() {
            debug!("FCM notification sent successfully");
            return Ok(());
        }

        // On FCM a 404 genuinely is `UNREGISTERED` — the registration token no
        // longer exists — so it belongs to the device-removal set. It is
        // mapped here rather than in `error::map_status` because 404 means
        // something entirely different on APNs (`BadPath`); see that
        // function's documentation.
        if status.as_u16() == 404 {
            return Err(PushError::Unregistered(token.to_string()));
        }

        Err(map_status(
            "FCM",
            status,
            response.headers(),
            token,
            payload_size,
            FCM_MAX_PAYLOAD,
        ))
    }
}

#[async_trait]
impl PushProvider for FcmProvider {
    async fn send(&self, token: &str, notification: &Notification) -> Result<()> {
        let access_token = self.get_access_token().await?;
        let body = self.build_message_body(notification);
        self.send_prepared(token, &access_token, &body).await
    }

    /// Overrides the default fan-out so the OAuth2 access token is fetched
    /// once and the recipient-invariant message body is built once for the
    /// whole batch. Concurrency and result ordering match the trait default.
    async fn send_batch(&self, tokens: &[String], notification: &Notification) -> Vec<Result<()>> {
        let access_token = match self.get_access_token().await {
            Ok(t) => t,
            Err(e) => {
                let message = e.to_string();
                return tokens
                    .iter()
                    .map(|_| Err(PushError::Auth(message.clone())))
                    .collect();
            }
        };

        let body = self.build_message_body(notification);
        let (access_token, body) = (&access_token, &body);

        futures_util::stream::iter(0..tokens.len())
            .map(|i| self.send_prepared(&tokens[i], access_token, body))
            .buffered(crate::provider::BATCH_CONCURRENCY)
            .collect()
            .await
    }

    fn platform(&self) -> Platform {
        Platform::Android
    }
}

// FCM API types

#[derive(Serialize)]
struct FcmRequest<'a> {
    message: FcmMessage<'a>,
}

/// The wire message: a borrowed token stamped onto a borrowed, shared body.
///
/// Split this way so a batch send serializes one `FcmMessageBody` N times
/// instead of *building* it N times.
#[derive(Serialize)]
struct FcmMessage<'a> {
    token: &'a str,
    #[serde(flatten)]
    body: &'a FcmMessageBody,
}

#[derive(Serialize)]
struct FcmMessageBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<FcmNotification>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    android: Option<FcmAndroidConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    apns: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    webpush: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct FcmNotification {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

#[derive(Serialize)]
struct FcmAndroidConfig {
    priority: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collapse_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<FcmAndroidNotification>,
}

#[derive(Serialize)]
struct FcmAndroidNotification {
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    click_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NotificationAction, Urgency};

    const FAKE_KEY: &str =
        "-----BEGIN RSA PRIVATE KEY-----\nSUPERSECRETKEYMATERIAL\n-----END RSA PRIVATE KEY-----";

    fn config() -> FcmConfig {
        FcmConfig::new(
            "test-project",
            "svc@test-project.iam.gserviceaccount.com",
            FAKE_KEY,
        )
    }

    #[test]
    fn timeouts_default_to_the_documented_values() {
        // Making these configurable must not change the overall-timeout
        // behavior for anyone who does not set them.
        let config = config();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
    }

    #[test]
    fn timeout_builders_override_the_defaults() {
        let config = config()
            .timeout(Duration::from_millis(250))
            .connect_timeout(Duration::from_millis(50));
        assert_eq!(config.timeout, Duration::from_millis(250));
        assert_eq!(config.connect_timeout, Duration::from_millis(50));

        // `FcmProvider::new` performs a network token exchange, so acceptance
        // by the provider constructor is asserted in `tests/fcm.rs` against a
        // stub server. Here we assert the reqwest client builder itself takes
        // the values, which is the part that could reject them.
        assert!(
            Client::builder()
                .timeout(config.timeout)
                .connect_timeout(config.connect_timeout)
                .build()
                .is_ok()
        );
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
            "FcmConfig Debug leaked the private key: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "got {rendered}");
        // Non-secret fields must survive so the Debug output stays useful.
        assert!(rendered.contains("test-project"), "got {rendered}");
        assert!(
            rendered.contains("svc@test-project.iam.gserviceaccount.com"),
            "got {rendered}"
        );
    }

    #[test]
    fn credentials_debug_does_not_leak_private_key() {
        let rendered = format!("{:?}", config().credentials);
        assert!(
            !rendered.contains("SUPERSECRETKEYMATERIAL"),
            "FcmCredentials Debug leaked the private key: {rendered}"
        );
        assert!(rendered.contains("<redacted>"), "got {rendered}");
        assert!(
            rendered.contains("svc@test-project.iam.gserviceaccount.com"),
            "got {rendered}"
        );
    }

    /// Golden payload: a fully-populated `Notification` must round-trip every
    /// field FCM can carry. Nothing else in the suite asserts an FCM request
    /// *body*, so without this a `build_message_body` that dropped every field
    /// would still pass.
    #[test]
    fn build_message_body_golden() {
        let provider_config = config();
        let notification = Notification::new("Title", "Body")
            .icon("https://example.com/icon.png")
            .image("https://example.com/image.png")
            .badge(7)
            .sound("chime.caf")
            .click_action("OPEN_ACTIVITY")
            .tag("group-a")
            .data("order_id", "12345")
            .priority(Priority::High)
            .urgency(Urgency::High)
            .ttl(600)
            .topic("com.example.app")
            .collapse_key("collapse-me")
            .mutable_content()
            .action(NotificationAction::new("view", "View"));

        // `build_message_body` needs no network, so construct the provider fields
        // directly rather than going through the token-exchanging `new`.
        let provider = FcmProvider {
            config: provider_config,
            client: Client::new(),
            access_token: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        };

        let body = provider.build_message_body(&notification);
        let json = serde_json::to_value(FcmRequest {
            message: FcmMessage {
                token: "device-token",
                body: &body,
            },
        })
        .unwrap();

        let expected = serde_json::json!({
            "message": {
                "token": "device-token",
                "notification": {
                    "title": "Title",
                    "body": "Body",
                    "image": "https://example.com/image.png"
                },
                "data": { "order_id": "12345" },
                "android": {
                    "priority": "high",
                    "ttl": "600s",
                    "collapse_key": "collapse-me",
                    "notification": {
                        "icon": "https://example.com/icon.png",
                        "sound": "chime.caf",
                        "tag": "group-a",
                        "click_action": "OPEN_ACTIVITY",
                        "notification_count": 7
                    }
                }
            }
        });

        assert_eq!(
            json, expected,
            "FCM payload drifted from the golden fixture"
        );
    }

    #[test]
    fn build_message_body_maps_badge_to_notification_count() {
        let provider = FcmProvider {
            config: config(),
            client: Client::new(),
            access_token: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        };
        let notification = Notification::new("Hi", "there").badge(3);
        let json = serde_json::to_value(provider.build_message_body(&notification)).unwrap();

        assert_eq!(
            json["android"]["notification"]["notification_count"],
            serde_json::json!(3),
            "Notification::badge must reach FCM's notification_count: {json}"
        );
    }

    #[test]
    fn silent_notification_omits_the_notification_block() {
        let provider = FcmProvider {
            config: config(),
            client: Client::new(),
            access_token: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        };
        let notification = Notification::data_only().data("k", "v");
        let json = serde_json::to_value(provider.build_message_body(&notification)).unwrap();

        assert!(json.get("notification").is_none(), "got {json}");
        assert_eq!(json["data"]["k"], serde_json::json!("v"));
    }

    #[test]
    fn non_https_api_base_is_rejected_without_opt_in() {
        let err = require_https_or_loopback("api_base", "http://fcm.example.com", false)
            .expect_err("plain http against a public host must be rejected");
        assert!(matches!(err, PushError::Config(_)), "got {err:?}");
    }

    #[test]
    fn loopback_http_requires_explicit_opt_in() {
        assert!(
            require_https_or_loopback("api_base", "http://127.0.0.1:8080", false).is_err(),
            "loopback http must be refused unless opted in"
        );
        assert!(require_https_or_loopback("api_base", "http://127.0.0.1:8080", true).is_ok());
        assert!(require_https_or_loopback("api_base", "http://localhost:8080", true).is_ok());
    }

    #[test]
    fn non_loopback_http_is_rejected_even_with_opt_in() {
        // The opt-in relaxes the scheme only for loopback, never for a
        // routable host.
        let err = require_https_or_loopback("api_base", "http://169.254.169.254", true)
            .expect_err("opt-in must not permit http to a non-loopback host");
        assert!(matches!(err, PushError::Config(_)), "got {err:?}");
    }

    /// Mirrors `apns::tests::poisoned_locks_do_not_permanently_brick_the_provider`.
    ///
    /// FCM carried the byte-identical `access_token.read().unwrap()` /
    /// `.write().unwrap()` that APNS was fixed for, so on the same host APNS
    /// degraded gracefully while FCM panicked. Poison is benign here — the
    /// RwLock guards a plain `Option<AccessToken>` with no cross-field
    /// invariant — so it must be recovered, not propagated. (`refresh_lock` is
    /// a `tokio::sync::Mutex`, which has no poisoning.)
    #[test]
    fn poisoned_token_lock_does_not_permanently_brick_the_provider() {
        let provider = FcmProvider {
            config: config(),
            client: Client::new(),
            access_token: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        };

        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _guard = provider.access_token.write().unwrap();
                    panic!("poison the token lock");
                })
                .join()
                .expect_err("the spawned thread should have panicked");
        });

        assert!(provider.access_token.is_poisoned());

        // Reading through the poisoned RwLock must not panic.
        assert!(provider.cached_token().is_none());
    }

    /// The FCM half of the clock fix: `refresh_token` used to
    /// `duration_since(UNIX_EPOCH).unwrap()`. It now goes through the shared
    /// helper, whose failure is a `PushError::Config`. We cannot move the
    /// system clock from a test, so pin that `refresh_token` reaches key
    /// parsing (i.e. past the clock read) and reports a `Config` error rather
    /// than panicking.
    #[tokio::test]
    async fn refresh_token_reports_a_config_error_instead_of_panicking() {
        let provider = FcmProvider {
            config: config(),
            client: Client::new(),
            access_token: RwLock::new(None),
            refresh_lock: tokio::sync::Mutex::new(()),
        };

        match provider.refresh_token().await {
            Err(PushError::Config(msg)) => {
                assert!(
                    msg.contains("Invalid private key"),
                    "expected to reach key parsing past the clock read, got {msg}"
                );
            }
            other => panic!("expected a Config error from the fixture key, got {other:?}"),
        }
    }

    #[test]
    fn https_is_always_accepted() {
        assert!(require_https_or_loopback("api_base", "https://fcm.googleapis.com", false).is_ok());
    }
}
