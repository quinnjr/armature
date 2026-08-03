//! Tenant Context
//!
//! Provides tenant information and request-scoped tenant context.

use crate::resolver::TenantError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Maximum length of a PostgreSQL identifier (`NAMEDATALEN - 1`).
const MAX_IDENTIFIER_LEN: usize = 63;

/// Whether `name` is safe to interpolate into SQL as a bare schema identifier.
///
/// Exposed so that [`crate::schema::SchemaProvider`] implementations can
/// re-check any schema name they did not obtain from [`Tenant::with_schema`]
/// (for instance one built by [`crate::schema::SchemaConfig::schema_name`] from
/// an unvalidated tenant id).
pub fn is_valid_schema_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_IDENTIFIER_LEN {
        return false;
    }
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Tenant information
///
/// Marked `#[non_exhaustive]` so new fields can be added without breaking
/// downstream callers that construct or match on `Tenant`. Downstream code
/// must build a `Tenant` via [`Tenant::new`] plus the `with_*` builder
/// methods rather than a struct literal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
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

    /// Set schema name.
    ///
    /// The schema name ends up interpolated into SQL (`SET search_path TO
    /// <schema>`) by [`crate::schema::SchemaProvider`] implementations, because
    /// a schema selector cannot be a bind parameter. It is therefore validated
    /// here — at the only place it enters a [`Tenant`] — so that a
    /// tenant-controlled value can never carry SQL out of the identifier.
    ///
    /// # Errors
    ///
    /// Returns [`TenantError::Invalid`] unless `schema` is a plain unquoted SQL
    /// identifier: `[A-Za-z_][A-Za-z0-9_]*`, at most 63 bytes (PostgreSQL
    /// truncates identifiers beyond `NAMEDATALEN - 1`, which would silently
    /// alias two distinct tenants onto one schema).
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::Tenant;
    ///
    /// let tenant = Tenant::new("tenant-123", "acme")
    ///     .with_schema("tenant_acme")
    ///     .unwrap();
    /// assert_eq!(tenant.schema.as_deref(), Some("tenant_acme"));
    ///
    /// assert!(
    ///     Tenant::new("tenant-123", "acme")
    ///         .with_schema("public; DROP TABLE users --")
    ///         .is_err()
    /// );
    /// ```
    pub fn with_schema(mut self, schema: impl Into<String>) -> Result<Self, TenantError> {
        let schema = schema.into();
        if !is_valid_schema_name(&schema) {
            return Err(TenantError::Invalid(format!(
                "Invalid schema name: {:?} (expected an unquoted SQL identifier)",
                schema
            )));
        }
        self.schema = Some(schema);
        Ok(self)
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
    /// The `id` segment is length-prefixed (`"tenant:{id.len()}:{id}:{key}"`)
    /// specifically so that no `id`/`key` content can ever produce a
    /// colliding key, regardless of what characters `id` or `key` contain
    /// (including `':'`). This matters because [`Tenant::new`] accepts any
    /// caller-supplied `id` with no validation, and
    /// [`crate::cache::TenantCache::clear_tenant`] (plus any other consumer
    /// that treats `cache_key` output as a prefix) matches keys with a
    /// plain `starts_with` test.
    ///
    /// Without the length prefix, two different `(id, key)` pairs could
    /// format to the same string whenever one `id` embeds a `':'` — e.g.
    /// `id = "x"`, `key = "y:z"` and `id = "x:y"`, `key = "z"` would both
    /// produce `"tenant:x:y:z"`. Prefixing the `id` segment with its own
    /// length makes that impossible: two ids of different lengths always
    /// diverge at the length prefix, and two ids of the same length can
    /// only produce equal output if the ids (and therefore the keys) are
    /// themselves equal.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_tenancy::Tenant;
    ///
    /// let tenant = Tenant::new("tenant-123", "acme-corp");
    /// let key = tenant.cache_key("users:1");
    /// assert_eq!(key, "tenant:10:tenant-123:users:1");
    /// ```
    pub fn cache_key(&self, key: &str) -> String {
        format!("tenant:{}:{}:{}", self.id.len(), self.id, key)
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
            .unwrap()
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
        assert_eq!(key, "tenant:10:tenant-123:users:1");
    }

    #[test]
    fn test_cache_key_collision_resistant_to_colon_in_id() {
        // Under the old `format!("tenant:{}:{}", self.id, key)` scheme,
        // these two (id, key) pairs both formatted to the identical string
        // "tenant:x:y:z", which would let one tenant's cache operations
        // (including `TenantCache::clear_tenant`'s prefix scan) collide
        // with another's. The length-prefixed scheme must keep them apart.
        let tenant_a = Tenant::new("x", "tenant-a");
        let tenant_b = Tenant::new("x:y", "tenant-b");

        let key_a = tenant_a.cache_key("y:z");
        let key_b = tenant_b.cache_key("z");

        assert_ne!(
            key_a, key_b,
            "cache keys for distinct (id, key) pairs must never collide, \
             even when id contains ':'"
        );
    }

    #[test]
    fn test_with_schema_rejects_sql_injection() {
        let too_long = "a".repeat(64);
        for bad in [
            "public; DROP TABLE users --",
            "public\"",
            "pg_catalog, public",
            "1tenant",
            "",
            "tenant acme",
            "tenant-acme",
            too_long.as_str(),
        ] {
            let result = Tenant::new("tenant-1", "acme").with_schema(bad);
            assert!(
                matches!(result, Err(TenantError::Invalid(_))),
                "schema {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn test_with_schema_accepts_plain_identifiers() {
        let max_len = "a".repeat(63);
        for good in ["public", "_private", "tenant_acme_1", max_len.as_str()] {
            let tenant = Tenant::new("tenant-1", "acme")
                .with_schema(good)
                .unwrap_or_else(|e| panic!("schema {good:?} must be accepted: {e}"));
            assert_eq!(tenant.schema.as_deref(), Some(good));
        }
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
