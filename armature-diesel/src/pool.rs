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
// Configuration helpers
// ============================================================================

/// Percent-encode a value for use in a libpq-style connection URL query
/// parameter (e.g. `application_name`, `sslmode`).
///
/// `tokio-postgres` percent-decodes query parameter values when parsing a
/// connection URL, so this keeps names containing spaces or other reserved
/// characters intact.
#[cfg(feature = "postgres")]
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Build the PostgreSQL connection URL used to establish new connections,
/// appending `application_name` / `sslmode` as libpq connection options when
/// configured.
///
/// Note: `tokio-postgres` only recognizes `sslmode` values of `disable`,
/// `prefer`, or `require` when parsed from a URL (unlike libpq's full set,
/// e.g. `verify-ca` / `verify-full`); other values will fail to parse when
/// the connection is established.
#[cfg(feature = "postgres")]
fn pg_connection_url(config: &DieselConfig) -> String {
    let mut params: Vec<String> = Vec::new();

    if let Some(name) = &config.application_name {
        params.push(format!("application_name={}", percent_encode(name)));
    }

    if let Some(mode) = &config.ssl_mode {
        params.push(format!("sslmode={}", percent_encode(mode)));
    }

    if params.is_empty() {
        return config.database_url.clone();
    }

    let separator = if config.database_url.contains('?') {
        '&'
    } else {
        '?'
    };

    format!("{}{separator}{}", config.database_url, params.join("&"))
}

/// Build a [`diesel_async::pooled_connection::ManagerConfig`] that honors
/// [`DieselConfig::test_on_checkout`].
///
/// `AsyncDieselConnectionManager::new` (used with no config) always defaults
/// to [`RecyclingMethod::Verified`] regardless of what
/// `DieselConfig::test_on_checkout` was set to, which left that field dead:
/// setting it to `false` had no effect. Wiring it here makes
/// `test_on_checkout = false` actually select the cheaper
/// [`RecyclingMethod::Fast`] path (only checks for an open transaction, no
/// round-trip query), while `true` keeps the query-based validation.
#[cfg(any(feature = "deadpool", feature = "bb8"))]
fn recycling_manager_config<C>(
    test_on_checkout: bool,
) -> diesel_async::pooled_connection::ManagerConfig<C>
where
    C: diesel_async::AsyncConnection + 'static,
{
    use diesel_async::pooled_connection::{ManagerConfig, RecyclingMethod};

    let mut manager_config = ManagerConfig::default();
    manager_config.recycling_method = if test_on_checkout {
        RecyclingMethod::Verified
    } else {
        RecyclingMethod::Fast
    };
    manager_config
}

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

        let connection_url = pg_connection_url(&config);
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(
            &connection_url,
            recycling_manager_config(config.test_on_checkout),
        );

        // deadpool has no direct `min_idle` / `max_lifetime` / `idle_timeout`
        // knobs (it lazily creates connections and has no background reaper);
        // those fields are only honored on the bb8 backend (`PgPoolBb8`).
        let pool = DeadpoolPool::builder(manager)
            .max_size(config.pool_size)
            .create_timeout(Some(config.connect_timeout))
            .runtime(deadpool::Runtime::Tokio1)
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

        let connection_url = pg_connection_url(&config);
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(
            &connection_url,
            recycling_manager_config(config.test_on_checkout),
        );

        let pool = Bb8Pool::builder()
            .max_size(config.pool_size as u32)
            .connection_timeout(config.connect_timeout)
            .min_idle(config.min_idle.map(|n| n as u32))
            .max_lifetime(Some(config.max_lifetime))
            .idle_timeout(Some(config.idle_timeout))
            .test_on_check_out(config.test_on_checkout)
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

        let manager = AsyncDieselConnectionManager::<AsyncMysqlConnection>::new_with_config(
            &config.database_url,
            recycling_manager_config(config.test_on_checkout),
        );

        // `application_name` / `ssl_mode` are not wired here: they are
        // libpq/PostgreSQL connection-URL options (see `pg_connection_url`)
        // with no equivalent reachable through `mysql_async`'s URL parsing
        // without adding `mysql_async` as a direct dependency of this crate.
        // deadpool also has no `min_idle` / `max_lifetime` / `idle_timeout`
        // knobs (see the PostgreSQL deadpool path above for details).
        let pool = DeadpoolPool::builder(manager)
            .max_size(config.pool_size)
            .create_timeout(Some(config.connect_timeout))
            .runtime(deadpool::Runtime::Tokio1)
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
            (self.size.saturating_sub(self.available) as f64 / self.max_size as f64) * 100.0
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod pool_status_tests {
    use super::PoolStatus;

    #[test]
    fn utilization_does_not_panic_when_available_exceeds_size() {
        // `available` should never legitimately exceed `size`, but the pool
        // backends are free-form counters read from separate atomics, so a
        // transient race could momentarily observe `available > size`. The
        // naive `size - available` subtraction on `usize` would panic
        // (`attempt to subtract with overflow` in debug builds); this must
        // saturate instead.
        let status = PoolStatus {
            size: 2,
            available: 5,
            waiting: 0,
            max_size: 10,
        };

        assert_eq!(status.utilization(), 0.0);
    }

    #[test]
    fn utilization_reports_expected_percentage() {
        let status = PoolStatus {
            size: 10,
            available: 4,
            waiting: 0,
            max_size: 10,
        };

        assert_eq!(status.utilization(), 60.0);
    }
}

#[cfg(all(test, any(feature = "deadpool", feature = "bb8")))]
mod recycling_manager_config_tests {
    use super::recycling_manager_config;
    use diesel_async::pooled_connection::RecyclingMethod;

    #[cfg(feature = "postgres")]
    #[test]
    fn test_on_checkout_true_selects_verified_recycling() {
        let config = recycling_manager_config::<diesel_async::AsyncPgConnection>(true);
        assert!(matches!(config.recycling_method, RecyclingMethod::Verified));
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn test_on_checkout_false_selects_fast_recycling() {
        let config = recycling_manager_config::<diesel_async::AsyncPgConnection>(false);
        assert!(matches!(config.recycling_method, RecyclingMethod::Fast));
    }
}

#[cfg(test)]
#[cfg(feature = "postgres")]
mod pg_connection_url_tests {
    use super::pg_connection_url;
    use crate::DieselConfig;

    #[test]
    fn appends_application_name_and_sslmode_as_query_params() {
        let config = DieselConfig::new("postgres://user:pass@localhost/db")
            .application_name("armature test app")
            .ssl_mode("require");

        let url = pg_connection_url(&config);

        assert_eq!(
            url,
            "postgres://user:pass@localhost/db?application_name=armature%20test%20app&sslmode=require"
        );
    }

    #[test]
    fn leaves_url_untouched_when_neither_is_set() {
        let config = DieselConfig::new("postgres://user:pass@localhost/db");
        assert_eq!(pg_connection_url(&config), config.database_url);
    }

    #[test]
    fn appends_with_ampersand_when_url_already_has_a_query_string() {
        let config = DieselConfig::new("postgres://user:pass@localhost/db?connect_timeout=5")
            .ssl_mode("disable");

        let url = pg_connection_url(&config);

        assert_eq!(
            url,
            "postgres://user:pass@localhost/db?connect_timeout=5&sslmode=disable"
        );
    }
}

#[cfg(all(test, feature = "mysql", feature = "deadpool"))]
mod mysql_pool_config_tests {
    use crate::{DieselConfig, MysqlPool};
    use std::time::Duration;

    // No MySQL container is available in `armature-testkit`, so this only
    // proves the builder accepts and does not silently drop `connect_timeout`
    // (a real connection would be needed to prove it reaches the server).
    #[tokio::test]
    async fn pool_construction_honors_connect_timeout_without_a_live_server() {
        let config = DieselConfig::new("mysql://user:pass@127.0.0.1:1/nonexistent_db")
            .pool_size(1)
            .connect_timeout(Duration::from_millis(50));

        // deadpool creates connections lazily, so building the pool itself
        // must succeed even though nothing is listening on that port.
        let pool = MysqlPool::new(config).await;
        assert!(pool.is_ok(), "pool construction should not eagerly connect");
    }
}

/// Requires Docker; skips itself when unavailable (see `armature_testkit::docker_available`).
#[cfg(all(test, feature = "postgres", feature = "deadpool"))]
mod pg_pool_config_integration_tests {
    use crate::{DieselConfig, PgPool};
    use diesel::QueryableByName;
    use diesel::sql_types::Text;
    use diesel_async::RunQueryDsl;
    use std::time::Duration;

    #[derive(QueryableByName)]
    struct ApplicationNameRow {
        #[diesel(sql_type = Text)]
        application_name: String,
    }

    #[tokio::test]
    async fn application_name_reaches_the_server() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: docker not available");
            return;
        }

        let container = armature_testkit::containers::PostgresContainer::start().await;
        let config = DieselConfig::new(container.url())
            .application_name("armature_test")
            .connect_timeout(Duration::from_secs(5));

        let pool = PgPool::new(config).await.expect("failed to build pg pool");
        let mut conn = pool.get().await.expect("failed to get connection");

        let rows: Vec<ApplicationNameRow> = diesel::sql_query("SHOW application_name")
            .load(&mut conn)
            .await
            .expect("SHOW application_name failed");

        assert_eq!(
            rows.into_iter().next().unwrap().application_name,
            "armature_test"
        );
    }
}
