//! Ferron configuration generation
//!
//! This module provides types and utilities for generating Ferron configuration
//! files from Armature application metadata.
//!
//! ## Configuration Format
//!
//! Ferron uses KDL (KDL Document Language) for configuration. This module generates
//! valid KDL configuration that can be written to a file or passed directly to Ferron.
//!
//! ## Example
//!
//! ```rust
//! use armature_ferron::{FerronConfig, Backend, Location};
//!
//! let config = FerronConfig::builder()
//!     .domain("api.example.com")
//!     .backend(Backend::new("http://localhost:3000"))
//!     .location(Location::new("/api").remove_base(true))
//!     .tls_auto(true)
//!     .build()
//!     .unwrap();
//!
//! let kdl = config.to_kdl().unwrap();
//! ```

use crate::error::{FerronError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;
use url::Url;

/// Load balancing strategy for multiple backends
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Round-robin distribution (default)
    #[default]
    RoundRobin,
    /// Least connections - route to backend with fewest active connections
    LeastConnections,
    /// IP hash - consistent routing based on client IP
    IpHash,
    /// Random selection
    Random,
    /// Weighted round-robin based on backend weights
    Weighted,
}

impl LoadBalanceStrategy {
    /// Convert to Ferron configuration value
    pub fn to_ferron_value(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::LeastConnections => "least_conn",
            Self::IpHash => "ip_hash",
            Self::Random => "random",
            Self::Weighted => "weighted",
        }
    }
}

/// Backend server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backend {
    /// Backend URL (e.g., "http://localhost:3000")
    pub url: String,
    /// Weight for load balancing (higher = more traffic)
    pub weight: u32,
    /// Maximum connections to this backend
    pub max_connections: Option<u32>,
    /// Connection timeout in seconds
    pub timeout: Option<u32>,
    /// Whether this backend is a backup (used only when primaries are down)
    pub backup: bool,
    /// Custom headers to add when proxying to this backend
    pub headers: HashMap<String, String>,
}

impl Backend {
    /// Create a new backend with the given URL
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            weight: 1,
            max_connections: None,
            timeout: None,
            backup: false,
            headers: HashMap::new(),
        }
    }

    /// Set the weight for load balancing
    pub fn weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }

    /// Set the maximum connections
    pub fn max_connections(mut self, max: u32) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Set the connection timeout in seconds
    pub fn timeout(mut self, seconds: u32) -> Self {
        self.timeout = Some(seconds);
        self
    }

    /// Mark as backup backend
    pub fn backup(mut self) -> Self {
        self.backup = true;
        self
    }

    /// Add a custom header
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Validate the backend configuration
    pub fn validate(&self) -> Result<()> {
        Url::parse(&self.url)?;
        if self.weight == 0 {
            return Err(FerronError::invalid_config(
                "weight",
                "must be greater than 0",
            ));
        }
        Ok(())
    }
}

/// Load balancer configuration for multiple backends
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadBalancer {
    /// Load balancing strategy
    pub strategy: LoadBalanceStrategy,
    /// Backend servers
    pub backends: Vec<Backend>,
    /// Health check interval in seconds
    pub health_check_interval: Option<u32>,
    /// Health check path (e.g., "/health")
    pub health_check_path: Option<String>,
    /// Number of failures before marking backend unhealthy
    pub health_check_threshold: Option<u32>,
}

impl LoadBalancer {
    /// Create a new load balancer
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the load balancing strategy
    pub fn strategy(mut self, strategy: LoadBalanceStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Add a backend server
    pub fn backend(mut self, backend: Backend) -> Self {
        self.backends.push(backend);
        self
    }

    /// Set health check interval in seconds
    pub fn health_check_interval(mut self, seconds: u32) -> Self {
        self.health_check_interval = Some(seconds);
        self
    }

    /// Set the health check path
    pub fn health_check_path(mut self, path: impl Into<String>) -> Self {
        self.health_check_path = Some(path.into());
        self
    }

    /// Set the health check failure threshold
    pub fn health_check_threshold(mut self, threshold: u32) -> Self {
        self.health_check_threshold = Some(threshold);
        self
    }

    /// Validate the load balancer configuration
    pub fn validate(&self) -> Result<()> {
        if self.backends.is_empty() {
            return Err(FerronError::invalid_config(
                "backends",
                "at least one backend is required",
            ));
        }
        for backend in &self.backends {
            backend.validate()?;
        }
        Ok(())
    }
}

/// Location block configuration for path-based routing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    /// Path prefix to match (e.g., "/api")
    pub path: String,
    /// Whether to remove the base path when proxying
    pub remove_base: bool,
    /// Backend URL for this location (overrides default)
    pub proxy: Option<String>,
    /// Static file root directory
    pub root: Option<String>,
    /// Custom response headers
    pub headers: HashMap<String, String>,
    /// Rate limiting for this location
    pub rate_limit: Option<RateLimitConfig>,
}

impl Location {
    /// Create a new location for the given path
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            remove_base: false,
            proxy: None,
            root: None,
            headers: HashMap::new(),
            rate_limit: None,
        }
    }

    /// Set whether to remove the base path when proxying
    pub fn remove_base(mut self, remove: bool) -> Self {
        self.remove_base = remove;
        self
    }

    /// Set the proxy backend for this location
    pub fn proxy(mut self, url: impl Into<String>) -> Self {
        self.proxy = Some(url.into());
        self
    }

    /// Set the static file root directory
    pub fn root(mut self, path: impl Into<String>) -> Self {
        self.root = Some(path.into());
        self
    }

    /// Add a response header
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Set rate limiting for this location
    pub fn rate_limit(mut self, config: RateLimitConfig) -> Self {
        self.rate_limit = Some(config);
        self
    }
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Requests per second limit
    pub requests_per_second: u32,
    /// Burst size (maximum requests allowed in a burst)
    pub burst: u32,
    /// Key to use for rate limiting (e.g., "ip", "header:X-API-Key")
    pub key: String,
}

impl RateLimitConfig {
    /// Create a new rate limit configuration
    pub fn new(requests_per_second: u32) -> Self {
        Self {
            requests_per_second,
            burst: requests_per_second * 2,
            key: "ip".to_string(),
        }
    }

    /// Set the burst size
    pub fn burst(mut self, burst: u32) -> Self {
        self.burst = burst;
        self
    }

    /// Set the rate limit key
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }
}

/// TLS configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Enable automatic TLS with Let's Encrypt
    pub auto: bool,
    /// Path to certificate file (for manual TLS)
    pub cert_path: Option<String>,
    /// Path to private key file (for manual TLS)
    pub key_path: Option<String>,
    /// Email for Let's Encrypt registration
    pub email: Option<String>,
    /// Enable HSTS (HTTP Strict Transport Security)
    pub hsts: bool,
    /// HSTS max-age in seconds
    pub hsts_max_age: Option<u64>,
    /// Minimum TLS version (e.g., "1.2", "1.3")
    pub min_version: Option<String>,
}

impl TlsConfig {
    /// Create automatic TLS configuration
    pub fn auto() -> Self {
        Self {
            auto: true,
            hsts: true,
            hsts_max_age: Some(31536000), // 1 year
            ..Default::default()
        }
    }

    /// Create manual TLS configuration with certificate files
    pub fn manual(cert_path: impl Into<String>, key_path: impl Into<String>) -> Self {
        Self {
            auto: false,
            cert_path: Some(cert_path.into()),
            key_path: Some(key_path.into()),
            hsts: true,
            hsts_max_age: Some(31536000),
            ..Default::default()
        }
    }

    /// Set the email for Let's Encrypt
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// Enable or disable HSTS
    pub fn hsts(mut self, enabled: bool) -> Self {
        self.hsts = enabled;
        self
    }

    /// Set HSTS max-age
    pub fn hsts_max_age(mut self, seconds: u64) -> Self {
        self.hsts_max_age = Some(seconds);
        self
    }

    /// Set minimum TLS version
    pub fn min_version(mut self, version: impl Into<String>) -> Self {
        self.min_version = Some(version.into());
        self
    }
}

/// Proxy route configuration (simplified alternative to Location)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRoute {
    /// Path pattern to match
    pub path: String,
    /// Backend URL to proxy to
    pub backend: String,
    /// Strip the path prefix when proxying
    pub strip_prefix: bool,
    /// Add path prefix when proxying
    pub add_prefix: Option<String>,
    /// Websocket support
    pub websocket: bool,
    /// Request timeout in seconds
    pub timeout: Option<u32>,
}

impl ProxyRoute {
    /// Create a new proxy route
    pub fn new(path: impl Into<String>, backend: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            backend: backend.into(),
            strip_prefix: false,
            add_prefix: None,
            websocket: false,
            timeout: None,
        }
    }

    /// Strip the path prefix when proxying
    pub fn strip_prefix(mut self) -> Self {
        self.strip_prefix = true;
        self
    }

    /// Add a prefix to the path when proxying
    pub fn add_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.add_prefix = Some(prefix.into());
        self
    }

    /// Enable WebSocket support
    pub fn websocket(mut self) -> Self {
        self.websocket = true;
        self
    }

    /// Set request timeout
    pub fn timeout(mut self, seconds: u32) -> Self {
        self.timeout = Some(seconds);
        self
    }
}

/// Main Ferron configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FerronConfig {
    /// Domain name(s) for this server block
    pub domains: Vec<String>,
    /// Default backend URL
    pub backend: Option<String>,
    /// Load balancer configuration (for multiple backends)
    pub load_balancer: Option<LoadBalancer>,
    /// Location blocks for path-based routing
    pub locations: Vec<Location>,
    /// Proxy routes (simplified location blocks)
    pub routes: Vec<ProxyRoute>,
    /// TLS configuration
    pub tls: Option<TlsConfig>,
    /// HTTP port (default: 80)
    pub http_port: u16,
    /// HTTPS port (default: 443)
    pub https_port: u16,
    /// Global rate limiting
    pub rate_limit: Option<RateLimitConfig>,
    /// Access log path
    pub access_log: Option<String>,
    /// Error log path
    pub error_log: Option<String>,
    /// Custom response headers for all responses
    pub headers: HashMap<String, String>,
    /// Enable gzip compression
    pub gzip: bool,
    /// Gzip compression level (1-9)
    pub gzip_level: Option<u8>,
    /// Enable request logging
    pub logging: bool,
}

impl Default for FerronConfig {
    fn default() -> Self {
        Self {
            domains: Vec::new(),
            backend: None,
            load_balancer: None,
            locations: Vec::new(),
            routes: Vec::new(),
            tls: None,
            http_port: 80,
            https_port: 443,
            rate_limit: None,
            access_log: None,
            error_log: None,
            headers: HashMap::new(),
            gzip: true,
            gzip_level: Some(6),
            logging: true,
        }
    }
}

impl FerronConfig {
    /// Create a new configuration builder
    pub fn builder() -> FerronConfigBuilder {
        FerronConfigBuilder::default()
    }

    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        if self.domains.is_empty() {
            return Err(FerronError::invalid_config(
                "domains",
                "at least one domain is required",
            ));
        }

        if self.backend.is_none() && self.load_balancer.is_none() && self.locations.is_empty() {
            return Err(FerronError::invalid_config(
                "backend",
                "at least one backend, load balancer, or location is required",
            ));
        }

        if let Some(ref backend) = self.backend {
            Url::parse(backend)?;
        }

        if let Some(ref lb) = self.load_balancer {
            lb.validate()?;
        }

        if let Some(ref tls) = self.tls
            && !tls.auto
            && (tls.cert_path.is_none() || tls.key_path.is_none())
        {
            return Err(FerronError::invalid_config(
                "tls",
                "manual TLS requires both cert_path and key_path",
            ));
        }

        Ok(())
    }

    /// Generate KDL configuration string
    pub fn to_kdl(&self) -> Result<String> {
        self.validate()?;
        let mut output = String::new();

        // Domain block
        let domains = self.domains.join(" ");
        writeln!(output, "{} {{", domains).map_err(|e| FerronError::config(e.to_string()))?;

        // TLS configuration
        if let Some(ref tls) = self.tls {
            if tls.auto {
                writeln!(output, "    tls auto").map_err(|e| FerronError::config(e.to_string()))?;
                if let Some(ref email) = tls.email {
                    writeln!(output, "    tls_email \"{}\"", email)
                        .map_err(|e| FerronError::config(e.to_string()))?;
                }
            } else if let (Some(cert), Some(key)) = (&tls.cert_path, &tls.key_path) {
                writeln!(output, "    tls \"{}\" \"{}\"", cert, key)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }
            if tls.hsts
                && let Some(max_age) = tls.hsts_max_age
            {
                writeln!(output, "    hsts max_age={}", max_age)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }
            if let Some(ref min_ver) = tls.min_version {
                writeln!(output, "    tls_min_version \"{}\"", min_ver)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }
        }

        // Compression
        if self.gzip {
            if let Some(level) = self.gzip_level {
                writeln!(output, "    gzip level={}", level)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            } else {
                writeln!(output, "    gzip").map_err(|e| FerronError::config(e.to_string()))?;
            }
        }

        // Logging
        if let Some(ref access_log) = self.access_log {
            writeln!(output, "    log \"{}\"", access_log)
                .map_err(|e| FerronError::config(e.to_string()))?;
        }

        // Global headers
        for (name, value) in &self.headers {
            writeln!(output, "    header \"{}\" \"{}\"", name, value)
                .map_err(|e| FerronError::config(e.to_string()))?;
        }

        // Rate limiting
        if let Some(ref rl) = self.rate_limit {
            writeln!(
                output,
                "    rate_limit key=\"{}\" rate={} burst={}",
                rl.key, rl.requests_per_second, rl.burst
            )
            .map_err(|e| FerronError::config(e.to_string()))?;
        }

        // Load balancer or single backend
        if let Some(ref lb) = self.load_balancer {
            writeln!(
                output,
                "    lb_method \"{}\"",
                lb.strategy.to_ferron_value()
            )
            .map_err(|e| FerronError::config(e.to_string()))?;

            for backend in &lb.backends {
                let mut proxy_line = format!("    proxy \"{}\"", backend.url);
                if backend.weight != 1 {
                    proxy_line.push_str(&format!(" weight={}", backend.weight));
                }
                if backend.backup {
                    proxy_line.push_str(" backup");
                }
                if let Some(max_conn) = backend.max_connections {
                    proxy_line.push_str(&format!(" max_conn={}", max_conn));
                }
                writeln!(output, "{}", proxy_line)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }

            if let Some(interval) = lb.health_check_interval {
                let mut hc_line = format!("    lb_health_check interval={}", interval);
                if let Some(ref path) = lb.health_check_path {
                    hc_line.push_str(&format!(" path=\"{}\"", path));
                }
                if let Some(threshold) = lb.health_check_threshold {
                    hc_line.push_str(&format!(" threshold={}", threshold));
                }
                writeln!(output, "{}", hc_line).map_err(|e| FerronError::config(e.to_string()))?;
            }
        } else if let Some(ref backend) = self.backend {
            writeln!(output, "    proxy \"{}\"", backend)
                .map_err(|e| FerronError::config(e.to_string()))?;
        }

        // Location blocks
        for location in &self.locations {
            let mut loc_attrs = String::new();
            if location.remove_base {
                loc_attrs.push_str(" remove_base=true");
            }

            writeln!(
                output,
                "    location \"{}\"{}  {{",
                location.path, loc_attrs
            )
            .map_err(|e| FerronError::config(e.to_string()))?;

            if let Some(ref proxy) = location.proxy {
                writeln!(output, "        proxy \"{}\"", proxy)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }

            if let Some(ref root) = location.root {
                writeln!(output, "        root \"{}\"", root)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }

            for (name, value) in &location.headers {
                writeln!(output, "        header \"{}\" \"{}\"", name, value)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }

            if let Some(ref rl) = location.rate_limit {
                writeln!(
                    output,
                    "        rate_limit key=\"{}\" rate={} burst={}",
                    rl.key, rl.requests_per_second, rl.burst
                )
                .map_err(|e| FerronError::config(e.to_string()))?;
            }

            writeln!(output, "    }}").map_err(|e| FerronError::config(e.to_string()))?;
        }

        // Proxy routes (simplified)
        for route in &self.routes {
            let mut attrs = String::new();
            if route.strip_prefix {
                attrs.push_str(" remove_base=true");
            }

            writeln!(output, "    location \"{}\"{}  {{", route.path, attrs)
                .map_err(|e| FerronError::config(e.to_string()))?;

            writeln!(output, "        proxy \"{}\"", route.backend)
                .map_err(|e| FerronError::config(e.to_string()))?;

            if route.websocket {
                writeln!(output, "        websocket")
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }

            if let Some(timeout) = route.timeout {
                writeln!(output, "        timeout {}", timeout)
                    .map_err(|e| FerronError::config(e.to_string()))?;
            }

            writeln!(output, "    }}").map_err(|e| FerronError::config(e.to_string()))?;
        }

        writeln!(output, "}}").map_err(|e| FerronError::config(e.to_string()))?;

        Ok(output)
    }

    /// Write configuration to a file
    pub async fn write_to_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        let kdl = self.to_kdl()?;
        tokio::fs::write(path, kdl).await?;
        Ok(())
    }
}

/// Builder for FerronConfig
#[derive(Debug, Clone, Default)]
pub struct FerronConfigBuilder {
    config: FerronConfig,
}

impl FerronConfigBuilder {
    /// Add a domain
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.config.domains.push(domain.into());
        self
    }

    /// Add multiple domains
    pub fn domains(mut self, domains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.config
            .domains
            .extend(domains.into_iter().map(Into::into));
        self
    }

    /// Set the default backend
    pub fn backend(mut self, backend: Backend) -> Self {
        self.config.backend = Some(backend.url);
        self
    }

    /// Set the default backend URL
    pub fn backend_url(mut self, url: impl Into<String>) -> Self {
        self.config.backend = Some(url.into());
        self
    }

    /// Configure load balancing
    pub fn load_balancer(mut self, lb: LoadBalancer) -> Self {
        self.config.load_balancer = Some(lb);
        self
    }

    /// Add a location block
    pub fn location(mut self, location: Location) -> Self {
        self.config.locations.push(location);
        self
    }

    /// Add a proxy route
    pub fn route(mut self, route: ProxyRoute) -> Self {
        self.config.routes.push(route);
        self
    }

    /// Enable automatic TLS
    pub fn tls_auto(mut self, enabled: bool) -> Self {
        if enabled {
            self.config.tls = Some(TlsConfig::auto());
        }
        self
    }

    /// Configure TLS
    pub fn tls(mut self, config: TlsConfig) -> Self {
        self.config.tls = Some(config);
        self
    }

    /// Set HTTP port
    pub fn http_port(mut self, port: u16) -> Self {
        self.config.http_port = port;
        self
    }

    /// Set HTTPS port
    pub fn https_port(mut self, port: u16) -> Self {
        self.config.https_port = port;
        self
    }

    /// Set global rate limiting
    pub fn rate_limit(mut self, config: RateLimitConfig) -> Self {
        self.config.rate_limit = Some(config);
        self
    }

    /// Set access log path
    pub fn access_log(mut self, path: impl Into<String>) -> Self {
        self.config.access_log = Some(path.into());
        self
    }

    /// Set error log path
    pub fn error_log(mut self, path: impl Into<String>) -> Self {
        self.config.error_log = Some(path.into());
        self
    }

    /// Add a response header
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.headers.insert(name.into(), value.into());
        self
    }

    /// Enable/disable gzip compression
    pub fn gzip(mut self, enabled: bool) -> Self {
        self.config.gzip = enabled;
        self
    }

    /// Set gzip compression level
    pub fn gzip_level(mut self, level: u8) -> Self {
        self.config.gzip_level = Some(level.min(9).max(1));
        self
    }

    /// Enable/disable logging
    pub fn logging(mut self, enabled: bool) -> Self {
        self.config.logging = enabled;
        self
    }

    /// Build the configuration
    pub fn build(self) -> Result<FerronConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_builder() {
        let backend = Backend::new("http://localhost:3000")
            .weight(3)
            .max_connections(100)
            .timeout(30)
            .backup()
            .header("X-Forwarded-For", "$remote_addr");

        assert_eq!(backend.url, "http://localhost:3000");
        assert_eq!(backend.weight, 3);
        assert_eq!(backend.max_connections, Some(100));
        assert_eq!(backend.timeout, Some(30));
        assert!(backend.backup);
        assert_eq!(
            backend.headers.get("X-Forwarded-For"),
            Some(&"$remote_addr".to_string())
        );
    }

    #[test]
    fn test_config_builder() {
        let config = FerronConfig::builder()
            .domain("api.example.com")
            .backend_url("http://localhost:3000")
            .tls_auto(true)
            .gzip(true)
            .header("X-Frame-Options", "DENY")
            .build()
            .unwrap();

        assert_eq!(config.domains, vec!["api.example.com"]);
        assert_eq!(config.backend, Some("http://localhost:3000".to_string()));
        assert!(config.tls.is_some());
        assert!(config.gzip);
    }

    #[test]
    fn test_load_balancer() {
        let lb = LoadBalancer::new()
            .strategy(LoadBalanceStrategy::RoundRobin)
            .backend(Backend::new("http://backend1:3000").weight(3))
            .backend(Backend::new("http://backend2:3000").weight(1))
            .health_check_interval(30)
            .health_check_path("/health");

        assert_eq!(lb.backends.len(), 2);
        assert_eq!(lb.strategy, LoadBalanceStrategy::RoundRobin);
        assert_eq!(lb.health_check_interval, Some(30));
    }

    #[test]
    fn test_kdl_generation() {
        let config = FerronConfig::builder()
            .domain("api.example.com")
            .backend_url("http://localhost:3000")
            .tls_auto(true)
            .build()
            .unwrap();

        let kdl = config.to_kdl().unwrap();
        assert!(kdl.contains("api.example.com"));
        assert!(kdl.contains("proxy \"http://localhost:3000\""));
        assert!(kdl.contains("tls auto"));
    }

    #[test]
    fn test_validation_no_domain() {
        let result = FerronConfig::builder()
            .backend_url("http://localhost:3000")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_validation_no_backend() {
        let result = FerronConfig::builder().domain("example.com").build();

        assert!(result.is_err());
    }
}
