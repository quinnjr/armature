# armature-diesel

Diesel async database integration for the Armature framework.

## Features

- **Async/Await** - Non-blocking database operations via `diesel-async`
- **Connection Pooling** - `deadpool` (default) or `bb8` backed pools
- **PostgreSQL** - Full PostgreSQL support (`postgres` feature, on by default)
- **MySQL** - MySQL/MariaDB support (`mysql` feature)
- **Transactions** - `TransactionExt::transaction` / `transaction_with_isolation`,
  plus a manual `TransactionGuard`
- **Connection Health** - Optional checkout validation via `test_on_checkout`

## Installation

```toml
[dependencies]
armature-diesel = "0.1"
```

## Quick Start

```rust,ignore
use armature_diesel::{DieselConfig, PgPool};
use diesel_async::RunQueryDsl;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a configuration
    let config = DieselConfig::new("postgres://user:pass@localhost/mydb")
        .pool_size(10);

    // Build the pool (PostgreSQL + deadpool, the crate defaults)
    let pool = PgPool::new(config).await?;

    // Get a connection and run a query
    let mut conn = pool.get().await?;
    let users = users::table.load::<User>(&mut conn).await?;

    Ok(())
}
```

## With Transactions

`TransactionExt` is implemented directly on the pool types (`PgPool`,
`MysqlPool`) - there is no separate `pool.interact(...)` step:

```rust,ignore
use armature_diesel::TransactionExt;

pool.transaction(async |conn| {
    diesel::insert_into(users::table)
        .values(&new_user)
        .execute(conn)
        .await?;

    diesel::insert_into(profiles::table)
        .values(&new_profile)
        .execute(conn)
        .await?;

    Ok(())
})
.await?;
```

Need a specific isolation level for the transaction? Use
`transaction_with_isolation`:

```rust,ignore
use armature_diesel::{IsolationLevel, TransactionExt};

pool.transaction_with_isolation(IsolationLevel::Serializable, async |conn| {
    // ...
    Ok(())
})
.await?;
```

For cases where a transaction needs to span multiple calls instead of a
single closure, use `TransactionGuard` around a connection that already has
an open transaction. Because `COMMIT`/`ROLLBACK` are async round-trips and
`Drop` is synchronous, the guard cannot roll back automatically when
dropped - call `commit()` or `rollback()` explicitly:

```rust,ignore
use armature_diesel::TransactionGuard;
use diesel_async::RunQueryDsl;

let mut conn = pool.get().await?;
diesel::sql_query("BEGIN").execute(&mut conn).await?;

let mut guard = TransactionGuard::new(&mut conn);
diesel::insert_into(users::table)
    .values(&new_user)
    .execute(guard.conn())
    .await?;

guard.commit().await?;
```

## Pool Configuration

```rust,ignore
use armature_diesel::{DieselConfig, PgPool};
use std::time::Duration;

let config = DieselConfig::new("postgres://localhost/mydb")
    .pool_size(20)
    .min_idle(5)
    .connect_timeout(Duration::from_secs(5))
    .max_lifetime(Duration::from_secs(30 * 60))
    .idle_timeout(Duration::from_secs(10 * 60))
    .test_on_checkout(true);

let pool = PgPool::new(config).await?;

// Or from environment variables (DATABASE_URL, DATABASE_POOL_SIZE, ...):
let config = DieselConfig::from_env()?;
```

The `bb8` feature enables an alternate pool type, `PgPoolBb8`, built the
same way from a `DieselConfig`.

## License

MIT OR Apache-2.0
