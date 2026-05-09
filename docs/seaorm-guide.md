# SeaORM Integration Guide

SeaORM database integration for the Armature framework.

## Overview

The `armature-seaorm` crate provides:

- **Multiple Backends**: PostgreSQL, MySQL, and SQLite support
- **Connection Pooling**: Built-in connection pooling via SQLx
- **Transaction Management**: Easy-to-use transaction helpers
- **Active Record Pattern**: Entity-based CRUD operations
- **Pagination**: Built-in pagination utilities
- **Query Helpers**: Fluent query building

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-seaorm = "0.1"
```

### Feature Flags

```toml
[dependencies]
# PostgreSQL with Tokio + Rustls (default)
armature-seaorm = "0.1"

# MySQL
armature-seaorm = { version = "0.1", default-features = false, features = ["runtime-tokio-rustls", "sqlx-mysql"] }

# SQLite
armature-seaorm = { version = "0.1", default-features = false, features = ["runtime-tokio-rustls", "sqlx-sqlite"] }

# With extra features
armature-seaorm = { version = "0.1", features = ["with-json", "with-chrono", "with-uuid"] }
```

Available features:
- `runtime-tokio-rustls` - Tokio + Rustls (default)
- `runtime-tokio-native-tls` - Tokio + Native TLS
- `sqlx-postgres` - PostgreSQL (default)
- `sqlx-mysql` - MySQL
- `sqlx-sqlite` - SQLite
- `with-json` - JSON column support
- `with-chrono` - Chrono datetime support
- `with-uuid` - UUID support
- `mock` - Mock database for testing
- `debug-print` - Print SQL queries

## Quick Start

### Basic Setup

```rust
use armature_seaorm::{Database, DatabaseConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration
    let config = DatabaseConfig::new("postgres://user:pass@localhost/mydb")
        .max_connections(10)
        .connect_timeout(Duration::from_secs(5));

    // Connect to database
    let db = Database::connect(config).await?;

    // Check connection
    db.ping().await?;

    Ok(())
}
```

### From Environment Variables

```rust
use armature_seaorm::Database;

// Uses DATABASE_URL and other DATABASE_* variables
let db = Database::connect_from_env().await?;
```

Environment variables:
- `DATABASE_URL` - Required database URL
- `DATABASE_MAX_CONNECTIONS` - Max connections (default: 10)
- `DATABASE_MIN_CONNECTIONS` - Min connections (default: 1)
- `DATABASE_CONNECT_TIMEOUT` - Connect timeout in seconds
- `DATABASE_SQLX_LOGGING` - Enable SQL logging (true/false)

## Configuration

### DatabaseConfig Options

```rust
use armature_seaorm::DatabaseConfig;
use std::time::Duration;

let config = DatabaseConfig::new("postgres://localhost/mydb")
    // Pool settings
    .max_connections(20)
    .min_connections(5)

    // Timeouts
    .connect_timeout(Duration::from_secs(30))
    .max_lifetime(Duration::from_secs(30 * 60))
    .idle_timeout(Duration::from_secs(10 * 60))

    // Logging
    .sqlx_logging(true)

    // PostgreSQL schema
    .schema("public");
```

## Entity Definition

Define your entities using SeaORM's derive macros:

```rust
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub name: String,
    pub email: String,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}

impl Related<super::post::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Posts.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

## CRUD Operations

### Create

```rust
use sea_orm::{ActiveModelTrait, Set};

let user = user::ActiveModel {
    name: Set("Alice".to_owned()),
    email: Set("alice@example.com".to_owned()),
    ..Default::default()
};

let user = user.insert(&db).await?;
```

### Read

```rust
use sea_orm::EntityTrait;

// Find by ID
let user = User::find_by_id(1).one(&db).await?;

// Find all
let users = User::find().all(&db).await?;

// Find with filter
use sea_orm::QueryFilter;
let admins = User::find()
    .filter(user::Column::Role.eq("admin"))
    .all(&db)
    .await?;
```

### Update

```rust
use sea_orm::{ActiveModelTrait, Set, IntoActiveModel};

let mut user: user::ActiveModel = user.into_active_model();
user.name = Set("New Name".to_owned());
let user = user.update(&db).await?;
```

### Delete

```rust
use sea_orm::{EntityTrait, ModelTrait};

// Delete by model
user.delete(&db).await?;

// Delete by ID
User::delete_by_id(1).exec(&db).await?;

// Delete with filter
User::delete_many()
    .filter(user::Column::Active.eq(false))
    .exec(&db)
    .await?;
```

## Transactions

### Basic Transactions

```rust
use armature_seaorm::TransactionExt;

db.transaction(|txn| async move {
    let user = user::ActiveModel {
        name: Set("Alice".to_owned()),
        email: Set("alice@example.com".to_owned()),
        ..Default::default()
    };
    user.insert(&txn).await?;

    let profile = profile::ActiveModel {
        user_id: Set(1),
        ..Default::default()
    };
    profile.insert(&txn).await?;

    Ok::<_, sea_orm::DbErr>(())
}).await?;
```

### With Isolation Level

```rust
use armature_seaorm::{TransactionExt, IsolationLevel};

db.transaction_with_isolation(
    IsolationLevel::Serializable,
    |txn| async move {
        // Critical operations
        Ok::<_, sea_orm::DbErr>(())
    }
).await?;
```

## Pagination

### Offset Pagination

```rust
use armature_seaorm::{Paginate, PaginationOptions};

let options = PaginationOptions::new(1, 20); // page 1, 20 per page

let result = User::find()
    .paginate(&db, &options)
    .await?;

println!("Page: {}", result.meta.page);
println!("Total: {}", result.meta.total_items);
println!("Has next: {}", result.meta.has_next);

for user in result.items {
    println!("{}", user.name);
}
```

### Custom Pagination

```rust
use armature_seaorm::{Paginated, PaginationMeta};

let paginated = Paginated::new(items, page, per_page, total_items);

// Map to different type
let dto_paginated = paginated.map(|user| UserDto::from(user));
```

## Query Helpers

### QueryExt Trait

```rust
use armature_seaorm::QueryExt;

let users = User::find()
    .where_eq(user::Column::Role, "admin")
    .where_gt(user::Column::Age, 18)
    .where_like(user::Column::Email, "%@example.com")
    .where_not_null(user::Column::VerifiedAt)
    .order_desc(user::Column::CreatedAt)
    .all(&db)
    .await?;
```

Available methods:
- `where_eq`, `where_ne` - Equality
- `where_gt`, `where_gte`, `where_lt`, `where_lte` - Comparisons
- `where_like` - Pattern matching
- `where_null`, `where_not_null` - Null checks
- `where_in` - IN clause
- `where_between` - Range queries
- `order_asc`, `order_desc` - Ordering

### Search Filters

```rust
use armature_seaorm::SearchFilters;

// Parse from query parameters
let filters: SearchFilters = serde_json::from_str(r#"{
    "q": "alice",
    "sort": "created_at",
    "order": "desc",
    "page": 1,
    "per_page": 20
}"#)?;

let pagination = filters.pagination();
```

## Health Checks

```rust
// Simple ping
db.ping().await?;

// Full health check
let health = db.health_check().await;

println!("Healthy: {}", health.is_healthy);
println!("Response time: {}ms", health.response_time_ms);
println!("Backend: {}", health.backend.name());

if let Some(error) = health.error {
    eprintln!("Error: {}", error);
}
```

## With Armature DI

```rust
use armature_framework::prelude::*;
use armature_seaorm::{Database, DatabaseConfig};

#[module_impl]
impl DatabaseModule {
    #[provider(singleton)]
    async fn database() -> Arc<Database> {
        let config = DatabaseConfig::from_env().expect("DATABASE_URL not set");
        Arc::new(Database::connect(config).await.unwrap())
    }
}

#[controller("/users")]
struct UserController {
    db: Arc<Database>,
}

impl UserController {
    #[get("")]
    async fn list(&self) -> Result<Json<Vec<user::Model>>, Error> {
        let users = User::find().all(self.db.as_ref()).await?;
        Ok(Json(users))
    }

    #[get("/:id")]
    async fn get(&self, req: HttpRequest) -> Result<Json<user::Model>, Error> {
        let id: i32 = req.param("id").unwrap().parse()?;
        let user = User::find_by_id(id)
            .one(self.db.as_ref())
            .await?
            .ok_or(Error::NotFound)?;
        Ok(Json(user))
    }
}
```

## Relations

### Define Relations

```rust
// In user entity
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}

// In post entity
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}
```

### Load Relations

```rust
// Find with related
let user = User::find_by_id(1)
    .find_with_related(Post)
    .all(&db)
    .await?;

// Eager loading
let users_with_posts = User::find()
    .find_with_related(Post)
    .all(&db)
    .await?;
```

## Error Handling

```rust
use armature_seaorm::{SeaOrmError, SeaOrmResult};

fn handle_error(err: SeaOrmError) {
    match err {
        SeaOrmError::Connection(msg) => eprintln!("Connection failed: {}", msg),
        SeaOrmError::Query(msg) => eprintln!("Query error: {}", msg),
        SeaOrmError::Database(e) => eprintln!("Database error: {}", e),
        SeaOrmError::Transaction(msg) => eprintln!("Transaction failed: {}", msg),
        SeaOrmError::NotFound(msg) => eprintln!("Not found: {}", msg),
        SeaOrmError::Validation(msg) => eprintln!("Validation error: {}", msg),
        SeaOrmError::Migration(msg) => eprintln!("Migration failed: {}", msg),
        SeaOrmError::Config(msg) => eprintln!("Config error: {}", msg),
        SeaOrmError::Serialization(msg) => eprintln!("Serialization error: {}", msg),
    }
}
```

## Migrations

Use SeaORM's migration tool:

```bash
# Install CLI
cargo install sea-orm-cli

# Generate migration
sea-orm-cli migrate generate create_users_table

# Run migrations
sea-orm-cli migrate up

# Rollback
sea-orm-cli migrate down
```

## Best Practices

### 1. Use Connection Pooling

```rust
let config = DatabaseConfig::new(url)
    .max_connections(10)
    .min_connections(2);
```

### 2. Set Appropriate Timeouts

```rust
let config = DatabaseConfig::new(url)
    .connect_timeout(Duration::from_secs(5))
    .idle_timeout(Duration::from_secs(300));
```

### 3. Use Transactions for Multi-Step Operations

```rust
db.transaction(|txn| async move {
    // All operations succeed or fail together
    Ok::<_, sea_orm::DbErr>(())
}).await?;
```

### 4. Use Pagination for Large Result Sets

```rust
let options = PaginationOptions::new(page, 100);
let result = User::find().paginate(&db, &options).await?;
```

### 5. Enable SQL Logging in Development

```rust
let config = DatabaseConfig::new(url)
    .sqlx_logging(cfg!(debug_assertions));
```

## Summary

| Feature | Description |
|---------|-------------|
| Backends | PostgreSQL, MySQL, SQLite |
| ORM style | Active Record pattern |
| Pooling | Built-in via SQLx |
| Transactions | Full support with isolation |
| Pagination | Offset and cursor-based |
| Query helpers | Fluent query building |
| Relations | Has many, belongs to, many-to-many |
| Migrations | SeaORM CLI |

