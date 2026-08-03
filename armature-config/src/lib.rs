//! Configuration management for Armature framework
//!
//! Provides flexible configuration loading from multiple sources including
//! environment variables, JSON, TOML, and programmatic values.
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```
//! use armature_config::ConfigManager;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = ConfigManager::new();
//!
//! // Set configuration values
//! manager.set("app.name", "MyApp")?;
//! manager.set("app.port", 3000i64)?;
//!
//! // Get configuration values
//! let name: String = manager.get("app.name")?;
//! let port: i64 = manager.get("app.port")?;
//!
//! assert_eq!(name, "MyApp");
//! assert_eq!(port, 3000);
//! # Ok(())
//! # }
//! ```
//!
//! ## Loading from Environment Variables
//!
//! ```
//! use armature_config::EnvLoader;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let loader = EnvLoader::new(None);
//!
//! // Load all environment variables
//! let vars = loader.load()?;
//!
//! // Or load a specific variable
//! match loader.load_var("HOME") {
//!     Ok(value) => println!("Home directory: {}", value),
//!     Err(_) => println!("HOME not set"),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Type Conversions
//!
//! ```
//! use armature_config::ConfigManager;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = ConfigManager::new();
//!
//! manager.set("debug", true)?;
//! manager.set("timeout", 30i64)?;
//! manager.set("rate", 0.5)?;
//!
//! // Get with automatic type conversion
//! let debug = manager.get_bool("debug")?;
//! let timeout = manager.get_int("timeout")?;
//! let rate = manager.get_float("rate")?;
//!
//! assert!(debug);
//! assert_eq!(timeout, 30);
//! assert_eq!(rate, 0.5);
//! # Ok(())
//! # }
//! ```
//!
//! ## Nested Configuration
//!
//! ```
//! use armature_config::ConfigManager;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = ConfigManager::new();
//!
//! // Set nested configuration using dot notation
//! manager.set("database.host", "localhost")?;
//! manager.set("database.port", 5432i64)?;
//! manager.set("database.username", "admin")?;
//! manager.set("cache.enabled", true)?;
//! manager.set("cache.ttl", 3600i64)?;
//!
//! // Retrieve nested values
//! let db_host: String = manager.get("database.host")?;
//! let db_port: i64 = manager.get("database.port")?;
//! let cache_enabled: bool = manager.get("cache.enabled")?;
//!
//! assert_eq!(db_host, "localhost");
//! assert_eq!(db_port, 5432);
//! assert!(cache_enabled);
//! # Ok(())
//! # }
//! ```
//!
//! ## Default Values
//!
//! ```
//! use armature_config::ConfigManager;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = ConfigManager::new();
//!
//! // Set a value
//! manager.set("max_connections", 100i64)?;
//!
//! // Get existing value
//! let max_conn = manager.get_or("max_connections", 50i64);
//! assert_eq!(max_conn, 100);
//!
//! // Get non-existent value (returns default)
//! let min_conn = manager.get_or("min_connections", 10i64);
//! assert_eq!(min_conn, 10);
//!
//! // Check if key exists
//! assert!(manager.has("max_connections"));
//! assert!(!manager.has("min_connections"));
//! # Ok(())
//! # }
//! ```

pub mod config_service;
pub mod env;
pub mod error;
pub mod loader;
pub mod validation;

pub use config_service::ConfigService;
pub use env::EnvLoader;
pub use error::{ConfigError, Result};
pub use loader::{ConfigLoader, FileFormat};
pub use validation::{ConfigValidator, Validate};

use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Main configuration manager
#[derive(Clone)]
pub struct ConfigManager {
    config: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    env_prefix: Option<String>,
}

impl ConfigManager {
    /// Create a new configuration manager
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(HashMap::new())),
            env_prefix: None,
        }
    }

    /// Create with environment variable prefix
    pub fn with_prefix(prefix: String) -> Self {
        Self {
            config: Arc::new(RwLock::new(HashMap::new())),
            env_prefix: Some(prefix),
        }
    }

    /// Load configuration from environment variables
    pub fn load_env(&self) -> Result<()> {
        let loader = EnvLoader::new(self.env_prefix.clone());
        let env_vars = loader.load()?;

        let mut config = self.config.write().unwrap();
        for (key, value) in env_vars {
            config.insert(key, serde_json::Value::String(value));
        }

        Ok(())
    }

    /// Load configuration from .env file
    pub fn load_dotenv(&self, path: Option<&str>) -> Result<()> {
        if let Some(path) = path {
            dotenvy::from_path(path).map_err(|e| ConfigError::LoadError(e.to_string()))?;
        } else {
            dotenvy::dotenv().ok(); // Ignore if .env doesn't exist
        }
        self.load_env()
    }

    /// Load configuration from file
    pub fn load_file(&self, path: &str, format: FileFormat) -> Result<()> {
        let loader = ConfigLoader::new(format);
        let data = loader.load_file(path)?;

        let mut config = self.config.write().unwrap();
        if let serde_json::Value::Object(map) = data {
            for (key, value) in map {
                config.insert(key, value);
            }
        }

        Ok(())
    }

    /// Set a configuration value
    pub fn set<T: serde::Serialize>(&self, key: &str, value: T) -> Result<()> {
        let json_value = serde_json::to_value(value)
            .map_err(|e| ConfigError::SerializationError(e.to_string()))?;

        let mut config = self.config.write().unwrap();
        config.insert(key.to_string(), json_value);

        Ok(())
    }

    /// Resolve a key to a value.
    ///
    /// An exact (flat) key match takes precedence. Failing that, a dotted key
    /// such as `"database.host"` is resolved by walking into nested objects
    /// that were produced when loading structured config files (JSON/TOML).
    fn resolve(&self, key: &str) -> Option<serde_json::Value> {
        let config = self.config.read().unwrap();
        Self::lookup(&config, key).cloned()
    }

    /// Borrow the value for `key` out of an already-locked map.
    ///
    /// Traversal is by reference and clones nothing: the previous version
    /// cloned the *entire* subtree at every level of a dotted path, so
    /// `get("database.host")` deep-copied the whole `database` object (and
    /// then every intermediate object beneath it) to read one string.
    /// Callers that only need a bool ([`ConfigManager::has`]) now clone
    /// nothing at all.
    fn lookup<'a>(
        config: &'a HashMap<String, serde_json::Value>,
        key: &str,
    ) -> Option<&'a serde_json::Value> {
        // Exact flat-key match wins (this is how `set` and env vars store keys).
        if let Some(value) = config.get(key) {
            return Some(value);
        }

        // Fall back to dot-path traversal into nested objects.
        if !key.contains('.') {
            return None;
        }

        let mut parts = key.split('.');
        let mut current = config.get(parts.next()?)?;
        for part in parts {
            current = match current {
                serde_json::Value::Object(map) => map.get(part)?,
                _ => return None,
            };
        }
        Some(current)
    }

    /// Get a configuration value
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        let value = self
            .resolve(key)
            .ok_or_else(|| ConfigError::KeyNotFound(key.to_string()))?;

        serde_json::from_value(value).map_err(|e| ConfigError::DeserializationError(e.to_string()))
    }

    /// Get a configuration value with default
    pub fn get_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.get(key).unwrap_or(default)
    }

    /// Get a string value
    pub fn get_string(&self, key: &str) -> Result<String> {
        let value = self
            .resolve(key)
            .ok_or_else(|| ConfigError::KeyNotFound(key.to_string()))?;
        match value {
            serde_json::Value::String(s) => Ok(s),
            other => serde_json::from_value(other)
                .map_err(|e| ConfigError::DeserializationError(e.to_string())),
        }
    }

    /// Get an integer value.
    ///
    /// Values stored as strings (e.g. loaded from environment variables or a
    /// `.env` file) are coerced by parsing.
    pub fn get_int(&self, key: &str) -> Result<i64> {
        let value = self
            .resolve(key)
            .ok_or_else(|| ConfigError::KeyNotFound(key.to_string()))?;
        match value {
            serde_json::Value::String(s) => s.trim().parse::<i64>().map_err(|e| {
                ConfigError::DeserializationError(format!("cannot parse '{}' as integer: {}", s, e))
            }),
            other => serde_json::from_value(other)
                .map_err(|e| ConfigError::DeserializationError(e.to_string())),
        }
    }

    /// Get a boolean value.
    ///
    /// Values stored as strings (e.g. loaded from environment variables or a
    /// `.env` file) are coerced by parsing.
    pub fn get_bool(&self, key: &str) -> Result<bool> {
        let value = self
            .resolve(key)
            .ok_or_else(|| ConfigError::KeyNotFound(key.to_string()))?;
        match value {
            serde_json::Value::String(s) => s.trim().parse::<bool>().map_err(|e| {
                ConfigError::DeserializationError(format!("cannot parse '{}' as bool: {}", s, e))
            }),
            other => serde_json::from_value(other)
                .map_err(|e| ConfigError::DeserializationError(e.to_string())),
        }
    }

    /// Get a float value.
    ///
    /// Values stored as strings (e.g. loaded from environment variables or a
    /// `.env` file) are coerced by parsing.
    pub fn get_float(&self, key: &str) -> Result<f64> {
        let value = self
            .resolve(key)
            .ok_or_else(|| ConfigError::KeyNotFound(key.to_string()))?;
        match value {
            serde_json::Value::String(s) => s.trim().parse::<f64>().map_err(|e| {
                ConfigError::DeserializationError(format!("cannot parse '{}' as float: {}", s, e))
            }),
            other => serde_json::from_value(other)
                .map_err(|e| ConfigError::DeserializationError(e.to_string())),
        }
    }

    /// Check if a key exists.
    ///
    /// Matches both flat keys and dotted paths into nested config objects.
    ///
    /// Answers from a borrow; nothing is cloned.
    pub fn has(&self, key: &str) -> bool {
        let config = self.config.read().unwrap();
        Self::lookup(&config, key).is_some()
    }

    /// Remove a key from the configuration
    pub fn remove(&self, key: &str) {
        let mut config = self.config.write().unwrap();
        config.remove(key);
    }

    /// Clear all configuration
    pub fn clear(&self) {
        let mut config = self.config.write().unwrap();
        config.clear();
    }

    /// Get all configuration keys
    pub fn keys(&self) -> Vec<String> {
        let config = self.config.read().unwrap();
        config.keys().cloned().collect()
    }

    /// Merge configuration from another manager
    pub fn merge(&self, other: &ConfigManager) -> Result<()> {
        let other_config = other.config.read().unwrap();
        let mut config = self.config.write().unwrap();

        for (key, value) in other_config.iter() {
            config.insert(key.clone(), value.clone());
        }

        Ok(())
    }

    /// Load and validate configuration
    pub fn load_validated<T: DeserializeOwned + Validate>(&self) -> Result<T> {
        let config = self.config.read().unwrap();
        let json_value =
            serde_json::Value::Object(config.iter().map(|(k, v)| (k.clone(), v.clone())).collect());

        let validated: T = serde_json::from_value(json_value)
            .map_err(|e| ConfigError::DeserializationError(e.to_string()))?;

        validated.validate()?;

        Ok(validated)
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Prelude for common imports.
///
/// ```
/// use armature_config::prelude::*;
/// ```
pub mod prelude {
    pub use crate::ConfigManager;
    pub use crate::config_service::ConfigService;
    pub use crate::env::EnvLoader;
    pub use crate::error::{ConfigError, Result};
    pub use crate::loader::{ConfigLoader, FileFormat};
    pub use crate::validation::{ConfigValidator, Validate};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get() {
        let manager = ConfigManager::new();
        manager.set("test_key", "test_value").unwrap();

        let value: String = manager.get("test_key").unwrap();
        assert_eq!(value, "test_value");
    }

    #[test]
    fn test_get_or_default() {
        let manager = ConfigManager::new();

        let value: String = manager.get_or("missing_key", "default_value".to_string());
        assert_eq!(value, "default_value");
    }

    #[test]
    fn test_has_key() {
        let manager = ConfigManager::new();
        manager.set("existing_key", "value").unwrap();

        assert!(manager.has("existing_key"));
        assert!(!manager.has("missing_key"));
    }

    #[test]
    fn test_type_conversions() {
        let manager = ConfigManager::new();

        manager.set("string_key", "hello").unwrap();
        manager.set("int_key", 42i64).unwrap();
        manager.set("bool_key", true).unwrap();
        manager.set("float_key", 3.15).unwrap();

        assert_eq!(manager.get_string("string_key").unwrap(), "hello");
        assert_eq!(manager.get_int("int_key").unwrap(), 42);
        assert!(manager.get_bool("bool_key").unwrap());
        assert_eq!(manager.get_float("float_key").unwrap(), 3.15);
    }

    #[test]
    fn test_get_missing_key() {
        let manager = ConfigManager::new();
        let result: Result<String> = manager.get("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_set_overwrites() {
        let manager = ConfigManager::new();
        manager.set("key", "value1").unwrap();
        manager.set("key", "value2").unwrap();

        let value: String = manager.get("key").unwrap();
        assert_eq!(value, "value2");
    }

    #[test]
    fn test_multiple_keys() {
        let manager = ConfigManager::new();
        manager.set("key1", "value1").unwrap();
        manager.set("key2", "value2").unwrap();
        manager.set("key3", "value3").unwrap();

        assert!(manager.has("key1"));
        assert!(manager.has("key2"));
        assert!(manager.has("key3"));
    }

    #[test]
    fn test_nested_keys() {
        let manager = ConfigManager::new();
        manager.set("database.host", "localhost").unwrap();
        manager.set("database.port", 5432i64).unwrap();

        assert_eq!(manager.get_string("database.host").unwrap(), "localhost");
        assert_eq!(manager.get_int("database.port").unwrap(), 5432);
    }

    #[test]
    fn test_empty_string() {
        let manager = ConfigManager::new();
        manager.set("empty", "").unwrap();

        let value: String = manager.get("empty").unwrap();
        assert_eq!(value, "");
    }

    #[test]
    fn test_zero_values() {
        let manager = ConfigManager::new();
        manager.set("zero_int", 0i64).unwrap();
        manager.set("zero_float", 0.0).unwrap();

        assert_eq!(manager.get_int("zero_int").unwrap(), 0);
        assert_eq!(manager.get_float("zero_float").unwrap(), 0.0);
    }

    #[test]
    fn test_negative_numbers() {
        let manager = ConfigManager::new();
        manager.set("negative_int", -42i64).unwrap();
        manager.set("negative_float", -3.15).unwrap();

        assert_eq!(manager.get_int("negative_int").unwrap(), -42);
        assert_eq!(manager.get_float("negative_float").unwrap(), -3.15);
    }

    #[test]
    fn test_bool_values() {
        let manager = ConfigManager::new();
        manager.set("true_val", true).unwrap();
        manager.set("false_val", false).unwrap();

        assert!(manager.get_bool("true_val").unwrap());
        assert!(!manager.get_bool("false_val").unwrap());
    }

    #[test]
    fn test_large_numbers() {
        let manager = ConfigManager::new();
        manager.set("large_int", i64::MAX).unwrap();
        manager.set("large_float", f64::MAX).unwrap();

        assert_eq!(manager.get_int("large_int").unwrap(), i64::MAX);
        assert_eq!(manager.get_float("large_float").unwrap(), f64::MAX);
    }

    #[test]
    fn test_special_characters_in_values() {
        let manager = ConfigManager::new();
        manager.set("special", "value!@#$%^&*()").unwrap();

        let value: String = manager.get("special").unwrap();
        assert_eq!(value, "value!@#$%^&*()");
    }

    #[test]
    fn test_unicode_values() {
        let manager = ConfigManager::new();
        manager.set("unicode", "Hello 世界 🌍").unwrap();

        let value: String = manager.get("unicode").unwrap();
        assert_eq!(value, "Hello 世界 🌍");
    }

    #[test]
    fn test_whitespace_values() {
        let manager = ConfigManager::new();
        manager.set("spaces", "  value with spaces  ").unwrap();

        let value: String = manager.get("spaces").unwrap();
        assert_eq!(value, "  value with spaces  ");
    }

    #[test]
    fn test_newline_in_values() {
        let manager = ConfigManager::new();
        manager.set("multiline", "line1\nline2\nline3").unwrap();

        let value: String = manager.get("multiline").unwrap();
        assert!(value.contains("\n"));
    }

    #[test]
    fn test_get_or_with_existing_key() {
        let manager = ConfigManager::new();
        manager.set("key", "actual").unwrap();

        let value: String = manager.get_or("key", "default".to_string());
        assert_eq!(value, "actual");
    }

    #[test]
    fn test_clone_config_manager() {
        let manager1 = ConfigManager::new();
        manager1.set("key", "value").unwrap();

        let manager2 = manager1.clone();
        let value: String = manager2.get("key").unwrap();
        assert_eq!(value, "value");
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let manager = Arc::new(ConfigManager::new());
        manager.set("counter", 0i64).unwrap();

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let manager = Arc::clone(&manager);
                thread::spawn(move || {
                    manager.set(&format!("key{}", i), i).unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        for i in 0..10 {
            assert!(manager.has(&format!("key{}", i)));
        }
    }

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "armature-config-{}-{}-{}",
            std::process::id(),
            n,
            name
        ))
    }

    #[test]
    fn test_load_file_json() {
        let path = unique_temp_path("load.json");
        std::fs::write(&path, r#"{"service_name": "api", "workers": 4}"#).unwrap();

        let manager = ConfigManager::new();
        manager
            .load_file(path.to_str().unwrap(), FileFormat::Json)
            .unwrap();

        assert_eq!(manager.get_string("service_name").unwrap(), "api");
        assert_eq!(manager.get_int("workers").unwrap(), 4);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_dot_path_into_loaded_nested_file() {
        // Regression: a nested config file exposes leaves via dotted keys.
        let path = unique_temp_path("nested.json");
        std::fs::write(
            &path,
            r#"{"database": {"host": "db.example.com", "port": 5432}}"#,
        )
        .unwrap();

        let manager = ConfigManager::new();
        manager
            .load_file(path.to_str().unwrap(), FileFormat::Json)
            .unwrap();

        assert_eq!(
            manager.get_string("database.host").unwrap(),
            "db.example.com"
        );
        assert_eq!(manager.get_int("database.port").unwrap(), 5432);
        assert!(manager.has("database.host"));
        assert!(!manager.has("database.missing"));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_typed_getters_coerce_string_values() {
        // Regression: env/.env values arrive as JSON strings; typed getters
        // must parse-coerce them.
        let manager = ConfigManager::new();
        manager.set("port", "3000").unwrap();
        manager.set("debug", "true").unwrap();
        manager.set("rate", "0.25").unwrap();

        assert_eq!(manager.get_int("port").unwrap(), 3000);
        assert!(manager.get_bool("debug").unwrap());
        assert_eq!(manager.get_float("rate").unwrap(), 0.25);
    }

    #[test]
    fn test_case_sensitive_keys() {
        let manager = ConfigManager::new();
        manager.set("Key", "value1").unwrap();
        manager.set("key", "value2").unwrap();

        assert!(manager.has("Key"));
        assert!(manager.has("key"));
        assert_ne!(
            manager.get_string("Key").unwrap(),
            manager.get_string("key").unwrap()
        );
    }
}
