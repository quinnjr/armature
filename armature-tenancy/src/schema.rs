//! PostgreSQL Schema Per Tenant
//!
//! Provides PostgreSQL schema isolation for multi-tenancy.
//!
//! # Usage
//!
//! ```rust,ignore
//! // Set search_path for tenant
//! let schema_manager = SchemaManager::new(pool);
//! schema_manager.set_search_path(&tenant, &mut conn).await?;
//!
//! // Now all queries use the tenant's schema
//! sqlx::query("SELECT * FROM users")
//!     .fetch_all(&mut conn)
//!     .await?;
//! ```

use crate::TenantError;
use crate::tenant::Tenant;
use async_trait::async_trait;

/// PostgreSQL schema provider trait
///
/// Users implement this trait with their PostgreSQL client.
///
/// # Security
///
/// A schema name cannot be a bind parameter — `SET search_path`, `CREATE
/// SCHEMA` and `DROP SCHEMA` all take a bare identifier — so every
/// implementation of this trait necessarily interpolates `schema_name` into
/// SQL text. **Implementors must treat `schema_name` as untrusted and validate
/// it before interpolation**, or quote it as an identifier. Names that arrive
/// via [`Tenant::with_schema`] are already checked to be plain unquoted
/// identifiers; names built any other way (e.g. by
/// [`SchemaConfig::schema_name`], which formats an unvalidated tenant id into a
/// pattern) are not. [`crate::is_valid_schema_name`] performs the same check
/// and is the intended guard for those paths.
#[async_trait]
pub trait SchemaProvider: Send + Sync {
    /// Connection type
    type Connection: Send;

    /// Set search_path for connection
    async fn set_search_path(
        &self,
        conn: &mut Self::Connection,
        schema_name: &str,
    ) -> Result<(), TenantError>;

    /// Check if schema exists
    async fn schema_exists(
        &self,
        conn: &mut Self::Connection,
        schema_name: &str,
    ) -> Result<bool, TenantError>;

    /// Create schema
    async fn create_schema(
        &self,
        conn: &mut Self::Connection,
        schema_name: &str,
    ) -> Result<(), TenantError>;

    /// Drop schema
    async fn drop_schema(
        &self,
        conn: &mut Self::Connection,
        schema_name: &str,
    ) -> Result<(), TenantError>;
}

/// PostgreSQL schema manager
pub struct SchemaManager<P: SchemaProvider> {
    provider: P,
}

impl<P: SchemaProvider> SchemaManager<P> {
    /// Create new schema manager with injected provider
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let provider = MyPostgresProvider::new();
    /// let manager = SchemaManager::new(provider);
    /// ```
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Set search_path for tenant
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tenant = Tenant::new("tenant-1", "acme")
    ///     .with_schema("acme_schema")?;
    ///
    /// manager.set_search_path(&tenant, &mut conn).await?;
    /// ```
    pub async fn set_search_path(
        &self,
        tenant: &Tenant,
        conn: &mut P::Connection,
    ) -> Result<(), TenantError> {
        let schema_name = tenant
            .schema
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no schema configured".to_string()))?;

        self.provider.set_search_path(conn, schema_name).await
    }

    /// Check if tenant schema exists
    pub async fn schema_exists(
        &self,
        tenant: &Tenant,
        conn: &mut P::Connection,
    ) -> Result<bool, TenantError> {
        let schema_name = tenant
            .schema
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no schema configured".to_string()))?;

        self.provider.schema_exists(conn, schema_name).await
    }

    /// Create schema for tenant
    pub async fn create_schema(
        &self,
        tenant: &Tenant,
        conn: &mut P::Connection,
    ) -> Result<(), TenantError> {
        let schema_name = tenant
            .schema
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no schema configured".to_string()))?;

        self.provider.create_schema(conn, schema_name).await
    }

    /// Drop tenant schema
    pub async fn drop_schema(
        &self,
        tenant: &Tenant,
        conn: &mut P::Connection,
    ) -> Result<(), TenantError> {
        let schema_name = tenant
            .schema
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no schema configured".to_string()))?;

        self.provider.drop_schema(conn, schema_name).await
    }
}

/// Schema configuration
#[derive(Debug, Clone)]
pub struct SchemaConfig {
    /// Schema name pattern (e.g., "tenant_{id}")
    pub name_pattern: String,

    /// Whether to create schemas automatically
    pub auto_create: bool,

    /// Default schema (fallback)
    pub default_schema: String,
}

impl SchemaConfig {
    /// Create new schema config
    pub fn new(name_pattern: impl Into<String>) -> Self {
        Self {
            name_pattern: name_pattern.into(),
            auto_create: false,
            default_schema: "public".to_string(),
        }
    }

    /// Enable auto-creation of schemas
    pub fn with_auto_create(mut self, auto_create: bool) -> Self {
        self.auto_create = auto_create;
        self
    }

    /// Set default schema
    pub fn with_default_schema(mut self, schema: impl Into<String>) -> Self {
        self.default_schema = schema.into();
        self
    }

    /// Generate schema name for tenant
    pub fn schema_name(&self, tenant: &Tenant) -> String {
        self.name_pattern
            .replace("{id}", &tenant.id)
            .replace("{name}", &tenant.name)
    }
}

impl Default for SchemaConfig {
    fn default() -> Self {
        Self::new("tenant_{id}")
    }
}

/// Query wrapper for automatic schema setting
///
/// Wraps a query execution to automatically set the search_path.
pub struct TenantQuery<'a, P: SchemaProvider> {
    manager: &'a SchemaManager<P>,
    tenant: &'a Tenant,
}

impl<'a, P: SchemaProvider> TenantQuery<'a, P> {
    /// Create new tenant query
    pub fn new(manager: &'a SchemaManager<P>, tenant: &'a Tenant) -> Self {
        Self { manager, tenant }
    }

    /// Execute query with tenant's schema
    pub async fn execute<F, T>(&self, conn: &mut P::Connection, f: F) -> Result<T, TenantError>
    where
        F: FnOnce(
            &mut P::Connection,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, TenantError>> + Send + '_>,
        >,
    {
        // Set search_path
        self.manager.set_search_path(self.tenant, conn).await?;

        // Execute query
        f(conn).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockSchemaProvider {
        current_schema: tokio::sync::Mutex<String>,
    }

    impl MockSchemaProvider {
        fn new() -> Self {
            Self {
                current_schema: tokio::sync::Mutex::new("public".to_string()),
            }
        }

        async fn get_current_schema(&self) -> String {
            self.current_schema.lock().await.clone()
        }
    }

    #[async_trait]
    impl SchemaProvider for MockSchemaProvider {
        type Connection = (); // Mock connection

        async fn set_search_path(
            &self,
            _conn: &mut Self::Connection,
            schema_name: &str,
        ) -> Result<(), TenantError> {
            *self.current_schema.lock().await = schema_name.to_string();
            Ok(())
        }

        async fn schema_exists(
            &self,
            _conn: &mut Self::Connection,
            schema_name: &str,
        ) -> Result<bool, TenantError> {
            Ok(schema_name.starts_with("tenant_"))
        }

        async fn create_schema(
            &self,
            _conn: &mut Self::Connection,
            _schema_name: &str,
        ) -> Result<(), TenantError> {
            Ok(())
        }

        async fn drop_schema(
            &self,
            _conn: &mut Self::Connection,
            _schema_name: &str,
        ) -> Result<(), TenantError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_set_search_path() {
        let provider = MockSchemaProvider::new();
        let manager = SchemaManager::new(provider);

        let tenant = Tenant::new("tenant-1", "acme")
            .with_schema("acme_schema")
            .unwrap();

        let mut conn = ();
        manager.set_search_path(&tenant, &mut conn).await.unwrap();

        let current = manager.provider.get_current_schema().await;
        assert_eq!(current, "acme_schema");
    }

    #[tokio::test]
    async fn test_schema_config() {
        let config = SchemaConfig::new("tenant_{name}")
            .with_auto_create(true)
            .with_default_schema("custom_public");

        let tenant = Tenant::new("tenant-123", "acme");
        let schema_name = config.schema_name(&tenant);

        assert_eq!(schema_name, "tenant_acme");
        assert!(config.auto_create);
        assert_eq!(config.default_schema, "custom_public");
    }

    #[tokio::test]
    async fn test_schema_exists() {
        let provider = MockSchemaProvider::new();
        let manager = SchemaManager::new(provider);

        let tenant = Tenant::new("tenant-1", "acme")
            .with_schema("tenant_acme")
            .unwrap();

        let mut conn = ();
        let exists = manager.schema_exists(&tenant, &mut conn).await.unwrap();
        assert!(exists);
    }
}
