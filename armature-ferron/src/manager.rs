//! Ferron manager for integrated proxy management
//!
//! This module provides a high-level manager that coordinates configuration
//! generation, process management, health checking, and service discovery.

use crate::config::{Backend, FerronConfig, LoadBalancer};
use crate::error::{FerronError, Result};
use crate::health::{HealthCheckConfig, HealthState};
use crate::process::{FerronProcess, ProcessConfig, ProcessStatus};
use crate::registry::ServiceRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Ferron manager for complete proxy lifecycle management
pub struct FerronManager {
    /// Configuration file path
    config_path: PathBuf,
    /// Ferron configuration
    config: Arc<RwLock<Option<FerronConfig>>>,
    /// Ferron process handle
    process: Arc<FerronProcess>,
    /// Service registry for dynamic backends
    registry: Option<Arc<ServiceRegistry>>,
    /// Health state tracker
    health_state: Option<Arc<HealthState>>,
    /// Auto-reload on config changes
    auto_reload: bool,
    /// Watch handle for config file changes
    #[allow(dead_code)]
    watch_handle: Option<tokio::task::JoinHandle<()>>,
}

impl FerronManager {
    /// Create a new manager builder
    pub fn builder() -> FerronManagerBuilder {
        FerronManagerBuilder::default()
    }

    /// Get current process status
    pub async fn status(&self) -> ProcessStatus {
        self.process.status().await
    }

    /// Get process ID if running
    pub async fn pid(&self) -> Option<u32> {
        self.process.pid().await
    }

    /// Start Ferron with the current configuration
    pub async fn start(&self) -> Result<()> {
        // Generate configuration if using registry
        if let Some(ref registry) = self.registry {
            self.regenerate_config_from_registry(registry).await?;
        }

        // Start the process
        self.process.start().await
    }

    /// Stop Ferron
    pub async fn stop(&self) -> Result<()> {
        self.process.stop().await
    }

    /// Restart Ferron
    pub async fn restart(&self) -> Result<()> {
        self.process.restart().await
    }

    /// Reload Ferron configuration
    pub async fn reload(&self) -> Result<()> {
        // Regenerate config if using registry
        if let Some(ref registry) = self.registry {
            self.regenerate_config_from_registry(registry).await?;
        }

        self.process.reload().await
    }

    /// Get the service registry if configured
    pub fn registry(&self) -> Option<&Arc<ServiceRegistry>> {
        self.registry.as_ref()
    }

    /// Get the health state if configured
    pub fn health_state(&self) -> Option<&Arc<HealthState>> {
        self.health_state.as_ref()
    }

    /// Update configuration and reload
    pub async fn update_config(&self, config: FerronConfig) -> Result<()> {
        // Write new configuration
        config.write_to_file(&self.config_path).await?;
        *self.config.write().await = Some(config);

        // Reload if running
        if self.process.status().await == ProcessStatus::Running {
            self.process.reload().await?;
        }

        Ok(())
    }

    /// Register a backend and update configuration
    pub async fn register_backend(&self, service_name: &str, url: &str) -> Result<String> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| FerronError::registry("Service registry not configured"))?;

        let id = registry.register(service_name, url).await?;

        // Regenerate and reload if auto-reload is enabled
        if self.auto_reload && self.process.status().await == ProcessStatus::Running {
            self.regenerate_config_from_registry(registry).await?;
            self.process.reload().await?;
        }

        Ok(id)
    }

    /// Deregister a backend and update configuration
    pub async fn deregister_backend(&self, service_name: &str, instance_id: &str) -> Result<()> {
        let registry = self
            .registry
            .as_ref()
            .ok_or_else(|| FerronError::registry("Service registry not configured"))?;

        registry.deregister(service_name, instance_id).await?;

        // Regenerate and reload if auto-reload is enabled
        if self.auto_reload && self.process.status().await == ProcessStatus::Running {
            self.regenerate_config_from_registry(registry).await?;
            self.process.reload().await?;
        }

        Ok(())
    }

    /// Regenerate configuration from service registry
    async fn regenerate_config_from_registry(&self, registry: &ServiceRegistry) -> Result<()> {
        let mut config_guard = self.config.write().await;
        let config = config_guard
            .as_mut()
            .ok_or_else(|| FerronError::config("No base configuration set"))?;

        // Get all services and their URLs
        let services = registry.get_services().await;

        // Update load balancer with discovered backends
        let mut backends = Vec::new();
        for service in &services {
            let urls = registry.get_healthy_urls(service).await;
            for url in urls {
                backends.push(Backend::new(url));
            }
        }

        if !backends.is_empty() {
            let lb = LoadBalancer::new();
            let mut lb_with_backends = lb;
            for backend in backends {
                lb_with_backends = lb_with_backends.backend(backend);
            }
            config.load_balancer = Some(lb_with_backends);
        }

        // Write updated configuration
        config.write_to_file(&self.config_path).await?;
        info!("Regenerated Ferron configuration from service registry");

        Ok(())
    }

    /// Start with supervision (auto-restart on crash)
    pub async fn start_supervised(self: Arc<Self>) -> Result<tokio::task::JoinHandle<()>> {
        // Start health checking if configured
        if let Some(ref health_state) = self.health_state
            && let Some(ref registry) = self.registry
        {
            let backends: Vec<String> = {
                let services = registry.get_services().await;
                let mut urls = Vec::new();
                for service in services {
                    urls.extend(registry.get_urls(&service).await);
                }
                urls
            };

            if !backends.is_empty() {
                let _ = health_state.clone().start_background_checks(backends).await;
            }
        }

        // Start the process with supervision
        self.process.clone().start_with_supervision().await
    }
}

/// Builder for FerronManager
#[derive(Default)]
pub struct FerronManagerBuilder {
    binary_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    config: Option<FerronConfig>,
    registry: Option<Arc<ServiceRegistry>>,
    health_config: Option<HealthCheckConfig>,
    auto_reload: bool,
    auto_restart: bool,
    working_dir: Option<PathBuf>,
}

impl FerronManagerBuilder {
    /// Set the Ferron binary path
    pub fn binary_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.binary_path = Some(path.into());
        self
    }

    /// Set the configuration file path
    pub fn config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Set the Ferron configuration
    pub fn config(mut self, config: FerronConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the service registry for dynamic discovery
    pub fn service_registry(mut self, registry: ServiceRegistry) -> Self {
        self.registry = Some(Arc::new(registry));
        self
    }

    /// Set an existing service registry
    pub fn service_registry_arc(mut self, registry: Arc<ServiceRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Enable health checking with configuration
    pub fn health_check(mut self, config: HealthCheckConfig) -> Self {
        self.health_config = Some(config);
        self
    }

    /// Enable auto-reload on configuration changes
    pub fn auto_reload(mut self, enabled: bool) -> Self {
        self.auto_reload = enabled;
        self
    }

    /// Enable auto-restart on process crash
    pub fn auto_restart(mut self, enabled: bool) -> Self {
        self.auto_restart = enabled;
        self
    }

    /// Set working directory
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(path.into());
        self
    }

    /// Build the FerronManager
    pub fn build(self) -> Result<FerronManager> {
        let config_path = self
            .config_path
            .unwrap_or_else(|| PathBuf::from("/etc/ferron/ferron.conf"));

        let binary_path = self.binary_path.unwrap_or_else(|| PathBuf::from("ferron"));

        // Create process config
        let mut process_config = ProcessConfig::new(&binary_path, &config_path);
        process_config.auto_restart = self.auto_restart;

        if let Some(dir) = self.working_dir {
            process_config = process_config.working_dir(dir);
        }

        // Create health state if configured
        let health_state = self
            .health_config
            .map(|config| Arc::new(HealthState::new(config)));

        // Write initial config if provided
        if let Some(ref config) = self.config {
            // Write config synchronously for builder
            let kdl = config.to_kdl()?;
            std::fs::write(&config_path, kdl)?;
        }

        Ok(FerronManager {
            config_path,
            config: Arc::new(RwLock::new(self.config)),
            process: Arc::new(FerronProcess::new(process_config)),
            registry: self.registry,
            health_state,
            auto_reload: self.auto_reload,
            watch_handle: None,
        })
    }
}

/// Convenience functions for common Ferron operations
pub mod helpers {
    use super::*;

    /// Generate a basic reverse proxy configuration
    pub fn reverse_proxy_config(
        domain: impl Into<String>,
        backend_url: impl Into<String>,
    ) -> Result<FerronConfig> {
        FerronConfig::builder()
            .domain(domain)
            .backend_url(backend_url)
            .tls_auto(true)
            .gzip(true)
            .build()
    }

    /// Generate a load-balanced configuration
    pub fn load_balanced_config(
        domain: impl Into<String>,
        backends: Vec<impl Into<String>>,
    ) -> Result<FerronConfig> {
        let mut lb = LoadBalancer::new();
        for backend in backends {
            lb = lb.backend(Backend::new(backend));
        }

        FerronConfig::builder()
            .domain(domain)
            .load_balancer(lb)
            .tls_auto(true)
            .gzip(true)
            .build()
    }

    /// Generate configuration for an Armature application
    pub fn armature_app_config(domain: impl Into<String>, app_port: u16) -> Result<FerronConfig> {
        use crate::config::{Location, RateLimitConfig};

        FerronConfig::builder()
            .domain(domain)
            .backend_url(format!("http://127.0.0.1:{}", app_port))
            .tls_auto(true)
            .gzip(true)
            // API routes with rate limiting
            .location(
                Location::new("/api")
                    .proxy(format!("http://127.0.0.1:{}/api", app_port))
                    .rate_limit(RateLimitConfig::new(100).burst(200)),
            )
            // WebSocket support
            .location(Location::new("/ws").proxy(format!("http://127.0.0.1:{}/ws", app_port)))
            // Health endpoint (no rate limit)
            .location(
                Location::new("/health").proxy(format!("http://127.0.0.1:{}/health", app_port)),
            )
            // Security headers
            .header("X-Frame-Options", "DENY")
            .header("X-Content-Type-Options", "nosniff")
            .header("X-XSS-Protection", "1; mode=block")
            .header("Referrer-Policy", "strict-origin-when-cross-origin")
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_proxy_config() {
        let config = helpers::reverse_proxy_config("example.com", "http://localhost:3000").unwrap();

        assert_eq!(config.domains, vec!["example.com"]);
        assert_eq!(config.backend, Some("http://localhost:3000".to_string()));
        assert!(config.tls.is_some());
    }

    #[test]
    fn test_load_balanced_config() {
        let config = helpers::load_balanced_config(
            "example.com",
            vec!["http://localhost:3001", "http://localhost:3002"],
        )
        .unwrap();

        assert!(config.load_balancer.is_some());
        let lb = config.load_balancer.unwrap();
        assert_eq!(lb.backends.len(), 2);
    }

    #[test]
    fn test_armature_app_config() {
        let config = helpers::armature_app_config("api.example.com", 3000).unwrap();

        assert_eq!(config.domains, vec!["api.example.com"]);
        assert!(!config.locations.is_empty());
        assert!(config.headers.contains_key("X-Frame-Options"));
    }

    #[test]
    fn test_manager_builder() {
        // Note: This test doesn't actually start Ferron, just tests builder
        let config = FerronConfig::builder()
            .domain("example.com")
            .backend_url("http://localhost:3000")
            .build()
            .unwrap();

        // Builder should work even without Ferron installed
        let result = FerronManager::builder()
            .binary_path("/nonexistent/ferron")
            .config_path("/tmp/test_ferron.conf")
            .config(config)
            .auto_reload(true)
            .auto_restart(true)
            .build();

        assert!(result.is_ok());
    }
}
