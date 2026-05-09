# Diesel Integration Guide

Async Diesel database integration for the Armature framework.

## Overview

The `armature-diesel` crate provides:

- **Async Connection Pools**: Built on `diesel-async` with `deadpool`, `bb8`, or `mobc`
- **Multiple Backends**: PostgreSQL and MySQL support
- **Transaction Management**: Easy-to-use transaction helpers
- **DI Integration**: Works with Armature's dependency injection
- **Connection Health**: Automatic connection validation

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-diesel = "0.1"
```

### Feature Flags

```toml
[dependencies]
# PostgreSQL with deadpool (default)
armature-diesel = "0.1"

# MySQL with deadpool
armature-diesel = { version = "0.1", default-features = false, features = ["mysql", "deadpool"] }

# PostgreSQL with bb8
armature-diesel = { version = "0.1", default-features = false, features = ["postgres", "bb8"] }

# With migrations support
armature-diesel = { version = "0.1", features = ["migrations"] }
```

Available features:
- `postgres` - PostgreSQL backend (default)
- `mysql` - MySQL backend
- `deadpool` - Deadpool connection pool (default)
- `bb8` - BB8 connection pool
- `mobc` - MOBC connection pool
- `migrations` - Diesel migrations support
- `tracing` - Tracing integration

## Quick Start

### Basic Setup

```rust
use armature_diesel::{DieselPool, DieselConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration
    let config = DieselConfig::new("postgres://user:pass@localhost/mydb")
        .pool_size(10)
        .connect_timeout(Duration::from_secs(5));

    // Create pool
    let pool = DieselPool::new(config).await?;

    // Get connection
    let mut conn = pool.get().await?;

    // Run queries using diesel
    // let users = users::table.load::<User>(&mut conn).await?;

    Ok(())
}
```

### From Environment Variables

```rust
use armature_diesel::{DieselPool, DieselConfig};

// Uses DATABASE_URL and other DATABASE_* variables
let config = DieselConfig::from_env()?;
let pool = DieselPool::new(config).await?;
```

Environment variables:
- `DATABASE_URL` - Required database URL
- `DATABASE_POOL_SIZE` - Pool size (default: 10)
- `DATABASE_CONNECT_TIMEOUT` - Connect timeout in seconds
- `DATABASE_MAX_LIFETIME` - Max connection lifetime in seconds
- `DATABASE_IDLE_TIMEOUT` - Idle timeout in seconds

## Configuration

### DieselConfig Options

```rust
use armature_diesel::DieselConfig;
use std::time::Duration;

let config = DieselConfig::new("postgres://localhost/mydb")
    // Pool settings
    .pool_size(20)
    .min_idle(5)

    // Timeouts
    .connect_timeout(Duration::from_secs(30))
    .max_lifetime(Duration::from_secs(30 * 60))
    .idle_timeout(Duration::from_secs(10 * 60))

    // Connection testing
    .test_on_checkout(true)

    // PostgreSQL settings
    .application_name("my-app")
    .ssl_mode("require");
```

## Transactions

### Basic Transactions

```rust
use armature_diesel::{DieselPool, TransactionExt};

async fn create_user(pool: &DieselPool) -> Result<(), DieselError> {
    pool.transaction(|conn| async move {
        // Insert user
        diesel::insert_into(users::table)
            .values(&NewUser { name: "Alice" })
            .execute(conn)
            .await?;

        // Insert profile
        diesel::insert_into(profiles::table)
            .values(&NewProfile { user_id: 1 })
            .execute(conn)
            .await?;

        Ok(())
    }).await
}
```

### With Isolation Level

```rust
use armature_diesel::{DieselPool, TransactionExt, IsolationLevel};

pool.transaction_with_isolation(
    IsolationLevel::Serializable,
    |conn| async move {
        // Critical operations
        Ok(())
    }
).await?;
```

Available isolation levels:
- `ReadUncommitted`
- `ReadCommitted` (PostgreSQL default)
- `RepeatableRead` (MySQL default)
- `Serializable`

## Pool Statistics

```rust
let status = pool.status();

println!("Pool size: {}", status.size);
println!("Available: {}", status.available);
println!("Waiting: {}", status.waiting);
println!("Max size: {}", status.max_size);
println!("Utilization: {:.1}%", status.utilization());

if status.is_under_pressure() {
    println!("Pool is under pressure!");
}
```

## With Armature DI

```rust
use armature_framework::prelude::*;
use armature_diesel::{DieselPool, DieselConfig};

#[module_impl]
impl DatabaseModule {
    #[provider(singleton)]
    async fn database() -> Arc<DieselPool> {
        let config = DieselConfig::from_env().expect("DATABASE_URL not set");
        Arc::new(DieselPool::new(config).await.unwrap())
    }
}

#[controller("/users")]
struct UserController {
    db: Arc<DieselPool>,
}

impl UserController {
    #[get("")]
    async fn list(&self) -> Result<Json<Vec<User>>, Error> {
        let mut conn = self.db.get().await?;
        let users = users::table.load::<User>(&mut conn).await?;
        Ok(Json(users))
    }
}
```

## Schema Definition

Define your schema using Diesel's macro:

```rust
// schema.rs
diesel::table! {
    users (id) {
        id -> Int4,
        name -> Varchar,
        email -> Varchar,
        created_at -> Timestamp,
    }
}

diesel::table! {
    posts (id) {
        id -> Int4,
        user_id -> Int4,
        title -> Varchar,
        body -> Text,
    }
}

diesel::joinable!(posts -> users (user_id));
diesel::allow_tables_to_appear_in_same_query!(users, posts);
```

## Models

```rust
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Queryable, Selectable, Serialize)]
#[diesel(table_name = users)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Insertable, Deserialize)]
#[diesel(table_name = users)]
pub struct NewUser<'a> {
    pub name: &'a str,
    pub email: &'a str,
}
```

## Queries

### Select

```rust
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

// All users
let users = users::table.load::<User>(&mut conn).await?;

// With filter
let admins = users::table
    .filter(users::role.eq("admin"))
    .load::<User>(&mut conn)
    .await?;

// With join
let posts_with_users = posts::table
    .inner_join(users::table)
    .select((posts::title, users::name))
    .load::<(String, String)>(&mut conn)
    .await?;
```

### Insert

```rust
diesel::insert_into(users::table)
    .values(&NewUser { name: "Bob", email: "bob@example.com" })
    .execute(&mut conn)
    .await?;

// Get inserted record
let user: User = diesel::insert_into(users::table)
    .values(&NewUser { name: "Bob", email: "bob@example.com" })
    .get_result(&mut conn)
    .await?;
```

### Update

```rust
diesel::update(users::table.find(1))
    .set(users::name.eq("New Name"))
    .execute(&mut conn)
    .await?;
```

### Delete

```rust
diesel::delete(users::table.filter(users::id.eq(1)))
    .execute(&mut conn)
    .await?;
```

## Migrations

Enable the `migrations` feature:

```toml
armature-diesel = { version = "0.1", features = ["migrations"] }
```

```rust
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

// Run migrations
conn.run_pending_migrations(MIGRATIONS)?;
```

## Error Handling

```rust
use armature_diesel::{DieselError, DieselResult};

fn handle_error(err: DieselError) {
    match err {
        DieselError::Connection(msg) => eprintln!("Connection failed: {}", msg),
        DieselError::Pool(msg) => eprintln!("Pool error: {}", msg),
        DieselError::Query(e) => eprintln!("Query failed: {}", e),
        DieselError::Transaction(msg) => eprintln!("Transaction failed: {}", msg),
        DieselError::Config(msg) => eprintln!("Config error: {}", msg),
        DieselError::Timeout(msg) => eprintln!("Timeout: {}", msg),
        #[cfg(feature = "migrations")]
        DieselError::Migration(msg) => eprintln!("Migration failed: {}", msg),
    }
}
```

## Best Practices

### 1. Use Connection Pooling

Always use connection pools in production:

```rust
let config = DieselConfig::new(url)
    .pool_size(10)
    .min_idle(2);
```

### 2. Set Appropriate Timeouts

```rust
let config = DieselConfig::new(url)
    .connect_timeout(Duration::from_secs(5))
    .idle_timeout(Duration::from_secs(300));
```

### 3. Use Transactions for Multi-Step Operations

```rust
pool.transaction(|conn| async move {
    // All operations succeed or fail together
    Ok(())
}).await?;
```

### 4. Monitor Pool Health

```rust
let status = pool.status();
if status.utilization() > 80.0 {
    log::warn!("Database pool utilization high: {:.1}%", status.utilization());
}
```

## Summary

| Feature | Description |
|---------|-------------|
| Async pools | `deadpool`, `bb8`, `mobc` |
| Backends | PostgreSQL, MySQL |
| Transactions | Full support with isolation levels |
| DI integration | Works with Armature modules |
| Health checks | Pool status and utilization |

