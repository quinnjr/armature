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
        let tenant_id = request
            .headers
            .get(&self.header_name.to_lowercase())
            .ok_or_else(|| {
                TenantError::NotFound(format!("Missing header: {}", self.header_name))
            })?;

        let tenant = self
            .store
            .find_by_id(tenant_id)
            .await?
            .ok_or_else(|| TenantError::NotFound(tenant_id.clone()))?;

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
        if let Some(subdomain) = host.strip_suffix(&format!(".{}", self.base_domain)) {
            if !subdomain.is_empty() && !subdomain.contains('.') {
                return Some(subdomain.to_string());
            }
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
#[allow(dead_code)]
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

        // Parse JWT claims (simplified - in production use armature-jwt)
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
    fn extract_claim(&self, _token: &str) -> Result<String, TenantError> {
        // Simplified JWT parsing - in production, use armature-jwt to decode and validate
        // For now, this is a placeholder
        Err(TenantError::ResolutionFailed(
            "JWT parsing requires armature-jwt integration".to_string(),
        ))
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
        let captures = self
            .pattern
            .captures(&request.path)
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
            .insert("x-tenant-id".to_string(), "tenant-1".to_string());

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
            .insert("host".to_string(), "acme.example.com".to_string());

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.name, "acme");
    }

    #[tokio::test]
    async fn test_path_resolver() {
        let store: Arc<dyn TenantStore> = Arc::new(MockTenantStore::new());
        let resolver = PathTenantResolver::new(store, r"^/tenants/([^/]+)", 1).unwrap();

        let request = create_request("GET", "/tenants/acme/users");

        let tenant = resolver.resolve(&request).await.unwrap();
        assert_eq!(tenant.name, "acme");
    }
}
