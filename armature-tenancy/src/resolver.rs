//! Tenant Resolution
//!
//! Strategies for resolving tenant from HTTP requests.

use crate::tenant::Tenant;
use armature_core::HttpRequest;
use async_trait::async_trait;
use regex::Regex;
use std::sync::Arc;

/// Tenant resolution errors
#[derive(Debug, thiserror::Error)]
pub enum TenantError {
    #[error("Tenant not found: {0}")]
    NotFound(String),

    #[error("Invalid tenant identifier: {0}")]
    Invalid(String),

    #[error("Tenant resolution failed: {0}")]
    ResolutionFailed(String),

    #[error("Tenant is inactive")]
    Inactive,

    #[error("Storage error: {0}")]
    Storage(String),
}

/// Tenant resolver trait
///
/// Implement this trait to provide tenant resolution logic.
/// Users must inject their own tenant store via DI.
#[async_trait]
pub trait TenantResolver: Send + Sync {
    /// Resolve tenant from request
    async fn resolve(&self, request: &HttpRequest) -> Result<Tenant, TenantError>;
}

/// Tenant store trait (implement with your database)
///
/// Users provide their own implementation using their database of choice.
#[async_trait]
pub trait TenantStore: Send + Sync {
    /// Find tenant by ID
    async fn find_by_id(&self, id: &str) -> Result<Option<Tenant>, TenantError>;

    /// Find tenant by name/slug
    async fn find_by_name(&self, name: &str) -> Result<Option<Tenant>, TenantError>;

    /// Find tenant by domain
    async fn find_by_domain(&self, domain: &str) -> Result<Option<Tenant>, TenantError>;
}

/// Header-based tenant resolver
///
/// Resolves tenant from a request header (e.g., `X-Tenant-ID`).
pub struct HeaderTenantResolver {
    store: Arc<dyn TenantStore>,
    header_name: String,
}

impl HeaderTenantResolver {
    /// Create new header-based resolver
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::HeaderTenantResolver;
    /// use std::sync::Arc;
    ///
    /// # struct MyTenantStore;
    /// # #[async_trait::async_trait]
    /// # impl armature_tenancy::TenantStore for MyTenantStore {
    /// #     async fn find_by_id(&self, id: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_name(&self, name: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_domain(&self, domain: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// # }
    /// let store: Arc<dyn armature_tenancy::TenantStore> = Arc::new(MyTenantStore);
    /// let resolver = HeaderTenantResolver::new(store, "X-Tenant-ID");
    /// ```
    pub fn new(store: Arc<dyn TenantStore>, header_name: impl Into<String>) -> Self {
        Self {
            store,
            header_name: header_name.into(),
        }
    }
}

#[async_trait]
impl TenantResolver for HeaderTenantResolver {
    async fn resolve(&self, request: &HttpRequest) -> Result<Tenant, TenantError> {
        // `HeaderMap::get` is already case-insensitive, so lowercasing the
        // configured name here would allocate a `String` per request for nothing.
        let tenant_id = request.headers.get(&self.header_name).ok_or_else(|| {
            TenantError::NotFound(format!("Missing header: {}", self.header_name))
        })?;

        let tenant = self
            .store
            .find_by_id(tenant_id)
            .await?
            .ok_or_else(|| TenantError::NotFound(tenant_id.to_owned()))?;

        if !tenant.active {
            return Err(TenantError::Inactive);
        }

        Ok(tenant)
    }
}

/// Subdomain-based tenant resolver
///
/// Resolves tenant from subdomain (e.g., `acme.example.com` -> tenant "acme").
pub struct SubdomainTenantResolver {
    store: Arc<dyn TenantStore>,
    base_domain: String,
}

impl SubdomainTenantResolver {
    /// Create new subdomain-based resolver
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::SubdomainTenantResolver;
    /// use std::sync::Arc;
    ///
    /// # struct MyTenantStore;
    /// # #[async_trait::async_trait]
    /// # impl armature_tenancy::TenantStore for MyTenantStore {
    /// #     async fn find_by_id(&self, id: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_name(&self, name: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_domain(&self, domain: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// # }
    /// let store: Arc<dyn armature_tenancy::TenantStore> = Arc::new(MyTenantStore);
    /// let resolver = SubdomainTenantResolver::new(store, "example.com");
    /// ```
    pub fn new(store: Arc<dyn TenantStore>, base_domain: impl Into<String>) -> Self {
        Self {
            store,
            base_domain: base_domain.into(),
        }
    }

    /// Extract subdomain from host header
    fn extract_subdomain(&self, host: &str) -> Option<String> {
        // Remove port if present
        let host = host.split(':').next().unwrap_or(host);

        // Remove base domain
        if let Some(subdomain) = host.strip_suffix(&format!(".{}", self.base_domain))
            && !subdomain.is_empty()
            && !subdomain.contains('.')
        {
            return Some(subdomain.to_string());
        }

        None
    }
}

#[async_trait]
impl TenantResolver for SubdomainTenantResolver {
    async fn resolve(&self, request: &HttpRequest) -> Result<Tenant, TenantError> {
        let host = request
            .headers
            .get("host")
            .ok_or_else(|| TenantError::ResolutionFailed("Missing Host header".to_string()))?;

        let subdomain = self
            .extract_subdomain(host)
            .ok_or_else(|| TenantError::ResolutionFailed(format!("No subdomain in: {}", host)))?;

        let tenant = self
            .store
            .find_by_name(&subdomain)
            .await?
            .ok_or_else(|| TenantError::NotFound(subdomain.clone()))?;

        if !tenant.active {
            return Err(TenantError::Inactive);
        }

        Ok(tenant)
    }
}

/// JWT claim-based tenant resolver
///
/// Resolves tenant from JWT token claims.
///
/// # Security
///
/// **This resolver does NOT verify the JWT signature.** It only splits the
/// token and base64url-decodes the payload segment to read the configured
/// claim. Anyone can forge an unsigned payload, so this resolver MUST be
/// deployed behind authentication middleware (e.g. `armature-auth` /
/// `armature-jwt`) that has already verified the token's signature and
/// expiration. Tenant resolution from an unverified JWT is only safe
/// post-authentication; never use this resolver as an authentication or
/// authorization mechanism on its own.
///
/// Malformed tokens fail closed: any structural, encoding, or JSON error
/// results in a resolution error, never a default tenant.
pub struct JwtTenantResolver {
    store: Arc<dyn TenantStore>,
    claim_name: String,
}

impl JwtTenantResolver {
    /// Create new JWT-based resolver
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::JwtTenantResolver;
    /// use std::sync::Arc;
    ///
    /// # struct MyTenantStore;
    /// # #[async_trait::async_trait]
    /// # impl armature_tenancy::TenantStore for MyTenantStore {
    /// #     async fn find_by_id(&self, id: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_name(&self, name: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_domain(&self, domain: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// # }
    /// let store: Arc<dyn armature_tenancy::TenantStore> = Arc::new(MyTenantStore);
    /// let resolver = JwtTenantResolver::new(store, "tenant_id");
    /// ```
    pub fn new(store: Arc<dyn TenantStore>, claim_name: impl Into<String>) -> Self {
        Self {
            store,
            claim_name: claim_name.into(),
        }
    }
}

#[async_trait]
impl TenantResolver for JwtTenantResolver {
    async fn resolve(&self, request: &HttpRequest) -> Result<Tenant, TenantError> {
        // Extract JWT from Authorization header
        let auth_header = request.headers.get("authorization").ok_or_else(|| {
            TenantError::ResolutionFailed("Missing Authorization header".to_string())
        })?;

        // Extract token (assumes "Bearer <token>")
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| TenantError::Invalid("Invalid Authorization format".to_string()))?;

        // Parse the JWT payload WITHOUT signature verification (see the
        // security note on this type): an authentication layer must already
        // have verified this token before tenant resolution runs.
        let tenant_id = self.extract_claim(token)?;

        let tenant = self
            .store
            .find_by_id(&tenant_id)
            .await?
            .ok_or_else(|| TenantError::NotFound(tenant_id.clone()))?;

        if !tenant.active {
            return Err(TenantError::Inactive);
        }

        Ok(tenant)
    }
}

impl JwtTenantResolver {
    /// Extract the configured claim from the JWT payload.
    ///
    /// **Does NOT verify the signature** — see the security note on
    /// [`JwtTenantResolver`]. Fails closed on any malformed input.
    fn extract_claim(&self, token: &str) -> Result<String, TenantError> {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        // A compact JWS is exactly three dot-separated segments.
        let mut segments = token.split('.');
        let (Some(_header), Some(payload), Some(_signature), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err(TenantError::Invalid(
                "Malformed JWT: expected three segments".to_string(),
            ));
        };

        // Payload is base64url; tolerate (strip) any trailing padding.
        let payload_bytes = URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|_| TenantError::Invalid("Malformed JWT payload encoding".to_string()))?;

        let claims: serde_json::Value = serde_json::from_slice(&payload_bytes)
            .map_err(|_| TenantError::Invalid("Malformed JWT payload JSON".to_string()))?;

        match claims.get(&self.claim_name) {
            Some(serde_json::Value::String(value)) if !value.is_empty() => Ok(value.clone()),
            Some(_) => Err(TenantError::Invalid(format!(
                "JWT claim '{}' is not a non-empty string",
                self.claim_name
            ))),
            None => Err(TenantError::ResolutionFailed(format!(
                "JWT is missing the '{}' claim",
                self.claim_name
            ))),
        }
    }
}

/// Path-based tenant resolver
///
/// Resolves tenant from URL path (e.g., `/tenants/acme/users`).
pub struct PathTenantResolver {
    store: Arc<dyn TenantStore>,
    pattern: Regex,
    group_index: usize,
}

impl PathTenantResolver {
    /// Create new path-based resolver
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::PathTenantResolver;
    /// use std::sync::Arc;
    ///
    /// # struct MyTenantStore;
    /// # #[async_trait::async_trait]
    /// # impl armature_tenancy::TenantStore for MyTenantStore {
    /// #     async fn find_by_id(&self, id: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_name(&self, name: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// #     async fn find_by_domain(&self, domain: &str) -> Result<Option<armature_tenancy::Tenant>, armature_tenancy::TenantError> { Ok(None) }
    /// # }
    /// let store: Arc<dyn armature_tenancy::TenantStore> = Arc::new(MyTenantStore);
    /// let resolver = PathTenantResolver::new(store, r"^/tenants/([^/]+)", 1).unwrap();
    /// ```
    pub fn new(
        store: Arc<dyn TenantStore>,
        pattern: &str,
        group_index: usize,
    ) -> Result<Self, regex::Error> {
        Ok(Self {
            store,
            pattern: Regex::new(pattern)?,
            group_index,
        })
    }
}

#[async_trait]
impl TenantResolver for PathTenantResolver {
    async fn resolve(&self, request: &HttpRequest) -> Result<Tenant, TenantError> {
        // `path_only`, not `path`: the raw target still carries the query
        // string, which would otherwise be captured as part of the tenant id
        // (`/tenants/acme?page=2` -> `acme?page=2`) and fail to resolve.
        let captures = self
            .pattern
            .captures(request.path_only())
            .ok_or_else(|| TenantError::ResolutionFailed("Path pattern not matched".to_string()))?;

        let tenant_name = captures
            .get(self.group_index)
            .ok_or_else(|| TenantError::ResolutionFailed("Capture group not found".to_string()))?
            .as_str();

        let tenant = self
            .store
            .find_by_name(tenant_name)
            .await?
            .ok_or_else(|| TenantError::NotFound(tenant_name.to_string()))?;

        if !tenant.active {
            return Err(TenantError::Inactive);
        }

        Ok(tenant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockTenantStore {
        tenants: HashMap<String, Tenant>,
    }

    impl MockTenantStore {
        fn new() -> Self {
            let mut tenants = HashMap::new();
            tenants.insert(
                "tenant-1".to_string(),
                Tenant::new("tenant-1", "acme").with_domain("acme.example.com"),
            );
            tenants.insert(
                "tenant-2".to_string(),
                Tenant::new("tenant-2", "globex").with_domain("globex.example.com"),
            );
            Self { tenants }
        }
    }

    #[async_trait]
    impl TenantStore for MockTenantStore {
        async fn find_by_id(&self, id: &str) -> Result<Option<Tenant>, TenantError> {
            Ok(self.tenants.get(id).cloned())
        }

        async fn find_by_name(&self, name: &str) -> Result<Option<Tenant>, TenantError> {
            Ok(self.tenants.values().find(|t| t.name == name).cloned())
        }

        async fn find_by_domain(&self, domain: &str) -> Result<Option<Tenant>, TenantError> {
            Ok(self
                .tenants
                .values()
                .find(|t| t.domain.as_deref() == Some(domain))
                .cloned())
        }
    }

    fn create_request(method: &str, path: &str) -> HttpRequest {
        HttpRequest::new(method.to_string(), path.to_string())
    }

    #[tokio::test]
    async fn test_header_resolver() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = HeaderTenantResolver::new(store, "X-Tenant-ID");

        let mut request = create_request("GET", "/api/users");
        request
            .headers
            .insert("x-tenant-id", "tenant-1".to_string());

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.id, "tenant-1");
        assert_eq!(tenant.name, "acme");
    }

    #[tokio::test]
    async fn test_subdomain_resolver() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = SubdomainTenantResolver::new(store, "example.com");

        let mut request = create_request("GET", "/api/users");
        request
            .headers
            .insert("host", "acme.example.com".to_string());

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.name, "acme");
    }

    /// Build an (unsigned) JWT with the given JSON payload.
    fn make_jwt(payload: &serde_json::Value) -> String {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;

        format!(
            "{}.{}.{}",
            URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#),
            URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes()),
            URL_SAFE_NO_PAD.encode(b"signature")
        )
    }

    #[tokio::test]
    async fn test_jwt_resolver_valid_token() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = JwtTenantResolver::new(store, "tenant_id");

        let token = make_jwt(&serde_json::json!({ "sub": "user-1", "tenant_id": "tenant-1" }));
        let mut request = create_request("GET", "/api/users");
        request
            .headers
            .insert("authorization", format!("Bearer {}", token));

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.id, "tenant-1");
        assert_eq!(tenant.name, "acme");
    }

    #[tokio::test]
    async fn test_jwt_resolver_missing_claim() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = JwtTenantResolver::new(store, "tenant_id");

        let token = make_jwt(&serde_json::json!({ "sub": "user-1" }));
        let mut request = create_request("GET", "/api/users");
        request
            .headers
            .insert("authorization", format!("Bearer {}", token));

        let result = resolver.resolve(&request).await;
        assert!(matches!(result, Err(TenantError::ResolutionFailed(_))));
    }

    #[tokio::test]
    async fn test_jwt_resolver_non_string_claim() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = JwtTenantResolver::new(store, "tenant_id");

        let token = make_jwt(&serde_json::json!({ "tenant_id": 42 }));
        let mut request = create_request("GET", "/api/users");
        request
            .headers
            .insert("authorization", format!("Bearer {}", token));

        let result = resolver.resolve(&request).await;
        assert!(matches!(result, Err(TenantError::Invalid(_))));
    }

    #[tokio::test]
    async fn test_jwt_resolver_malformed_token() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = JwtTenantResolver::new(store, "tenant_id");

        for bad_token in [
            "not-a-jwt",
            "one.two",
            "one.two.three.four",
            "aaa.!!!not-base64!!!.ccc",
            // Valid base64url payload, but not JSON
            "aaa.bm90LWpzb24.ccc",
        ] {
            let mut request = create_request("GET", "/api/users");
            request
                .headers
                .insert("authorization", format!("Bearer {}", bad_token));

            let result = resolver.resolve(&request).await;
            assert!(
                matches!(result, Err(TenantError::Invalid(_))),
                "token {:?} should fail closed",
                bad_token
            );
        }
    }

    #[tokio::test]
    async fn test_jwt_resolver_missing_authorization_header() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = JwtTenantResolver::new(store, "tenant_id");

        let request = create_request("GET", "/api/users");
        let result = resolver.resolve(&request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_path_resolver() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = PathTenantResolver::new(store, r"^/tenants/([^/]+)", 1).unwrap();

        let request = create_request("GET", "/tenants/acme/users");

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.name, "acme");
    }

    #[tokio::test]
    async fn test_path_resolver_ignores_query_string() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = PathTenantResolver::new(store, r"^/tenants/([^/]+)", 1).unwrap();

        let request = create_request("GET", "/tenants/acme/users?page=2");

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.name, "acme");
    }

    #[tokio::test]
    async fn test_path_resolver_query_directly_after_tenant() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = PathTenantResolver::new(store, r"^/tenants/([^/]+)", 1).unwrap();

        let request = create_request("GET", "/tenants/acme?page=2");

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.name, "acme");
    }

    #[tokio::test]
    async fn test_header_resolver_name_case_insensitive() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = HeaderTenantResolver::new(store, "X-Tenant-ID");

        let mut request = create_request("GET", "/api/users");
        request
            .headers
            .insert("X-TENANT-ID", "tenant-1".to_string());

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.id, "tenant-1");
    }
}
