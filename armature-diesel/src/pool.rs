//! Connection pool implementations for Diesel.

use crate::{DieselConfig, DieselError, DieselResult};
use armature_log::{debug, info};
use std::sync::Arc;

#[cfg(feature = "postgres")]
use diesel_async::AsyncPgConnection;

#[cfg(feature = "mysql")]
use diesel_async::AsyncMysqlConnection;

#[cfg(feature = "deadpool")]
use diesel_async::pooled_connection::deadpool::Pool as DeadpoolPool;

#[cfg(feature = "deadpool")]
use diesel_async::pooled_connection::deadpool::Object as DeadpoolObject;

#[cfg(feature = "bb8")]
use diesel_async::pooled_connection::bb8::Pool as Bb8Pool;

// ============================================================================
// PostgreSQL Pool
// ============================================================================

/// PostgreSQL connection pool using deadpool.
#[cfg(all(feature = "postgres", feature = "deadpool"))]
pub struct PgPool {
    pool: DeadpoolPool<AsyncPgConnection>,
    config: Arc<DieselConfig>,
}

#[cfg(all(feature = "postgres", feature = "deadpool"))]
impl PgPool {
    /// Create a new PostgreSQL connection pool.
    pub async fn new(config: DieselConfig) -> DieselResult<Self> {
        use diesel_async::pooled_connection::AsyncDieselConnectionManager;

        info!("Creating PostgreSQL connection pool");
        debug!(
            "Pool size: {}, URL: {}",
            config.pool_size,
            &config.database_url[..config
                .database_url
                .find('@')
                .unwrap_or(config.database_url.len())]
        );

        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);

        let pool = DeadpoolPool::builder(manager)
            .max_size(config.pool_size)
            .build()
            .map_err(|e| DieselError::Pool(e.to_string()))?;

        info!("PostgreSQL connection pool created successfully");

        Ok(Self {
            pool,
            config: Arc::new(config),
        })
    }

    /// Get a connection from the pool.
    pub async fn get(&self) -> DieselResult<DeadpoolObject<AsyncPgConnection>> {
        debug!("Acquiring PostgreSQL connection from pool");
        self.pool
            .get()
            .await
            .map_err(|e| DieselError::Pool(e.to_string()))
    }

    /// Get pool statistics.
    pub fn status(&self) -> PoolStatus {
        let status = self.pool.status();
        PoolStatus {
            size: status.size,
            available: status.available,
            waiting: status.waiting,
            max_size: status.max_size,
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &DieselConfig {
        &self.config
    }
}

/// PostgreSQL connection pool using bb8.
#[cfg(all(feature = "postgres", feature = "bb8"))]
pub struct PgPoolBb8 {
    pool: Bb8Pool<AsyncPgConnection>,
    config: Arc<DieselConfig>,
}

#[cfg(all(feature = "postgres", feature = "bb8"))]
impl PgPoolBb8 {
    /// Create a new PostgreSQL connection pool with bb8.
    pub async fn new(config: DieselConfig) -> DieselResult<Self> {
        use diesel_async::pooled_connection::AsyncDieselConnectionManager;

        info!("Creating PostgreSQL bb8 connection pool");

        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);

        let pool = Bb8Pool::builder()
            .max_size(config.pool_size as u32)
            .connection_timeout(config.connect_timeout)
            .build(manager)
            .await
            .map_err(|e| DieselError::Pool(e.to_string()))?;

        info!("PostgreSQL bb8 connection pool created successfully");

        Ok(Self {
            pool,
            config: Arc::new(config),
        })
    }

    /// Get a connection from the pool.
    pub async fn get(
        &self,
    ) -> DieselResult<
        bb8::PooledConnection<
            '_,
            diesel_async::pooled_connection::AsyncDieselConnectionManager<AsyncPgConnection>,
        >,
    > {
        debug!("Acquiring PostgreSQL connection from bb8 pool");
        self.pool
            .get()
            .await
            .map_err(|e| DieselError::Pool(e.to_string()))
    }

    /// Get pool statistics.
    pub fn status(&self) -> PoolStatus {
        let state = self.pool.state();
        PoolStatus {
            size: state.connections as usize,
            available: state.idle_connections as usize,
            waiting: 0, // bb8 doesn't expose this
            max_size: self.config.pool_size,
        }
    }
}

// ============================================================================
// MySQL Pool
// ============================================================================

/// MySQL connection pool using deadpool.
#[cfg(all(feature = "mysql", feature = "deadpool"))]
pub struct MysqlPool {
    pool: DeadpoolPool<AsyncMysqlConnection>,
    #[allow(dead_code)] // kept for API symmetry with PgPool; not yet read on the mysql path
    config: Arc<DieselConfig>,
}

#[cfg(all(feature = "mysql", feature = "deadpool"))]
impl MysqlPool {
    /// Create a new MySQL connection pool.
    pub async fn new(config: DieselConfig) -> DieselResult<Self> {
        use diesel_async::pooled_connection::AsyncDieselConnectionManager;

        info!("Creating MySQL connection pool");

        let manager =
            AsyncDieselConnectionManager::<AsyncMysqlConnection>::new(&config.database_url);

        let pool = DeadpoolPool::builder(manager)
            .max_size(config.pool_size)
            .build()
            .map_err(|e| DieselError::Pool(e.to_string()))?;

        info!("MySQL connection pool created successfully");

        Ok(Self {
            pool,
            config: Arc::new(config),
        })
    }

    /// Get a connection from the pool.
    pub async fn get(&self) -> DieselResult<DeadpoolObject<AsyncMysqlConnection>> {
        debug!("Acquiring MySQL connection from pool");
        self.pool
            .get()
            .await
            .map_err(|e| DieselError::Pool(e.to_string()))
    }

    /// Get pool statistics.
    pub fn status(&self) -> PoolStatus {
        let status = self.pool.status();
        PoolStatus {
            size: status.size,
            available: status.available,
            waiting: status.waiting,
            max_size: status.max_size,
        }
    }
}

// ============================================================================
// Pool Status
// ============================================================================

/// Connection pool statistics.
#[derive(Debug, Clone)]
pub struct PoolStatus {
    /// Current number of connections.
    pub size: usize,
    /// Number of available (idle) connections.
    pub available: usize,
    /// Number of tasks waiting for a connection.
    pub waiting: usize,
    /// Maximum pool size.
    pub max_size: usize,
}

impl PoolStatus {
    /// Get the utilization percentage.
    pub fn utilization(&self) -> f64 {
        if self.max_size == 0 {
            0.0
        } else {
            ((self.size - self.available) as f64 / self.max_size as f64) * 100.0
        }
    }

    /// Check if the pool is under pressure.
    pub fn is_under_pressure(&self) -> bool {
        self.waiting > 0 || self.utilization() > 80.0
    }
}

// ============================================================================
// Type Aliases
// ============================================================================

/// Default PostgreSQL pool type.
#[cfg(all(feature = "postgres", feature = "deadpool"))]
pub type DieselPool = PgPool;

/// Default MySQL pool type.
#[cfg(all(feature = "mysql", feature = "deadpool", not(feature = "postgres")))]
pub type DieselPool = MysqlPool;
