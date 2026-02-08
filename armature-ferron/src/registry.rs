//! Service registry for dynamic backend discovery
//!
//! This module provides a service registry for dynamically discovering
//! and managing backend services behind Ferron proxy.

use crate::error::{FerronError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// A registered service instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInstance {
    /// Unique instance ID
    pub id: String,
    /// Service name
    pub service_name: String,
    /// Instance URL (e.g., "http://localhost:3000")
    pub url: String,
    /// Instance weight for load balancing
    pub weight: u32,
    /// Metadata for the instance
    pub metadata: HashMap<String, String>,
    /// Registration timestamp
    pub registered_at: DateTime<Utc>,
    /// Last heartbeat timestamp
    pub last_heartbeat: DateTime<Utc>,
    /// Whether the instance is healthy
    pub healthy: bool,
    /// Tags for filtering
    pub tags: Vec<String>,
}

impl ServiceInstance {
    /// Create a new service instance
    pub fn new(service_name: impl Into<String>, url: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            service_name: service_name.into(),
            url: url.into(),
            weight: 1,
            metadata: HashMap::new(),
            registered_at: now,
            last_heartbeat: now,
            healthy: true,
            tags: Vec::new(),
        }
    }

    /// Set the weight
    pub fn weight(mut self, weight: u32) -> Self {
        self.weight = weight;
        self
    }

    /// Add metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Add a tag
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Update heartbeat timestamp
    pub fn heartbeat(&mut self) {
        self.last_heartbeat = Utc::now();
    }

    /// Check if the instance is stale (no heartbeat for given duration)
    pub fn is_stale(&self, max_age: chrono::Duration) -> bool {
        Utc::now() - self.last_heartbeat > max_age
    }
}

/// Service registry for managing backend instances
#[derive(Default)]
pub struct ServiceRegistry {
    /// Registered services: service_name -> instance_id -> instance
    services: Arc<RwLock<HashMap<String, HashMap<String, ServiceInstance>>>>,
    /// Change listeners
    listeners: Arc<RwLock<Vec<Box<dyn Fn(&str) + Send + Sync>>>>,
}

impl ServiceRegistry {
    /// Create a new service registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new service instance
    pub async fn register(
        &self,
        service_name: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<String> {
        let instance = ServiceInstance::new(service_name, url);
        self.register_instance(instance).await
    }

    /// Register a service instance with full configuration
    pub async fn register_instance(&self, instance: ServiceInstance) -> Result<String> {
        let service_name = instance.service_name.clone();
        let instance_id = instance.id.clone();

        let mut services = self.services.write().await;
        let instances = services.entry(service_name.clone()).or_default();
        instances.insert(instance_id.clone(), instance);

        info!(
            "Registered service instance {} for service {}",
            instance_id, service_name
        );

        // Notify listeners
        self.notify_listeners(&service_name).await;

        Ok(instance_id)
    }

    /// Deregister a service instance
    pub async fn deregister(&self, service_name: &str, instance_id: &str) -> Result<()> {
        let mut services = self.services.write().await;

        if let Some(instances) = services.get_mut(service_name)
            && instances.remove(instance_id).is_some()
        {
            info!(
                "Deregistered instance {} from service {}",
                instance_id, service_name
            );

            // Remove empty service entries
            if instances.is_empty() {
                services.remove(service_name);
            }

            // Notify listeners
            drop(services); // Release lock before notifying
            self.notify_listeners(service_name).await;

            return Ok(());
        }

        Err(FerronError::ServiceNotFound(format!(
            "{}:{}",
            service_name, instance_id
        )))
    }

    /// Update heartbeat for an instance
    pub async fn heartbeat(&self, service_name: &str, instance_id: &str) -> Result<()> {
        let mut services = self.services.write().await;

        if let Some(instances) = services.get_mut(service_name)
            && let Some(instance) = instances.get_mut(instance_id)
        {
            instance.heartbeat();
            debug!("Updated heartbeat for {}:{}", service_name, instance_id);
            return Ok(());
        }

        Err(FerronError::ServiceNotFound(format!(
            "{}:{}",
            service_name, instance_id
        )))
    }

    /// Get all instances for a service
    pub async fn get_instances(&self, service_name: &str) -> Vec<ServiceInstance> {
        let services = self.services.read().await;
        services
            .get(service_name)
            .map(|instances| instances.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Get healthy instances for a service
    pub async fn get_healthy_instances(&self, service_name: &str) -> Vec<ServiceInstance> {
        self.get_instances(service_name)
            .await
            .into_iter()
            .filter(|i| i.healthy)
            .collect()
    }

    /// Get instance URLs for a service
    pub async fn get_urls(&self, service_name: &str) -> Vec<String> {
        self.get_instances(service_name)
            .await
            .into_iter()
            .map(|i| i.url)
            .collect()
    }

    /// Get healthy instance URLs for a service
    pub async fn get_healthy_urls(&self, service_name: &str) -> Vec<String> {
        self.get_healthy_instances(service_name)
            .await
            .into_iter()
            .map(|i| i.url)
            .collect()
    }

    /// Get all service names
    pub async fn get_services(&self) -> Vec<String> {
        let services = self.services.read().await;
        services.keys().cloned().collect()
    }

    /// Mark an instance as unhealthy
    pub async fn mark_unhealthy(&self, service_name: &str, instance_id: &str) -> Result<()> {
        let mut services = self.services.write().await;

        if let Some(instances) = services.get_mut(service_name)
            && let Some(instance) = instances.get_mut(instance_id)
        {
            instance.healthy = false;
            warn!(
                "Marked instance {}:{} as unhealthy",
                service_name, instance_id
            );
            return Ok(());
        }

        Err(FerronError::ServiceNotFound(format!(
            "{}:{}",
            service_name, instance_id
        )))
    }

    /// Mark an instance as healthy
    pub async fn mark_healthy(&self, service_name: &str, instance_id: &str) -> Result<()> {
        let mut services = self.services.write().await;

        if let Some(instances) = services.get_mut(service_name)
            && let Some(instance) = instances.get_mut(instance_id)
        {
            instance.healthy = true;
            info!(
                "Marked instance {}:{} as healthy",
                service_name, instance_id
            );
            return Ok(());
        }

        Err(FerronError::ServiceNotFound(format!(
            "{}:{}",
            service_name, instance_id
        )))
    }

    /// Remove stale instances (no heartbeat for given duration)
    pub async fn remove_stale(&self, max_age: chrono::Duration) -> Vec<(String, String)> {
        let mut removed = Vec::new();
        let mut services = self.services.write().await;

        for (service_name, instances) in services.iter_mut() {
            let stale_ids: Vec<String> = instances
                .iter()
                .filter(|(_, i)| i.is_stale(max_age))
                .map(|(id, _)| id.clone())
                .collect();

            for id in stale_ids {
                instances.remove(&id);
                warn!(
                    "Removed stale instance {} from service {}",
                    id, service_name
                );
                removed.push((service_name.clone(), id));
            }
        }

        // Remove empty services
        services.retain(|_, instances| !instances.is_empty());

        removed
    }

    /// Register a change listener
    pub async fn on_change<F>(&self, callback: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let mut listeners = self.listeners.write().await;
        listeners.push(Box::new(callback));
    }

    /// Notify all listeners of a change
    async fn notify_listeners(&self, service_name: &str) {
        let listeners = self.listeners.read().await;
        for listener in listeners.iter() {
            listener(service_name);
        }
    }

    /// Start background cleanup of stale instances
    pub fn start_cleanup(
        self: Arc<Self>,
        check_interval: std::time::Duration,
        max_age: chrono::Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(check_interval);
            loop {
                interval.tick().await;
                let removed = self.remove_stale(max_age).await;
                if !removed.is_empty() {
                    info!("Removed {} stale instances", removed.len());
                }
            }
        })
    }

    /// Get registry statistics
    pub async fn stats(&self) -> RegistryStats {
        let services = self.services.read().await;
        let mut total_instances = 0;
        let mut healthy_instances = 0;

        for instances in services.values() {
            total_instances += instances.len();
            healthy_instances += instances.values().filter(|i| i.healthy).count();
        }

        RegistryStats {
            service_count: services.len(),
            total_instances,
            healthy_instances,
            unhealthy_instances: total_instances - healthy_instances,
        }
    }
}

/// Registry statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStats {
    /// Number of registered services
    pub service_count: usize,
    /// Total number of instances
    pub total_instances: usize,
    /// Number of healthy instances
    pub healthy_instances: usize,
    /// Number of unhealthy instances
    pub unhealthy_instances: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_get() {
        let registry = ServiceRegistry::new();

        let id = registry
            .register("api", "http://localhost:3000")
            .await
            .unwrap();
        assert!(!id.is_empty());

        let instances = registry.get_instances("api").await;
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].url, "http://localhost:3000");
    }

    #[tokio::test]
    async fn test_deregister() {
        let registry = ServiceRegistry::new();

        let id = registry
            .register("api", "http://localhost:3000")
            .await
            .unwrap();
        assert_eq!(registry.get_instances("api").await.len(), 1);

        registry.deregister("api", &id).await.unwrap();
        assert_eq!(registry.get_instances("api").await.len(), 0);
    }

    #[tokio::test]
    async fn test_health_marking() {
        let registry = ServiceRegistry::new();

        let id = registry
            .register("api", "http://localhost:3000")
            .await
            .unwrap();

        registry.mark_unhealthy("api", &id).await.unwrap();
        assert_eq!(registry.get_healthy_instances("api").await.len(), 0);

        registry.mark_healthy("api", &id).await.unwrap();
        assert_eq!(registry.get_healthy_instances("api").await.len(), 1);
    }

    #[tokio::test]
    async fn test_multiple_instances() {
        let registry = ServiceRegistry::new();

        registry
            .register("api", "http://localhost:3001")
            .await
            .unwrap();
        registry
            .register("api", "http://localhost:3002")
            .await
            .unwrap();
        registry
            .register("api", "http://localhost:3003")
            .await
            .unwrap();

        assert_eq!(registry.get_instances("api").await.len(), 3);

        let urls = registry.get_urls("api").await;
        assert!(urls.contains(&"http://localhost:3001".to_string()));
        assert!(urls.contains(&"http://localhost:3002".to_string()));
        assert!(urls.contains(&"http://localhost:3003".to_string()));
    }

    #[tokio::test]
    async fn test_stats() {
        let registry = ServiceRegistry::new();

        let id1 = registry
            .register("api", "http://localhost:3001")
            .await
            .unwrap();
        registry
            .register("api", "http://localhost:3002")
            .await
            .unwrap();
        registry
            .register("web", "http://localhost:8080")
            .await
            .unwrap();

        registry.mark_unhealthy("api", &id1).await.unwrap();

        let stats = registry.stats().await;
        assert_eq!(stats.service_count, 2);
        assert_eq!(stats.total_instances, 3);
        assert_eq!(stats.healthy_instances, 2);
        assert_eq!(stats.unhealthy_instances, 1);
    }

    #[test]
    fn test_service_instance_builder() {
        let instance = ServiceInstance::new("api", "http://localhost:3000")
            .weight(3)
            .metadata("version", "1.0.0")
            .tag("production");

        assert_eq!(instance.service_name, "api");
        assert_eq!(instance.url, "http://localhost:3000");
        assert_eq!(instance.weight, 3);
        assert_eq!(instance.metadata.get("version"), Some(&"1.0.0".to_string()));
        assert!(instance.tags.contains(&"production".to_string()));
    }
}
