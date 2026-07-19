//! Transaction management for SeaORM.

use crate::Database;
use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction,
    IsolationLevel as SeaIsolationLevel, TransactionTrait,
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

    /// Begin a new database transaction using the full set of
    /// [`TransactionOptions`] (isolation level, read-only access mode, and
    /// PostgreSQL `DEFERRABLE`) and return the raw [`DatabaseTransaction`].
    ///
    /// `isolation` and `read_only` are applied through SeaORM's native
    /// `BEGIN [ISOLATION LEVEL ...] [READ ONLY | READ WRITE]` support
    /// (`begin_with_config`). `deferrable` has no equivalent parameter in
    /// SeaORM's transaction API, so when the connection is PostgreSQL this
    /// issues a follow-up `SET TRANSACTION DEFERRABLE` inside the just-opened
    /// transaction (matching `psql`'s own two-statement idiom for the same
    /// setting). On non-PostgreSQL backends `deferrable` is a documented
    /// no-op: it is silently ignored (with a debug log) rather than erroring,
    /// exactly as PostgreSQL itself treats `DEFERRABLE` as inert unless the
    /// transaction is also `SERIALIZABLE READ ONLY`.
    ///
    /// As with [`TransactionExt::begin_transaction`], no closure is taken and
    /// no auto-commit/auto-rollback occurs; the caller commits or rolls back
    /// explicitly.
    fn begin_transaction_with_options(
        &self,
        options: &TransactionOptions,
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

    async fn begin_transaction_with_options(
        &self,
        options: &TransactionOptions,
    ) -> Result<DatabaseTransaction, sea_orm::DbErr> {
        armature_log::debug!("Starting database transaction with options {:?}", options);
        let txn = self
            .connection()
            .begin_with_config(options.to_isolation_level(), options.to_access_mode())
            .await?;
        apply_deferrable(&txn, options).await?;
        Ok(txn)
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

    async fn begin_transaction_with_options(
        &self,
        options: &TransactionOptions,
    ) -> Result<DatabaseTransaction, sea_orm::DbErr> {
        let txn = self
            .begin_with_config(options.to_isolation_level(), options.to_access_mode())
            .await?;
        apply_deferrable(&txn, options).await?;
        Ok(txn)
    }
}

/// Apply `options.deferrable` to an already-opened transaction.
///
/// SeaORM's `begin_with_config`/`transaction_with_config` only accept an
/// isolation level and an [`AccessMode`]; there is no parameter for
/// PostgreSQL's `DEFERRABLE` transaction property. This issues the
/// PostgreSQL-only follow-up statement directly on the transaction when
/// applicable, and is a documented no-op elsewhere.
async fn apply_deferrable(
    txn: &DatabaseTransaction,
    options: &TransactionOptions,
) -> Result<(), sea_orm::DbErr> {
    match deferrable_statement(options, txn.get_database_backend()) {
        Some(stmt) => {
            txn.execute_unprepared(stmt).await?;
            Ok(())
        }
        None => {
            if options.deferrable {
                armature_log::debug!(
                    "TransactionOptions::deferrable has no effect outside PostgreSQL; ignoring"
                );
            }
            Ok(())
        }
    }
}

/// Pure helper: decide whether (and which) `SET TRANSACTION` statement to
/// issue for `options.deferrable` on the given backend.
///
/// Factored out of [`apply_deferrable`] so the decision can be unit-tested
/// without a database connection.
fn deferrable_statement(
    options: &TransactionOptions,
    backend: DatabaseBackend,
) -> Option<&'static str> {
    if options.deferrable && backend == DatabaseBackend::Postgres {
        Some("SET TRANSACTION DEFERRABLE")
    } else {
        None
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
/// let result = run_transaction(db.connection(), |txn| {
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
    ///
    /// Applied via [`TransactionExt::begin_transaction_with_options`] using
    /// SeaORM's native `begin_with_config` isolation-level parameter.
    pub isolation: Option<IsolationLevel>,
    /// Read-only transaction.
    ///
    /// Applied via [`TransactionExt::begin_transaction_with_options`] using
    /// SeaORM's native `begin_with_config` access-mode parameter (see
    /// [`TransactionOptions::to_access_mode`]).
    pub read_only: bool,
    /// Deferrable (PostgreSQL only).
    ///
    /// SeaORM's transaction API has no parameter for this — `begin_with_config`
    /// only accepts an isolation level and an access mode. When applied via
    /// [`TransactionExt::begin_transaction_with_options`] on a PostgreSQL
    /// connection, this is honored by issuing a follow-up
    /// `SET TRANSACTION DEFERRABLE` statement inside the opened transaction.
    /// On any other backend it is a documented no-op (ignored, not an error),
    /// matching PostgreSQL's own rule that `DEFERRABLE` has no effect unless
    /// the transaction is also `SERIALIZABLE READ ONLY`.
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

    /// Convert to SeaORM's isolation level type, for passing to
    /// `begin_with_config`/`transaction_with_config`.
    pub fn to_isolation_level(&self) -> Option<SeaIsolationLevel> {
        self.isolation.map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_access_mode_reflects_read_only() {
        assert_eq!(
            TransactionOptions::new().read_only(true).to_access_mode(),
            Some(AccessMode::ReadOnly)
        );
        assert_eq!(
            TransactionOptions::new().read_only(false).to_access_mode(),
            None
        );
        assert_eq!(TransactionOptions::new().to_access_mode(), None);
    }

    #[test]
    fn to_isolation_level_maps_every_variant() {
        let cases = [
            (
                IsolationLevel::ReadUncommitted,
                SeaIsolationLevel::ReadUncommitted,
            ),
            (
                IsolationLevel::ReadCommitted,
                SeaIsolationLevel::ReadCommitted,
            ),
            (
                IsolationLevel::RepeatableRead,
                SeaIsolationLevel::RepeatableRead,
            ),
            (
                IsolationLevel::Serializable,
                SeaIsolationLevel::Serializable,
            ),
        ];

        for (ours, sea) in cases {
            let opts = TransactionOptions::new().isolation(ours);
            assert_eq!(opts.to_isolation_level(), Some(sea));
        }

        assert_eq!(TransactionOptions::new().to_isolation_level(), None);
    }

    #[test]
    fn deferrable_statement_only_fires_on_postgres_when_requested() {
        let deferrable = TransactionOptions::new().deferrable(true);
        let not_deferrable = TransactionOptions::new().deferrable(false);

        assert_eq!(
            deferrable_statement(&deferrable, DatabaseBackend::Postgres),
            Some("SET TRANSACTION DEFERRABLE")
        );
        assert_eq!(
            deferrable_statement(&not_deferrable, DatabaseBackend::Postgres),
            None
        );
        assert_eq!(
            deferrable_statement(&deferrable, DatabaseBackend::MySql),
            None
        );
        assert_eq!(
            deferrable_statement(&deferrable, DatabaseBackend::Sqlite),
            None
        );
    }

    #[test]
    fn transaction_options_builder_sets_all_fields() {
        let opts = TransactionOptions::new()
            .isolation(IsolationLevel::Serializable)
            .read_only(true)
            .deferrable(true);

        assert_eq!(opts.isolation, Some(IsolationLevel::Serializable));
        assert!(opts.read_only);
        assert!(opts.deferrable);
    }
}
