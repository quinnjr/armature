//! Consul service discovery implementation

use crate::service::{DiscoveryError, ServiceDiscovery, ServiceInstance};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{debug, info};

/// Consul service discovery client
pub struct ConsulDiscovery {
    base_url: String,
    client: reqwest::Client,
}

impl ConsulDiscovery {
    /// Create new Consul discovery client
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_discovery::ConsulDiscovery;
    ///
    /// let consul = ConsulDiscovery::new("http://localhost:8500")?;
    /// ```
    pub fn new(base_url: impl Into<String>) -> Result<Self, DiscoveryError> {
        Ok(Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl ServiceDiscovery for ConsulDiscovery {
    async fn register(&self, service: &ServiceInstance) -> Result<(), DiscoveryError> {
        let url = format!("{}/v1/agent/service/register", self.base_url);

        // Build Consul registration payload
        let mut payload = serde_json::json!({
            "ID": service.id,
            "Name": service.name,
            "Address": service.address,
            "Port": service.port,
            "Tags": service.tags,
            "Meta": service.metadata,
        });

        // Add health check if configured
        if let Some(health_url) = &service.health_check_url {
            payload["Check"] = serde_json::json!({
                "HTTP": health_url,
                "Interval": "10s",
                "Timeout": "5s",
            });
        }

        let response = self.client.put(&url).json(&payload).send().await?;

        if response.status().is_success() {
            info!("Registered service {} with Consul", service.id);
            Ok(())
        } else {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(DiscoveryError::RegistrationFailed(error))
        }
    }

    async fn deregister(&self, service_id: &str) -> Result<(), DiscoveryError> {
        let url = format!(
            "{}/v1/agent/service/deregister/{}",
            self.base_url, service_id
        );

        let response = self.client.put(&url).send().await?;

        if response.status().is_success() {
            info!("Deregistered service {} from Consul", service_id);
            Ok(())
        } else {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            Err(DiscoveryError::DeregistrationFailed(error))
        }
    }

    async fn discover(&self, service_name: &str) -> Result<Vec<ServiceInstance>, DiscoveryError> {
        // `passing=true` asks Consul to filter out instances whose health
        // checks are not all passing, so callers never get routed to an
        // unhealthy/critical instance.
        let url = format!(
            "{}/v1/health/service/{}?passing=true",
            self.base_url, service_name
        );

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(DiscoveryError::ServiceNotFound(service_name.to_string()));
        }

        #[derive(Deserialize)]
        struct ConsulService {
            #[serde(rename = "Service")]
            service: ConsulServiceDetail,
        }

        #[derive(Deserialize)]
        struct ConsulServiceDetail {
            #[serde(rename = "ID")]
            id: String,
            #[serde(rename = "Service")]
            service: String,
            #[serde(rename = "Address")]
            address: String,
            #[serde(rename = "Port")]
            port: u16,
            #[serde(rename = "Tags")]
            tags: Vec<String>,
            #[serde(rename = "Meta")]
            meta: Option<HashMap<String, String>>,
        }

        let consul_services: Vec<ConsulService> = response.json().await?;

        let instances: Vec<ServiceInstance> = consul_services
            .into_iter()
            .map(|cs| {
                let mut instance = ServiceInstance::new(
                    cs.service.id,
                    cs.service.service,
                    cs.service.address,
                    cs.service.port,
                );
                instance.tags = cs.service.tags;
                instance.metadata = cs.service.meta.unwrap_or_default();
                instance
            })
            .collect();

        debug!(
            "Discovered {} instances of service {}",
            instances.len(),
            service_name
        );
        Ok(instances)
    }

    async fn get_service(&self, service_id: &str) -> Result<ServiceInstance, DiscoveryError> {
        let url = format!("{}/v1/agent/service/{}", self.base_url, service_id);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(DiscoveryError::ServiceNotFound(service_id.to_string()));
        }

        #[derive(Deserialize)]
        struct ConsulServiceDetail {
            #[serde(rename = "ID")]
            id: String,
            #[serde(rename = "Service")]
            service: String,
            #[serde(rename = "Address")]
            address: String,
            #[serde(rename = "Port")]
            port: u16,
            #[serde(rename = "Tags")]
            tags: Vec<String>,
            #[serde(rename = "Meta")]
            meta: Option<HashMap<String, String>>,
        }

        let service: ConsulServiceDetail = response.json().await?;

        let mut instance =
            ServiceInstance::new(service.id, service.service, service.address, service.port);
        instance.tags = service.tags;
        instance.metadata = service.meta.unwrap_or_default();

        Ok(instance)
    }

    async fn list_services(&self) -> Result<Vec<String>, DiscoveryError> {
        let url = format!("{}/v1/catalog/services", self.base_url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(DiscoveryError::RegistrationFailed(
                "Failed to list services".to_string(),
            ));
        }

        let services: HashMap<String, Vec<String>> = response.json().await?;
        Ok(services.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_testkit::http_stub::{StubResponse, StubServer};

    #[test]
    fn test_consul_discovery_creation() {
        let consul = ConsulDiscovery::new("http://localhost:8500");
        assert!(consul.is_ok());
    }

    #[tokio::test]
    async fn discover_requests_passing_true_so_unhealthy_instances_are_excluded() {
        let body = serde_json::json!([{
            "Service": {
                "ID": "svc-1",
                "Service": "api",
                "Address": "localhost",
                "Port": 8080,
                "Tags": [],
                "Meta": null,
            }
        }])
        .to_string();

        let server = StubServer::builder()
            .route(
                "GET",
                "/v1/health/service/api",
                StubResponse::json(200, body),
            )
            .start()
            .await;

        let consul = ConsulDiscovery::new(server.url()).unwrap();
        let instances = consul.discover("api").await.unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].id, "svc-1");

        let req = server.assert_received("GET", "/v1/health/service/api");
        assert_eq!(
            req.query.as_deref(),
            Some("passing=true"),
            "discover() must ask Consul to filter to passing (healthy) instances only"
        );
    }
}
