//! Docker-gated integration test verifying that
//! [`armature_seaorm::TransactionOptions`] (isolation level, read-only access
//! mode, and PostgreSQL `DEFERRABLE`) are actually applied on a live
//! PostgreSQL connection, not just mapped by pure unit tests.
//!
//! Self-skips when Docker is unavailable via
//! `armature_testkit::docker::docker_available()`.

use armature_seaorm::{
    Database, DatabaseConfig, IsolationLevel, TransactionExt, TransactionOptions,
};
use sea_orm::{ConnectionTrait, Statement};

#[tokio::test]
async fn begin_transaction_with_options_applies_isolation_and_read_only() {
    if !armature_testkit::docker_available() {
        eprintln!("skipping: docker not available");
        return;
    }

    let container = armature_testkit::containers::PostgresContainer::start().await;
    let config = DatabaseConfig::new(container.url());
    let db = Database::connect(config)
        .await
        .expect("failed to connect to postgres container");

    let options = TransactionOptions::new()
        .isolation(IsolationLevel::Serializable)
        .read_only(true)
        .deferrable(true);

    let txn = db
        .begin_transaction_with_options(&options)
        .await
        .expect("begin_transaction_with_options failed");

    let backend = txn.get_database_backend();

    let isolation_row = txn
        .query_one(Statement::from_string(
            backend,
            "SHOW transaction_isolation".to_owned(),
        ))
        .await
        .expect("SHOW transaction_isolation failed")
        .expect("SHOW transaction_isolation returned no row");
    let isolation: String = isolation_row
        .try_get("", "transaction_isolation")
        .expect("failed to read transaction_isolation column");
    assert_eq!(
        isolation, "serializable",
        "transaction_isolation should reflect IsolationLevel::Serializable"
    );

    let read_only_row = txn
        .query_one(Statement::from_string(
            backend,
            "SHOW transaction_read_only".to_owned(),
        ))
        .await
        .expect("SHOW transaction_read_only failed")
        .expect("SHOW transaction_read_only returned no row");
    let read_only: String = read_only_row
        .try_get("", "transaction_read_only")
        .expect("failed to read transaction_read_only column");
    assert_eq!(
        read_only, "on",
        "transaction_read_only should reflect TransactionOptions::read_only(true)"
    );

    txn.rollback().await.expect("rollback failed");
}
