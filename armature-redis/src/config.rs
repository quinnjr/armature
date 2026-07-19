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

        // Upgrade to TLS scheme if configured, regardless of how the config
        // was constructed (builder's `tls()` already rewrites the scheme at
        // set-time, but a directly-constructed or deserialized config with
        // `tls: true` and a plain `redis://` URL must still be upgraded here).
        if self.tls && url.starts_with("redis://") {
            url = url.replacen("redis://", "rediss://", 1);
        }

        // Add auth if provided. Username/password are percent-encoded before
        // splicing into the URL: an un-encoded password containing `@`, `/`,
        // `?`, `#`, or whitespace would otherwise corrupt the URL (e.g. be
        // parsed as a path/query separator or a second userinfo boundary).
        if let Some(password) = &self.password {
            let encoded_password = percent_encode(password);
            if let Some(username) = &self.username {
                // Redis 6+ ACL format: redis://username:password@host
                let encoded_username = percent_encode(username);
                url = url.replacen(
                    "redis://",
                    &format!("redis://{}:{}@", encoded_username, encoded_password),
                    1,
                );
                url = url.replacen(
                    "rediss://",
                    &format!("rediss://{}:{}@", encoded_username, encoded_password),
                    1,
                );
            } else {
                // Legacy format: redis://:password@host
                url = url.replacen("redis://", &format!("redis://:{}@", encoded_password), 1);
                url = url.replacen("rediss://", &format!("rediss://:{}@", encoded_password), 1);
            }
        }

        // Add database if provided, but only when the URL doesn't already
        // carry a db path segment after the `scheme://[auth@]host[:port]`
        // prefix. A bare `redis://host:port` always contains `/` (from the
        // scheme separator), so checking `url.contains('/')` against the
        // *whole* URL never appends the database; we must only look past the
        // scheme separator for an actual path segment. The scan must also
        // skip past any userinfo (`user:pass@`) segment — a `/` inside
        // percent-encoded (or, before encoding was added, raw) credentials
        // would otherwise be mistaken for a path separator and wrongly
        // suppress the database append.
        if let Some(db) = self.database {
            let scheme_end = url.find("://").map(|i| i + 3).unwrap_or(0);
            let after_auth = url[scheme_end..]
                .find('@')
                .map(|i| scheme_end + i + 1)
                .unwrap_or(scheme_end);
            let has_db_segment = url[after_auth..].contains('/');
            if !has_db_segment {
                url = format!("{}/{}", url.trim_end_matches('/'), db);
            }
        }

        url
    }
}

/// Percent-encode a string for safe use in the userinfo (username/password)
/// segment of a Redis URL, escaping everything outside
/// `[A-Za-z0-9\-_.~]`. Mirrors the equivalent helper in
/// `armature-diesel/src/pool.rs`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_url_appends_database_when_no_db_segment_present() {
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            database: Some(3),
            ..Default::default()
        };
        assert_eq!(config.connection_url(), "redis://h:6379/3");
    }

    #[test]
    fn connection_url_keeps_existing_db_segment() {
        let config = RedisConfig {
            url: "redis://h:6379/1".to_string(),
            database: Some(3),
            ..Default::default()
        };
        assert_eq!(config.connection_url(), "redis://h:6379/1");
    }

    #[test]
    fn connection_url_without_database_is_unchanged() {
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            database: None,
            ..Default::default()
        };
        assert_eq!(config.connection_url(), "redis://h:6379");
    }

    #[test]
    fn connection_url_with_auth_and_database() {
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            database: Some(2),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            ..Default::default()
        };
        assert_eq!(config.connection_url(), "redis://user:pass@h:6379/2");
    }

    #[test]
    fn connection_url_upgrades_to_tls_scheme_when_tls_true_directly_constructed() {
        // Constructed directly (not via the builder's `tls()` setter, which
        // rewrites the scheme eagerly) — `connection_url` must still upgrade
        // the scheme when `tls` is true.
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            tls: true,
            ..Default::default()
        };
        assert!(config.connection_url().starts_with("rediss://"));
    }

    #[test]
    fn connection_url_tls_upgrade_still_appends_database() {
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            tls: true,
            database: Some(5),
            ..Default::default()
        };
        assert_eq!(config.connection_url(), "rediss://h:6379/5");
    }

    #[test]
    fn connection_url_percent_encodes_password_with_special_chars() {
        // A raw `@` or `/` in the password must not be spliced verbatim —
        // that would corrupt the URL (e.g. introduce a bogus userinfo
        // boundary or path segment). It must come through percent-encoded.
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            password: Some("p@ss/word".to_string()),
            ..Default::default()
        };
        let url = config.connection_url();
        assert_eq!(url, "redis://:p%40ss%2Fword@h:6379");
        assert!(!url.contains("p@ss/word"));
    }

    #[test]
    fn connection_url_percent_encodes_username_and_password() {
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            username: Some("us/er".to_string()),
            password: Some("pa:ss@word".to_string()),
            ..Default::default()
        };
        assert_eq!(
            config.connection_url(),
            "redis://us%2Fer:pa%3Ass%40word@h:6379"
        );
    }

    #[test]
    fn connection_url_password_with_slash_still_appends_database() {
        // Before the fix, `has_db_segment` scanned the whole
        // `scheme://user:pass@host` string, so a `/` inside the raw
        // (pre-encoding) password would be mistaken for a path separator and
        // wrongly suppress the database append. Even now that passwords are
        // percent-encoded (so a literal `/` can no longer reach the URL via
        // password), this guards the userinfo-skip logic itself: the scan
        // must start after the `@` boundary, not from the scheme end.
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            password: Some("p/ss".to_string()),
            database: Some(4),
            ..Default::default()
        };
        assert_eq!(config.connection_url(), "redis://:p%2Fss@h:6379/4");
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
