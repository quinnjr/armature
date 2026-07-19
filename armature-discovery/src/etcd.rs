//! etcd service discovery implementation

use crate::service::{DiscoveryError, ServiceDiscovery, ServiceInstance};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use serde_json;
use tracing::{debug, info};

/// etcd service discovery client
pub struct EtcdDiscovery {
    base_url: String,
    prefix: String,
    client: reqwest::Client,
}

impl EtcdDiscovery {
    /// Create new etcd discovery client
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_discovery::EtcdDiscovery;
    ///
    /// let etcd = EtcdDiscovery::new("http://localhost:2379", "/services")?;
    /// ```
    pub fn new(
        base_url: impl Into<String>,
        prefix: impl Into<String>,
    ) -> Result<Self, DiscoveryError> {
        Ok(Self {
            base_url: base_url.into(),
            prefix: prefix.into(),
            client: reqwest::Client::new(),
        })
    }

    /// Prefix under which all instances of `service_name` are stored:
    /// `{prefix}/{service_name}/`.
    fn service_name_prefix(&self, service_name: &str) -> String {
        format!("{}/{}/", self.prefix, service_name)
    }

    /// Composite key a service instance is stored under:
    /// `{prefix}/{service_name}/{service_id}`. This makes `discover`'s
    /// range scan over `service_name_prefix` find instances written by
    /// `register`.
    fn composite_key(&self, service_name: &str, service_id: &str) -> String {
        format!("{}{}", self.service_name_prefix(service_name), service_id)
    }

    /// Prefix covering every service registered under this client: `{prefix}/`.
    fn all_services_prefix(&self) -> String {
        format!("{}/", self.prefix)
    }

    /// Range-scan etcd for every key/value pair under `prefix`, decoding
    /// each value as a `ServiceInstance`. Returns `(raw_key, instance)` pairs.
    async fn scan_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<(String, ServiceInstance)>, DiscoveryError> {
        let url = format!("{}/v3/kv/range", self.base_url);
        let key_b64 = general_purpose::STANDARD.encode(prefix.as_bytes());
        let range_end_b64 = general_purpose::STANDARD.encode(format!("{}~", prefix).as_bytes());

        let payload = serde_json::json!({
            "key": key_b64,
            "range_end": range_end_b64,
        });

        let response = self.client.post(&url).json(&payload).send().await?;

        if !response.status().is_success() {
            return Err(DiscoveryError::InvalidConfiguration(format!(
                "etcd range scan over {} failed with status {}",
                prefix,
                response.status()
            )));
        }

        #[derive(serde::Deserialize)]
        struct EtcdResponse {
            kvs: Option<Vec<EtcdKV>>,
        }

        #[derive(serde::Deserialize)]
        struct EtcdKV {
            key: String,
            value: String,
        }

        let etcd_response: EtcdResponse = response.json().await?;

        etcd_response
            .kvs
            .unwrap_or_default()
            .into_iter()
            .map(|kv| {
                let key_bytes = general_purpose::STANDARD
                    .decode(&kv.key)
                    .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;
                let key_str = String::from_utf8(key_bytes)
                    .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;

                let value_bytes = general_purpose::STANDARD
                    .decode(&kv.value)
                    .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;
                let value_str = String::from_utf8(value_bytes)
                    .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;
                let instance: ServiceInstance = serde_json::from_str(&value_str)
                    .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;

                Ok((key_str, instance))
            })
            .collect()
    }
}

#[async_trait]
impl ServiceDiscovery for EtcdDiscovery {
    async fn register(&self, service: &ServiceInstance) -> Result<(), DiscoveryError> {
        let url = format!("{}/v3/kv/put", self.base_url);
        let key = self.composite_key(&service.name, &service.id);
        let value = serde_json::to_string(service)
            .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;

        // Base64 encode key and value for etcd v3 API
        let key_b64 = general_purpose::STANDARD.encode(key.as_bytes());
        let value_b64 = general_purpose::STANDARD.encode(value.as_bytes());

        let payload = serde_json::json!({
            "key": key_b64,
            "value": value_b64,
        });

        let response = self.client.post(&url).json(&payload).send().await?;

        if response.status().is_success() {
            info!("Registered service {} with etcd", service.id);
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
        // The composite key is `{prefix}/{name}/{id}`, but deregister only
        // receives the id, so find the actual stored key via a full scan
        // rather than trying to reconstruct it without the name.
        let all = self.scan_prefix(&self.all_services_prefix()).await?;
        let (key, _) = all
            .into_iter()
            .find(|(_, instance)| instance.id == service_id)
            .ok_or_else(|| DiscoveryError::ServiceNotFound(service_id.to_string()))?;

        let url = format!("{}/v3/kv/deleterange", self.base_url);
        let key_b64 = general_purpose::STANDARD.encode(key.as_bytes());

        let payload = serde_json::json!({
            "key": key_b64,
        });

        let response = self.client.post(&url).json(&payload).send().await?;

        if response.status().is_success() {
            info!("Deregistered service {} from etcd", service_id);
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
        let prefix = self.service_name_prefix(service_name);
        let instances: Vec<ServiceInstance> = self
            .scan_prefix(&prefix)
            .await?
            .into_iter()
            .map(|(_, instance)| instance)
            .collect();

        if instances.is_empty() {
            Err(DiscoveryError::ServiceNotFound(service_name.to_string()))
        } else {
            debug!(
                "Discovered {} instances of service {}",
                instances.len(),
                service_name
            );
            Ok(instances)
        }
    }

    async fn get_service(&self, service_id: &str) -> Result<ServiceInstance, DiscoveryError> {
        // get_service only receives the id, so the composite key can't be
        // reconstructed directly; scan the whole prefix and match by id
        // (the same composite-keyed entries that register/discover use).
        let all = self.scan_prefix(&self.all_services_prefix()).await?;
        all.into_iter()
            .find(|(_, instance)| instance.id == service_id)
            .map(|(_, instance)| instance)
            .ok_or_else(|| DiscoveryError::ServiceNotFound(service_id.to_string()))
    }

    async fn list_services(&self) -> Result<Vec<String>, DiscoveryError> {
        let all = self.scan_prefix(&self.all_services_prefix()).await?;
        let mut names: Vec<String> = all.into_iter().map(|(_, instance)| instance.name).collect();
        names.sort();
        names.dedup();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_testkit::http_stub::{StubResponse, StubServer};

    #[test]
    fn test_etcd_discovery_creation() {
        let etcd = EtcdDiscovery::new("http://localhost:2379", "/services");
        assert!(etcd.is_ok());
    }

    #[test]
    fn composite_key_lives_under_the_prefix_discover_scans() {
        let etcd = EtcdDiscovery::new("http://localhost:2379", "/services").unwrap();
        let key = etcd.composite_key("api", "svc-1");
        let prefix = etcd.service_name_prefix("api");

        assert!(
            key.starts_with(&prefix),
            "register key {key} must live under the prefix discover range-scans ({prefix})"
        );
        assert_eq!(&key[prefix.len()..], "svc-1");
    }

    #[tokio::test]
    async fn register_then_discover_and_get_service_round_trip() {
        let instance = ServiceInstance::new("svc-1", "api", "localhost", 8080);
        let value_json = serde_json::to_string(&instance).unwrap();

        let key_builder = EtcdDiscovery::new("http://placeholder", "/services").unwrap();
        let composite_key = key_builder.composite_key("api", "svc-1");

        let kv_json = serde_json::json!({
            "kvs": [{
                "key": general_purpose::STANDARD.encode(composite_key.as_bytes()),
                "value": general_purpose::STANDARD.encode(value_json.as_bytes()),
            }]
        })
        .to_string();

        let server = StubServer::builder()
            .route("POST", "/v3/kv/put", StubResponse::json(200, "{}"))
            .route("POST", "/v3/kv/range", StubResponse::json(200, kv_json))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();

        etcd.register(&instance).await.unwrap();

        // register() must write the composite key, not just the bare id.
        let put_req = server.assert_received("POST", "/v3/kv/put");
        let put_body: serde_json::Value = serde_json::from_str(&put_req.body_string()).unwrap();
        let put_key_bytes = general_purpose::STANDARD
            .decode(put_body["key"].as_str().unwrap())
            .unwrap();
        let put_key = String::from_utf8(put_key_bytes).unwrap();
        assert_eq!(put_key, composite_key);
        assert!(put_key.starts_with(&etcd.service_name_prefix("api")));

        // discover() must find the instance registered above.
        let discovered = etcd.discover("api").await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id, "svc-1");
        assert_eq!(discovered[0].name, "api");

        // get_service() must also find it by id alone.
        let fetched = etcd.get_service("svc-1").await.unwrap();
        assert_eq!(fetched.name, "api");
        assert_eq!(fetched.address, "localhost");
        assert_eq!(fetched.port, 8080);
    }

    #[tokio::test]
    async fn list_services_returns_distinct_names_from_a_full_prefix_scan() {
        let api = ServiceInstance::new("a-1", "api", "localhost", 8080);
        let worker = ServiceInstance::new("w-1", "worker", "localhost", 9090);

        let kv_json = serde_json::json!({
            "kvs": [
                {
                    "key": general_purpose::STANDARD.encode(b"/services/api/a-1".as_slice()),
                    "value": general_purpose::STANDARD.encode(serde_json::to_string(&api).unwrap()),
                },
                {
                    "key": general_purpose::STANDARD.encode(b"/services/worker/w-1".as_slice()),
                    "value": general_purpose::STANDARD.encode(serde_json::to_string(&worker).unwrap()),
                },
            ]
        })
        .to_string();

        let server = StubServer::builder()
            .route("POST", "/v3/kv/range", StubResponse::json(200, kv_json))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        let mut names = etcd.list_services().await.unwrap();
        names.sort();

        assert_eq!(names, vec!["api".to_string(), "worker".to_string()]);
    }
}
