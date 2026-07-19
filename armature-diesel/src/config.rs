//! Configuration for Diesel connection pools.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for a Diesel database connection pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DieselConfig {
    /// Database URL (e.g., `postgres://user:pass@localhost/db`).
    pub database_url: String,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,

    /// Minimum number of idle connections to maintain.
    #[serde(default)]
    pub min_idle: Option<usize>,

    /// Connection timeout duration.
    #[serde(default = "default_connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect_timeout: Duration,

    /// Maximum lifetime of a connection.
    #[serde(default = "default_max_lifetime")]
    #[serde(with = "humantime_serde")]
    pub max_lifetime: Duration,

    /// Idle timeout for connections.
    #[serde(default = "default_idle_timeout")]
    #[serde(with = "humantime_serde")]
    pub idle_timeout: Duration,

    /// Whether to test connections on checkout.
    #[serde(default = "default_test_on_checkout")]
    pub test_on_checkout: bool,

    /// Application name for connection identification.
    #[serde(default)]
    pub application_name: Option<String>,

    /// SSL mode for PostgreSQL connections.
    #[serde(default)]
    pub ssl_mode: Option<String>,
}

fn default_pool_size() -> usize {
    10
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_max_lifetime() -> Duration {
    Duration::from_secs(30 * 60) // 30 minutes
}

fn default_idle_timeout() -> Duration {
    Duration::from_secs(10 * 60) // 10 minutes
}

fn default_test_on_checkout() -> bool {
    true
}

impl DieselConfig {
    /// Create a new configuration with the given database URL.
    pub fn new(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            pool_size: default_pool_size(),
            min_idle: None,
            connect_timeout: default_connect_timeout(),
            max_lifetime: default_max_lifetime(),
            idle_timeout: default_idle_timeout(),
            test_on_checkout: default_test_on_checkout(),
            application_name: None,
            ssl_mode: None,
        }
    }

    /// Create configuration from environment variables.
    ///
    /// Uses the following environment variables:
    /// - `DATABASE_URL`: Required database URL
    /// - `DATABASE_POOL_SIZE`: Pool size (default: 10)
    /// - `DATABASE_CONNECT_TIMEOUT`: Connect timeout in seconds
    /// - `DATABASE_MAX_LIFETIME`: Max connection lifetime in seconds
    /// - `DATABASE_IDLE_TIMEOUT`: Idle timeout in seconds
    pub fn from_env() -> Result<Self, crate::DieselError> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| crate::DieselError::Config("DATABASE_URL not set".into()))?;

        let mut config = Self::new(database_url);

        if let Ok(size) = std::env::var("DATABASE_POOL_SIZE") {
            config.pool_size = size
                .parse()
                .map_err(|_| crate::DieselError::Config("Invalid DATABASE_POOL_SIZE".into()))?;
        }

        if let Ok(timeout) = std::env::var("DATABASE_CONNECT_TIMEOUT") {
            config.connect_timeout = Duration::from_secs(timeout.parse().map_err(|_| {
                crate::DieselError::Config("Invalid DATABASE_CONNECT_TIMEOUT".into())
            })?);
        }

        if let Ok(lifetime) = std::env::var("DATABASE_MAX_LIFETIME") {
            config.max_lifetime =
                Duration::from_secs(lifetime.parse().map_err(|_| {
                    crate::DieselError::Config("Invalid DATABASE_MAX_LIFETIME".into())
                })?);
        }

        if let Ok(idle) = std::env::var("DATABASE_IDLE_TIMEOUT") {
            config.idle_timeout =
                Duration::from_secs(idle.parse().map_err(|_| {
                    crate::DieselError::Config("Invalid DATABASE_IDLE_TIMEOUT".into())
                })?);
        }

        Ok(config)
    }

    /// Set the pool size.
    pub fn pool_size(mut self, size: usize) -> Self {
        self.pool_size = size;
        self
    }

    /// Set the minimum idle connections.
    pub fn min_idle(mut self, min: usize) -> Self {
        self.min_idle = Some(min);
        self
    }

    /// Set the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the maximum connection lifetime.
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = lifetime;
        self
    }

    /// Set the idle timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Enable or disable connection testing on checkout.
    pub fn test_on_checkout(mut self, test: bool) -> Self {
        self.test_on_checkout = test;
        self
    }

    /// Set the application name.
    pub fn application_name(mut self, name: impl Into<String>) -> Self {
        self.application_name = Some(name.into());
        self
    }

    /// Set the SSL mode.
    pub fn ssl_mode(mut self, mode: impl Into<String>) -> Self {
        self.ssl_mode = Some(mode.into());
        self
    }
}

impl Default for DieselConfig {
    fn default() -> Self {
        Self::new("postgres://localhost/armature")
    }
}

/// Serde helper: (de)serializes a `Duration` as an integer number of
/// seconds (not humantime strings), despite the module name.
mod humantime_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}
