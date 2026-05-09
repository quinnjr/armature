//! Redis configuration.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Redis configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// Redis URL (redis://host:port or rediss://host:port for TLS).
    pub url: String,
    /// Connection pool size.
    pub pool_size: u32,
    /// Minimum idle connections.
    pub min_idle: Option<u32>,
    /// Connection timeout.
    #[serde(with = "humantime_serde", default = "default_connection_timeout")]
    pub connection_timeout: Duration,
    /// Command timeout.
    #[serde(with = "humantime_serde", default = "default_command_timeout")]
    pub command_timeout: Duration,
    /// Database number (0-15).
    pub database: Option<u8>,
    /// Username for Redis 6+ ACL.
    pub username: Option<String>,
    /// Password.
    pub password: Option<String>,
    /// Cluster mode.
    pub cluster: bool,
    /// Cluster nodes (for cluster mode).
    #[serde(default)]
    pub cluster_nodes: Vec<String>,
    /// Use TLS.
    pub tls: bool,
    /// Connection name (for CLIENT SETNAME).
    pub connection_name: Option<String>,
}

fn default_connection_timeout() -> Duration {
    Duration::from_secs(5)
}

fn default_command_timeout() -> Duration {
    Duration::from_secs(30)
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
            pool_size: 10,
            min_idle: Some(1),
            connection_timeout: default_connection_timeout(),
            command_timeout: default_command_timeout(),
            database: None,
            username: None,
            password: None,
            cluster: false,
            cluster_nodes: Vec::new(),
            tls: false,
            connection_name: None,
        }
    }
}

impl RedisConfig {
    /// Create a new configuration.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Create a builder.
    pub fn builder() -> RedisConfigBuilder {
        RedisConfigBuilder::new()
    }

    /// Load configuration from environment variables.
    pub fn from_env() -> RedisConfigBuilder {
        let mut builder = RedisConfigBuilder::new();

        if let Ok(url) = std::env::var("REDIS_URL") {
            builder = builder.url(url);
        }

        if let Ok(pool_size) = std::env::var("REDIS_POOL_SIZE")
            && let Ok(size) = pool_size.parse()
        {
            builder = builder.pool_size(size);
        }

        if let Ok(db) = std::env::var("REDIS_DATABASE")
            && let Ok(db_num) = db.parse()
        {
            builder = builder.database(db_num);
        }

        if let Ok(username) = std::env::var("REDIS_USERNAME") {
            builder = builder.username(username);
        }

        if let Ok(password) = std::env::var("REDIS_PASSWORD") {
            builder = builder.password(password);
        }

        if std::env::var("REDIS_TLS").is_ok() {
            builder = builder.tls(true);
        }

        if std::env::var("REDIS_CLUSTER").is_ok() {
            builder = builder.cluster(true);
        }

        if let Ok(nodes) = std::env::var("REDIS_CLUSTER_NODES") {
            let nodes: Vec<String> = nodes.split(',').map(|s| s.trim().to_string()).collect();
            builder = builder.cluster_nodes(nodes);
        }

        builder
    }

    /// Get the full Redis URL with auth and database.
    pub fn connection_url(&self) -> String {
        let mut url = self.url.clone();

        // Add auth if provided
        if let Some(password) = &self.password {
            if let Some(username) = &self.username {
                // Redis 6+ ACL format: redis://username:password@host
                url = url.replacen(
                    "redis://",
                    &format!("redis://{}:{}@", username, password),
                    1,
                );
                url = url.replacen(
                    "rediss://",
                    &format!("rediss://{}:{}@", username, password),
                    1,
                );
            } else {
                // Legacy format: redis://:password@host
                url = url.replacen("redis://", &format!("redis://:{}@", password), 1);
                url = url.replacen("rediss://", &format!("rediss://:{}@", password), 1);
            }
        }

        // Add database if provided
        if let Some(db) = self.database
            && (!url.contains('/') || url.ends_with(':'))
        {
            url = format!("{}/{}", url.trim_end_matches('/'), db);
        }

        url
    }
}

/// Builder for Redis configuration.
#[derive(Default)]
pub struct RedisConfigBuilder {
    config: RedisConfig,
}

impl RedisConfigBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            config: RedisConfig::default(),
        }
    }

    /// Set the Redis URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.config.url = url.into();
        self
    }

    /// Set the pool size.
    pub fn pool_size(mut self, size: u32) -> Self {
        self.config.pool_size = size;
        self
    }

    /// Set the minimum idle connections.
    pub fn min_idle(mut self, min_idle: u32) -> Self {
        self.config.min_idle = Some(min_idle);
        self
    }

    /// Set the connection timeout.
    pub fn connection_timeout(mut self, timeout: Duration) -> Self {
        self.config.connection_timeout = timeout;
        self
    }

    /// Set the command timeout.
    pub fn command_timeout(mut self, timeout: Duration) -> Self {
        self.config.command_timeout = timeout;
        self
    }

    /// Set the database number.
    pub fn database(mut self, db: u8) -> Self {
        self.config.database = Some(db);
        self
    }

    /// Set the username (Redis 6+ ACL).
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.config.username = Some(username.into());
        self
    }

    /// Set the password.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.config.password = Some(password.into());
        self
    }

    /// Enable cluster mode.
    pub fn cluster(mut self, enabled: bool) -> Self {
        self.config.cluster = enabled;
        self
    }

    /// Set cluster nodes.
    pub fn cluster_nodes(mut self, nodes: Vec<String>) -> Self {
        self.config.cluster_nodes = nodes;
        self.config.cluster = true;
        self
    }

    /// Enable TLS.
    pub fn tls(mut self, enabled: bool) -> Self {
        self.config.tls = enabled;
        if enabled && self.config.url.starts_with("redis://") {
            self.config.url = self.config.url.replacen("redis://", "rediss://", 1);
        }
        self
    }

    /// Set the connection name.
    pub fn connection_name(mut self, name: impl Into<String>) -> Self {
        self.config.connection_name = Some(name.into());
        self
    }

    /// Build the configuration.
    pub fn build(self) -> RedisConfig {
        self.config
    }
}

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
