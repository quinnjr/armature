// Environment variable loading

use crate::{ConfigError, Result};
use std::collections::HashMap;
use std::env;

/// Environment variable loader
pub struct EnvLoader {
    prefix: Option<String>,
}

impl EnvLoader {
    /// Create a new environment loader
    pub fn new(prefix: Option<String>) -> Self {
        Self { prefix }
    }

    /// Load all environment variables.
    ///
    /// Keys are lowercased, the prefix (if any) is stripped, and a double
    /// underscore becomes a dot: `APP__DATABASE__URL` with prefix `APP`
    /// becomes `database.url`. Without that mapping no environment variable
    /// could ever satisfy a dotted lookup like `get("database.url")`, because
    /// a `.` is not portable in an environment variable name — `APP_DATABASE_URL`
    /// only ever produced the flat key `database_url`.
    ///
    /// Single underscores are left alone, so a name that is genuinely one
    /// segment (`MAX_CONNECTIONS` -> `max_connections`) still works.
    pub fn load(&self) -> Result<HashMap<String, String>> {
        let mut config = HashMap::new();

        for (key, value) in env::vars() {
            let key = match self.prefix {
                Some(ref prefix) => {
                    if !key.starts_with(prefix) {
                        continue;
                    }
                    key.trim_start_matches(prefix)
                        .trim_start_matches('_')
                        .to_string()
                }
                None => key,
            };
            config.insert(Self::normalize_key(&key), value);
        }

        Ok(config)
    }

    /// Lowercase a variable name and map `__` to the path separator `.`.
    fn normalize_key(key: &str) -> String {
        key.to_lowercase().replace("__", ".")
    }

    /// Load a specific environment variable
    pub fn load_var(&self, key: &str) -> Result<String> {
        let full_key = if let Some(ref prefix) = self.prefix {
            format!("{}_{}", prefix, key.to_uppercase())
        } else {
            key.to_uppercase()
        };

        env::var(&full_key).map_err(ConfigError::EnvError)
    }

    /// Load with default value
    pub fn load_var_or(&self, key: &str, default: &str) -> String {
        self.load_var(key).unwrap_or_else(|_| default.to_string())
    }
}

impl Default for EnvLoader {
    fn default() -> Self {
        Self::new(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Environment variable tests are inherently difficult to test safely
    // in Rust 1.78+ because std::env::set_var is unsafe (not thread-safe).
    // These tests use existing environment variables or test default behavior.

    /// `__` is the only way an environment variable can address a nested key,
    /// since `.` is not portable in a variable name. Tested on the key
    /// normalizer directly so no process-global env mutation is needed.
    #[test]
    fn test_double_underscore_maps_to_a_dotted_path() {
        assert_eq!(EnvLoader::normalize_key("DATABASE__URL"), "database.url");
        assert_eq!(
            EnvLoader::normalize_key("SERVER__TLS__CERT_PATH"),
            "server.tls.cert_path"
        );
        // A single underscore is part of the segment name, not a separator.
        assert_eq!(
            EnvLoader::normalize_key("MAX_CONNECTIONS"),
            "max_connections"
        );
    }

    #[test]
    fn test_env_loader_with_default() {
        let loader = EnvLoader::new(None);
        let value = loader.load_var_or("NONEXISTENT_VAR_12345", "default");

        assert_eq!(value, "default");
    }

    #[test]
    fn test_env_loader_missing_var() {
        let loader = EnvLoader::new(Some("ARMATURE_TEST".to_string()));
        let result = loader.load_var("MISSING_VAR_67890");

        assert!(result.is_err());
    }

    #[test]
    fn test_env_loader_path_exists() {
        // PATH is almost always set on any system
        let loader = EnvLoader::new(None);
        let result = loader.load_var("PATH");

        // PATH should exist on most systems
        if std::env::var("PATH").is_ok() {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_env_loader_prefix() {
        let loader = EnvLoader::new(Some("MY_APP".to_string()));
        // This tests the prefix logic without needing to set env vars
        // The prefix should be applied when looking up "FOO" -> "MY_APP_FOO"
        let result = loader.load_var("NONEXISTENT_99999");
        assert!(result.is_err());
    }
}
