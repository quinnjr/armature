//! Consul service discovery implementation

use crate::service::{DiscoveryError, ServiceDiscovery, ServiceInstance};
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info};
use url::Url;

/// Timeout applied to every Consul HTTP request so a stalled/unreachable
/// Consul agent fails fast instead of hanging the caller indefinitely.
const CONSUL_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

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
        let client = reqwest::Client::builder()
            .timeout(CONSUL_REQUEST_TIMEOUT)
            .build()
            .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;

        Ok(Self {
            base_url: base_url.into(),
            client,
        })
    }

    /// Build `{base_url}/{segments...}`, percent-encoding each segment so
    /// caller-controlled values (service names/ids) can't inject extra
    /// path segments, query strings, or fragments into the request.
    fn build_url(&self, segments: &[&str]) -> Result<Url, DiscoveryError> {
        let mut url = Url::parse(&self.base_url)
            .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;
        {
            let mut path_segments = url.path_segments_mut().map_err(|_| {
                DiscoveryError::InvalidConfiguration(
                    "Consul base_url cannot be a base URL".to_string(),
                )
            })?;
            for segment in segments {
                path_segments.push(segment);
            }
        }
        Ok(url)
    }
}

#[async_trait]
impl ServiceDiscovery for ConsulDiscovery {
    async fn register(&self, service: &ServiceInstance) -> Result<(), DiscoveryError> {
        let url = self.build_url(&["v1", "agent", "service", "register"])?;

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

        let response = self.client.put(url).json(&payload).send().await?;

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
        let url = self.build_url(&["v1", "agent", "service", "deregister", service_id])?;

        let response = self.client.put(url).send().await?;

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
        // unhealthy/critical instance. `service_name` is percent-encoded
        // as a path segment so a name containing `?`, `#`, or `/` can't
        // rewrite the request's path or query string.
        let mut url = self.build_url(&["v1", "health", "service", service_name])?;
        url.query_pairs_mut().append_pair("passing", "true");

        let response = self.client.get(url).send().await?;

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
        let url = self.build_url(&["v1", "agent", "service", service_id])?;

        let response = self.client.get(url).send().await?;

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
        let url = self.build_url(&["v1", "catalog", "services"])?;

        let response = self.client.get(url).send().await?;

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

    #[tokio::test]
    async fn discover_returns_ok_empty_for_unregistered_service_name() {
        let server = StubServer::builder()
            .route(
                "GET",
                "/v1/health/service/nonexistent",
                StubResponse::json(200, "[]"),
            )
            .start()
            .await;

        let consul = ConsulDiscovery::new(server.url()).unwrap();
        let instances = consul.discover("nonexistent").await.unwrap();
        assert!(
            instances.is_empty(),
            "discover() of an unregistered name must return Ok(vec![]), matching the other backends"
        );
    }

    #[tokio::test]
    async fn discover_percent_encodes_service_name_path_segment() {
        let server = StubServer::builder()
            .default_response(StubResponse::json(200, "[]"))
            .start()
            .await;

        let consul = ConsulDiscovery::new(server.url()).unwrap();
        // A crafted name that would otherwise inject a path segment / query
        // string must be percent-encoded, not interpreted as URL syntax.
        consul.discover("api/../secret?x=1").await.unwrap();

        let requests = server.requests();
        let req = requests
            .iter()
            .find(|r| r.method == "GET")
            .expect("expected a GET request");
        assert!(
            req.path.contains("%2F") || !req.path.contains("../secret"),
            "service_name must be percent-encoded, not left as raw path syntax: {}",
            req.path
        );
    }
}
