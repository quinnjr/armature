//! MCP Authentication and Authorization
//!
//! Provides configurable authentication for MCP endpoints using:
//! - API tokens (generated bearer tokens)
//! - JWT validation
//! - OAuth2 access tokens

use crate::error::{McpError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Authentication method for MCP access
#[derive(Clone)]
pub enum McpAuthMethod {
    /// No authentication required (not recommended for production)
    None,

    /// Static API tokens - simple bearer tokens validated against a list
    ApiToken(ApiTokenAuth),

    /// JWT-based authentication with signature verification
    Jwt(JwtAuth),

    /// OAuth2 token validation via introspection or user info endpoint
    OAuth2(OAuth2Auth),

    /// Multiple authentication methods (any one succeeds)
    AnyOf(Vec<McpAuthMethod>),

    /// Custom authentication handler
    Custom(Arc<dyn McpAuthenticator>),
}

impl std::fmt::Debug for McpAuthMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::ApiToken(a) => f.debug_tuple("ApiToken").field(a).finish(),
            Self::Jwt(a) => f.debug_tuple("Jwt").field(a).finish(),
            Self::OAuth2(a) => f.debug_tuple("OAuth2").field(a).finish(),
            Self::AnyOf(m) => f.debug_tuple("AnyOf").field(m).finish(),
            Self::Custom(_) => f.debug_tuple("Custom").field(&"<custom>").finish(),
        }
    }
}

/// API token authentication configuration
#[derive(Debug, Clone)]
pub struct ApiTokenAuth {
    /// Valid API tokens
    pub tokens: HashSet<String>,
    /// Header name to read token from (default: "Authorization")
    pub header_name: String,
    /// Token prefix (default: "Bearer ")
    pub token_prefix: String,
    /// Required scopes (empty = no scope check)
    pub required_scopes: Vec<String>,
}

impl Default for ApiTokenAuth {
    fn default() -> Self {
        Self {
            tokens: HashSet::new(),
            header_name: "Authorization".to_string(),
            token_prefix: "Bearer ".to_string(),
            required_scopes: Vec::new(),
        }
    }
}

impl ApiTokenAuth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tokens<I, S>(mut self, tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tokens = tokens.into_iter().map(|s| s.into()).collect();
        self
    }

    pub fn add_token(mut self, token: impl Into<String>) -> Self {
        self.tokens.insert(token.into());
        self
    }

    pub fn with_header(mut self, header: impl Into<String>) -> Self {
        self.header_name = header.into();
        self
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.token_prefix = prefix.into();
        self
    }

    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_scopes = scopes.into_iter().map(|s| s.into()).collect();
        self
    }
}

/// JWT authentication configuration
#[derive(Clone)]
pub struct JwtAuth {
    /// Secret key for HMAC algorithms (HS256, HS384, HS512)
    pub secret: Option<String>,
    /// Public key for RSA/EC algorithms (RS256, ES256, etc.)
    pub public_key: Option<String>,
    /// Expected issuer (optional)
    pub issuer: Option<String>,
    /// Expected audience (optional)
    pub audience: Option<String>,
    /// Required scopes claim (optional)
    pub required_scopes: Vec<String>,
    /// Scope claim name (default: "scope" or "scopes")
    pub scope_claim: String,
    /// Algorithm (default: HS256)
    pub algorithm: String,
    /// Lazily-built, cached verify-only `armature_jwt::JwtManager` for this
    /// auth config. `JwtAuth` is a pure function of its other fields (which
    /// never change after construction), so the manager - and the PEM
    /// parsing / `Validation` construction it does internally - only needs
    /// to be built once and reused across every `authenticate_jwt` call.
    /// Wrapped in `Arc` so clones of `JwtAuth` (e.g. inside `McpAuthMethod`)
    /// share the same cached manager rather than each re-initializing it.
    jwt_manager: Arc<OnceLock<armature_jwt::JwtManager>>,
}

impl std::fmt::Debug for JwtAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwtAuth")
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .field(
                "public_key",
                &self.public_key.as_ref().map(|_| "<redacted>"),
            )
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("required_scopes", &self.required_scopes)
            .field("scope_claim", &self.scope_claim)
            .field("algorithm", &self.algorithm)
            .finish()
    }
}

impl Default for JwtAuth {
    fn default() -> Self {
        Self {
            secret: None,
            public_key: None,
            issuer: None,
            audience: None,
            required_scopes: Vec::new(),
            scope_claim: "scope".to_string(),
            algorithm: "HS256".to_string(),
            jwt_manager: Arc::new(OnceLock::new()),
        }
    }
}

impl JwtAuth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }

    pub fn with_public_key(mut self, key: impl Into<String>) -> Self {
        self.public_key = Some(key.into());
        self
    }

    pub fn with_issuer(mut self, issuer: impl Into<String>) -> Self {
        self.issuer = Some(issuer.into());
        self
    }

    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_scopes = scopes.into_iter().map(|s| s.into()).collect();
        self
    }

    pub fn with_algorithm(mut self, algorithm: impl Into<String>) -> Self {
        self.algorithm = algorithm.into();
        self
    }
}

/// A cached, successful token validation and the instant it expires.
type CachedValidation = (McpAuthContext, Instant);

/// Upper bound on the number of tokens held in the validation cache. When the
/// cache is full, expired entries are dropped first and, failing that, the
/// entry that expires soonest is evicted to make room.
const OAUTH2_CACHE_MAX_ENTRIES: usize = 4096;

/// OAuth2 authentication configuration
#[derive(Debug, Clone)]
pub struct OAuth2Auth {
    /// Token introspection endpoint URL
    pub introspection_url: Option<String>,
    /// User info endpoint URL (alternative to introspection)
    pub user_info_url: Option<String>,
    /// Client ID for introspection
    pub client_id: Option<String>,
    /// Client secret for introspection
    pub client_secret: Option<String>,
    /// Required scopes
    pub required_scopes: Vec<String>,
    /// Cache validated tokens (TTL in seconds)
    pub cache_ttl: Option<u64>,
    /// In-memory cache of successful token validations, keyed by a hash of the
    /// bearer token (raw tokens are never stored). Shared across clones — and
    /// therefore across requests, since the config is cloned/held by the
    /// long-lived service — via `Arc`. Only populated when `cache_ttl` is
    /// `Some`; only successes are cached (fail-closed).
    ///
    /// This is a private field, so `OAuth2Auth` can no longer be constructed
    /// with a struct literal; use `OAuth2Auth::new()` / the builder methods.
    cache: Arc<Mutex<HashMap<u64, CachedValidation>>>,
}

impl Default for OAuth2Auth {
    fn default() -> Self {
        Self {
            introspection_url: None,
            user_info_url: None,
            client_id: None,
            client_secret: None,
            required_scopes: Vec::new(),
            cache_ttl: Some(300), // 5 minutes default
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl OAuth2Auth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_introspection(
        mut self,
        url: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.introspection_url = Some(url.into());
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }

    pub fn with_user_info(mut self, url: impl Into<String>) -> Self {
        self.user_info_url = Some(url.into());
        self
    }

    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_scopes = scopes.into_iter().map(|s| s.into()).collect();
        self
    }

    pub fn with_cache_ttl(mut self, seconds: u64) -> Self {
        self.cache_ttl = Some(seconds);
        self
    }

    pub fn no_cache(mut self) -> Self {
        self.cache_ttl = None;
        self
    }

    /// Look up a previously validated token in the cache.
    ///
    /// Returns the cached context only when caching is enabled (`cache_ttl` is
    /// `Some`) and a non-expired entry exists. Expired entries are evicted
    /// opportunistically on every lookup so the map cannot accumulate stale
    /// tokens.
    fn cache_lookup(&self, token: &str) -> Option<McpAuthContext> {
        self.cache_ttl?; // caching disabled -> always a miss
        let key = hash_token(token);
        let now = Instant::now();
        let mut map = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        // Opportunistic eviction of everything that has expired.
        map.retain(|_, (_, expiry)| *expiry > now);
        map.get(&key).map(|(ctx, _)| ctx.clone())
    }

    /// Store a successful validation in the cache with an expiry of
    /// `now + cache_ttl`. No-op when caching is disabled. Enforces a size cap
    /// so the cache cannot grow without bound.
    fn cache_store(&self, token: &str, ctx: &McpAuthContext) {
        let Some(ttl) = self.cache_ttl else {
            return;
        };
        let key = hash_token(token);
        let now = Instant::now();
        let expiry = now + Duration::from_secs(ttl);
        let mut map = self.cache.lock().unwrap_or_else(|e| e.into_inner());

        if map.len() >= OAUTH2_CACHE_MAX_ENTRIES {
            // Drop expired entries first; if still full, evict the entry that
            // expires soonest (effectively the oldest).
            map.retain(|_, (_, e)| *e > now);
            if map.len() >= OAUTH2_CACHE_MAX_ENTRIES
                && let Some(oldest) = map.iter().min_by_key(|(_, (_, e))| *e).map(|(k, _)| *k)
            {
                map.remove(&oldest);
            }
        }

        map.insert(key, (ctx.clone(), expiry));
    }
}

/// Hash a bearer token to a compact cache key so raw tokens are not retained
/// in the cache map. A non-cryptographic hash is sufficient for a lookup key.
fn hash_token(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

/// Authenticated user/client information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAuthContext {
    /// Subject identifier (user ID, client ID, etc.)
    pub subject: Option<String>,
    /// Scopes granted to this token
    pub scopes: Vec<String>,
    /// Additional claims/metadata
    pub claims: serde_json::Value,
    /// Authentication method used
    pub auth_method: String,
}

impl Default for McpAuthContext {
    fn default() -> Self {
        Self {
            subject: None,
            scopes: Vec::new(),
            claims: serde_json::Value::Null,
            auth_method: "none".to_string(),
        }
    }
}

impl McpAuthContext {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope || s == "*")
    }

    pub fn has_any_scope(&self, scopes: &[String]) -> bool {
        scopes.iter().any(|s| self.has_scope(s))
    }

    pub fn has_all_scopes(&self, scopes: &[String]) -> bool {
        scopes.iter().all(|s| self.has_scope(s))
    }
}

/// Trait for custom authentication handlers
#[async_trait]
pub trait McpAuthenticator: Send + Sync {
    /// Authenticate a request and return the auth context
    async fn authenticate(
        &self,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<McpAuthContext>;
}

/// MCP authentication configuration
#[derive(Clone)]
pub struct McpAuthConfig {
    /// Authentication method
    pub method: McpAuthMethod,
    /// Allow unauthenticated access to tool/resource listing
    pub allow_list_unauthenticated: bool,
    /// Allow unauthenticated access to ping/initialize
    pub allow_init_unauthenticated: bool,
}

impl Default for McpAuthConfig {
    fn default() -> Self {
        Self {
            method: McpAuthMethod::None,
            allow_list_unauthenticated: false,
            allow_init_unauthenticated: true,
        }
    }
}

impl McpAuthConfig {
    pub fn new(method: McpAuthMethod) -> Self {
        Self {
            method,
            ..Default::default()
        }
    }

    /// No authentication required
    pub fn none() -> Self {
        Self::new(McpAuthMethod::None)
    }

    /// API token authentication
    pub fn api_token(auth: ApiTokenAuth) -> Self {
        Self::new(McpAuthMethod::ApiToken(auth))
    }

    /// JWT authentication
    pub fn jwt(auth: JwtAuth) -> Self {
        Self::new(McpAuthMethod::Jwt(auth))
    }

    /// OAuth2 authentication
    pub fn oauth2(auth: OAuth2Auth) -> Self {
        Self::new(McpAuthMethod::OAuth2(auth))
    }

    /// Allow any of the provided methods
    pub fn any_of(methods: Vec<McpAuthMethod>) -> Self {
        Self::new(McpAuthMethod::AnyOf(methods))
    }

    /// Custom authenticator
    pub fn custom(authenticator: Arc<dyn McpAuthenticator>) -> Self {
        Self::new(McpAuthMethod::Custom(authenticator))
    }

    pub fn allow_list_unauthenticated(mut self, allow: bool) -> Self {
        self.allow_list_unauthenticated = allow;
        self
    }

    pub fn allow_init_unauthenticated(mut self, allow: bool) -> Self {
        self.allow_init_unauthenticated = allow;
        self
    }
}

/// Authenticate a request using the configured method
pub async fn authenticate(
    config: &McpAuthConfig,
    headers: &std::collections::HashMap<String, String>,
    method: &str,
) -> Result<McpAuthContext> {
    // Check if this method allows unauthenticated access
    match method {
        "initialize" | "ping" if config.allow_init_unauthenticated => {
            return Ok(McpAuthContext::default());
        }
        "tools/list" | "resources/list" if config.allow_list_unauthenticated => {
            return Ok(McpAuthContext::default());
        }
        _ => {}
    }

    authenticate_with_method(&config.method, headers).await
}

fn authenticate_with_method<'a>(
    method: &'a McpAuthMethod,
    headers: &'a std::collections::HashMap<String, String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<McpAuthContext>> + Send + 'a>> {
    Box::pin(async move {
        match method {
            McpAuthMethod::None => Ok(McpAuthContext::default()),

            McpAuthMethod::ApiToken(auth) => authenticate_api_token(auth, headers).await,

            McpAuthMethod::Jwt(auth) => authenticate_jwt(auth, headers).await,

            McpAuthMethod::OAuth2(auth) => authenticate_oauth2(auth, headers).await,

            McpAuthMethod::AnyOf(methods) => {
                let mut last_error = None;
                for m in methods {
                    match authenticate_with_method(m, headers).await {
                        Ok(ctx) => return Ok(ctx),
                        Err(e) => last_error = Some(e),
                    }
                }
                Err(last_error.unwrap_or_else(|| {
                    McpError::InvalidRequest("No authentication methods configured".into())
                }))
            }

            McpAuthMethod::Custom(authenticator) => authenticator.authenticate(headers).await,
        }
    })
}

async fn authenticate_api_token(
    auth: &ApiTokenAuth,
    headers: &std::collections::HashMap<String, String>,
) -> Result<McpAuthContext> {
    // Get header (case-insensitive)
    let header_value = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(&auth.header_name))
        .map(|(_, v)| v)
        .ok_or_else(|| McpError::InvalidRequest(format!("Missing {} header", auth.header_name)))?;

    // Extract token
    let token = if auth.token_prefix.is_empty() {
        header_value.as_str()
    } else {
        header_value
            .strip_prefix(&auth.token_prefix)
            .ok_or_else(|| {
                McpError::InvalidRequest(format!(
                    "Invalid token format, expected prefix: {}",
                    auth.token_prefix
                ))
            })?
    };

    // Validate token
    if !auth.tokens.contains(token) {
        return Err(McpError::InvalidRequest("Invalid API token".into()));
    }

    Ok(McpAuthContext {
        subject: Some(format!("api-token:{}", &token[..token.len().min(8)])),
        scopes: vec!["*".to_string()], // API tokens get all scopes by default
        claims: serde_json::Value::Null,
        auth_method: "api_token".to_string(),
    })
}

/// Build (on first use) and thereafter reuse the verify-only `JwtManager` for
/// a given `JwtAuth`. The manager is a pure function of `auth`'s key
/// material, issuer/audience, and algorithm - none of which change after
/// construction - so it is safe to cache for the lifetime of the `JwtAuth`
/// (and its clones, via the shared `Arc<OnceLock<_>>`).
///
/// Verification behavior is unchanged: same algorithm pinning, same
/// signature/expiration checks performed by `armature_jwt`, same error
/// mapping to `McpError::InvalidRequest` on construction failure.
fn get_or_build_jwt_manager(auth: &JwtAuth) -> Result<&armature_jwt::JwtManager> {
    if let Some(manager) = auth.jwt_manager.get() {
        return Ok(manager);
    }

    let algorithm = parse_jwt_algorithm(&auth.algorithm)?;

    let mut config = match algorithm {
        armature_jwt::Algorithm::HS256
        | armature_jwt::Algorithm::HS384
        | armature_jwt::Algorithm::HS512 => {
            let secret = auth.secret.clone().ok_or_else(|| {
                McpError::InvalidRequest("JWT auth is missing a configured secret".into())
            })?;
            armature_jwt::JwtConfig::new(secret).with_algorithm(algorithm)
        }
        _ => {
            let public_key = auth.public_key.clone().ok_or_else(|| {
                McpError::InvalidRequest("JWT auth is missing a configured public key".into())
            })?;
            let mut config = armature_jwt::JwtConfig::new(String::new()).with_algorithm(algorithm);
            config.secret = None;
            config.public_key = Some(public_key);
            config
        }
    };

    if let Some(iss) = &auth.issuer {
        config = config.with_issuer(iss.clone());
    }
    if let Some(aud) = &auth.audience {
        config = config.with_audience(vec![aud.clone()]);
    }

    let manager = armature_jwt::JwtManager::verify_only(config)
        .map_err(|e| McpError::InvalidRequest(format!("Invalid JWT auth configuration: {e}")))?;

    // `OnceLock::get_or_init` would require an infallible closure; since
    // building the manager is fallible, set explicitly. If another thread
    // won the race, `set` returns the value back as `Err`, which is fine -
    // both are equivalent managers for the same immutable config, so we just
    // fall back to whatever ended up in the cell.
    let _ = auth.jwt_manager.set(manager);
    Ok(auth
        .jwt_manager
        .get()
        .expect("jwt_manager was just set or already initialized"))
}

async fn authenticate_jwt(
    auth: &JwtAuth,
    headers: &std::collections::HashMap<String, String>,
) -> Result<McpAuthContext> {
    // Get Authorization header
    let header_value = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v)
        .ok_or_else(|| McpError::InvalidRequest("Missing Authorization header".into()))?;

    // Extract token
    let token = header_value
        .strip_prefix("Bearer ")
        .ok_or_else(|| McpError::InvalidRequest("Invalid Bearer token format".into()))?;

    // Reject anything that isn't even shaped like a JWT before doing any
    // cryptographic work.
    if token.split('.').count() != 3 {
        return Err(McpError::InvalidRequest("Invalid JWT format".into()));
    }

    // Build a verify-only armature-jwt manager from this JwtAuth's configured
    // key material and algorithm, then VERIFY the signature (and expiration)
    // before trusting anything in the payload. This is the only path into
    // the claims below - a token that fails verification never reaches it.
    //
    // `JwtAuth` is a pure function of its (immutable, post-construction)
    // fields, so the manager - which re-parses PEM key material and rebuilds
    // a `Validation` internally - only needs to be built once per `JwtAuth`
    // and reused across every call, instead of being rebuilt (and re-logged)
    // on every single request.
    let manager = get_or_build_jwt_manager(auth)?;

    let claims: serde_json::Value = manager
        .verify(token)
        .map_err(|e| McpError::InvalidRequest(format!("JWT verification failed: {e}")))?;

    // Extract subject
    let subject = claims.get("sub").and_then(|v| v.as_str()).map(String::from);

    // Extract scopes
    let scopes = extract_scopes(&claims, &auth.scope_claim);

    // Check required scopes
    if !auth.required_scopes.is_empty() {
        let has_required = auth.required_scopes.iter().all(|s| scopes.contains(s));
        if !has_required {
            return Err(McpError::InvalidRequest("Insufficient scopes".into()));
        }
    }

    // Validate issuer if configured
    if let Some(expected_iss) = &auth.issuer {
        let actual_iss = claims.get("iss").and_then(|v| v.as_str());
        if actual_iss != Some(expected_iss.as_str()) {
            return Err(McpError::InvalidRequest("Invalid JWT issuer".into()));
        }
    }

    // Validate audience if configured
    if let Some(expected_aud) = &auth.audience {
        let aud_valid = match claims.get("aud") {
            Some(serde_json::Value::String(s)) => s == expected_aud,
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .any(|v| v.as_str() == Some(expected_aud.as_str())),
            _ => false,
        };
        if !aud_valid {
            return Err(McpError::InvalidRequest("Invalid JWT audience".into()));
        }
    }

    // Check expiration
    if let Some(exp) = claims.get("exp").and_then(|v| v.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        if exp < now {
            return Err(McpError::InvalidRequest("JWT has expired".into()));
        }
    }

    Ok(McpAuthContext {
        subject,
        scopes,
        claims,
        auth_method: "jwt".to_string(),
    })
}

async fn authenticate_oauth2(
    auth: &OAuth2Auth,
    headers: &std::collections::HashMap<String, String>,
) -> Result<McpAuthContext> {
    // Get Authorization header
    let header_value = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v)
        .ok_or_else(|| McpError::InvalidRequest("Missing Authorization header".into()))?;

    // Extract token
    let token = header_value
        .strip_prefix("Bearer ")
        .ok_or_else(|| McpError::InvalidRequest("Invalid Bearer token format".into()))?;

    // Serve a still-valid, previously validated token from the cache without
    // an HTTP round-trip. Only successes are ever cached (fail-closed).
    if let Some(ctx) = auth.cache_lookup(token) {
        return Ok(ctx);
    }

    // Validate via user info or introspection endpoint, whichever is configured.
    let ctx = if let Some(user_info_url) = &auth.user_info_url {
        validate_oauth2_via_userinfo(user_info_url, token, auth).await?
    } else if let Some(introspection_url) = &auth.introspection_url {
        validate_oauth2_via_introspection(introspection_url, token, auth).await?
    } else {
        return Err(McpError::InvalidRequest(
            "OAuth2 validation endpoint not configured".into(),
        ));
    };

    // Cache only on success; failures above short-circuit via `?` and are
    // never cached, so an invalid token is never served from cache.
    auth.cache_store(token, &ctx);
    Ok(ctx)
}

/// Timeout applied to OAuth2 validation requests (fail closed on expiry).
const OAUTH2_HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Shared HTTP client for OAuth2 validation requests.
fn oauth2_http_client() -> &'static armature_http_client::HttpClient {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<armature_http_client::HttpClient> = OnceLock::new();
    CLIENT.get_or_init(|| {
        armature_http_client::HttpClient::new(
            armature_http_client::HttpClientConfig::builder()
                .timeout(OAUTH2_HTTP_TIMEOUT)
                .build(),
        )
    })
}

/// Enforce the configured required scopes against the scopes granted to the
/// token. Fails closed: if scopes are required but none were returned by the
/// authorization server, the request is rejected.
fn check_required_scopes(auth: &OAuth2Auth, scopes: &[String]) -> Result<()> {
    if !auth.required_scopes.is_empty() {
        let has_required = auth
            .required_scopes
            .iter()
            .all(|s| scopes.iter().any(|granted| granted == s || granted == "*"));
        if !has_required {
            return Err(McpError::InvalidRequest("Insufficient scopes".into()));
        }
    }
    Ok(())
}

/// Validate an OAuth2 access token by calling the user info endpoint.
///
/// A `200 OK` response means the token is valid; the JSON body is used to
/// extract the subject (`sub`) and any advertised scopes. Any transport
/// error, timeout, or non-success status fails closed.
async fn validate_oauth2_via_userinfo(
    url: &str,
    token: &str,
    auth: &OAuth2Auth,
) -> Result<McpAuthContext> {
    let response = oauth2_http_client()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| McpError::InvalidRequest(format!("OAuth2 user info request failed: {}", e)))?;

    if !response.is_success() {
        return Err(McpError::InvalidRequest(format!(
            "OAuth2 token rejected by user info endpoint (status {})",
            response.status().as_u16()
        )));
    }

    let claims: serde_json::Value = response
        .json()
        .map_err(|_| McpError::InvalidRequest("Invalid OAuth2 user info response body".into()))?;

    let subject = claims.get("sub").and_then(|v| v.as_str()).map(String::from);
    let scopes = extract_scopes(&claims, "scope");

    // Note: most user info endpoints do not echo granted scopes, so
    // configuring `required_scopes` together with user info validation will
    // reject tokens unless the endpoint returns them (fail closed).
    check_required_scopes(auth, &scopes)?;

    Ok(McpAuthContext {
        subject,
        scopes,
        claims,
        auth_method: "oauth2_userinfo".to_string(),
    })
}

/// Validate an OAuth2 access token via RFC 7662 token introspection.
///
/// POSTs `token=<token>` as a form to the introspection endpoint,
/// authenticating with the configured client credentials (HTTP Basic) when
/// present. The token is accepted only if the endpoint returns a JSON body
/// with `"active": true`. Any transport error, timeout, non-success status,
/// or malformed body fails closed.
async fn validate_oauth2_via_introspection(
    url: &str,
    token: &str,
    auth: &OAuth2Auth,
) -> Result<McpAuthContext> {
    let mut request = oauth2_http_client()
        .post(url)
        .form(&[("token", token), ("token_type_hint", "access_token")])
        .map_err(|e| {
            McpError::InvalidRequest(format!(
                "failed to build OAuth2 introspection request: {}",
                e
            ))
        })?;

    // RFC 7662 requires the caller to authenticate; use HTTP Basic with the
    // configured client credentials when available.
    if let Some(client_id) = &auth.client_id {
        request = request.basic_auth(client_id, auth.client_secret.as_deref());
    }

    let response = request.send().await.map_err(|e| {
        McpError::InvalidRequest(format!("OAuth2 introspection request failed: {}", e))
    })?;

    if !response.is_success() {
        return Err(McpError::InvalidRequest(format!(
            "OAuth2 introspection endpoint returned status {}",
            response.status().as_u16()
        )));
    }

    let body: serde_json::Value = response.json().map_err(|_| {
        McpError::InvalidRequest("Invalid OAuth2 introspection response body".into())
    })?;

    // RFC 7662: only an explicit `"active": true` means the token is valid.
    if body.get("active").and_then(|v| v.as_bool()) != Some(true) {
        return Err(McpError::InvalidRequest(
            "OAuth2 token is not active".into(),
        ));
    }

    let subject = body
        .get("sub")
        .or_else(|| body.get("username"))
        .or_else(|| body.get("client_id"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let scopes = extract_scopes(&body, "scope");

    check_required_scopes(auth, &scopes)?;

    Ok(McpAuthContext {
        subject,
        scopes,
        claims: body,
        auth_method: "oauth2_introspection".to_string(),
    })
}

/// Parse a `JwtAuth::algorithm` string (e.g. `"HS256"`) into the
/// `armature_jwt`/`jsonwebtoken` `Algorithm` enum used to actually verify
/// tokens.
fn parse_jwt_algorithm(algorithm: &str) -> Result<armature_jwt::Algorithm> {
    use armature_jwt::Algorithm;
    match algorithm {
        "HS256" => Ok(Algorithm::HS256),
        "HS384" => Ok(Algorithm::HS384),
        "HS512" => Ok(Algorithm::HS512),
        "RS256" => Ok(Algorithm::RS256),
        "RS384" => Ok(Algorithm::RS384),
        "RS512" => Ok(Algorithm::RS512),
        "ES256" => Ok(Algorithm::ES256),
        "ES384" => Ok(Algorithm::ES384),
        "PS256" => Ok(Algorithm::PS256),
        "PS384" => Ok(Algorithm::PS384),
        "PS512" => Ok(Algorithm::PS512),
        "EdDSA" => Ok(Algorithm::EdDSA),
        other => Err(McpError::InvalidRequest(format!(
            "Unsupported JWT algorithm: {other}"
        ))),
    }
}

fn extract_scopes(claims: &serde_json::Value, scope_claim: &str) -> Vec<String> {
    // Try the configured claim name
    if let Some(scope_value) = claims.get(scope_claim) {
        return parse_scope_value(scope_value);
    }

    // Try common alternatives
    for claim_name in &["scope", "scopes", "scp"] {
        if let Some(scope_value) = claims.get(*claim_name) {
            return parse_scope_value(scope_value);
        }
    }

    Vec::new()
}

fn parse_scope_value(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => s.split_whitespace().map(String::from).collect(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_token_auth_builder() {
        let auth = ApiTokenAuth::new()
            .with_tokens(vec!["token1", "token2"])
            .add_token("token3")
            .with_header("X-API-Key")
            .with_prefix("");

        assert_eq!(auth.tokens.len(), 3);
        assert_eq!(auth.header_name, "X-API-Key");
        assert!(auth.token_prefix.is_empty());
    }

    #[test]
    fn test_jwt_auth_builder() {
        let auth = JwtAuth::new()
            .with_secret("my-secret")
            .with_issuer("my-app")
            .with_audience("mcp-clients")
            .with_scopes(vec!["mcp:read", "mcp:write"]);

        assert_eq!(auth.secret, Some("my-secret".to_string()));
        assert_eq!(auth.issuer, Some("my-app".to_string()));
        assert_eq!(auth.required_scopes.len(), 2);
    }

    #[test]
    fn test_mcp_auth_context_scopes() {
        let ctx = McpAuthContext {
            subject: Some("user:123".to_string()),
            scopes: vec!["mcp:read".to_string(), "mcp:write".to_string()],
            claims: serde_json::Value::Null,
            auth_method: "jwt".to_string(),
        };

        assert!(ctx.has_scope("mcp:read"));
        assert!(ctx.has_scope("mcp:write"));
        assert!(!ctx.has_scope("mcp:admin"));
    }

    #[tokio::test]
    async fn test_api_token_authentication() {
        let auth = ApiTokenAuth::new()
            .with_tokens(vec!["valid-token-123"])
            .with_header("Authorization")
            .with_prefix("Bearer ");

        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer valid-token-123".to_string(),
        );

        let result = authenticate_api_token(&auth, &headers).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().auth_method, "api_token");
    }

    #[tokio::test]
    async fn test_api_token_authentication_invalid() {
        let auth = ApiTokenAuth::new().with_tokens(vec!["valid-token-123"]);

        let mut headers = std::collections::HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer wrong-token".to_string(),
        );

        let result = authenticate_api_token(&auth, &headers).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_no_auth_allows_all() {
        let config = McpAuthConfig::none();
        let headers = std::collections::HashMap::new();

        let result = authenticate(&config, &headers, "tools/call").await;
        assert!(result.is_ok());
    }

    /// Mint a signed HS256 JWT via armature-jwt, carrying an `exp` claim
    /// `seconds_from_now` seconds in the future (negative for already-expired).
    fn mint_jwt(secret: &str, sub: &str, scope: &str, seconds_from_now: i64) -> String {
        let config = armature_jwt::JwtConfig::new(secret.to_string());
        let manager = armature_jwt::JwtManager::new(config).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let claims = serde_json::json!({
            "sub": sub,
            "scope": scope,
            "exp": now + seconds_from_now,
        });
        manager.sign(&claims).unwrap()
    }

    #[tokio::test]
    async fn jwt_auth_rejects_bad_signature() {
        // JwtAuth is configured with the correct secret, but the token is
        // signed with a different one. This must NOT be accepted just
        // because the payload happens to decode to well-formed JSON.
        let auth = JwtAuth::new().with_secret("correct-secret");
        let forged = mint_jwt("wrong-secret", "attacker", "mcp:read mcp:write", 3600);

        let result = authenticate_jwt(&auth, &bearer_headers(&forged)).await;
        assert!(
            result.is_err(),
            "a token signed with the wrong secret must be rejected"
        );
    }

    #[tokio::test]
    async fn jwt_auth_accepts_valid_signature_and_enforces_exp() {
        let auth = JwtAuth::new().with_secret("correct-secret");

        // Correctly signed, unexpired token -> accepted with the right claims.
        let valid = mint_jwt("correct-secret", "user-42", "mcp:read", 3600);
        let ctx = authenticate_jwt(&auth, &bearer_headers(&valid))
            .await
            .expect("correctly signed, unexpired token should be accepted");
        assert_eq!(ctx.subject, Some("user-42".to_string()));
        assert_eq!(ctx.scopes, vec!["mcp:read".to_string()]);
        assert_eq!(ctx.auth_method, "jwt");

        // Correctly signed but expired token -> rejected.
        let expired = mint_jwt("correct-secret", "user-42", "mcp:read", -3600);
        let result = authenticate_jwt(&auth, &bearer_headers(&expired)).await;
        assert!(result.is_err(), "an expired token must be rejected");
    }

    /// Base64url encode (no padding), used to hand-craft JWTs that no signing
    /// helper would ever produce (e.g. `alg:none`), so we can assert the
    /// verifier rejects them regardless of what the header *claims*.
    fn b64url(input: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                out.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(CHARS[(n & 0x3F) as usize] as char);
            }
        }
        out
    }

    #[tokio::test]
    async fn jwt_auth_rejects_alg_none() {
        // Classic "alg:none" algorithm-confusion attack: a token that
        // declares no signature algorithm and carries an empty signature
        // segment. A JwtAuth configured for HS256 must never accept this,
        // no matter how well-formed the claims are.
        let auth = JwtAuth::new().with_secret("correct-secret");

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let header = b64url(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = b64url(
            serde_json::json!({
                "sub": "attacker",
                "scope": "mcp:read mcp:write",
                "exp": now + 3600,
            })
            .to_string()
            .as_bytes(),
        );
        let alg_none_token = format!("{header}.{payload}.");

        let result = authenticate_jwt(&auth, &bearer_headers(&alg_none_token)).await;
        assert!(
            result.is_err(),
            "an alg:none token with an empty signature must be rejected"
        );
    }

    #[tokio::test]
    async fn jwt_auth_rejects_algorithm_mismatch() {
        // The JwtAuth is configured for HS256, but the token's header claims
        // a different algorithm (RS256). This must be rejected even before
        // signature verification is attempted - accepting it would open the
        // door to classic HMAC/RSA algorithm-confusion attacks.
        let auth = JwtAuth::new().with_secret("correct-secret");

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload = b64url(
            serde_json::json!({
                "sub": "attacker",
                "scope": "mcp:read mcp:write",
                "exp": now + 3600,
            })
            .to_string()
            .as_bytes(),
        );
        // Signature content is irrelevant - a mismatched `alg` must be
        // rejected regardless of what's in the signature segment.
        let mismatched_token = format!("{header}.{payload}.bm90LWEtcmVhbC1zaWc");

        let result = authenticate_jwt(&auth, &bearer_headers(&mismatched_token)).await;
        assert!(
            result.is_err(),
            "a token whose header alg doesn't match the configured algorithm must be rejected"
        );
    }

    #[tokio::test]
    async fn jwt_auth_cached_manager_is_consistent() {
        // The JwtManager backing a JwtAuth is built lazily and cached in an
        // `Arc<OnceLock<..>>`. Repeated calls on the same JwtAuth must reuse
        // that cached manager without capturing stale/wrong key material, so
        // two sequential authentications of the same valid token must both
        // succeed and produce identical claims.
        let auth = JwtAuth::new().with_secret("correct-secret");
        let token = mint_jwt("correct-secret", "user-42", "mcp:read", 3600);

        let first = authenticate_jwt(&auth, &bearer_headers(&token))
            .await
            .expect("first call with a valid token should succeed");
        let second = authenticate_jwt(&auth, &bearer_headers(&token))
            .await
            .expect("second call on the same JwtAuth with the same valid token should succeed");

        assert_eq!(first.subject, second.subject);
        assert_eq!(first.scopes, second.scopes);
        assert_eq!(first.auth_method, second.auth_method);
    }

    /// Spawn a minimal HTTP/1.1 stub server that answers every connection
    /// with `status` and `body`. Returns the base URL (`http://127.0.0.1:port`).
    async fn spawn_stub_server(status: u16, body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    // Read the full request: headers, then Content-Length body bytes.
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);

                        if let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&data[..header_end]);
                            let content_length = headers
                                .lines()
                                .find_map(|l| {
                                    let (name, value) = l.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            if data.len() >= header_end + 4 + content_length {
                                break;
                            }
                        }
                    }

                    let reason = match status {
                        200 => "OK",
                        401 => "Unauthorized",
                        _ => "Error",
                    };
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status,
                        reason,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });

        format!("http://{}", addr)
    }

    /// Like [`spawn_stub_server`] but also returns a counter incremented once
    /// per fully received HTTP request, so tests can assert how many times the
    /// endpoint was actually hit.
    async fn spawn_counting_stub_server(
        status: u16,
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let counter_srv = counter.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let counter_conn = counter_srv.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            break;
                        }
                        data.extend_from_slice(&buf[..n]);

                        if let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&data[..header_end]);
                            let content_length = headers
                                .lines()
                                .find_map(|l| {
                                    let (name, value) = l.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            if data.len() >= header_end + 4 + content_length {
                                break;
                            }
                        }
                    }

                    counter_conn.fetch_add(1, Ordering::SeqCst);

                    let reason = match status {
                        200 => "OK",
                        401 => "Unauthorized",
                        _ => "Error",
                    };
                    let response = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status,
                        reason,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });

        (format!("http://{}", addr), counter)
    }

    /// Return a URL pointing at a port with no listener.
    async fn unreachable_url() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{}", addr)
    }

    fn bearer_headers(token: &str) -> std::collections::HashMap<String, String> {
        let mut headers = std::collections::HashMap::new();
        headers.insert("Authorization".to_string(), format!("Bearer {}", token));
        headers
    }

    #[tokio::test]
    async fn test_oauth2_userinfo_valid_token() {
        let url = spawn_stub_server(200, r#"{"sub":"user-1","scope":"mcp:read"}"#).await;
        let auth = OAuth2Auth::new().with_user_info(url);

        let ctx = authenticate_oauth2(&auth, &bearer_headers("good-token"))
            .await
            .unwrap();
        assert_eq!(ctx.subject, Some("user-1".to_string()));
        assert_eq!(ctx.scopes, vec!["mcp:read".to_string()]);
        assert_eq!(ctx.auth_method, "oauth2_userinfo");
    }

    #[tokio::test]
    async fn test_oauth2_userinfo_rejected_token() {
        let url = spawn_stub_server(401, r#"{"error":"invalid_token"}"#).await;
        let auth = OAuth2Auth::new().with_user_info(url);

        let result = authenticate_oauth2(&auth, &bearer_headers("bad-token")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oauth2_userinfo_unreachable_fails_closed() {
        let url = unreachable_url().await;
        let auth = OAuth2Auth::new().with_user_info(url);

        let result = authenticate_oauth2(&auth, &bearer_headers("any-token")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oauth2_introspection_active_token() {
        let url = spawn_stub_server(
            200,
            r#"{"active":true,"sub":"svc-1","scope":"mcp:read mcp:write"}"#,
        )
        .await;
        let auth = OAuth2Auth::new().with_introspection(url, "client-id", "client-secret");

        let ctx = authenticate_oauth2(&auth, &bearer_headers("good-token"))
            .await
            .unwrap();
        assert_eq!(ctx.subject, Some("svc-1".to_string()));
        assert_eq!(
            ctx.scopes,
            vec!["mcp:read".to_string(), "mcp:write".to_string()]
        );
        assert_eq!(ctx.auth_method, "oauth2_introspection");
    }

    #[tokio::test]
    async fn test_oauth2_introspection_inactive_token() {
        let url = spawn_stub_server(200, r#"{"active":false}"#).await;
        let auth = OAuth2Auth::new().with_introspection(url, "client-id", "client-secret");

        let result = authenticate_oauth2(&auth, &bearer_headers("revoked-token")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oauth2_introspection_unreachable_fails_closed() {
        let url = unreachable_url().await;
        let auth = OAuth2Auth::new().with_introspection(url, "client-id", "client-secret");

        let result = authenticate_oauth2(&auth, &bearer_headers("any-token")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oauth2_introspection_insufficient_scopes() {
        let url = spawn_stub_server(200, r#"{"active":true,"scope":"other"}"#).await;
        let auth = OAuth2Auth::new()
            .with_introspection(url, "client-id", "client-secret")
            .with_scopes(vec!["mcp:read"]);

        let result = authenticate_oauth2(&auth, &bearer_headers("token")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_oauth2_cache_hits_avoid_second_request() {
        use std::sync::atomic::Ordering;
        let (url, count) =
            spawn_counting_stub_server(200, r#"{"sub":"user-1","scope":"mcp:read"}"#).await;
        let auth = OAuth2Auth::new().with_user_info(url).with_cache_ttl(300);

        let first = authenticate_oauth2(&auth, &bearer_headers("cached-token"))
            .await
            .unwrap();
        assert_eq!(first.subject, Some("user-1".to_string()));

        let second = authenticate_oauth2(&auth, &bearer_headers("cached-token"))
            .await
            .unwrap();
        assert_eq!(second.subject, Some("user-1".to_string()));

        // Second validation was served from cache: only one HTTP request.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_oauth2_no_cache_hits_server_each_time() {
        use std::sync::atomic::Ordering;
        let (url, count) =
            spawn_counting_stub_server(200, r#"{"sub":"user-1","scope":"mcp:read"}"#).await;
        let auth = OAuth2Auth::new().with_user_info(url).no_cache();

        authenticate_oauth2(&auth, &bearer_headers("uncached-token"))
            .await
            .unwrap();
        authenticate_oauth2(&auth, &bearer_headers("uncached-token"))
            .await
            .unwrap();

        // Caching disabled: every request hits the endpoint.
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_oauth2_expired_entry_rehits_server() {
        use std::sync::atomic::Ordering;
        let (url, count) =
            spawn_counting_stub_server(200, r#"{"sub":"user-1","scope":"mcp:read"}"#).await;
        let auth = OAuth2Auth::new().with_user_info(url).with_cache_ttl(300);

        // First validation populates the cache.
        authenticate_oauth2(&auth, &bearer_headers("aging-token"))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);

        // Force the cached entry to be expired by rewriting its expiry into the
        // past (the field is private but reachable from the in-module test).
        {
            let key = hash_token("aging-token");
            let mut map = auth.cache.lock().unwrap();
            let entry = map.get_mut(&key).expect("token should be cached");
            entry.1 = Instant::now() - Duration::from_secs(1);
        }

        // Expired entry is evicted on lookup, so the endpoint is hit again.
        authenticate_oauth2(&auth, &bearer_headers("aging-token"))
            .await
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_oauth2_invalid_token_never_cached() {
        use std::sync::atomic::Ordering;
        // 401 => invalid token; must never be served from cache (fail-closed).
        let (url, count) = spawn_counting_stub_server(401, r#"{"error":"invalid_token"}"#).await;
        let auth = OAuth2Auth::new().with_user_info(url).with_cache_ttl(300);

        assert!(
            authenticate_oauth2(&auth, &bearer_headers("bad-token"))
                .await
                .is_err()
        );
        assert!(
            authenticate_oauth2(&auth, &bearer_headers("bad-token"))
                .await
                .is_err()
        );

        // Both attempts hit the server: negatives are not cached.
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(auth.cache.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_init_unauthenticated_by_default() {
        let auth = ApiTokenAuth::new().with_tokens(vec!["secret"]);
        let config = McpAuthConfig::api_token(auth);
        let headers = std::collections::HashMap::new();

        // Initialize should work without auth by default
        let result = authenticate(&config, &headers, "initialize").await;
        assert!(result.is_ok());
    }
}
