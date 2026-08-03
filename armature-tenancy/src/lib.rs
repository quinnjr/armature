//! Multi-Tenancy for Armature
//!
//! Comprehensive multi-tenancy support with tenant isolation, database per tenant,
//! schema per tenant, tenant-aware caching, and full lifecycle management.
//!
//! # Features
//!
//! - 🏢 **Tenant Isolation** - Request-scoped tenant context
//! - 🗄️ **Database Per Tenant** - Separate database connections (with DI)
//! - 📊 **Schema Per Tenant** - PostgreSQL schema isolation
//! - 🔍 **Tenant Resolution** - Multiple resolution strategies
//! - 🚀 **Auto Middleware** - Automatic tenant resolution
//! - 💾 **Tenant-Aware Caching** - Automatic cache key prefixing
//! - 📝 **Tenant Management** - Full CRUD and lifecycle management
//! - 🎛️ **Plans & Limits** - Built-in usage tracking and limits
//!
//! # Quick Start
//!
//! ## 1. Define Tenant Store (with your database)
//!
//! ```rust,ignore
//! use armature_tenancy::*;
//!
//! struct MyTenantStore {
//!     db: MyDatabasePool,
//! }
//!
//! #[async_trait]
//! impl TenantStore for MyTenantStore {
//!     async fn find_by_id(&self, id: &str) -> Result<Option<Tenant>, TenantError> {
//!         // Query from your database
//!         self.db.query("SELECT * FROM tenants WHERE id = $1", &[id]).await
//!     }
//!
//!     // ... implement other methods
//! }
//! ```
//!
//! ## 2. Set Up Tenant Resolution
//!
//! ```rust,ignore
//! use armature_tenancy::*;
//!
//! // Inject tenant store via DI
//! let store: Arc<dyn TenantStore> = Arc::new(MyTenantStore::new(db_pool));
//! container.register(store.clone());
//!
//! // Header-based resolution
//! let resolver = HeaderTenantResolver::new(store, "X-Tenant-ID");
//!
//! // Or subdomain-based
//! let resolver = SubdomainTenantResolver::new(store, "example.com");
//!
//! // Add middleware
//! let middleware = TenantMiddleware::new(Arc::new(resolver));
//! app.middleware(Arc::new(middleware));
//! ```
//!
//! ## 3. Database Per Tenant (with DI)
//!
//! ```rust,ignore
//! use armature_tenancy::*;
//!
//! // Implement database provider with your DB client
//! struct MyDatabaseProvider {
//!     pool: MyDatabasePool,
//! }
//!
//! #[async_trait]
//! impl DatabaseProvider for MyDatabaseProvider {
//!     type Connection = MyConnection;
//!
//!     async fn get_connection(&self, database_name: &str) -> Result<Self::Connection, TenantError> {
//!         self.pool.connect(database_name).await
//!     }
//! }
//!
//! // Inject via DI
//! let db_provider = Arc::new(MyDatabaseProvider::new(pool));
//! let db_manager = TenantDatabaseManager::new(db_provider);
//!
//! // Get tenant-specific connection
//! let conn = db_manager.get_connection(&tenant).await?;
//! ```
//!
//! ## 4. Schema Per Tenant (PostgreSQL)
//!
//! ```rust,ignore
//! // Set search_path for tenant
//! let schema_manager = SchemaManager::new(postgres_provider);
//! schema_manager.set_search_path(&tenant, &mut conn).await?;
//!
//! // Now all queries use tenant's schema
//! sqlx::query("SELECT * FROM users").fetch_all(&mut conn).await?;
//! ```
//!
//! ## 5. Tenant-Aware Caching
//!
//! ```rust,ignore
//! let cache = TenantCache::new(redis_provider);
//!
//! // Automatically prefixed with tenant ID
//! cache.set(&tenant, "users:1", data, None).await?;
//! let value = cache.get(&tenant, "users:1").await?;
//! ```
//!
//! ## 6. Tenant Management
//!
//! ```rust,ignore
//! use armature_tenancy::*;
//!
//! // Create a tenant manager
//! let store = Arc::new(InMemoryManagedTenantStore::new());
//! let manager = TenantManager::with_store(store);
//!
//! // Create a new tenant
//! let request = CreateTenantRequest::new("acme-corp")
//!     .with_display_name("Acme Corporation")
//!     .with_plan(TenantPlan::Professional);
//!
//! let tenant = manager.create(request).await?;
//!
//! // Manage lifecycle
//! manager.suspend(&tenant.tenant.id, "Payment overdue").await?;
//! manager.activate(&tenant.tenant.id).await?;
//!
//! // Check usage against limits
//! let violations = manager.check_limits(&tenant.tenant.id).await?;
//! ```

pub mod cache;
pub mod database;
pub mod management;
pub mod middleware;
pub mod resolver;
pub mod schema;
pub mod tenant;

pub use cache::{CacheError, CacheKeyBuilder, CacheProvider, TenantCache};
pub use database::{DatabaseProvider, TenantDatabaseConfig, TenantDatabaseManager};
pub use management::{
    CreateTenantRequest, InMemoryManagedTenantStore, ManagedTenant, ManagedTenantStore,
    NoOpProvisioner, TenantFilter, TenantLimits, TenantManager, TenantPlan, TenantProvisioner,
    TenantStatus, TenantUsage, UpdateTenantRequest,
};
pub use middleware::{TenantMiddleware, get_tenant_id, get_tenant_name};
pub use resolver::{
    HeaderTenantResolver, JwtTenantResolver, PathTenantResolver, SubdomainTenantResolver,
    TenantError, TenantResolver, TenantStore,
};
pub use schema::{SchemaConfig, SchemaManager, SchemaProvider, TenantQuery};
pub use tenant::{Tenant, TenantContext, is_valid_schema_name};

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::cache::{CacheProvider, TenantCache};
    pub use crate::database::{DatabaseProvider, TenantDatabaseManager};
    pub use crate::management::{
        CreateTenantRequest, ManagedTenant, ManagedTenantStore, TenantFilter, TenantLimits,
        TenantManager, TenantPlan, TenantProvisioner, TenantStatus, TenantUsage,
        UpdateTenantRequest,
    };
    pub use crate::middleware::TenantMiddleware;
    pub use crate::resolver::{
        HeaderTenantResolver, JwtTenantResolver, PathTenantResolver, SubdomainTenantResolver,
        TenantError, TenantResolver, TenantStore,
    };
    pub use crate::schema::SchemaManager;
    pub use crate::tenant::{Tenant, TenantContext};
}
