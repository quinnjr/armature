//! In-memory service discovery (for testing)

use crate::service::{DiscoveryError, ServiceDiscovery, ServiceInstance};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory service discovery (for testing/development)
#[derive(Clone)]
pub struct InMemoryDiscovery {
    services: Arc<RwLock<HashMap<String, ServiceInstance>>>,
}

impl InMemoryDiscovery {
    /// Create new in-memory discovery
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Clear all registered services
    pub async fn clear(&self) {
        self.services.write().await.clear();
    }

    /// Get count of registered services
    pub async fn count(&self) -> usize {
        self.services.read().await.len()
    }
}

impl Default for InMemoryDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ServiceDiscovery for InMemoryDiscovery {
    async fn register(&self, service: &ServiceInstance) -> Result<(), DiscoveryError> {
        self.services
            .write()
            .await
            .insert(service.id.clone(), service.clone());
        Ok(())
    }

    async fn deregister(&self, service_id: &str) -> Result<(), DiscoveryError> {
        self.services
            .write()
            .await
            .remove(service_id)
            .ok_or_else(|| DiscoveryError::ServiceNotFound(service_id.to_string()))?;
        Ok(())
    }

    async fn discover(&self, service_name: &str) -> Result<Vec<ServiceInstance>, DiscoveryError> {
        let services = self.services.read().await;
        let instances: Vec<ServiceInstance> = services
            .values()
            .filter(|s| s.name == service_name)
            .cloned()
            .collect();

        // An empty result is a normal discovery outcome, not an error —
        // matches Consul's contract, and ServiceResolver::resolve is the
        // place that turns "no instances" into ServiceNotFound.
        Ok(instances)
    }

    async fn get_service(&self, service_id: &str) -> Result<ServiceInstance, DiscoveryError> {
        self.services
            .read()
            .await
            .get(service_id)
            .cloned()
            .ok_or_else(|| DiscoveryError::ServiceNotFound(service_id.to_string()))
    }

    async fn list_services(&self) -> Result<Vec<String>, DiscoveryError> {
        let services = self.services.read().await;
        let mut service_names: Vec<String> = services.values().map(|s| s.name.clone()).collect();

        service_names.sort();
        service_names.dedup();

        Ok(service_names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_discovery() {
        let discovery = InMemoryDiscovery::new();

        let service = ServiceInstance::new("svc-1", "api", "localhost", 8080);

        // Register
        discovery.register(&service).await.unwrap();
        assert_eq!(discovery.count().await, 1);

        // Discover
        let instances = discovery.discover("api").await.unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].id, "svc-1");

        // Get service
        let retrieved = discovery.get_service("svc-1").await.unwrap();
        assert_eq!(retrieved.name, "api");

        // Deregister
        discovery.deregister("svc-1").await.unwrap();
        assert_eq!(discovery.count().await, 0);
    }

    #[tokio::test]
    async fn discover_returns_ok_empty_for_unregistered_service_name() {
        let discovery = InMemoryDiscovery::new();

        let instances = discovery.discover("nonexistent").await.unwrap();
        assert!(
            instances.is_empty(),
            "discover() of an unregistered name must return Ok(vec![]), matching the other backends"
        );
    }

    #[tokio::test]
    async fn get_service_still_errors_for_an_unknown_id() {
        let discovery = InMemoryDiscovery::new();

        let result = discovery.get_service("missing").await;
        assert!(matches!(result, Err(DiscoveryError::ServiceNotFound(_))));
    }
}
