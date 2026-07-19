# armature-tenancy

Multi-tenancy support for the Armature framework.

## Features

- **Tenant Isolation** - Request-scoped tenant context, stored server-side
- **Multiple Strategies** - Schema per tenant, database per tenant
- **Tenant Resolution** - Subdomain, header, path, and JWT-claim based
- **Middleware** - Automatic tenant resolution into request extensions
- **Database Routing** - Per-tenant connections via `TenantDatabaseManager`

## Installation

```toml
[dependencies]
armature-tenancy = "0.1"
```

## Security model

Tenant identity is **never** trusted from client-supplied headers. A
[`TenantResolver`] looks the tenant up (e.g. by subdomain, header, path, or
verified JWT claim) against your [`TenantStore`], and only the resolver's
result is stored — in the request's server-side `extensions`, by
[`TenantMiddleware`]. Any incoming `__tenant*`-prefixed header is stripped
before resolution runs, so a client can never seed or spoof the resolved
tenant. Handlers read the tenant with [`get_tenant_id`] / [`get_tenant_name`],
never from request headers directly.

## Quick Start

### 1. Provide a tenant store

Implement [`TenantStore`] against your own database:

```rust,ignore
use armature_tenancy::*;

struct MyTenantStore {
    db: MyDatabasePool,
}

#[async_trait::async_trait]
impl TenantStore for MyTenantStore {
    async fn find_by_id(&self, id: &str) -> Result<Option<Tenant>, TenantError> {
        // Query from your database
        self.db.query("SELECT * FROM tenants WHERE id = $1", &[id]).await
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Tenant>, TenantError> {
        // ...
        # todo!()
    }

    async fn find_by_domain(&self, domain: &str) -> Result<Option<Tenant>, TenantError> {
        // ...
        # todo!()
    }
}
```

### 2. Resolve the tenant and wire up the middleware

Build a concrete resolver from your store, then register `TenantMiddleware`:

```rust,ignore
use armature_tenancy::*;
use std::sync::Arc;

let store: Arc<dyn TenantStore> = Arc::new(MyTenantStore::new(db_pool));

// Header-based resolution
let resolver = HeaderTenantResolver::new(store.clone(), "X-Tenant-ID");

// Or subdomain-based: acme.example.com -> tenant "acme"
let resolver = SubdomainTenantResolver::new(store.clone(), "example.com");

// Or path-based: /tenants/acme/... -> tenant "acme"
let resolver = PathTenantResolver::new(store.clone(), r"^/tenants/([^/]+)", 1)?;

// Or JWT-claim based (only safe AFTER an auth layer has verified the token's
// signature; this resolver reads the claim but does not itself verify it)
let resolver = JwtTenantResolver::new(store, "tenant_id");

let middleware = TenantMiddleware::new(Arc::new(resolver));
app.middleware(Arc::new(middleware));
```

### 3. Read the resolved tenant in a handler

The middleware stores the resolved tenant in the request's server-side
extensions. Handlers read it with [`get_tenant_id`] / [`get_tenant_name`] —
never from a client header:

```rust,ignore
use armature_tenancy::get_tenant_id;

app.get("/data", |req| async move {
    let tenant_id = get_tenant_id(&req)
        .ok_or_else(|| Error::Unauthorized("No tenant resolved".into()))?;
    let data = fetch_tenant_data(&tenant_id).await?;
    Ok(HttpResponse::ok().json(data))
});
```

By default, unresolved tenants reject the request with 401. Call
`.with_optional(true)` on `TenantMiddleware` to let unresolved requests
proceed with no tenant context instead.

## Database Isolation

### Database Per Tenant

Implement [`DatabaseProvider`] with your database client, then manage
per-tenant connections with [`TenantDatabaseManager`]:

```rust,ignore
use armature_tenancy::*;
use std::sync::Arc;

struct MyDatabaseProvider {
    pool: MyDatabasePool,
}

#[async_trait::async_trait]
impl DatabaseProvider for MyDatabaseProvider {
    type Connection = MyConnection;

    async fn get_connection(&self, database_name: &str) -> Result<Self::Connection, TenantError> {
        self.pool.connect(database_name).await
    }

    async fn database_exists(&self, database_name: &str) -> Result<bool, TenantError> {
        // ...
        # todo!()
    }
}

let db_provider = Arc::new(MyDatabaseProvider::new(pool));
let db_manager = TenantDatabaseManager::new(db_provider);

// Get a tenant-specific connection
let conn = db_manager.get_connection(&tenant).await?;
```

### Schema Per Tenant (PostgreSQL)

Implement [`SchemaProvider`] with your PostgreSQL client, then manage
`search_path` switching with [`SchemaManager`]:

```rust,ignore
use armature_tenancy::SchemaManager;

let schema_manager = SchemaManager::new(postgres_provider);
schema_manager.set_search_path(&tenant, &mut conn).await?;

// Now all queries use the tenant's schema
sqlx::query("SELECT * FROM users").fetch_all(&mut conn).await?;
```

## Tenant-Aware Caching

```rust,ignore
let cache = TenantCache::new(redis_provider);

// Automatically prefixed with tenant ID
cache.set(&tenant, "users:1", data, None).await?;
let value = cache.get(&tenant, "users:1").await?;
```

## Tenant Management

```rust,ignore
use armature_tenancy::*;
use std::sync::Arc;

// Create a tenant manager
let store = Arc::new(InMemoryManagedTenantStore::new());
let manager = TenantManager::with_store(store);

// Create a new tenant
let request = CreateTenantRequest::new("acme-corp")
    .with_display_name("Acme Corporation")
    .with_plan(TenantPlan::Professional);

let tenant = manager.create(request).await?;

// Manage lifecycle
manager.suspend(&tenant.tenant.id, "Payment overdue").await?;
manager.activate(&tenant.tenant.id).await?;

// Check usage against limits
let violations = manager.check_limits(&tenant.tenant.id).await?;
```

## License

MIT OR Apache-2.0
