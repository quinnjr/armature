# armature-seaorm

SeaORM integration for the Armature framework.

## Features

- **Async ORM** - Full async/await support
- **Active Record** - Entity-based CRUD operations (via re-exported SeaORM)
- **Connection Pooling** - Built-in connection management
- **Multiple Databases** - PostgreSQL, MySQL, SQLite
- **Transactions** - Isolation levels, read-only mode, and (PostgreSQL) `DEFERRABLE` via `TransactionOptions`
- **Pagination** - Offset-based, count-free, and keyset (cursor) pagination helpers

## Installation

```toml
[dependencies]
armature-seaorm = "0.1"
```

## Quick Start

```rust,ignore
use armature_seaorm::{Database, DatabaseConfig};
use entity::user;
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, Set};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DatabaseConfig::new("postgres://localhost/mydb");
    let db = Database::connect(config).await?;
    let conn = db.connection();

    // Find by ID
    let user = user::Entity::find_by_id(1).one(conn).await?;

    // Find all
    let users = user::Entity::find().all(conn).await?;

    // Insert
    let new_user = user::ActiveModel {
        name: Set("Alice".to_owned()),
        ..Default::default()
    };
    let user = new_user.insert(conn).await?;

    // Update
    let mut user: user::ActiveModel = user.into_active_model();
    user.name = Set("Bob".to_owned());
    user.update(conn).await?;

    // Delete
    user::Entity::delete_by_id(1).exec(conn).await?;

    Ok(())
}
```

## Transactions

```rust,ignore
use armature_seaorm::run_transaction;

let result = run_transaction(db.connection(), |txn| {
    Box::pin(async move {
        // ... do work with txn ...
        Ok::<_, sea_orm::DbErr>(42)
    })
})
.await?;
```

For explicit control over isolation level, read-only mode, and (on
PostgreSQL) `DEFERRABLE`, build `TransactionOptions` and open the
transaction with `TransactionExt::begin_transaction_with_options`:

```rust,ignore
use armature_seaorm::{IsolationLevel, TransactionExt, TransactionOptions};

let options = TransactionOptions::new()
    .isolation(IsolationLevel::Serializable)
    .read_only(true)
    .deferrable(true);

let txn = db.begin_transaction_with_options(&options).await?;
// ... do work with txn ...
txn.commit().await?;
```

## Pagination

```rust,ignore
use armature_seaorm::{Paginate, PaginationOptions};

let options = PaginationOptions::new(1, 10);
let page = user::Entity::find().paginate(db.connection(), &options).await?;

println!("{} of {} total items", page.items.len(), page.meta.total_items);
```

## License

MIT OR Apache-2.0
