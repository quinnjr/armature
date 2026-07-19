//! etcd service discovery implementation

use crate::service::{DiscoveryError, ServiceDiscovery, ServiceInstance};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use serde_json;
use std::time::Duration;
use tracing::{debug, info};

/// Timeout applied to every etcd HTTP request so a stalled/unreachable
/// etcd endpoint fails fast instead of hanging the caller indefinitely.
const ETCD_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

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
        let client = reqwest::Client::builder()
            .timeout(ETCD_REQUEST_TIMEOUT)
            .build()
            .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;

        Ok(Self {
            base_url: base_url.into(),
            prefix: prefix.into(),
            client,
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

    /// Secondary index key mapping a bare service id straight to its
    /// serialized instance: `{prefix}__idx/{service_id}`. This lives
    /// outside the `{prefix}/` namespace scanned by `discover`/
    /// `list_services`, so a point `get`/`delete` on this key lets
    /// `get_service`/`deregister` avoid a full-prefix scan.
    fn idx_key(&self, service_id: &str) -> String {
        format!("{}__idx/{}", self.prefix, service_id)
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
        let range_end_b64 = general_purpose::STANDARD.encode(prefix_range_end(prefix.as_bytes()));

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

        let etcd_response: EtcdResponse = response.json().await?;

        etcd_response
            .kvs
            .unwrap_or_default()
            .into_iter()
            .map(decode_kv)
            .collect()
    }

    /// Point-get a single key from etcd, decoding its value as a
    /// `ServiceInstance`. Returns `Ok(None)` if the key doesn't exist.
    async fn get_key(&self, key: &str) -> Result<Option<ServiceInstance>, DiscoveryError> {
        let url = format!("{}/v3/kv/range", self.base_url);
        let key_b64 = general_purpose::STANDARD.encode(key.as_bytes());

        let payload = serde_json::json!({
            "key": key_b64,
        });

        let response = self.client.post(&url).json(&payload).send().await?;

        if !response.status().is_success() {
            return Err(DiscoveryError::InvalidConfiguration(format!(
                "etcd point get for {} failed with status {}",
                key,
                response.status()
            )));
        }

        let etcd_response: EtcdResponse = response.json().await?;

        match etcd_response.kvs.unwrap_or_default().into_iter().next() {
            Some(kv) => {
                let (_, instance) = decode_kv(kv)?;
                Ok(Some(instance))
            }
            None => Ok(None),
        }
    }

    /// PUT a single key/value pair into etcd.
    async fn put_key(&self, key: &str, value: &str) -> Result<reqwest::Response, DiscoveryError> {
        let url = format!("{}/v3/kv/put", self.base_url);
        let key_b64 = general_purpose::STANDARD.encode(key.as_bytes());
        let value_b64 = general_purpose::STANDARD.encode(value.as_bytes());

        let payload = serde_json::json!({
            "key": key_b64,
            "value": value_b64,
        });

        Ok(self.client.post(&url).json(&payload).send().await?)
    }

    /// Point-delete a single key from etcd.
    async fn delete_key(&self, key: &str) -> Result<reqwest::Response, DiscoveryError> {
        let url = format!("{}/v3/kv/deleterange", self.base_url);
        let key_b64 = general_purpose::STANDARD.encode(key.as_bytes());

        let payload = serde_json::json!({
            "key": key_b64,
        });

        Ok(self.client.post(&url).json(&payload).send().await?)
    }
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

fn decode_kv(kv: EtcdKV) -> Result<(String, ServiceInstance), DiscoveryError> {
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
}

/// Compute the canonical etcd prefix-successor for a range scan's
/// `range_end`: the smallest key that is NOT prefixed by `prefix`.
///
/// This is `prefix` with its last byte incremented (carrying into
/// preceding bytes on overflow). If every byte is `0xFF`, there is no
/// finite successor, so the scan is opened up to cover the rest of the
/// keyspace (an empty `range_end` means "no upper bound" in etcd v3).
///
/// The previous implementation appended a literal `~` (0x7E) to the
/// prefix, which silently excluded any key whose first differing byte
/// was `>= 0x7E` (e.g. ids containing `~`, `\x7f`, or high-bit bytes).
fn prefix_range_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    for i in (0..end.len()).rev() {
        if end[i] != 0xFF {
            end[i] += 1;
            end.truncate(i + 1);
            return end;
        }
    }
    // All bytes were 0xFF (or prefix was empty): no finite successor,
    // so scan to the end of the keyspace.
    Vec::new()
}

#[async_trait]
impl ServiceDiscovery for EtcdDiscovery {
    async fn register(&self, service: &ServiceInstance) -> Result<(), DiscoveryError> {
        let composite_key = self.composite_key(&service.name, &service.id);
        let idx_key = self.idx_key(&service.id);
        let value = serde_json::to_string(service)
            .map_err(|e| DiscoveryError::InvalidConfiguration(e.to_string()))?;

        let response = self.put_key(&composite_key, &value).await?;
        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::RegistrationFailed(error));
        }

        // Secondary index: {prefix}__idx/{id} -> serialized instance, so
        // get_service/deregister can do a single point lookup instead of
        // scanning every service under {prefix}/.
        let idx_response = self.put_key(&idx_key, &value).await?;
        if !idx_response.status().is_success() {
            let error = idx_response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::RegistrationFailed(error));
        }

        info!("Registered service {} with etcd", service.id);
        Ok(())
    }

    async fn deregister(&self, service_id: &str) -> Result<(), DiscoveryError> {
        // Point-lookup the secondary index to learn the instance (and
        // therefore its composite key) with a single get, instead of a
        // full-prefix scan over every registered service.
        let idx_key = self.idx_key(service_id);
        let instance = self
            .get_key(&idx_key)
            .await?
            .ok_or_else(|| DiscoveryError::ServiceNotFound(service_id.to_string()))?;
        let composite_key = self.composite_key(&instance.name, &instance.id);

        let response = self.delete_key(&composite_key).await?;
        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::DeregistrationFailed(error));
        }

        let idx_response = self.delete_key(&idx_key).await?;
        if !idx_response.status().is_success() {
            let error = idx_response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::DeregistrationFailed(error));
        }

        info!("Deregistered service {} from etcd", service_id);
        Ok(())
    }

    async fn discover(&self, service_name: &str) -> Result<Vec<ServiceInstance>, DiscoveryError> {
        let prefix = self.service_name_prefix(service_name);
        let instances: Vec<ServiceInstance> = self
            .scan_prefix(&prefix)
            .await?
            .into_iter()
            .map(|(_, instance)| instance)
            .collect();

        // An empty result is a normal (if uninteresting) discovery outcome,
        // not an error condition — it's the caller's (ServiceResolver's)
        // job to decide whether "no instances" should fail. Other backends
        // (InMemory, Consul) agree on this contract.
        debug!(
            "Discovered {} instances of service {}",
            instances.len(),
            service_name
        );
        Ok(instances)
    }

    async fn get_service(&self, service_id: &str) -> Result<ServiceInstance, DiscoveryError> {
        let idx_key = self.idx_key(service_id);
        self.get_key(&idx_key)
            .await?
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

    #[test]
    fn idx_key_does_not_live_under_the_all_services_prefix() {
        let etcd = EtcdDiscovery::new("http://localhost:2379", "/services").unwrap();
        let idx = etcd.idx_key("svc-1");
        let all = etcd.all_services_prefix();

        assert!(
            !idx.starts_with(&all),
            "idx key {idx} must not be visible to a scan over {all} (list_services/discover)"
        );
    }

    #[test]
    fn prefix_range_end_increments_the_last_byte_with_carry() {
        assert_eq!(prefix_range_end(b"/services/"), b"/services0".to_vec());
        // last byte 0xFF carries into the previous byte
        assert_eq!(prefix_range_end(&[0x01, 0xFF]), vec![0x02]);
        // all bytes 0xFF: no finite successor, scan to end of keyspace
        assert_eq!(prefix_range_end(&[0xFF, 0xFF]), Vec::<u8>::new());
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

        // get_service() must also find it by id alone, via the idx key.
        let fetched = etcd.get_service("svc-1").await.unwrap();
        assert_eq!(fetched.name, "api");
        assert_eq!(fetched.address, "localhost");
        assert_eq!(fetched.port, 8080);
    }

    #[tokio::test]
    async fn discover_returns_ok_empty_for_unregistered_service_name() {
        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/kv/range",
                StubResponse::json(200, serde_json::json!({ "kvs": [] }).to_string()),
            )
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        let instances = etcd.discover("nonexistent").await.unwrap();
        assert!(instances.is_empty());
    }

    #[tokio::test]
    async fn deregister_and_get_service_use_the_idx_key_not_a_full_scan() {
        let instance = ServiceInstance::new("svc-1", "api", "localhost", 8080);
        let value_json = serde_json::to_string(&instance).unwrap();

        let key_builder = EtcdDiscovery::new("http://placeholder", "/services").unwrap();
        let idx_key = key_builder.idx_key("svc-1");

        let kv_json = serde_json::json!({
            "kvs": [{
                "key": general_purpose::STANDARD.encode(idx_key.as_bytes()),
                "value": general_purpose::STANDARD.encode(value_json.as_bytes()),
            }]
        })
        .to_string();

        let server = StubServer::builder()
            .route("POST", "/v3/kv/range", StubResponse::json(200, kv_json))
            .route("POST", "/v3/kv/deleterange", StubResponse::json(200, "{}"))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();

        let fetched = etcd.get_service("svc-1").await.unwrap();
        assert_eq!(fetched.id, "svc-1");

        etcd.deregister("svc-1").await.unwrap();

        // Both the composite key and the idx key must be deleted.
        let delete_requests: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.method == "POST" && r.path == "/v3/kv/deleterange")
            .collect();
        assert_eq!(
            delete_requests.len(),
            2,
            "deregister must delete both the composite key and the idx key"
        );
    }

    #[tokio::test]
    async fn deregister_of_unknown_id_returns_service_not_found() {
        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/kv/range",
                StubResponse::json(200, serde_json::json!({ "kvs": [] }).to_string()),
            )
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        let result = etcd.deregister("missing").await;
        assert!(matches!(result, Err(DiscoveryError::ServiceNotFound(_))));
    }

    #[tokio::test]
    async fn range_scan_failure_returns_invalid_configuration() {
        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/kv/range",
                StubResponse::json(500, r#"{"error":"boom"}"#),
            )
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        let result = etcd.discover("api").await;
        assert!(matches!(
            result,
            Err(DiscoveryError::InvalidConfiguration(_))
        ));
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
