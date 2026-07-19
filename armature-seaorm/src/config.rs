//! Configuration for SeaORM database connections.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for a SeaORM database connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database URL.
    pub database_url: String,

    /// Maximum number of connections in the pool.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// Minimum number of connections in the pool.
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,

    /// Connection timeout.
    #[serde(default = "default_connect_timeout")]
    #[serde(with = "humantime_serde")]
    pub connect_timeout: Duration,

    /// Idle timeout for connections.
    #[serde(default = "default_idle_timeout")]
    #[serde(with = "humantime_serde")]
    pub idle_timeout: Duration,

    /// Maximum lifetime of a connection.
    #[serde(default = "default_max_lifetime")]
    #[serde(with = "humantime_serde")]
    pub max_lifetime: Duration,

    /// Enable SQLx logging.
    #[serde(default)]
    pub sqlx_logging: bool,

    /// SQLx log level.
    ///
    /// Parsed (case-insensitively) into a [`log::LevelFilter`] and applied via
    /// [`sea_orm::ConnectOptions::sqlx_logging_level`] in [`Self::to_connect_options`].
    /// Recognized values: `off`, `error`, `warn`, `info`, `debug`, `trace`. Unrecognized
    /// values fall back to `debug`.
    #[serde(default = "default_sqlx_log_level")]
    pub sqlx_log_level: String,

    /// Schema name (for PostgreSQL).
    #[serde(default)]
    pub schema: Option<String>,
}

fn default_max_connections() -> u32 {
    10
}

fn default_min_connections() -> u32 {
    1
}

fn default_connect_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_idle_timeout() -> Duration {
    Duration::from_secs(10 * 60) // 10 minutes
}

fn default_max_lifetime() -> Duration {
    Duration::from_secs(30 * 60) // 30 minutes
}

fn default_sqlx_log_level() -> String {
    "debug".to_string()
}

/// Parse a SQLx log level string (case-insensitive) into a [`log::LevelFilter`].
///
/// Falls back to [`log::LevelFilter::Debug`] for unrecognized values.
fn parse_sqlx_log_level(level: &str) -> log::LevelFilter {
    match level.to_ascii_lowercase().as_str() {
        "off" => log::LevelFilter::Off,
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "info" => log::LevelFilter::Info,
        "trace" => log::LevelFilter::Trace,
        _ => log::LevelFilter::Debug,
    }
}

impl DatabaseConfig {
    /// Create a new configuration with the given database URL.
    pub fn new(database_url: impl Into<String>) -> Self {
        Self {
            database_url: database_url.into(),
            max_connections: default_max_connections(),
            min_connections: default_min_connections(),
            connect_timeout: default_connect_timeout(),
            idle_timeout: default_idle_timeout(),
            max_lifetime: default_max_lifetime(),
            sqlx_logging: false,
            sqlx_log_level: default_sqlx_log_level(),
            schema: None,
        }
    }

    /// Create configuration from environment variables.
    ///
    /// Uses the following environment variables:
    /// - `DATABASE_URL`: Required database URL
    /// - `DATABASE_MAX_CONNECTIONS`: Max connections (default: 10)
    /// - `DATABASE_MIN_CONNECTIONS`: Min connections (default: 1)
    /// - `DATABASE_CONNECT_TIMEOUT`: Connect timeout in seconds
    /// - `DATABASE_IDLE_TIMEOUT`: Idle timeout in seconds
    /// - `DATABASE_SQLX_LOGGING`: Enable SQLx logging (true/false)
    pub fn from_env() -> Result<Self, crate::SeaOrmError> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| crate::SeaOrmError::Config("DATABASE_URL not set".into()))?;

        let mut config = Self::new(database_url);

        if let Ok(max) = std::env::var("DATABASE_MAX_CONNECTIONS") {
            config.max_connections = max.parse().map_err(|_| {
                crate::SeaOrmError::Config("Invalid DATABASE_MAX_CONNECTIONS".into())
            })?;
        }

        if let Ok(min) = std::env::var("DATABASE_MIN_CONNECTIONS") {
            config.min_connections = min.parse().map_err(|_| {
                crate::SeaOrmError::Config("Invalid DATABASE_MIN_CONNECTIONS".into())
            })?;
        }

        if let Ok(timeout) = std::env::var("DATABASE_CONNECT_TIMEOUT") {
            config.connect_timeout = Duration::from_secs(timeout.parse().map_err(|_| {
                crate::SeaOrmError::Config("Invalid DATABASE_CONNECT_TIMEOUT".into())
            })?);
        }

        if let Ok(timeout) = std::env::var("DATABASE_IDLE_TIMEOUT") {
            config.idle_timeout =
                Duration::from_secs(timeout.parse().map_err(|_| {
                    crate::SeaOrmError::Config("Invalid DATABASE_IDLE_TIMEOUT".into())
                })?);
        }

        if let Ok(logging) = std::env::var("DATABASE_SQLX_LOGGING") {
            config.sqlx_logging = logging == "true" || logging == "1";
        }

        Ok(config)
    }

    /// Set the maximum number of connections.
    pub fn max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the minimum number of connections.
    pub fn min_connections(mut self, min: u32) -> Self {
        self.min_connections = min;
        self
    }

    /// Set the connection timeout.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the idle timeout.
    pub fn idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set the maximum connection lifetime.
    pub fn max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = lifetime;
        self
    }

    /// Enable or disable SQLx logging.
    pub fn sqlx_logging(mut self, enabled: bool) -> Self {
        self.sqlx_logging = enabled;
        self
    }

    /// Set the schema name (PostgreSQL).
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Convert to SeaORM ConnectOptions.
    pub fn to_connect_options(&self) -> sea_orm::ConnectOptions {
        let mut options = sea_orm::ConnectOptions::new(&self.database_url);

        options
            .max_connections(self.max_connections)
            .min_connections(self.min_connections)
            .connect_timeout(self.connect_timeout)
            .idle_timeout(self.idle_timeout)
            .max_lifetime(self.max_lifetime)
            .sqlx_logging(self.sqlx_logging)
            .sqlx_logging_level(parse_sqlx_log_level(&self.sqlx_log_level));

        if let Some(ref schema) = self.schema {
            options.set_schema_search_path(schema.clone());
        }

        options
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self::new("postgres://localhost/armature")
    }
}

/// Humantime serde module for duration serialization.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// `std::env` is process-global; serialize tests that mutate it so they
    /// don't race against each other under `cargo test`'s default parallelism.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn to_connect_options_applies_sqlx_log_level() {
        let mut config = DatabaseConfig::new("postgres://localhost/test").sqlx_logging(true);
        config.sqlx_log_level = "warn".to_string();

        let options = config.to_connect_options();
        assert_eq!(options.get_sqlx_logging_level(), log::LevelFilter::Warn);
    }

    #[test]
    fn to_connect_options_defaults_unrecognized_log_level_to_debug() {
        let mut config = DatabaseConfig::new("postgres://localhost/test");
        config.sqlx_log_level = "not-a-real-level".to_string();

        let options = config.to_connect_options();
        assert_eq!(options.get_sqlx_logging_level(), log::LevelFilter::Debug);
    }

    #[test]
    fn to_connect_options_applies_all_recognized_levels() {
        let cases = [
            ("off", log::LevelFilter::Off),
            ("error", log::LevelFilter::Error),
            ("warn", log::LevelFilter::Warn),
            ("info", log::LevelFilter::Info),
            ("debug", log::LevelFilter::Debug),
            ("trace", log::LevelFilter::Trace),
            ("TRACE", log::LevelFilter::Trace),
        ];

        for (level, expected) in cases {
            let mut config = DatabaseConfig::new("postgres://localhost/test");
            config.sqlx_log_level = level.to_string();
            let options = config.to_connect_options();
            assert_eq!(
                options.get_sqlx_logging_level(),
                expected,
                "level {level} should map to {expected:?}"
            );
        }
    }

    #[test]
    fn from_env_reads_database_idle_timeout() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // SAFETY: guarded by ENV_LOCK to serialize against other tests in this
        // module that also mutate process env vars.
        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/idle_timeout_test");
            std::env::set_var("DATABASE_IDLE_TIMEOUT", "45");
        }

        let config = DatabaseConfig::from_env().expect("from_env should succeed");

        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("DATABASE_IDLE_TIMEOUT");
        }

        assert_eq!(config.idle_timeout, Duration::from_secs(45));
    }

    #[test]
    fn from_env_rejects_invalid_database_idle_timeout() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        unsafe {
            std::env::set_var("DATABASE_URL", "postgres://localhost/idle_timeout_bad");
            std::env::set_var("DATABASE_IDLE_TIMEOUT", "not-a-number");
        }

        let result = DatabaseConfig::from_env();

        unsafe {
            std::env::remove_var("DATABASE_URL");
            std::env::remove_var("DATABASE_IDLE_TIMEOUT");
        }

        assert!(result.is_err());
    }
}
