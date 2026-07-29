//! Web Push (VAPID) provider.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

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

/// What still has to happen before a validated endpoint may be connected to.
#[derive(Debug, PartialEq, Eq)]
enum EndpointTarget {
    /// A loopback host or an IP literal. There is no name to resolve, and the
    /// address in the URL is the address that will be dialed, so the shared
    /// client can be used directly.
    Literal,
    /// A DNS name. The literal checks say nothing about where it points, so it
    /// must be resolved, every returned address vetted, and the connection
    /// pinned to those addresses — see [`WebPushProvider::vetted_client`].
    Domain {
        /// The host as it appears in the URL.
        host: String,
        /// The port that will be connected to (the scheme default if implicit).
        port: u16,
    },
}

/// True for a resolved address a subscription endpoint must never reach.
///
/// Loopback is folded in unconditionally here: this function only ever sees
/// addresses that came back from DNS for a *non*-loopback name, and a name that
/// resolves to 127.0.0.1 is the classic way to launder the loopback check.
/// (`is_internal_v6` already covers IPv6 loopback; `is_internal_v4`
/// deliberately does not, so it is added explicitly.)
///
/// [`resolve_and_vet`] applies one additional, narrower exemption on top of
/// this function rather than inside it: a *resolved* loopback answer is let
/// through when the caller has opted into `allow_insecure_loopback`,
/// mirroring the exemption [`validate_endpoint`] already gives a *literal*
/// loopback URL (see [`is_pure_loopback`]). This function's own verdict stays
/// unconditional.
fn is_internal_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_internal_v4(&v4) || v4.is_loopback(),
        IpAddr::V6(v6) => is_internal_v6(&v6),
    }
}

/// True for a resolved address that is loopback in any spelling: plain IPv4
/// loopback, plain IPv6 loopback, or an IPv4-mapped/compatible IPv6 embedding
/// of either.
///
/// Split out from [`is_internal_addr`] so [`resolve_and_vet`] can exempt
/// *just* this case under `allow_insecure_loopback` — the same opt-in that
/// already exempts a literal loopback URL in [`validate_endpoint`] — without
/// relaxing any of the other internal ranges `is_internal_addr` rejects.
fn is_pure_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4().is_some_and(|v4| v4.is_loopback()),
    }
}

/// Abstraction over "resolve `host` as if connecting to it on `port`," so
/// [`resolve_and_vet`] can be driven by fixed, controlled records in tests
/// instead of the ambient DNS resolver.
///
/// Without this seam, the DNS-dependent regression tests in
/// `tests/web_push.rs` asked the *real* resolver a throwaway question first
/// and skipped themselves if the answer was not what they expected (a
/// resolver that hijacks NXDOMAIN into a landing page, or one that cannot
/// resolve a trailing-dot FQDN) — meaning they could report green having
/// asserted nothing at all, on exactly the kind of network/CI image where a
/// DNS-dependent guard most needs checking.
#[async_trait]
pub(crate) trait HostResolver: Send + Sync {
    /// Resolve `host` as if connecting to it on `port`. Mirrors
    /// `tokio::net::lookup_host`'s contract: an empty `Ok` and an `Err` are
    /// both possible failure shapes, so callers must handle both.
    async fn lookup_host(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>>;
}

/// Production resolver: a thin wrapper over `tokio::net::lookup_host`.
struct TokioHostResolver;

#[async_trait]
impl HostResolver for TokioHostResolver {
    async fn lookup_host(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host((host, port)).await?.collect())
    }
}

/// A fixed-answer resolver for deterministic tests.
///
/// Crate-private and test-only: it exists purely so this module's own
/// `#[cfg(test)] mod tests` can drive [`resolve_and_vet`] (via
/// [`WebPushProvider::new_with_resolver`]) without depending on the ambient
/// DNS resolver.
#[cfg(test)]
type FakeResolverRecords =
    std::sync::Arc<std::sync::Mutex<HashMap<(String, u16), Vec<SocketAddr>>>>;

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct FakeHostResolver {
    records: FakeResolverRecords,
}

#[cfg(test)]
impl FakeHostResolver {
    /// A resolver with no records. Every lookup fails as "unresolvable" until
    /// [`Self::set`] is called for that exact `host`/`port` pair.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record (or replace) the fixed answer for `host:port`.
    pub(crate) fn set(&self, host: &str, port: u16, addrs: Vec<SocketAddr>) {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((host.to_string(), port), addrs);
    }
}

#[cfg(test)]
#[async_trait]
impl HostResolver for FakeHostResolver {
    async fn lookup_host(&self, host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(host.to_string(), port))
            .cloned()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "fake resolver: no record")
            })
    }
}

/// Resolve `host` and refuse the whole endpoint if *any* answer is internal.
///
/// Rejecting on `any` rather than `all` is deliberate: a resolver that returns
/// one public and one internal address would otherwise leave the choice of
/// which to dial up to the connector.
///
/// The lookup itself is wrapped in `connect_timeout`, not just the connection
/// that follows it. `resolver.lookup_host` runs (in production) on tokio's
/// blocking pool, shared process-wide; with no timeout, a Web Push
/// subscription endpoint pointed at a DNS tarpit (an attacker-controlled
/// hostname resolving via a non-responsive authoritative server) held the
/// lookup for the full resolver retry budget, and enough concurrent malicious
/// endpoints could exhaust the shared pool. A timeout is folded into the same
/// "unresolvable" error a hard DNS failure already produces.
///
/// `allow_insecure_loopback` exempts a *resolved* loopback answer, the same
/// way [`validate_endpoint`] already exempts a *literal* loopback URL. It
/// does not relax any other internal range: see [`is_pure_loopback`].
async fn resolve_and_vet(
    resolver: &dyn HostResolver,
    host: &str,
    port: u16,
    connect_timeout: Duration,
    allow_insecure_loopback: bool,
) -> Result<Vec<SocketAddr>> {
    // The host is attacker-influenced, so — as everywhere else in this module —
    // it is never echoed into the error text.
    let unresolvable =
        || PushError::Network("web push endpoint host could not be resolved".to_string());

    let addrs: Vec<SocketAddr> =
        tokio::time::timeout(connect_timeout, resolver.lookup_host(host, port))
            .await
            .map_err(|_elapsed| unresolvable())?
            .map_err(|_io_err| unresolvable())?;

    if addrs.is_empty() {
        return Err(unresolvable());
    }

    if addrs.iter().any(|addr| {
        let ip = addr.ip();
        !(allow_insecure_loopback && is_pure_loopback(ip)) && is_internal_addr(ip)
    }) {
        return Err(PushError::Config(
            "web push endpoint resolves to an internal address".to_string(),
        ));
    }

    Ok(addrs)
}

/// SSRF guard for the client-supplied subscription endpoint: enforce https and
/// refuse IP-literal internal targets and `.internal`/`.local` hosts before we
/// open a connection.
///
/// This half is a pure literal/suffix check. It is deliberately *not* the whole
/// guard: a `Host::Domain` passes here and is then resolved and vetted by
/// [`resolve_and_vet`], because otherwise `https://metadata.attacker.example/`
/// with an A record of `169.254.169.254` would skip every check above — an
/// easier bypass than any of the IPv6 embeddings [`is_internal_v6`] handles.
/// The returned [`EndpointTarget`] tells the caller which case it got.
///
/// Loopback is exempt **only** when `allow_insecure_loopback` is set. That flag
/// defaults to `false`, which matters: the exemption's stated justification is
/// local stub tests, but it previously applied unconditionally, so in a
/// production binary an untrusted subscription endpoint of
/// `http://127.0.0.1:6379/...` passed validation and let a push subscription
/// drive requests at any loopback-bound service on the app host.
fn validate_endpoint(endpoint: &str, allow_insecure_loopback: bool) -> Result<EndpointTarget> {
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
            Ok(EndpointTarget::Literal)
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

    let host_lower = host_str.to_ascii_lowercase();
    if host_lower.ends_with(".internal") || host_lower.ends_with(".local") {
        return Err(PushError::Config(
            "web push endpoint targets an internal host".to_string(),
        ));
    }

    match url.host() {
        Some(Host::Ipv4(ip)) => {
            if is_internal_v4(&ip) {
                return Err(PushError::Config(
                    "web push endpoint targets an internal IPv4 address".to_string(),
                ));
            }
            Ok(EndpointTarget::Literal)
        }
        Some(Host::Ipv6(ip)) => {
            if is_internal_v6(&ip) {
                return Err(PushError::Config(
                    "web push endpoint targets an internal IPv6 address".to_string(),
                ));
            }
            Ok(EndpointTarget::Literal)
        }
        // The case the literal checks above cannot decide. Handing it back as
        // `Domain` is what forces the caller through `resolve_and_vet`.
        Some(Host::Domain(_)) => Ok(EndpointTarget::Domain {
            host: host_str.to_string(),
            port: url.port_or_known_default().ok_or_else(|| {
                PushError::Config("web push endpoint has no usable port".to_string())
            })?,
        }),
        None => Err(PushError::Config(
            "web push endpoint has no host".to_string(),
        )),
    }
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

/// How long a vetted DNS answer (and the client pinned to it) is reused.
///
/// Short enough that a legitimate push service can move, long enough that a
/// batch to one host does not re-resolve per notification.
const PINNED_CLIENT_TTL: Duration = Duration::from_secs(30);

/// Hard cap on the pinned-client cache. Subscription endpoints are
/// attacker-influenced, so an unbounded map keyed by their host is a memory
/// amplification primitive.
const PINNED_CLIENT_MAX: usize = 64;

/// A client whose DNS resolution is pinned to already-vetted addresses.
struct PinnedClient {
    client: reqwest::Client,
    expires_at: Instant,
}

/// Web Push provider using VAPID.
pub struct WebPushProvider {
    config: WebPushConfig,
    client: reqwest::Client,
    /// VAPID signing key, decoded once at construction; per-send we clone this
    /// and attach the subscription info rather than re-decoding the base64 key
    /// and re-deriving the EC key on every notification.
    vapid: PartialVapidSignatureBuilder,
    /// Per-host clients pinned to vetted addresses, keyed by `host:port`.
    ///
    /// Pinning is the point: without it the guard resolves the name, approves
    /// the answer, and then lets the connector resolve it *again* — a DNS
    /// rebinding window in which the second answer can be 169.254.169.254.
    /// Caching them keeps a batch to one push service from paying a fresh
    /// resolution and a fresh connection pool per notification.
    pinned_clients: std::sync::Mutex<HashMap<String, PinnedClient>>,
    /// DNS resolution seam for [`resolve_and_vet`]. Always [`TokioHostResolver`]
    /// outside tests; see [`WebPushProvider::new_with_resolver`].
    resolver: Box<dyn HostResolver>,
}

impl WebPushProvider {
    /// Create a new Web Push provider.
    pub fn new(config: WebPushConfig) -> Result<Self> {
        Self::build(config, Box::new(TokioHostResolver))
    }

    /// Test-only constructor: builds a provider whose DNS resolution is
    /// driven entirely by `resolver` instead of the ambient system resolver.
    ///
    /// Crate-private: it exists so this module's own tests can exercise
    /// [`resolve_and_vet`] and the pinning path in [`Self::vetted_client`]
    /// deterministically, without depending on the DNS environment the test
    /// happens to run in.
    #[cfg(test)]
    pub(crate) fn new_with_resolver(
        config: WebPushConfig,
        resolver: FakeHostResolver,
    ) -> Result<Self> {
        Self::build(config, Box::new(resolver))
    }

    fn build(config: WebPushConfig, resolver: Box<dyn HostResolver>) -> Result<Self> {
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
            pinned_clients: std::sync::Mutex::new(HashMap::new()),
            resolver,
        })
    }

    /// A client that will only ever connect `host` to addresses this guard has
    /// already vetted.
    ///
    /// Poison is recovered rather than propagated throughout: the guarded value
    /// is a plain cache with no cross-field invariant, so one panic elsewhere
    /// must not brick the provider.
    async fn vetted_client(&self, host: &str, port: u16) -> Result<reqwest::Client> {
        let key = format!("{host}:{port}");

        {
            let cache = self
                .pinned_clients
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(pinned) = cache.get(&key)
                && pinned.expires_at > Instant::now()
            {
                return Ok(pinned.client.clone());
            }
        }

        let addrs = resolve_and_vet(
            self.resolver.as_ref(),
            host,
            port,
            self.config.connect_timeout,
            self.config.allow_insecure_loopback,
        )
        .await?;

        // Same settings as `self.client` — redirects refused, both timeouts
        // present — plus the resolution override that closes the rebinding
        // window between the check above and the connect below.
        let client = reqwest::Client::builder()
            .timeout(self.config.timeout)
            .connect_timeout(self.config.connect_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addrs)
            .build()
            .map_err(|e| PushError::Config(e.to_string()))?;

        let mut cache = self
            .pinned_clients
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        cache.retain(|_, pinned| pinned.expires_at > now);
        if cache.len() >= PINNED_CLIENT_MAX {
            // Bounded, and cheap to rebuild: dropping everything is preferable
            // to letting attacker-chosen hosts grow this without limit.
            cache.clear();
        }
        cache.insert(
            key,
            PinnedClient {
                client: client.clone(),
                expires_at: now + PINNED_CLIENT_TTL,
            },
        );

        Ok(client)
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
        // sign anything or open a connection. A DNS name additionally gets
        // resolved, every answer vetted, and the connection pinned to those
        // answers so rebinding cannot swap the target between check and connect.
        let client =
            match validate_endpoint(&subscription.endpoint, self.config.allow_insecure_loopback)? {
                EndpointTarget::Literal => self.client.clone(),
                EndpointTarget::Domain { host, port } => self.vetted_client(&host, port).await?,
            };

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
        //
        // The target URL is built from `subscription.endpoint` directly, not
        // `message.endpoint`. `message.endpoint` is web-push's own re-parse of
        // that same string (via `http::Uri`, a different parser from the
        // `url::Url` that `validate_endpoint` used to derive the host handed to
        // `resolve_to_addrs`, and that reqwest itself uses internally). Passing
        // `message.endpoint` here risked the override key and the host actually
        // dialed diverging on a normalization difference between the two
        // parsers (e.g. a trailing-dot FQDN) — on any mismatch,
        // `resolve_to_addrs` silently falls back to the system resolver,
        // reopening the DNS-rebinding window pinning exists to close. Using the
        // identical string both times makes the two parses agree by
        // construction rather than by coincidence.
        let mut request = client
            .post(subscription.endpoint.as_str())
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
            } else if e.is_request() || e.is_body() {
                // A TCP reset after connect, a TLS renegotiation failure, or a
                // broken pipe mid-payload are the same transient class as a
                // connect failure. Routing them to the non-retryable
                // `Provider(None, ..)` meant a push service that resets under
                // load dropped notifications permanently, while the identical
                // failure occurring during *connect* was retried. The message
                // is a constant for the same reason as every other arm here:
                // `e.to_string()` appends ` for url (…)` and would re-introduce
                // the attacker-supplied endpoint.
                PushError::Network("web push request failed in transit".to_string())
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
    use armature_testkit::{StubResponse, StubServer};

    /// A valid VAPID key (borrowed from web-push's own test vectors), so a
    /// provider built with it actually succeeds and proves out timeout/
    /// resolver plumbing rather than failing at key parsing first.
    const VALID_VAPID_KEY: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";

    /// A provider whose resolver is `resolver` and whose SSRF guard permits
    /// loopback — the shape every `StubServer`-backed test in this module
    /// needs, since `resolve_and_vet` only accepts a resolved loopback answer
    /// under this opt-in.
    fn provider_with_resolver(resolver: FakeHostResolver) -> WebPushProvider {
        let config = WebPushConfig::new(VALID_VAPID_KEY, "mailto:test@example.com")
            .allow_insecure_loopback(true);
        WebPushProvider::new_with_resolver(config, resolver).expect("build provider")
    }

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
        let config = WebPushConfig::new(VALID_VAPID_KEY, "mailto:admin@example.com")
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
    fn a_domain_endpoint_is_deferred_to_dns_resolution() {
        // The gap this closes: `Some(Host::Domain(_))` had no arm at all, so
        // `https://metadata.attacker.example/push` with an A record of
        // 169.254.169.254 skipped every check. A domain must now come back as
        // `Domain`, which is what forces the caller through `resolve_and_vet`.
        assert_eq!(
            validate_endpoint("https://metadata.attacker.example/push", false).unwrap(),
            EndpointTarget::Domain {
                host: "metadata.attacker.example".to_string(),
                port: 443,
            }
        );
        // An explicit port is carried through rather than defaulted.
        assert_eq!(
            validate_endpoint("https://push.example.com:8443/x", false).unwrap(),
            EndpointTarget::Domain {
                host: "push.example.com".to_string(),
                port: 8443,
            }
        );
    }

    #[test]
    fn ip_literal_endpoints_need_no_resolution() {
        // Nothing to resolve and nothing to pin: the address in the URL is the
        // address that gets dialed.
        assert_eq!(
            validate_endpoint("https://93.184.216.34/push", false).unwrap(),
            EndpointTarget::Literal
        );
        assert_eq!(
            validate_endpoint("https://[2606:2800:220:1:248:1893:25c8:1946]/push", false).unwrap(),
            EndpointTarget::Literal
        );
        assert_eq!(
            validate_endpoint("http://127.0.0.1:8080/push", true).unwrap(),
            EndpointTarget::Literal
        );
    }

    #[test]
    fn resolved_internal_addresses_are_refused() {
        use std::net::{Ipv4Addr, Ipv6Addr};

        // The vetting applied to every DNS answer. Loopback has to be blocked
        // here even though `is_internal_v4` excludes it: a name resolving to
        // 127.0.0.1 is the standard way to launder the literal loopback check.
        for ip in [
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6("fd12:3456::1".parse().unwrap()),
        ] {
            assert!(is_internal_addr(ip), "{ip} must be refused");
        }

        assert!(!is_internal_addr(IpAddr::V4(Ipv4Addr::new(
            93, 184, 216, 34
        ))));
    }

    #[tokio::test]
    async fn resolve_and_vet_refuses_a_host_that_resolves_to_loopback() {
        // `lookup_host` parses an IP literal without touching DNS, so this runs
        // offline while still exercising the real resolve-then-vet path.
        let err = resolve_and_vet(
            &TokioHostResolver,
            "127.0.0.1",
            443,
            Duration::from_secs(1),
            false,
        )
        .await
        .expect_err("a loopback answer must be refused");
        assert!(matches!(err, PushError::Config(_)), "got {err:?}");

        let err = resolve_and_vet(
            &TokioHostResolver,
            "169.254.169.254",
            443,
            Duration::from_secs(1),
            false,
        )
        .await
        .expect_err("a link-local answer must be refused");
        assert!(matches!(err, PushError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn resolve_and_vet_accepts_a_public_answer_and_never_echoes_the_host() {
        let addrs = resolve_and_vet(
            &TokioHostResolver,
            "93.184.216.34",
            443,
            Duration::from_secs(1),
            false,
        )
        .await
        .expect("a public answer must be accepted");
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].port(), 443);

        let err = resolve_and_vet(
            &TokioHostResolver,
            "169.254.169.254",
            443,
            Duration::from_secs(1),
            false,
        )
        .await
        .unwrap_err();
        assert!(
            !err.to_string().contains("169.254"),
            "the attacker-influenced host must not be echoed: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_and_vet_accepts_a_loopback_answer_when_opted_in() {
        // The DNS-resolved mirror of `validate_endpoint`'s literal-loopback
        // exemption: opting in accepts a resolved loopback answer, but every
        // other internal range stays refused regardless.
        let addrs = resolve_and_vet(
            &TokioHostResolver,
            "127.0.0.1",
            443,
            Duration::from_secs(1),
            true,
        )
        .await
        .expect("a loopback answer must be accepted when opted in");
        assert_eq!(addrs, vec![SocketAddr::from(([127, 0, 0, 1], 443))]);

        let err = resolve_and_vet(
            &TokioHostResolver,
            "169.254.169.254",
            443,
            Duration::from_secs(1),
            true,
        )
        .await
        .expect_err("a link-local answer must stay refused even with loopback opted in");
        assert!(matches!(err, PushError::Config(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn resolve_and_vet_times_out_on_a_hanging_resolver() {
        // `resolver.lookup_host` used to run with no timeout, on tokio's
        // shared blocking pool in production. A hostname aimed at a DNS
        // tarpit held that pool for the full resolver retry budget; enough
        // concurrent malicious endpoints exhausted it process-wide. A
        // resolver that never returns exercises the same failure mode
        // offline, and the outer `tokio::time::timeout` guarantees this test
        // itself cannot hang if the fix regresses.
        struct HangingResolver;

        #[async_trait]
        impl HostResolver for HangingResolver {
            async fn lookup_host(
                &self,
                _host: &str,
                _port: u16,
            ) -> std::io::Result<Vec<SocketAddr>> {
                std::future::pending::<std::io::Result<Vec<SocketAddr>>>().await
            }
        }

        let err = tokio::time::timeout(
            Duration::from_secs(5),
            resolve_and_vet(
                &HangingResolver,
                "tarpit.example",
                443,
                Duration::from_millis(50),
                false,
            ),
        )
        .await
        .expect("resolve_and_vet must time out on its own rather than hang the caller")
        .expect_err("a hanging resolver must be treated as unresolvable");

        assert!(
            matches!(err, PushError::Network(_)),
            "expected Network, got {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "Network error: web push endpoint host could not be resolved",
            "a resolver timeout must map to the same error as a resolution failure"
        );
    }

    #[tokio::test]
    async fn send_to_web_subscription_fails_on_an_unresolvable_domain_endpoint() {
        // `validate_endpoint` only ever inspected `Host::Ipv4`/`Host::Ipv6`
        // literals: `Some(Host::Domain(_))` had no arm at all, so
        // `https://metadata.attacker.example/push` with an A record of
        // 169.254.169.254 skipped every check — an easier bypass than any of
        // the IPv6 embeddings `is_internal_v6` handles.
        //
        // Proving the resolution actually happens as part of the public send
        // path, without depending on the ambient DNS resolver: this used to
        // ask the real resolver a throwaway question first and skip itself if
        // a resolver hijacking NXDOMAIN into a landing page made the
        // assertion vacuous — meaning it could report green having asserted
        // nothing, on exactly the kind of network/CI image where this guard
        // most needs checking. A fake resolver with no record for the host
        // fails deterministically, the same way, on every machine.
        const HOST: &str = "metadata-guard-probe.invalid";

        let config = WebPushConfig::new(VALID_VAPID_KEY, "mailto:test@example.com");
        let provider =
            WebPushProvider::new_with_resolver(config, FakeHostResolver::new()).expect("build");
        let sub = WebPushSubscription::new(format!("https://{HOST}/push"), "p256dh", "auth");

        let err = provider
            .send_to_web_subscription(&sub, &Notification::new("Hello", "World"))
            .await
            .expect_err("an unresolvable endpoint host must fail");

        assert!(
            matches!(err, PushError::Network(_)),
            "expected Network, got {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "Network error: web push endpoint host could not be resolved",
            "the failure must come from the guard's own resolution, not from connect"
        );
        assert!(
            !err.to_string().contains(HOST),
            "the endpoint host must not be echoed: {err}"
        );
    }

    #[tokio::test]
    async fn send_to_web_subscription_rejects_a_domain_resolving_to_loopback_without_opt_in() {
        // The DNS half of the guard: `localhost` is caught by the literal
        // loopback check, but any *other* name pointing at 127.0.0.1 is not,
        // and that is the whole point of resolving. This used to rely on the
        // ambient resolver's handling of `localhost.` (trailing dot) to
        // exercise that path offline, and skipped itself if the resolver
        // could not resolve it — meaning it could report green having
        // asserted nothing. A fake resolver records the same answer
        // deterministically instead.
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let addr: SocketAddr = server
            .url()
            .trim_start_matches("http://")
            .parse()
            .expect("stub address");

        const HOST: &str = "domain-resolves-to-loopback-probe.invalid";
        let resolver = FakeHostResolver::new();
        resolver.set(HOST, addr.port(), vec![addr]);

        // Production defaults: loopback is *not* exempt, even for a resolved
        // answer (that opt-in is exercised separately, at the
        // `resolve_and_vet` level, by
        // `resolve_and_vet_accepts_a_loopback_answer_when_opted_in`).
        let config = WebPushConfig::new(VALID_VAPID_KEY, "mailto:test@example.com");
        let provider =
            WebPushProvider::new_with_resolver(config, resolver).expect("build provider");
        let sub = WebPushSubscription::new(
            format!("https://{HOST}:{}/push", addr.port()),
            "p256dh",
            "auth",
        );

        let err = provider
            .send_to_web_subscription(&sub, &Notification::new("Hello", "World"))
            .await
            .expect_err("a domain resolving to loopback must be refused");

        assert!(
            matches!(err, PushError::Config(_)),
            "expected Config, got {err:?}"
        );
        assert!(
            server.requests().is_empty(),
            "no connection should have been opened"
        );
    }

    #[tokio::test]
    async fn vetted_client_actually_connects_to_the_address_it_vetted() {
        // Every existing test targeting `vetted_client` either hits an
        // IP-literal endpoint (never reaching this code) or errors out of
        // `resolve_and_vet` before reaching the pinning logic — deleting
        // `.resolve_to_addrs(host, &addrs)` outright would not fail a single
        // test. Prove a pinned client actually completes a request against
        // the address it vetted.
        //
        // This also stands in as the regression test for the
        // `resolve_to_addrs`-key-vs-connect-time-host divergence fix in
        // `send_prepared`: `HOST` is an RFC 2606 `.invalid` name, which no
        // real resolver can ever answer. If the pinning override were keyed
        // by a host string that diverges even slightly from the one actually
        // dialed, reqwest would silently fall back to the system resolver —
        // which, for `.invalid`, can only fail. The request below only
        // succeeds if the override is actually used to reach `server`.
        //
        // Exercised at `vetted_client` directly rather than through
        // `send_to_web_subscription`: a *domain* endpoint (unlike a loopback
        // literal) requires `https` regardless of `allow_insecure_loopback`,
        // and `StubServer` only speaks plain http.
        const HOST: &str = "vetted-client-connect-probe.invalid";

        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let addr: SocketAddr = server
            .url()
            .trim_start_matches("http://")
            .parse()
            .expect("stub address");

        let resolver = FakeHostResolver::new();
        resolver.set(HOST, addr.port(), vec![addr]);
        let provider = provider_with_resolver(resolver);

        let client = provider
            .vetted_client(HOST, addr.port())
            .await
            .expect("a loopback answer must be accepted when opted in");

        let response = client
            .get(format!("http://{HOST}:{}/", addr.port()))
            .send()
            .await
            .expect("the pinned client must actually reach the vetted address");
        assert_eq!(response.status(), 200);
        server.assert_received("GET", "/");
    }

    #[tokio::test]
    async fn vetted_client_keeps_using_the_originally_vetted_address_after_a_later_dns_change() {
        // A rebinding attempt *after* vetting must not redirect an
        // already-pinned client. `stub_b` stands in for the address an
        // attacker's DNS would answer with on a second lookup.
        const HOST: &str = "vetted-client-rebind-probe.invalid";

        let stub_a = StubServer::start_single(StubResponse::new(200, "")).await;
        let stub_b = StubServer::start_single(StubResponse::new(200, "")).await;
        let addr_a: SocketAddr = stub_a.url().trim_start_matches("http://").parse().unwrap();
        let addr_b: SocketAddr = stub_b.url().trim_start_matches("http://").parse().unwrap();

        let resolver = FakeHostResolver::new();
        resolver.set(HOST, addr_a.port(), vec![addr_a]);
        let provider = provider_with_resolver(resolver.clone());

        let first = provider
            .vetted_client(HOST, addr_a.port())
            .await
            .expect("first vet must succeed");
        first
            .get(format!("http://{HOST}:{}/", addr_a.port()))
            .send()
            .await
            .expect("the first request must reach stub_a");
        stub_a.assert_received("GET", "/");
        assert!(
            stub_b.requests().is_empty(),
            "stub_b must not have been contacted yet"
        );

        // Simulate the DNS answer changing after vetting. If pinning were not
        // holding, the next request would land on stub_b instead.
        resolver.set(HOST, addr_a.port(), vec![addr_b]);

        let second = provider
            .vetted_client(HOST, addr_a.port())
            .await
            .expect("the cached pinned client must still be returned");
        second
            .get(format!("http://{HOST}:{}/", addr_a.port()))
            .send()
            .await
            .expect("the second request must still reach stub_a");

        assert_eq!(
            stub_a.requests().len(),
            2,
            "the pinned client must keep landing on stub_a"
        );
        assert!(
            stub_b.requests().is_empty(),
            "stub_b must never be contacted: pinning must survive a changed DNS answer"
        );
    }

    #[test]
    fn urgency_header_values_match_rfc_8030() {
        assert_eq!(urgency_header_value(Urgency::VeryLow), "very-low");
        assert_eq!(urgency_header_value(Urgency::Low), "low");
        assert_eq!(urgency_header_value(Urgency::Normal), "normal");
        assert_eq!(urgency_header_value(Urgency::High), "high");
    }
}
