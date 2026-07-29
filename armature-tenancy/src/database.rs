//! Database Per Tenant
//!
//! Provides separate database connections for each tenant with DI injection.
//!
//! # Usage
//!
//! Users must inject their database connection pool/handler via DI:
//!
//! ```rust,ignore
//! // In your application
//! let db_pool = MyDatabasePool::new("connection-string");
//! container.register(Arc::new(db_pool));
//!
//! // Create tenant database manager with injected pool
//! let db_manager = TenantDatabaseManager::new(db_pool);
//!
//! // Get tenant-specific connection
//! let conn = db_manager.get_connection(&tenant).await?;
//! ```

use crate::TenantError;
use crate::tenant::Tenant;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Database connection provider trait
///
/// Users must implement this trait with their database of choice.
/// The root application injects the implementation via DI.
#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    /// The connection type (e.g., sqlx::PgConnection, diesel::PgConnection)
    type Connection: Send;

    /// Get connection for a specific database
    async fn get_connection(&self, database_name: &str) -> Result<Self::Connection, TenantError>;

    /// Check if database exists
    async fn database_exists(&self, database_name: &str) -> Result<bool, TenantError>;

    /// Create database (optional - for dynamic tenant provisioning)
    async fn create_database(&self, database_name: &str) -> Result<(), TenantError> {
        let _ = database_name;
        Err(TenantError::Storage(
            "Database creation not implemented".to_string(),
        ))
    }
}

/// Tenant database manager
///
/// Manages database connections per tenant using an injected database provider.
pub struct TenantDatabaseManager<P: DatabaseProvider> {
    provider: Arc<P>,
    connection_cache: Arc<RwLock<HashMap<String, Arc<P::Connection>>>>,
}

impl<P: DatabaseProvider> TenantDatabaseManager<P> {
    /// Create new tenant database manager with injected provider
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_tenancy::TenantDatabaseManager;
    ///
    /// // Inject your database provider via DI
    /// let db_provider = MyDatabaseProvider::new();
    /// let manager = TenantDatabaseManager::new(Arc::new(db_provider));
    /// ```
    pub fn new(provider: Arc<P>) -> Self {
        Self {
            provider,
            connection_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get connection for tenant
    ///
    /// Returns a cached connection if available, otherwise creates a new one.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let tenant = Tenant::new("tenant-1", "acme")
    ///     .with_database("acme_db");
    ///
    /// let connection = manager.get_connection(&tenant).await?;
    /// ```
    pub async fn get_connection(&self, tenant: &Tenant) -> Result<Arc<P::Connection>, TenantError> {
        let database_name = tenant
            .database
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no database configured".to_string()))?;

        // Check cache first
        {
            let cache = self.connection_cache.read().await;
            if let Some(conn) = cache.get(database_name) {
                return Ok(Arc::clone(conn));
            }
        }

        // Cache miss: take the write lock and re-check before opening a
        // connection (double-checked locking). Without the re-check, two
        // concurrent first-requests for the same database would both open a
        // connection and one would overwrite the other, leaking a connection.
        // Holding the write lock across the provider call single-flights
        // creation so exactly one connection per tenant database is opened.
        let mut cache = self.connection_cache.write().await;
        if let Some(conn) = cache.get(database_name) {
            return Ok(Arc::clone(conn));
        }

        let connection = self.provider.get_connection(database_name).await?;
        let arc_conn = Arc::new(connection);
        cache.insert(database_name.clone(), Arc::clone(&arc_conn));

        Ok(arc_conn)
    }

    /// Clear connection cache
    pub async fn clear_cache(&self) {
        let mut cache = self.connection_cache.write().await;
        cache.clear();
    }

    /// Remove specific tenant from cache
    pub async fn invalidate_tenant(&self, tenant: &Tenant) {
        if let Some(database_name) = &tenant.database {
            let mut cache = self.connection_cache.write().await;
            cache.remove(database_name);
        }
    }

    /// Check if tenant database exists
    pub async fn database_exists(&self, tenant: &Tenant) -> Result<bool, TenantError> {
        let database_name = tenant
            .database
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no database configured".to_string()))?;

        self.provider.database_exists(database_name).await
    }

    /// Create database for tenant (if supported by provider)
    pub async fn create_database(&self, tenant: &Tenant) -> Result<(), TenantError> {
        let database_name = tenant
            .database
            .as_ref()
            .ok_or_else(|| TenantError::Invalid("Tenant has no database configured".to_string()))?;

        self.provider.create_database(database_name).await
    }
}

/// Database configuration for tenant
#[derive(Debug, Clone)]
pub struct TenantDatabaseConfig {
    /// Database name pattern (e.g., "tenant_{id}")
    pub name_pattern: String,

    /// Whether to create databases automatically
    pub auto_create: bool,

    /// Maximum connections per tenant
    pub max_connections: Option<u32>,
}

impl TenantDatabaseConfig {
    /// Create new database config
    pub fn new(name_pattern: impl Into<String>) -> Self {
        Self {
            name_pattern: name_pattern.into(),
            auto_create: false,
            max_connections: Some(10),
        }
    }

    /// Enable auto-creation of databases
    pub fn with_auto_create(mut self, auto_create: bool) -> Self {
        self.auto_create = auto_create;
        self
    }

    /// Set max connections per tenant
    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Generate database name for tenant
    pub fn database_name(&self, tenant: &Tenant) -> String {
        self.name_pattern
            .replace("{id}", &tenant.id)
            .replace("{name}", &tenant.name)
    }
}

impl Default for TenantDatabaseConfig {
    fn default() -> Self {
        Self::new("tenant_{id}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockDatabaseProvider;

    #[async_trait]
    impl DatabaseProvider for MockDatabaseProvider {
        type Connection = String; // Mock connection

        async fn get_connection(
            &self,
            database_name: &str,
        ) -> Result<Self::Connection, TenantError> {
            Ok(format!("Connection to {}", database_name))
        }

        async fn database_exists(&self, database_name: &str) -> Result<bool, TenantError> {
            Ok(database_name.starts_with("tenant_"))
        }

        async fn create_database(&self, _database_name: &str) -> Result<(), TenantError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_get_connection() {
        let provider = Arc::new(MockDatabaseProvider);
        let manager = TenantDatabaseManager::new(provider);

        let tenant = Tenant::new("tenant-1", "acme").with_database("acme_db");

        let conn = manager.get_connection(&tenant).await.unwrap();
        assert_eq!(*conn, "Connection to acme_db");
    }

    #[tokio::test]
    async fn test_connection_caching() {
        let provider = Arc::new(MockDatabaseProvider);
        let manager = TenantDatabaseManager::new(provider);

        let tenant = Tenant::new("tenant-1", "acme").with_database("acme_db");

        let conn1 = manager.get_connection(&tenant).await.unwrap();
        let conn2 = manager.get_connection(&tenant).await.unwrap();

        // Should return same cached connection
        assert!(Arc::ptr_eq(&conn1, &conn2));
    }

    #[tokio::test]
    async fn test_database_config() {
        let config = TenantDatabaseConfig::new("tenant_{id}")
            .with_auto_create(true)
            .with_max_connections(20);

        let tenant = Tenant::new("tenant-123", "acme");
        let db_name = config.database_name(&tenant);

        assert_eq!(db_name, "tenant_tenant-123");
        assert!(config.auto_create);
        assert_eq!(config.max_connections, Some(20));
    }

    #[tokio::test]
    async fn test_get_connection_no_stampede() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Provider that counts how many times it actually opens a connection
        // and yields mid-call to widen the race window.
        struct CountingProvider {
            calls: AtomicUsize,
        }

        #[async_trait]
        impl DatabaseProvider for CountingProvider {
            type Connection = String;

            async fn get_connection(
                &self,
                database_name: &str,
            ) -> Result<Self::Connection, TenantError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                // Simulate slow connection establishment so concurrent callers
                // overlap on the cache miss.
                tokio::task::yield_now().await;
                Ok(format!("Connection to {database_name}"))
            }

            async fn database_exists(&self, _database_name: &str) -> Result<bool, TenantError> {
                Ok(true)
            }
        }

        let provider = Arc::new(CountingProvider {
            calls: AtomicUsize::new(0),
        });
        let manager = Arc::new(TenantDatabaseManager::new(Arc::clone(&provider)));

        // Fire many concurrent first-requests for the same tenant database.
        let mut handles = Vec::new();
        for _ in 0..32 {
            let manager = Arc::clone(&manager);
            handles.push(tokio::spawn(async move {
                let tenant = Tenant::new("tenant-1", "acme").with_database("acme_db");
                manager.get_connection(&tenant).await.unwrap()
            }));
        }

        let mut conns = Vec::new();
        for handle in handles {
            conns.push(handle.await.unwrap());
        }

        // Double-checked locking must open the connection exactly once...
        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            1,
            "connection should be created exactly once despite concurrent first-requests"
        );
        // ...and every caller must observe that same shared connection.
        for conn in &conns {
            assert!(Arc::ptr_eq(&conns[0], conn));
        }
    }

    #[tokio::test]
    async fn test_database_exists() {
        let provider = Arc::new(MockDatabaseProvider);
        let manager = TenantDatabaseManager::new(provider);

        let tenant = Tenant::new("tenant-1", "acme").with_database("tenant_acme");

        let exists = manager.database_exists(&tenant).await.unwrap();
        assert!(exists);
    }
}
