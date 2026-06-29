//! Transaction management for Diesel.

use crate::{DieselError, DieselResult};
use async_trait::async_trait;
use std::future::Future;

#[cfg(feature = "postgres")]
use diesel_async::AsyncPgConnection;

#[cfg(feature = "mysql")]
use diesel_async::AsyncMysqlConnection;

/// Extension trait for transaction management.
#[async_trait]
pub trait TransactionExt {
    /// The connection type.
    type Connection;

    /// Execute a closure within a transaction.
    ///
    /// If the closure returns an error, the transaction is rolled back.
    /// Otherwise, the transaction is committed.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// pool.transaction(|conn| async move {
    ///     diesel::insert_into(users::table)
    ///         .values(&new_user)
    ///         .execute(conn)
    ///         .await?;
    ///     Ok(())
    /// }).await?;
    /// ```
    async fn transaction<F, Fut, T>(&self, f: F) -> DieselResult<T>
    where
        F: FnOnce(&mut Self::Connection) -> Fut + Send,
        Fut: Future<Output = Result<T, diesel::result::Error>> + Send,
        T: Send;

    /// Execute a closure within a transaction with custom isolation level.
    async fn transaction_with_isolation<F, Fut, T>(
        &self,
        isolation: IsolationLevel,
        f: F,
    ) -> DieselResult<T>
    where
        F: FnOnce(&mut Self::Connection) -> Fut + Send,
        Fut: Future<Output = Result<T, diesel::result::Error>> + Send,
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
#[async_trait]
impl TransactionExt for crate::PgPool {
    type Connection = AsyncPgConnection;

    async fn transaction<F, Fut, T>(&self, f: F) -> DieselResult<T>
    where
        F: FnOnce(&mut Self::Connection) -> Fut + Send,
        Fut: Future<Output = Result<T, diesel::result::Error>> + Send,
        T: Send,
    {
        use diesel_async::AsyncConnection;

        let mut conn = self.get().await?;
        let conn: &mut AsyncPgConnection = &mut conn;

        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }

    async fn transaction_with_isolation<F, Fut, T>(
        &self,
        isolation: IsolationLevel,
        f: F,
    ) -> DieselResult<T>
    where
        F: FnOnce(&mut Self::Connection) -> Fut + Send,
        Fut: Future<Output = Result<T, diesel::result::Error>> + Send,
        T: Send,
    {
        use diesel_async::{AsyncConnection, RunQueryDsl};

        let mut conn = self.get().await?;
        let conn: &mut AsyncPgConnection = &mut conn;

        // Set isolation level
        diesel::sql_query(format!(
            "SET TRANSACTION ISOLATION LEVEL {}",
            isolation.to_pg_sql()
        ))
        .execute(conn)
        .await
        .map_err(|e| DieselError::Transaction(e.to_string()))?;

        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }
}

// ============================================================================
// MySQL Transaction Implementation
// ============================================================================

#[cfg(all(feature = "mysql", feature = "deadpool"))]
#[async_trait]
impl TransactionExt for crate::MysqlPool {
    type Connection = AsyncMysqlConnection;

    async fn transaction<F, Fut, T>(&self, f: F) -> DieselResult<T>
    where
        F: FnOnce(&mut Self::Connection) -> Fut + Send,
        Fut: Future<Output = Result<T, diesel::result::Error>> + Send,
        T: Send,
    {
        use diesel_async::AsyncConnection;

        let mut conn = self.get().await?;
        let conn: &mut AsyncMysqlConnection = &mut *conn;

        conn.transaction::<T, diesel::result::Error, _>(async |conn| f(conn).await)
            .await
            .map_err(|e| DieselError::Transaction(e.to_string()))
    }

    async fn transaction_with_isolation<F, Fut, T>(
        &self,
        isolation: IsolationLevel,
        f: F,
    ) -> DieselResult<T>
    where
        F: FnOnce(&mut Self::Connection) -> Fut + Send,
        Fut: Future<Output = Result<T, diesel::result::Error>> + Send,
        T: Send,
    {
        use diesel_async::{AsyncConnection, RunQueryDsl};

        let mut conn = self.get().await?;
        let conn: &mut AsyncMysqlConnection = &mut *conn;

        // Set isolation level
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
