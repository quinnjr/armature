//! Service registration and discovery

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

/// Timeout applied to the default `health_check` HTTP probe so a
/// stalled/unreachable health endpoint fails fast instead of hanging the
/// caller indefinitely.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Service discovery errors
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    #[error("Registration failed: {0}")]
    RegistrationFailed(String),

    #[error("Deregistration failed: {0}")]
    DeregistrationFailed(String),

    #[error("Health check failed: {0}")]
    HealthCheckFailed(String),

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
}

/// Service instance information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInstance {
    /// Service ID (unique per instance)
    pub id: String,

    /// Service name
    pub name: String,

    /// Host/IP address
    pub address: String,

    /// Port number
    pub port: u16,

    /// Service tags
    pub tags: Vec<String>,

    /// Metadata
    pub metadata: HashMap<String, String>,

    /// Health check URL (optional)
    pub health_check_url: Option<String>,
}

impl ServiceInstance {
    /// Create new service instance
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        address: impl Into<String>,
        port: u16,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            address: address.into(),
            port,
            tags: Vec::new(),
            metadata: HashMap::new(),
            health_check_url: None,
        }
    }

    /// Add a tag
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Set health check URL
    pub fn with_health_check(mut self, url: impl Into<String>) -> Self {
        self.health_check_url = Some(url.into());
        self
    }

    /// Get full service URL
    pub fn url(&self) -> String {
        format!("http://{}:{}", self.address, self.port)
    }
}

/// Service discovery trait
#[async_trait]
pub trait ServiceDiscovery: Send + Sync {
    /// Register a service instance
    async fn register(&self, service: &ServiceInstance) -> Result<(), DiscoveryError>;

    /// Deregister a service instance
    async fn deregister(&self, service_id: &str) -> Result<(), DiscoveryError>;

    /// Discover services by name
    async fn discover(&self, service_name: &str) -> Result<Vec<ServiceInstance>, DiscoveryError>;

    /// Get a specific service instance
    async fn get_service(&self, service_id: &str) -> Result<ServiceInstance, DiscoveryError>;

    /// List all registered services
    async fn list_services(&self) -> Result<Vec<String>, DiscoveryError>;

    /// Perform health check on a service
    async fn health_check(&self, service_id: &str) -> Result<bool, DiscoveryError> {
        let service = self.get_service(service_id).await?;

        if let Some(health_url) = service.health_check_url {
            match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, reqwest::get(&health_url)).await {
                Ok(Ok(response)) => Ok(response.status().is_success()),
                // A transport error (connection refused, DNS failure, TLS
                // error, ...) is not the same thing as the service
                // reporting itself unhealthy — surface it distinctly so
                // callers can tell "unreachable" apart from "unhealthy".
                Ok(Err(e)) => Err(DiscoveryError::HealthCheckFailed(e.to_string())),
                // The probe didn't complete within the timeout budget —
                // treat that the same as any other transport failure.
                Err(_) => Err(DiscoveryError::HealthCheckFailed(format!(
                    "health check for {health_url} timed out after {HEALTH_CHECK_TIMEOUT:?}"
                ))),
            }
        } else {
            Ok(true) // No health check configured, assume healthy
        }
    }
}

/// Load balancing strategy
#[derive(Debug, Clone, Copy)]
pub enum LoadBalancingStrategy {
    /// Round-robin selection
    RoundRobin,

    /// Random selection
    Random,

    /// Always pick first available
    First,
}

/// Service resolver with load balancing
pub struct ServiceResolver<D: ServiceDiscovery> {
    discovery: D,
    strategy: LoadBalancingStrategy,
    round_robin_index: std::sync::atomic::AtomicUsize,
}

impl<D: ServiceDiscovery> ServiceResolver<D> {
    /// Create new service resolver
    pub fn new(discovery: D, strategy: LoadBalancingStrategy) -> Self {
        Self {
            discovery,
            strategy,
            round_robin_index: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Resolve a service instance using the configured strategy
    pub async fn resolve(&self, service_name: &str) -> Result<ServiceInstance, DiscoveryError> {
        let instances = self.discovery.discover(service_name).await?;

        if instances.is_empty() {
            return Err(DiscoveryError::ServiceNotFound(service_name.to_string()));
        }

        let instance = match self.strategy {
            LoadBalancingStrategy::RoundRobin => {
                let index = self
                    .round_robin_index
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                &instances[index % instances.len()]
            }
            LoadBalancingStrategy::Random => {
                let index = rand::random_range(0..instances.len());
                &instances[index]
            }
            LoadBalancingStrategy::First => &instances[0],
        };

        Ok(instance.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_instance() {
        let service = ServiceInstance::new("svc-1", "api", "localhost", 8080)
            .with_tag("production")
            .with_metadata("version", "1.0.0")
            .with_health_check("http://localhost:8080/health");

        assert_eq!(service.id, "svc-1");
        assert_eq!(service.name, "api");
        assert_eq!(service.url(), "http://localhost:8080");
        assert!(service.tags.contains(&"production".to_string()));
    }

    #[tokio::test]
    async fn health_check_returns_err_on_transport_failure_instead_of_ok_false() {
        // Port 0 is never a listening service, so connecting to it fails
        // immediately with a transport error rather than an HTTP response.
        let discovery = crate::memory::InMemoryDiscovery::new();
        let service = ServiceInstance::new("svc-1", "api", "localhost", 8080)
            .with_health_check("http://127.0.0.1:0/health");
        discovery.register(&service).await.unwrap();

        let result = discovery.health_check("svc-1").await;

        match result {
            Err(DiscoveryError::HealthCheckFailed(_)) => {}
            other => panic!(
                "expected Err(HealthCheckFailed) on transport error, got {:?}",
                other
            ),
        }
    }
}
