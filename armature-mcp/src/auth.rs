//! MCP Authentication and Authorization
//!
//! Provides configurable authentication for MCP endpoints using:
//! - API tokens (generated bearer tokens)
//! - JWT validation
//! - OAuth2 access tokens

use crate::error::{McpError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

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
#[derive(Debug, Clone)]
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

    // Decode JWT (without full verification for now - integrate with armature-jwt for production)
    // This is a simplified implementation - in production, use armature-jwt's JwtManager
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(McpError::InvalidRequest("Invalid JWT format".into()));
    }

    // Decode payload (base64url)
    let payload = base64_decode_url_safe(parts[1])
        .map_err(|_| McpError::InvalidRequest("Invalid JWT payload encoding".into()))?;

    let claims: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|_| McpError::InvalidRequest("Invalid JWT payload JSON".into()))?;

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

    // Validate via user info endpoint if configured
    if let Some(user_info_url) = &auth.user_info_url {
        return validate_oauth2_via_userinfo(user_info_url, token, auth).await;
    }

    // Validate via introspection endpoint if configured
    if let Some(introspection_url) = &auth.introspection_url {
        return validate_oauth2_via_introspection(introspection_url, token, auth).await;
    }

    Err(McpError::InvalidRequest(
        "OAuth2 validation endpoint not configured".into(),
    ))
}

async fn validate_oauth2_via_userinfo(
    url: &str,
    token: &str,
    auth: &OAuth2Auth,
) -> Result<McpAuthContext> {
    // Note: In production, use reqwest or similar HTTP client
    // This is a placeholder that would need actual HTTP implementation
    let _ = (url, token, auth);

    // For now, return a placeholder - in real implementation:
    // 1. Make HTTP GET to user_info_url with Bearer token
    // 2. Parse response for user info
    // 3. Extract subject and scopes

    Err(McpError::InvalidRequest(
        "OAuth2 user info validation requires HTTP client integration. \
         Consider using armature-http-client or integrating with armature-auth OAuth2 provider."
            .into(),
    ))
}

async fn validate_oauth2_via_introspection(
    url: &str,
    token: &str,
    auth: &OAuth2Auth,
) -> Result<McpAuthContext> {
    let _ = (url, token, auth);

    // For now, return a placeholder - in real implementation:
    // 1. Make HTTP POST to introspection_url with token and client credentials
    // 2. Parse response for active status, subject, scopes

    Err(McpError::InvalidRequest(
        "OAuth2 introspection validation requires HTTP client integration. \
         Consider using armature-http-client or integrating with armature-auth OAuth2 provider."
            .into(),
    ))
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

fn base64_decode_url_safe(input: &str) -> std::result::Result<Vec<u8>, ()> {
    // Add padding if needed
    let padded = match input.len() % 4 {
        2 => format!("{}==", input),
        3 => format!("{}=", input),
        _ => input.to_string(),
    };

    // Replace URL-safe characters
    let standard = padded.replace('-', "+").replace('_', "/");

    // Decode
    base64_decode(&standard)
}

fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, ()> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits_collected = 0;

    for c in input.chars() {
        if c == '=' {
            break;
        }

        let value = ALPHABET.iter().position(|&b| b == c as u8).ok_or(())?;
        buffer = (buffer << 6) | (value as u32);
        bits_collected += 6;

        if bits_collected >= 8 {
            bits_collected -= 8;
            output.push((buffer >> bits_collected) as u8);
            buffer &= (1 << bits_collected) - 1;
        }
    }

    Ok(output)
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

    #[test]
    fn test_base64_decode() {
        let decoded = base64_decode_url_safe("SGVsbG8gV29ybGQ").unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "Hello World");
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
