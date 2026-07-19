//! Tenant Context
//!
//! Provides tenant information and request-scoped tenant context.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tenant information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tenant {
    /// Unique tenant identifier
    pub id: String,

    /// Tenant name/slug
    pub name: String,

    /// Human-readable display name (distinct from the slug in `name`)
    pub display_name: Option<String>,

    /// Tenant domain (if using subdomain isolation)
    pub domain: Option<String>,

    /// Database name (for database-per-tenant)
    pub database: Option<String>,

    /// Schema name (for schema-per-tenant)
    pub schema: Option<String>,

    /// Whether tenant is active
    pub active: bool,

    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

impl Tenant {
    /// Create a new tenant
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::Tenant;
    ///
    /// let tenant = Tenant::new("tenant-123", "acme-corp");
    /// ```
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            display_name: None,
            domain: None,
            database: None,
            schema: None,
            active: true,
            metadata: HashMap::new(),
        }
    }

    /// Set the human-readable display name
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Set tenant domain
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Set database name
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());
        self
    }

    /// Set schema name
    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Set active status
    pub fn with_active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get cache key prefix for this tenant
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::Tenant;
    ///
    /// let tenant = Tenant::new("tenant-123", "acme-corp");
    /// let key = tenant.cache_key("users:1");
    /// assert_eq!(key, "tenant:tenant-123:users:1");
    /// ```
    pub fn cache_key(&self, key: &str) -> String {
        format!("tenant:{}:{}", self.id, key)
    }
}

/// Tenant context stored in request
#[derive(Debug, Clone)]
pub struct TenantContext {
    tenant: Option<Tenant>,
}

impl TenantContext {
    /// Create empty tenant context
    pub fn new() -> Self {
        Self { tenant: None }
    }

    /// Create with tenant
    pub fn with_tenant(tenant: Tenant) -> Self {
        Self {
            tenant: Some(tenant),
        }
    }

    /// Get tenant
    pub fn tenant(&self) -> Option<&Tenant> {
        self.tenant.as_ref()
    }

    /// Set tenant
    pub fn set_tenant(&mut self, tenant: Tenant) {
        self.tenant = Some(tenant);
    }

    /// Get tenant ID
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant.as_ref().map(|t| t.id.as_str())
    }

    /// Check if tenant is set
    pub fn has_tenant(&self) -> bool {
        self.tenant.is_some()
    }
}

impl Default for TenantContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tenant_new() {
        let tenant = Tenant::new("tenant-1", "acme");
        assert_eq!(tenant.id, "tenant-1");
        assert_eq!(tenant.name, "acme");
        assert!(tenant.active);
    }

    #[test]
    fn test_tenant_builder() {
        let tenant = Tenant::new("tenant-1", "acme")
            .with_domain("acme.example.com")
            .with_database("acme_db")
            .with_schema("acme_schema")
            .with_metadata("plan", "premium");

        assert_eq!(tenant.domain, Some("acme.example.com".to_string()));
        assert_eq!(tenant.database, Some("acme_db".to_string()));
        assert_eq!(tenant.schema, Some("acme_schema".to_string()));
        assert_eq!(tenant.metadata.get("plan"), Some(&"premium".to_string()));
    }

    #[test]
    fn test_cache_key() {
        let tenant = Tenant::new("tenant-123", "acme");
        let key = tenant.cache_key("users:1");
        assert_eq!(key, "tenant:tenant-123:users:1");
    }

    #[test]
    fn test_tenant_context() {
        let mut context = TenantContext::new();
        assert!(!context.has_tenant());

        let tenant = Tenant::new("tenant-1", "acme");
        context.set_tenant(tenant.clone());

        assert!(context.has_tenant());
        assert_eq!(context.tenant_id(), Some("tenant-1"));
        assert_eq!(context.tenant().unwrap().name, "acme");
    }
}
