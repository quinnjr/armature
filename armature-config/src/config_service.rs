// ConfigService - High-level configuration service

use crate::{ConfigManager, Result};
use serde::de::DeserializeOwned;

/// High-level configuration service
#[derive(Clone)]
pub struct ConfigService {
    manager: ConfigManager,
}

impl ConfigService {
    /// Create a new configuration service
    pub fn new() -> Self {
        Self {
            manager: ConfigManager::new(),
        }
    }

    /// Create from an existing manager
    pub fn from_manager(manager: ConfigManager) -> Self {
        Self { manager }
    }

    /// Builder for creating configured service
    pub fn builder() -> ConfigServiceBuilder {
        ConfigServiceBuilder::new()
    }

    /// Get configuration value
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        self.manager.get(key)
    }

    /// Get configuration value with default
    pub fn get_or<T: DeserializeOwned>(&self, key: &str, default: T) -> T {
        self.manager.get_or(key, default)
    }

    /// Get string value
    pub fn get_string(&self, key: &str) -> Result<String> {
        self.manager.get_string(key)
    }

    /// Get integer value
    pub fn get_int(&self, key: &str) -> Result<i64> {
        self.manager.get_int(key)
    }

    /// Get boolean value
    pub fn get_bool(&self, key: &str) -> Result<bool> {
        self.manager.get_bool(key)
    }

    /// Check if key exists
    pub fn has(&self, key: &str) -> bool {
        self.manager.has(key)
    }

    /// Get underlying manager
    pub fn manager(&self) -> &ConfigManager {
        &self.manager
    }
}

impl Default for ConfigService {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for ConfigService
pub struct ConfigServiceBuilder {
    manager: ConfigManager,
    load_env: bool,
    load_dotenv: bool,
    dotenv_path: Option<String>,
    config_files: Vec<(String, crate::FileFormat)>,
}

impl ConfigServiceBuilder {
    pub fn new() -> Self {
        Self {
            manager: ConfigManager::new(),
            load_env: false,
            load_dotenv: false,
            dotenv_path: None,
            config_files: Vec::new(),
        }
    }

    /// Set environment variable prefix
    pub fn with_prefix(mut self, prefix: String) -> Self {
        self.manager = ConfigManager::with_prefix(prefix);
        self
    }

    /// Enable loading from environment variables
    pub fn load_env(mut self) -> Self {
        self.load_env = true;
        self
    }

    /// Enable loading from .env file
    pub fn load_dotenv(mut self, path: Option<String>) -> Self {
        self.load_dotenv = true;
        self.dotenv_path = path;
        self
    }

    /// Add configuration file to load
    pub fn add_file(mut self, path: String, format: crate::FileFormat) -> Self {
        self.config_files.push((path, format));
        self
    }

    /// Build the configuration service
    pub fn build(self) -> Result<ConfigService> {
        // Load .env file first if specified. Surface the error so a caller who
        // pointed at a concrete `.env` path is told when it cannot be loaded.
        if self.load_dotenv {
            self.manager.load_dotenv(self.dotenv_path.as_deref())?;
        }

        // Load environment variables
        if self.load_env {
            self.manager.load_env()?;
        }

        // Load configuration files
        for (path, format) in self.config_files {
            self.manager.load_file(&path, format)?;
        }

        Ok(ConfigService::from_manager(self.manager))
    }
}

impl Default for ConfigServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_surfaces_missing_dotenv_path() {
        // Pointing at a concrete `.env` path that does not exist must fail the
        // build rather than silently succeeding.
        let missing = "/nonexistent/definitely/not/here/.env";
        let result = ConfigService::builder()
            .load_dotenv(Some(missing.to_string()))
            .build();

        assert!(
            result.is_err(),
            "build() should surface a missing concrete dotenv path as an error"
        );
    }

    #[test]
    fn test_build_without_dotenv_succeeds() {
        let result = ConfigService::builder().build();
        assert!(result.is_ok());
    }
}
