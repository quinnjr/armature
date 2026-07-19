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
// Isolation statement ordering (shared between backends)
// ============================================================================

/// One of the two operations `transaction_with_isolation` performs: opening
/// the transaction (`BEGIN`, via `conn.transaction()`) and applying the
/// isolation level (`SET TRANSACTION ISOLATION LEVEL ...`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxOp {
    /// `conn.transaction()` opening `BEGIN`.
    Begin,
    /// `SET TRANSACTION ISOLATION LEVEL ...`.
    SetIsolation,
}

/// Ordered sequence of operations for PostgreSQL.
///
/// `SET TRANSACTION ISOLATION LEVEL` only affects the transaction it runs
/// *inside*; run before `BEGIN` it is a silent no-op. It must therefore come
/// after `Begin`.
pub(crate) const PG_ISOLATION_ORDER: [TxOp; 2] = [TxOp::Begin, TxOp::SetIsolation];

/// Ordered sequence of operations for MySQL.
///
/// `SET TRANSACTION ISOLATION LEVEL` sets the level for the *next*
/// transaction and errors ("Transaction characteristics can't be changed
/// while a transaction is in progress") if issued after `BEGIN`. It must
/// therefore come before `Begin`.
// Only consumed by the MySQL `transaction_with_isolation` impl below (and by
// the docker-free ordering test); with the `mysql` feature disabled (the
// default), nothing in this crate reads it.
#[allow(dead_code)]
pub(crate) const MYSQL_ISOLATION_ORDER: [TxOp; 2] = [TxOp::SetIsolation, TxOp::Begin];

/// Runs `f` inside a transaction with `isolation_sql` applied, in the order
/// dictated by `order` (see [`PG_ISOLATION_ORDER`] / [`MYSQL_ISOLATION_ORDER`]).
///
/// Both backends need the same two operations - open the transaction and
/// apply the isolation level - and differ only in which one must run first.
/// Branching on the shared `order` constants here (instead of hand-writing
/// the sequence once per backend) means a docker-free test can assert
/// directly on those constants and know it is testing the exact same
/// decision the real Postgres/MySQL code paths make, not a copy of it that
/// could silently drift out of sync.
async fn run_with_ordered_isolation<Conn, F, T>(
    order: [TxOp; 2],
    conn: &mut Conn,
    isolation_sql: String,
    f: F,
) -> Result<T, diesel::result::Error>
where
    Conn: diesel_async::AsyncConnection,
    <Conn as diesel_async::AsyncConnectionCore>::Backend:
        diesel::backend::DieselReserveSpecialization,
    for<'r> F: AsyncFnOnce(&'r mut Conn) -> Result<T, diesel::result::Error>
        + AsyncTxFn<&'r mut Conn, Result<T, diesel::result::Error>>
        + Send,
    T: Send,
{
    use diesel_async::RunQueryDsl;

    if order[0] == TxOp::SetIsolation {
        // Before `Begin`: issue the isolation SQL first, then open the
        // transaction and run the body.
        diesel::sql_query(isolation_sql).execute(conn).await?;
        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
    } else {
        // After `Begin`: open the transaction, then issue the isolation SQL
        // as the first statement inside it, then run the body.
        conn.transaction::<T, diesel::result::Error, _>(async |conn| {
            diesel::sql_query(isolation_sql).execute(conn).await?;
            f(conn).await
        })
        .await
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
        let mut conn = self.get().await?;
        let conn: &mut AsyncPgConnection = &mut conn;

        let isolation_sql = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation.to_pg_sql());

        run_with_ordered_isolation(PG_ISOLATION_ORDER, conn, isolation_sql, f)
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
        let mut conn = self.get().await?;
        let conn: &mut AsyncMysqlConnection = &mut conn;

        let isolation_sql = format!(
            "SET TRANSACTION ISOLATION LEVEL {}",
            isolation.to_mysql_sql()
        );

        run_with_ordered_isolation(MYSQL_ISOLATION_ORDER, conn, isolation_sql, f)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }
}

/// Transaction guard for manual transaction management.
///
/// This wraps a connection that already has an open transaction (e.g. after
/// issuing a raw `BEGIN`) and tracks whether it has been explicitly finished.
///
/// **Important:** unlike a typical RAII guard, dropping a [`TransactionGuard`]
/// without calling [`commit`](Self::commit) or [`rollback`](Self::rollback)
/// does **not** roll back the transaction. Both `COMMIT` and `ROLLBACK` are
/// async database round-trips, and Rust's [`Drop`] is a synchronous callback,
/// so there is no way to `.await` a rollback query from it. Because a
/// lending async close over `&mut C` can't be driven from `Drop`, this type
/// cannot honestly offer rollback-on-drop: instead, `commit()`/`rollback()`
/// issue the real `COMMIT`/`ROLLBACK` SQL themselves, and `Drop` only logs a
/// warning if neither was called, so the outstanding transaction is never
/// silently forgotten. Callers must call one of them explicitly before the
/// guard goes out of scope.
pub struct TransactionGuard<'a, C> {
    conn: &'a mut C,
    finished: bool,
}

impl<'a, C> TransactionGuard<'a, C> {
    /// Create a new transaction guard around a connection with an already
    /// open transaction.
    pub fn new(conn: &'a mut C) -> Self {
        Self {
            conn,
            finished: false,
        }
    }

    /// Get a reference to the connection.
    pub fn conn(&mut self) -> &mut C {
        self.conn
    }
}

/// Generates `commit()`/`rollback()` inherent methods on
/// `TransactionGuard<'a, $conn>` for a concrete diesel-async connection type.
///
/// Both methods differ only in which SQL statement they issue, so this macro
/// (rather than four near-identical hand-written bodies, one pair per
/// backend) is the single place that owns the shared behavior: **the guard
/// is marked `finished` *before* the statement is sent**, not after it
/// succeeds. Marking it after would mean a `COMMIT`/`ROLLBACK` that fails
/// (e.g. connection dropped mid-flight) leaves `finished == false`, and since
/// `self` is consumed by value here regardless of the `Result`, `Drop` would
/// then fire its "dropped without calling commit()/rollback()" warning even
/// though one of them plainly *was* called. The guard's contract is "was
/// commit()/rollback() invoked", not "did it succeed" - callers still get the
/// real `Err` back to handle.
macro_rules! impl_transaction_guard_finish {
    ($conn:ty) => {
        impl<'a> TransactionGuard<'a, $conn> {
            /// Commit the transaction by issuing `COMMIT`.
            pub async fn commit(mut self) -> DieselResult<()> {
                self.finished = true;
                Self::finish(self.conn, "COMMIT").await
            }

            /// Roll back the transaction by issuing `ROLLBACK`.
            pub async fn rollback(mut self) -> DieselResult<()> {
                self.finished = true;
                Self::finish(self.conn, "ROLLBACK").await
            }

            async fn finish(conn: &mut $conn, sql: &'static str) -> DieselResult<()> {
                use diesel_async::RunQueryDsl;

                diesel::sql_query(sql)
                    .execute(conn)
                    .await
                    .map_err(|e| DieselError::Transaction(e.to_string()))?;
                Ok(())
            }
        }
    };
}

#[cfg(feature = "postgres")]
impl_transaction_guard_finish!(AsyncPgConnection);

#[cfg(feature = "mysql")]
impl_transaction_guard_finish!(AsyncMysqlConnection);

impl<'a, C> Drop for TransactionGuard<'a, C> {
    fn drop(&mut self) {
        if !self.finished {
            armature_log::warn!(
                "TransactionGuard dropped without calling commit() or rollback(); \
                 the open transaction was NOT rolled back (async rollback cannot run \
                 from a synchronous Drop) - call commit() or rollback() explicitly"
            );
        }
    }
}

/// Docker-free unit tests for the `IsolationLevel` -> SQL text mapping.
///
/// `transaction_with_isolation` builds its `SET TRANSACTION ISOLATION LEVEL
/// ...` statement from `IsolationLevel::to_pg_sql`/`to_mysql_sql`; that
/// string-building step is pure and needs no live database connection. The
/// existing `isolation_level_tests` module below proves the level is
/// actually *applied* by the server, but self-skips without Docker, so
/// Docker-free CI never exercises the enum -> SQL contract at all. These
/// tests close that gap by asserting the mapping directly.
#[cfg(test)]
mod isolation_level_sql_mapping_tests {
    use super::IsolationLevel;

    #[cfg(feature = "postgres")]
    #[test]
    fn to_pg_sql_maps_every_level_to_the_expected_sql_keyword() {
        assert_eq!(
            IsolationLevel::ReadUncommitted.to_pg_sql(),
            "READ UNCOMMITTED"
        );
        assert_eq!(IsolationLevel::ReadCommitted.to_pg_sql(), "READ COMMITTED");
        assert_eq!(
            IsolationLevel::RepeatableRead.to_pg_sql(),
            "REPEATABLE READ"
        );
        assert_eq!(IsolationLevel::Serializable.to_pg_sql(), "SERIALIZABLE");
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn to_mysql_sql_maps_every_level_to_the_expected_sql_keyword() {
        assert_eq!(
            IsolationLevel::ReadUncommitted.to_mysql_sql(),
            "READ UNCOMMITTED"
        );
        assert_eq!(
            IsolationLevel::ReadCommitted.to_mysql_sql(),
            "READ COMMITTED"
        );
        assert_eq!(
            IsolationLevel::RepeatableRead.to_mysql_sql(),
            "REPEATABLE READ"
        );
        assert_eq!(IsolationLevel::Serializable.to_mysql_sql(), "SERIALIZABLE");
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn pg_isolation_sql_statement_matches_the_exact_syntax_the_server_expects() {
        // Mirrors the `format!` call in `transaction_with_isolation` for
        // Postgres verbatim, so a change to that statement's shape (e.g. a
        // typo'd keyword or missing "LEVEL") fails here without Docker.
        let sql = format!(
            "SET TRANSACTION ISOLATION LEVEL {}",
            IsolationLevel::Serializable.to_pg_sql()
        );
        assert_eq!(sql, "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE");
    }

    #[cfg(feature = "mysql")]
    #[test]
    fn mysql_isolation_sql_statement_matches_the_exact_syntax_the_server_expects() {
        // Mirrors the `format!` call in `transaction_with_isolation` for
        // MySQL verbatim (see the note there about issuing this *before*
        // `BEGIN`, unlike Postgres).
        let sql = format!(
            "SET TRANSACTION ISOLATION LEVEL {}",
            IsolationLevel::RepeatableRead.to_mysql_sql()
        );
        assert_eq!(sql, "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ");
    }
}

/// Docker-free test proving the isolation-SQL-vs-`BEGIN` *ordering* itself,
/// not just the enum -> SQL string mapping (`isolation_level_sql_mapping_tests`
/// above). The old (broken-placement) code passed the mapping tests just as
/// well as the fixed code - the bug was in *where* the statement runs
/// relative to `BEGIN`, which those tests never exercised. `PG_ISOLATION_ORDER`
/// / `MYSQL_ISOLATION_ORDER` are the exact constants `run_with_ordered_isolation`
/// branches on for the real Postgres/MySQL code paths (see
/// `transaction_with_isolation` on `PgPool`/`MysqlPool`), so asserting on them
/// here catches a regression that swaps the order without needing Docker.
#[cfg(test)]
mod isolation_ordering_tests {
    use super::{MYSQL_ISOLATION_ORDER, PG_ISOLATION_ORDER, TxOp};

    #[test]
    fn postgres_runs_set_isolation_after_begin() {
        assert_eq!(PG_ISOLATION_ORDER, [TxOp::Begin, TxOp::SetIsolation]);
        assert_eq!(
            PG_ISOLATION_ORDER.iter().position(|op| *op == TxOp::Begin),
            Some(0),
            "BEGIN must be the first operation for PostgreSQL"
        );
        assert_eq!(
            PG_ISOLATION_ORDER
                .iter()
                .position(|op| *op == TxOp::SetIsolation),
            Some(1),
            "SET TRANSACTION ISOLATION LEVEL must run after BEGIN for PostgreSQL"
        );
    }

    #[test]
    fn mysql_runs_set_isolation_before_begin() {
        assert_eq!(MYSQL_ISOLATION_ORDER, [TxOp::SetIsolation, TxOp::Begin]);
        assert_eq!(
            MYSQL_ISOLATION_ORDER
                .iter()
                .position(|op| *op == TxOp::SetIsolation),
            Some(0),
            "SET TRANSACTION ISOLATION LEVEL must run before BEGIN for MySQL"
        );
        assert_eq!(
            MYSQL_ISOLATION_ORDER
                .iter()
                .position(|op| *op == TxOp::Begin),
            Some(1),
            "BEGIN must run after SET TRANSACTION ISOLATION LEVEL for MySQL"
        );
    }
}

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

#[cfg(all(test, feature = "postgres", feature = "deadpool"))]
mod transaction_guard_tests {
    use super::*;
    use crate::{DieselConfig, PgPool};
    use diesel::QueryableByName;
    use diesel::sql_types::BigInt;
    use diesel_async::RunQueryDsl;

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }

    async fn row_count(conn: &mut AsyncPgConnection) -> i64 {
        let rows: Vec<CountRow> = diesel::sql_query("SELECT COUNT(*) AS count FROM tg_test")
            .load(conn)
            .await
            .expect("count query failed");
        rows.into_iter().next().unwrap().count
    }

    #[tokio::test]
    async fn rollback_actually_reverts_the_change() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: docker not available");
            return;
        }

        let container = armature_testkit::containers::PostgresContainer::start().await;
        let config = DieselConfig::new(container.url());
        let pool = PgPool::new(config).await.expect("failed to build pg pool");
        let mut conn = pool.get().await.expect("failed to get connection");
        let conn: &mut AsyncPgConnection = &mut conn;

        diesel::sql_query("CREATE TABLE tg_test (id INT)")
            .execute(conn)
            .await
            .expect("create table failed");

        diesel::sql_query("BEGIN")
            .execute(conn)
            .await
            .expect("begin failed");

        let mut guard = TransactionGuard::new(conn);
        diesel::sql_query("INSERT INTO tg_test (id) VALUES (1)")
            .execute(guard.conn())
            .await
            .expect("insert failed");

        guard.rollback().await.expect("rollback failed");

        assert_eq!(
            row_count(conn).await,
            0,
            "row inserted before rollback() should not be visible after it"
        );
    }

    #[tokio::test]
    async fn commit_actually_persists_the_change() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: docker not available");
            return;
        }

        let container = armature_testkit::containers::PostgresContainer::start().await;
        let config = DieselConfig::new(container.url());
        let pool = PgPool::new(config).await.expect("failed to build pg pool");
        let mut conn = pool.get().await.expect("failed to get connection");
        let conn: &mut AsyncPgConnection = &mut conn;

        diesel::sql_query("CREATE TABLE tg_test (id INT)")
            .execute(conn)
            .await
            .expect("create table failed");

        diesel::sql_query("BEGIN")
            .execute(conn)
            .await
            .expect("begin failed");

        let mut guard = TransactionGuard::new(conn);
        diesel::sql_query("INSERT INTO tg_test (id) VALUES (1)")
            .execute(guard.conn())
            .await
            .expect("insert failed");

        guard.commit().await.expect("commit failed");

        assert_eq!(
            row_count(conn).await,
            1,
            "row inserted before commit() should be visible after it"
        );
    }
}
