//! Transaction management for Diesel.

use crate::{DieselError, DieselResult};
use std::future::Future;

#[cfg(feature = "postgres")]
use diesel_async::AsyncPgConnection;

#[cfg(feature = "mysql")]
use diesel_async::AsyncMysqlConnection;

/// Helper trait bridging real (lending) `async` closures with a `Send`
/// future bound.
///
/// A bound written as `F: FnOnce(&mut Conn) -> Fut, Fut: Future<...> + Send`
/// *looks* usable but can never actually be satisfied by a closure whose
/// future borrows `conn` across an `.await` point: the elided lifetime on
/// `&mut Conn` is desugared into a higher-ranked `for<'r> FnOnce(&'r mut
/// Conn) -> Fut` bound, while `Fut` remains one single, lifetime-independent
/// type - so no lending future can ever unify with it (`the impl is not
/// general enough` / "one type is more general than the other"). Native
/// `AsyncFnOnce` closures solve the lending problem via a lifetime-generic
/// associated future, but `AsyncFnOnce::CallOnceFuture` is not yet stable,
/// so it can't be bounded with `Send` directly either. This trait mirrors
/// the (internal, not publicly exported) `AsyncFunc` helper `diesel-async`
/// itself uses to work around exactly this - see
/// `diesel_async::AsyncConnection::transaction`'s own bound.
pub trait AsyncTxFn<T, R>:
    AsyncFnOnce(T) -> R + FnOnce(T) -> <Self as AsyncTxFn<T, R>>::Fut
{
    /// The concrete future type returned for a given call.
    type Fut: Future<Output = R> + Send;
}

impl<F, T, Fut, R> AsyncTxFn<T, R> for F
where
    F: AsyncFnOnce(T) -> R + FnOnce(T) -> Fut,
    Fut: Future<Output = R> + Send,
{
    type Fut = Fut;
}

/// Extension trait for transaction management.
///
/// `Connection` is a generic type *parameter* of the trait rather than an
/// associated type. That's not just style: combining an associated type
/// (`type Connection;`) with the higher-ranked `for<'r> F: ...` bound the
/// closures below need triggers a rustc limitation resolving associated-type
/// projections under a higher-ranked bound, which makes the bound
/// impossible to satisfy for *any* real closure - even inside this trait's
/// own impls. Using a generic parameter instead avoids that entirely.
#[allow(async_fn_in_trait)]
pub trait TransactionExt<Connection> {
    /// Execute a closure within a transaction.
    ///
    /// If the closure returns an error, the transaction is rolled back.
    /// Otherwise, the transaction is committed.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// pool.transaction(async |conn| {
    ///     diesel::insert_into(users::table)
    ///         .values(&new_user)
    ///         .execute(conn)
    ///         .await?;
    ///     Ok(())
    /// }).await?;
    /// ```
    async fn transaction<F, T>(&self, f: F) -> DieselResult<T>
    where
        for<'r> F: AsyncFnOnce(&'r mut Connection) -> Result<T, diesel::result::Error>
            + AsyncTxFn<&'r mut Connection, Result<T, diesel::result::Error>>
            + Send,
        T: Send;

    /// Execute a closure within a transaction with custom isolation level.
    async fn transaction_with_isolation<F, T>(
        &self,
        isolation: IsolationLevel,
        f: F,
    ) -> DieselResult<T>
    where
        for<'r> F: AsyncFnOnce(&'r mut Connection) -> Result<T, diesel::result::Error>
            + AsyncTxFn<&'r mut Connection, Result<T, diesel::result::Error>>
            + Send,
        T: Send;
}

/// Transaction isolation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Read uncommitted - lowest isolation, highest concurrency.
    ReadUncommitted,
    /// Read committed - default for PostgreSQL.
    ReadCommitted,
    /// Repeatable read - default for MySQL InnoDB.
    RepeatableRead,
    /// Serializable - highest isolation, lowest concurrency.
    Serializable,
}

impl IsolationLevel {
    /// Get the SQL representation for PostgreSQL.
    #[cfg(feature = "postgres")]
    pub fn to_pg_sql(&self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    }

    /// Get the SQL representation for MySQL.
    #[cfg(feature = "mysql")]
    pub fn to_mysql_sql(&self) -> &'static str {
        match self {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    }
}

// ============================================================================
// PostgreSQL Transaction Implementation
// ============================================================================

#[cfg(all(feature = "postgres", feature = "deadpool"))]
impl TransactionExt<AsyncPgConnection> for crate::PgPool {
    async fn transaction<F, T>(&self, f: F) -> DieselResult<T>
    where
        for<'r> F: AsyncFnOnce(&'r mut AsyncPgConnection) -> Result<T, diesel::result::Error>
            + AsyncTxFn<&'r mut AsyncPgConnection, Result<T, diesel::result::Error>>
            + Send,
        T: Send,
    {
        use diesel_async::AsyncConnection;

        let mut conn = self.get().await?;
        let conn: &mut AsyncPgConnection = &mut conn;

        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }

    async fn transaction_with_isolation<F, T>(
        &self,
        isolation: IsolationLevel,
        f: F,
    ) -> DieselResult<T>
    where
        for<'r> F: AsyncFnOnce(&'r mut AsyncPgConnection) -> Result<T, diesel::result::Error>
            + AsyncTxFn<&'r mut AsyncPgConnection, Result<T, diesel::result::Error>>
            + Send,
        T: Send,
    {
        use diesel_async::{AsyncConnection, RunQueryDsl};

        let mut conn = self.get().await?;
        let conn: &mut AsyncPgConnection = &mut conn;

        let isolation_sql = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation.to_pg_sql());

        conn.transaction::<T, diesel::result::Error, _>(async |conn| {
            // `SET TRANSACTION` only affects the *current* transaction when run
            // inside one; running it before `conn.transaction()` opens the
            // BEGIN block is a silent no-op in PostgreSQL. It must be the
            // first statement executed after BEGIN.
            diesel::sql_query(isolation_sql).execute(conn).await?;

            f(conn).await
        })
        .await
        .map_err(|e| DieselError::Transaction(e.to_string()))
    }
}

// ============================================================================
// MySQL Transaction Implementation
// ============================================================================

#[cfg(all(feature = "mysql", feature = "deadpool"))]
impl TransactionExt<AsyncMysqlConnection> for crate::MysqlPool {
    async fn transaction<F, T>(&self, f: F) -> DieselResult<T>
    where
        for<'r> F: AsyncFnOnce(&'r mut AsyncMysqlConnection) -> Result<T, diesel::result::Error>
            + AsyncTxFn<&'r mut AsyncMysqlConnection, Result<T, diesel::result::Error>>
            + Send,
        T: Send,
    {
        use diesel_async::AsyncConnection;

        let mut conn = self.get().await?;
        let conn: &mut AsyncMysqlConnection = &mut conn;

        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }

    async fn transaction_with_isolation<F, T>(
        &self,
        isolation: IsolationLevel,
        f: F,
    ) -> DieselResult<T>
    where
        for<'r> F: AsyncFnOnce(&'r mut AsyncMysqlConnection) -> Result<T, diesel::result::Error>
            + AsyncTxFn<&'r mut AsyncMysqlConnection, Result<T, diesel::result::Error>>
            + Send,
        T: Send,
    {
        use diesel_async::{AsyncConnection, RunQueryDsl};

        let mut conn = self.get().await?;
        let conn: &mut AsyncMysqlConnection = &mut conn;

        // Unlike PostgreSQL, MySQL's `SET TRANSACTION ISOLATION LEVEL` sets
        // the level for the *next* transaction and must be issued *before*
        // that transaction starts (issuing it after `BEGIN` raises
        // "Transaction characteristics can't be changed while a transaction
        // is in progress"). So, for MySQL, running it here - before
        // `conn.transaction()` opens the transaction - is correct.
        diesel::sql_query(format!(
            "SET TRANSACTION ISOLATION LEVEL {}",
            isolation.to_mysql_sql()
        ))
        .execute(conn)
        .await
        .map_err(|e| DieselError::Transaction(e.to_string()))?;

        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }
}

/// Transaction guard for manual transaction management.
#[allow(dead_code)]
pub struct TransactionGuard<'a, C> {
    conn: &'a mut C,
    committed: bool,
}

impl<'a, C> TransactionGuard<'a, C> {
    /// Create a new transaction guard.
    pub fn new(conn: &'a mut C) -> Self {
        Self {
            conn,
            committed: false,
        }
    }

    /// Get a reference to the connection.
    pub fn conn(&mut self) -> &mut C {
        self.conn
    }

    /// Commit the transaction.
    pub fn commit(mut self) {
        self.committed = true;
    }
}

// On drop, if not committed, the transaction will be rolled back
// (handled by the connection's transaction scope)

#[cfg(all(test, feature = "postgres", feature = "deadpool"))]
mod isolation_level_tests {
    use super::*;
    use crate::{DieselConfig, PgPool};
    use diesel::QueryableByName;
    use diesel::sql_types::Text;
    use diesel_async::RunQueryDsl;

    #[derive(QueryableByName)]
    struct IsolationRow {
        #[diesel(sql_type = Text)]
        transaction_isolation: String,
    }

    #[tokio::test]
    async fn transaction_with_isolation_actually_applies_the_level() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: docker not available");
            return;
        }

        let container = armature_testkit::containers::PostgresContainer::start().await;
        let config = DieselConfig::new(container.url());
        let pool = PgPool::new(config).await.expect("failed to build pg pool");

        let observed = pool
            .transaction_with_isolation(IsolationLevel::Serializable, async |conn| {
                let rows: Vec<IsolationRow> = diesel::sql_query("SHOW transaction_isolation")
                    .load(conn)
                    .await?;
                Ok(rows.into_iter().next().unwrap().transaction_isolation)
            })
            .await
            .expect("transaction_with_isolation failed");

        assert_eq!(observed, "serializable");
    }
}
