//! Database connection management for SeaORM.

use crate::{DatabaseConfig, SeaOrmError, SeaOrmResult};
use armature_log::{debug, info};
use sea_orm::{ConnectionTrait, DatabaseConnection};
use std::sync::Arc;

/// Database wrapper providing connection management.
///
/// `Clone` is implemented unless the `mock` feature is enabled, mirroring
/// SeaORM's own `DatabaseConnection`, which is not `Clone` under `mock`.
#[cfg_attr(not(feature = "mock"), derive(Clone))]
pub struct Database {
    conn: DatabaseConnection,
    config: Arc<DatabaseConfig>,
}

impl Database {
    /// Connect to the database with the given configuration.
    pub async fn connect(config: DatabaseConfig) -> SeaOrmResult<Self> {
        info!("Connecting to database");
        debug!(
            "Database URL: {}",
            &config.database_url[..config
                .database_url
                .find('@')
                .unwrap_or(config.database_url.len())]
        );

        let options = config.to_connect_options();
        let conn = sea_orm::Database::connect(options)
            .await
            .map_err(|e| SeaOrmError::Connection(e.to_string()))?;

        info!("Database connection established");

        Ok(Self {
            conn,
            config: Arc::new(config),
        })
    }

    /// Connect using environment variables.
    pub async fn connect_from_env() -> SeaOrmResult<Self> {
        let config = DatabaseConfig::from_env()?;
        Self::connect(config).await
    }

    /// Get a reference to the underlying connection.
    pub fn connection(&self) -> &DatabaseConnection {
        &self.conn
    }

    /// Get the configuration.
    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// Ping the database to check connectivity.
    pub async fn ping(&self) -> SeaOrmResult<()> {
        debug!("Pinging database");
        self.conn
            .ping()
            .await
            .map_err(|e| SeaOrmError::Connection(e.to_string()))
    }

    /// Close the database connection.
    pub async fn close(self) -> SeaOrmResult<()> {
        info!("Closing database connection");
        self.conn
            .close()
            .await
            .map_err(|e| SeaOrmError::Connection(e.to_string()))
    }

    /// Get database backend information.
    pub fn backend(&self) -> DatabaseBackend {
        match self.conn.get_database_backend() {
            sea_orm::DatabaseBackend::Postgres => DatabaseBackend::Postgres,
            sea_orm::DatabaseBackend::MySql => DatabaseBackend::MySql,
            sea_orm::DatabaseBackend::Sqlite => DatabaseBackend::Sqlite,
        }
    }
}

impl std::ops::Deref for Database {
    type Target = DatabaseConnection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl AsRef<DatabaseConnection> for Database {
    fn as_ref(&self) -> &DatabaseConnection {
        &self.conn
    }
}

/// Database backend types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    /// PostgreSQL.
    Postgres,
    /// MySQL/MariaDB.
    MySql,
    /// SQLite.
    Sqlite,
}

impl DatabaseBackend {
    /// Get the backend name.
    pub fn name(&self) -> &'static str {
        match self {
            DatabaseBackend::Postgres => "PostgreSQL",
            DatabaseBackend::MySql => "MySQL",
            DatabaseBackend::Sqlite => "SQLite",
        }
    }

    /// Check if this is PostgreSQL.
    pub fn is_postgres(&self) -> bool {
        matches!(self, DatabaseBackend::Postgres)
    }

    /// Check if this is MySQL.
    pub fn is_mysql(&self) -> bool {
        matches!(self, DatabaseBackend::MySql)
    }

    /// Check if this is SQLite.
    pub fn is_sqlite(&self) -> bool {
        matches!(self, DatabaseBackend::Sqlite)
    }
}

/// Health check for the database.
pub struct DatabaseHealth {
    /// Whether the database is reachable.
    pub is_healthy: bool,
    /// Response time in milliseconds.
    pub response_time_ms: u64,
    /// Backend type.
    pub backend: DatabaseBackend,
    /// Error message if unhealthy.
    pub error: Option<String>,
}

impl Database {
    /// Perform a health check.
    pub async fn health_check(&self) -> DatabaseHealth {
        use std::time::Instant;

        let start = Instant::now();
        let result = self.ping().await;
        let elapsed = start.elapsed();

        DatabaseHealth {
            is_healthy: result.is_ok(),
            response_time_ms: elapsed.as_millis() as u64,
            backend: self.backend(),
            error: result.err().map(|e| e.to_string()),
        }
    }
}
