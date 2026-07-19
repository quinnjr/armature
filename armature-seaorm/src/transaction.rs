//! Transaction management for SeaORM.

use crate::Database;
use sea_orm::{
    AccessMode, DatabaseConnection, DatabaseTransaction, IsolationLevel as SeaIsolationLevel,
    TransactionTrait,
};
use std::future::Future;
use std::pin::Pin;

/// Extension trait for transaction management.
pub trait TransactionExt {
    /// Begin a new database transaction and return the raw [`DatabaseTransaction`].
    ///
    /// This does **not** take a closure and does **not** auto-commit or
    /// auto-rollback: the caller is responsible for calling `.commit()` or
    /// `.rollback()` on the returned transaction. For closure-scoped
    /// transactions that commit on `Ok` and roll back on `Err` automatically,
    /// use [`run_transaction`] (or [`run_transaction_with_isolation`]) instead.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let txn = db.begin_transaction().await?;
    /// // ... do work with txn ...
    /// txn.commit().await?;
    /// ```
    fn begin_transaction(
        &self,
    ) -> impl Future<Output = Result<DatabaseTransaction, sea_orm::DbErr>> + Send;

    /// Begin a new database transaction with a custom isolation level and
    /// return the raw [`DatabaseTransaction`].
    ///
    /// As with [`TransactionExt::begin_transaction`], no closure is taken and
    /// no auto-commit/auto-rollback occurs; the caller commits or rolls back
    /// explicitly. See [`run_transaction_with_isolation`] for the
    /// closure-based, auto-committing equivalent.
    fn begin_transaction_with_isolation(
        &self,
        isolation: IsolationLevel,
    ) -> impl Future<Output = Result<DatabaseTransaction, sea_orm::DbErr>> + Send;
}

/// Transaction isolation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Read uncommitted - lowest isolation.
    ReadUncommitted,
    /// Read committed.
    ReadCommitted,
    /// Repeatable read.
    RepeatableRead,
    /// Serializable - highest isolation.
    Serializable,
}

impl From<IsolationLevel> for SeaIsolationLevel {
    fn from(level: IsolationLevel) -> Self {
        match level {
            IsolationLevel::ReadUncommitted => SeaIsolationLevel::ReadUncommitted,
            IsolationLevel::ReadCommitted => SeaIsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead => SeaIsolationLevel::RepeatableRead,
            IsolationLevel::Serializable => SeaIsolationLevel::Serializable,
        }
    }
}

impl TransactionExt for Database {
    async fn begin_transaction(&self) -> Result<DatabaseTransaction, sea_orm::DbErr> {
        armature_log::debug!("Starting database transaction");
        self.connection().begin().await
    }

    async fn begin_transaction_with_isolation(
        &self,
        isolation: IsolationLevel,
    ) -> Result<DatabaseTransaction, sea_orm::DbErr> {
        armature_log::debug!(
            "Starting database transaction with isolation level {:?}",
            isolation
        );
        self.connection()
            .begin_with_config(Some(isolation.into()), None)
            .await
    }
}

impl TransactionExt for DatabaseConnection {
    async fn begin_transaction(&self) -> Result<DatabaseTransaction, sea_orm::DbErr> {
        self.begin().await
    }

    async fn begin_transaction_with_isolation(
        &self,
        isolation: IsolationLevel,
    ) -> Result<DatabaseTransaction, sea_orm::DbErr> {
        self.begin_with_config(Some(isolation.into()), None).await
    }
}

/// Helper to run a transactional closure.
///
/// This is a convenience wrapper around SeaORM's transaction API.
///
/// # Example
///
/// ```rust,ignore
/// use armature_seaorm::run_transaction;
///
/// let result = run_transaction(&db.connection(), |txn| {
///     Box::pin(async move {
///         // Do work with txn
///         Ok::<_, sea_orm::DbErr>(42)
///     })
/// }).await?;
/// ```
pub async fn run_transaction<C, F, T, E>(conn: &C, f: F) -> Result<T, sea_orm::TransactionError<E>>
where
    C: TransactionTrait,
    F: for<'c> FnOnce(
            &'c DatabaseTransaction,
        ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'c>>
        + Send,
    T: Send,
    E: std::error::Error + Send,
{
    conn.transaction(f).await
}

/// Helper to run a transactional closure with custom isolation.
pub async fn run_transaction_with_isolation<C, F, T, E>(
    conn: &C,
    isolation: IsolationLevel,
    f: F,
) -> Result<T, sea_orm::TransactionError<E>>
where
    C: TransactionTrait,
    F: for<'c> FnOnce(
            &'c DatabaseTransaction,
        ) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'c>>
        + Send,
    T: Send,
    E: std::error::Error + Send,
{
    conn.transaction_with_config(f, Some(isolation.into()), None)
        .await
}

/// Transaction options for advanced control.
#[derive(Debug, Clone, Default)]
pub struct TransactionOptions {
    /// Isolation level.
    pub isolation: Option<IsolationLevel>,
    /// Read-only transaction.
    pub read_only: bool,
    /// Deferrable (PostgreSQL only).
    pub deferrable: bool,
}

impl TransactionOptions {
    /// Create new transaction options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set isolation level.
    pub fn isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation = Some(level);
        self
    }

    /// Set read-only mode.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Set deferrable (PostgreSQL only).
    pub fn deferrable(mut self, deferrable: bool) -> Self {
        self.deferrable = deferrable;
        self
    }

    /// Convert to SeaORM access mode.
    pub fn to_access_mode(&self) -> Option<AccessMode> {
        if self.read_only {
            Some(AccessMode::ReadOnly)
        } else {
            None
        }
    }
}
