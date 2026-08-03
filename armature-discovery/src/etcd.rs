//! etcd service discovery implementation

use crate::service::{DiscoveryError, ServiceDiscovery, ServiceInstance};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use serde_json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Timeout applied to every etcd HTTP request so a stalled/unreachable
/// etcd endpoint fails fast instead of hanging the caller indefinitely.
const ETCD_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Default time-to-live of the etcd lease attached to a registration. If the
/// registering process dies its keep-alive task dies with it, so etcd expires
/// the lease after this long and drops both registration keys — that is what
/// stops a crashed instance from staying discoverable forever.
const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(30);

/// Lower bound on the keep-alive refresh interval, so a very short TTL cannot
/// turn into a hot loop against etcd.
const MIN_KEEP_ALIVE_INTERVAL: Duration = Duration::from_millis(100);

/// A live registration owned by this client: the lease its keys hang off and
/// the task refreshing that lease.
struct Registration {
    lease_id: String,
    keep_alive: JoinHandle<()>,
}

/// etcd service discovery client.
///
/// # Liveness
///
/// Registrations are written under an etcd lease with a TTL (see
/// [`EtcdDiscovery::with_lease_ttl`], default 30s) and refreshed by a
/// background keep-alive task spawned by [`register`](ServiceDiscovery::register).
/// If the process holding the registration dies, the keep-alive stops, the
/// lease expires, and etcd removes the registration automatically — so a
/// crashed instance stops being discoverable within roughly one TTL.
///
/// The lease covers *liveness*, not *health*: this crate performs no
/// background health checking, and [`discover`](ServiceDiscovery::discover)
/// returns every registered instance whose lease is still alive, healthy or
/// not. Use [`health_check`](ServiceDiscovery::health_check) explicitly if you
/// need a probe.
///
/// Both registration keys are written in a single etcd transaction, so a
/// failed write can never leave an instance visible to `discover` but absent
/// from `get_service`.
///
/// The keep-alive tasks are aborted when the client is dropped; the leases
/// then expire on their own, which is the intended behavior for a process
/// that is shutting down.
pub struct EtcdDiscovery {
    base_url: String,
    prefix: String,
    client: reqwest::Client,
    lease_ttl: Duration,
    registrations: Mutex<HashMap<String, Registration>>,
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
            lease_ttl: DEFAULT_LEASE_TTL,
            registrations: Mutex::new(HashMap::new()),
        })
    }

    /// Set the TTL of the lease attached to future registrations.
    ///
    /// A shorter TTL evicts crashed instances sooner at the cost of more
    /// frequent keep-alive traffic (the keep-alive fires roughly every
    /// `ttl / 3`). etcd lease TTLs are whole seconds, so anything under one
    /// second is rounded up to one second.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_discovery::EtcdDiscovery;
    /// use std::time::Duration;
    ///
    /// let etcd = EtcdDiscovery::new("http://localhost:2379", "/services")?
    ///     .with_lease_ttl(Duration::from_secs(10));
    /// ```
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.lease_ttl = ttl.max(Duration::from_secs(1));
        self
    }

    /// TTL sent to etcd, in whole seconds (etcd's lease granularity).
    fn lease_ttl_secs(&self) -> u64 {
        self.lease_ttl.as_secs().max(1)
    }

    /// How often the keep-alive task refreshes the lease. A third of the TTL
    /// leaves room for two lost refreshes before etcd expires the lease.
    fn keep_alive_interval(&self) -> Duration {
        (self.lease_ttl / 3).max(MIN_KEEP_ALIVE_INTERVAL)
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

    /// Apply a list of operations as a single unconditional etcd transaction.
    /// Every operation commits or none does, which is what keeps the two
    /// registration keys from drifting apart.
    async fn txn(&self, operations: Vec<serde_json::Value>) -> Result<(), DiscoveryError> {
        let url = format!("{}/v3/kv/txn", self.base_url);
        let payload = serde_json::json!({ "success": operations });

        let response = self.client.post(&url).json(&payload).send().await?;
        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::InvalidConfiguration(format!(
                "etcd transaction failed: {error}"
            )));
        }

        Ok(())
    }

    /// Grant a lease with the configured TTL, returning its id.
    async fn grant_lease(&self) -> Result<String, DiscoveryError> {
        let url = format!("{}/v3/lease/grant", self.base_url);
        let payload = serde_json::json!({ "TTL": self.lease_ttl_secs() });

        let response = self.client.post(&url).json(&payload).send().await?;
        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::RegistrationFailed(format!(
                "etcd lease grant failed: {error}"
            )));
        }

        let body: serde_json::Value = response.json().await?;
        // etcd's JSON gateway renders int64 fields as strings, but older
        // gateways (and hand-rolled stubs) emit a bare number — accept both.
        let lease_id = match &body["ID"] {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        };

        lease_id.filter(|id| id.as_str() != "0").ok_or_else(|| {
            DiscoveryError::RegistrationFailed(format!(
                "etcd lease grant returned no usable lease id: {body}"
            ))
        })
    }

    /// Revoke a lease, which atomically removes every key attached to it.
    async fn revoke_lease(&self, lease_id: &str) -> Result<(), DiscoveryError> {
        let url = format!("{}/v3/lease/revoke", self.base_url);
        let payload = serde_json::json!({ "ID": lease_id });

        let response = self.client.post(&url).json(&payload).send().await?;
        if !response.status().is_success() {
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DiscoveryError::DeregistrationFailed(format!(
                "etcd lease revoke failed: {error}"
            )));
        }

        Ok(())
    }

    /// Spawn the task that refreshes `lease_id` for as long as the
    /// registration is alive. Dropping/aborting the task stops the refreshes,
    /// so etcd expires the lease and evicts the registration.
    fn spawn_keep_alive(&self, service_id: String, lease_id: String) -> JoinHandle<()> {
        let client = self.client.clone();
        let url = format!("{}/v3/lease/keepalive", self.base_url);
        let interval = self.keep_alive_interval();
        let payload = serde_json::json!({ "ID": lease_id });

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match client.post(&url).json(&payload).send().await {
                    Ok(response) if response.status().is_success() => {
                        debug!("Refreshed etcd lease for service {}", service_id);
                    }
                    Ok(response) => {
                        warn!(
                            "etcd lease keep-alive for service {} returned status {}",
                            service_id,
                            response.status()
                        );
                    }
                    Err(e) => {
                        warn!(
                            "etcd lease keep-alive for service {} failed: {}",
                            service_id, e
                        );
                    }
                }
            }
        })
    }

    /// Remove and abort the keep-alive for `service_id`, returning the lease
    /// it was refreshing (if this client owns the registration).
    fn take_registration(&self, service_id: &str) -> Option<String> {
        let registration = self
            .registrations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(service_id)?;
        registration.keep_alive.abort();
        Some(registration.lease_id)
    }
}

impl Drop for EtcdDiscovery {
    fn drop(&mut self) {
        let registrations = self
            .registrations
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for registration in registrations.values() {
            registration.keep_alive.abort();
        }
    }
}

/// A `RequestPut` transaction operation attaching `key` to `lease_id`.
fn put_op(key: &str, value: &str, lease_id: &str) -> serde_json::Value {
    serde_json::json!({
        "requestPut": {
            "key": general_purpose::STANDARD.encode(key.as_bytes()),
            "value": general_purpose::STANDARD.encode(value.as_bytes()),
            "lease": lease_id,
        }
    })
}

/// A `RequestDeleteRange` transaction operation covering a single key.
fn delete_op(key: &str) -> serde_json::Value {
    serde_json::json!({
        "requestDeleteRange": {
            "key": general_purpose::STANDARD.encode(key.as_bytes()),
        }
    })
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

        // The lease is what bounds the registration's lifetime: if this
        // process dies, nothing refreshes it and etcd drops both keys.
        let lease_id = self.grant_lease().await?;

        // Both keys go in one transaction. The secondary index
        // ({prefix}__idx/{id} -> serialized instance) lets
        // get_service/deregister do a point lookup instead of scanning every
        // service under {prefix}/, but only if it can never disagree with the
        // composite key about whether the instance exists.
        let write = self
            .txn(vec![
                put_op(&composite_key, &value, &lease_id),
                put_op(&idx_key, &value, &lease_id),
            ])
            .await;

        if let Err(e) = write {
            // Don't leak the lease we just granted for keys that were never
            // written; the TTL would clean it up eventually, but not now.
            let _ = self.revoke_lease(&lease_id).await;
            return Err(DiscoveryError::RegistrationFailed(e.to_string()));
        }

        let keep_alive = self.spawn_keep_alive(service.id.clone(), lease_id.clone());
        let previous = self
            .registrations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                service.id.clone(),
                Registration {
                    lease_id,
                    keep_alive,
                },
            );
        // Re-registering the same id supersedes the old lease; stop refreshing
        // it so it expires instead of keeping a stale value alive.
        if let Some(previous) = previous {
            previous.keep_alive.abort();
        }

        info!(
            "Registered service {} with etcd under a {}s lease",
            service.id,
            self.lease_ttl_secs()
        );
        Ok(())
    }

    async fn deregister(&self, service_id: &str) -> Result<(), DiscoveryError> {
        // If this client registered the service, revoking its lease removes
        // every key attached to it in one etcd operation — no lookup needed,
        // and no window where one key outlives the other.
        if let Some(lease_id) = self.take_registration(service_id) {
            self.revoke_lease(&lease_id).await?;
            info!("Deregistered service {} from etcd", service_id);
            return Ok(());
        }

        // Otherwise the registration belongs to another process (or predates
        // leases). Point-lookup the secondary index to learn the instance (and
        // therefore its composite key) with a single get, instead of a
        // full-prefix scan over every registered service, then delete both
        // keys in one transaction.
        let idx_key = self.idx_key(service_id);
        let instance = self
            .get_key(&idx_key)
            .await?
            .ok_or_else(|| DiscoveryError::ServiceNotFound(service_id.to_string()))?;
        let composite_key = self.composite_key(&instance.name, &instance.id);

        self.txn(vec![delete_op(&composite_key), delete_op(&idx_key)])
            .await
            .map_err(|e| DiscoveryError::DeregistrationFailed(e.to_string()))?;

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

    /// A `LeaseGrant` response as etcd's JSON gateway renders it (int64 as a
    /// string).
    const LEASE_JSON: &str = r#"{"ID":"7587822721650585186","TTL":"30"}"#;

    fn decode_b64(encoded: &str) -> String {
        String::from_utf8(general_purpose::STANDARD.decode(encoded).unwrap()).unwrap()
    }

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
            .route(
                "POST",
                "/v3/lease/grant",
                StubResponse::json(200, LEASE_JSON),
            )
            .route("POST", "/v3/kv/txn", StubResponse::json(200, "{}"))
            .route("POST", "/v3/kv/range", StubResponse::json(200, kv_json))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();

        etcd.register(&instance).await.unwrap();

        // register() must write the composite key, not just the bare id.
        let txn_req = server.assert_received("POST", "/v3/kv/txn");
        let txn_body: serde_json::Value = serde_json::from_str(&txn_req.body_string()).unwrap();
        let put_key = decode_b64(
            txn_body["success"][0]["requestPut"]["key"]
                .as_str()
                .unwrap(),
        );
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
            .route("POST", "/v3/kv/txn", StubResponse::json(200, "{}"))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();

        let fetched = etcd.get_service("svc-1").await.unwrap();
        assert_eq!(fetched.id, "svc-1");

        etcd.deregister("svc-1").await.unwrap();

        // Both the composite key and the idx key must be deleted, and in a
        // single transaction so neither can outlive the other.
        let txn_requests: Vec<_> = server
            .requests()
            .into_iter()
            .filter(|r| r.method == "POST" && r.path == "/v3/kv/txn")
            .collect();
        assert_eq!(
            txn_requests.len(),
            1,
            "deregister must delete both keys in one transaction"
        );
        let body: serde_json::Value = serde_json::from_str(&txn_requests[0].body_string()).unwrap();
        let deleted: Vec<String> = body["success"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| decode_b64(op["requestDeleteRange"]["key"].as_str().unwrap()))
            .collect();
        assert_eq!(
            deleted,
            vec![
                key_builder.composite_key("api", "svc-1"),
                key_builder.idx_key("svc-1"),
            ]
        );
    }

    #[tokio::test]
    async fn register_attaches_a_lease_to_both_keys_in_one_transaction() {
        let instance = ServiceInstance::new("svc-1", "api", "localhost", 8080);

        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/lease/grant",
                StubResponse::json(200, LEASE_JSON),
            )
            .route("POST", "/v3/kv/txn", StubResponse::json(200, "{}"))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        etcd.register(&instance).await.unwrap();

        // The TTL must actually be requested, otherwise a crashed instance
        // would stay discoverable forever.
        let grant = server.assert_received("POST", "/v3/lease/grant");
        let grant_body: serde_json::Value = serde_json::from_str(&grant.body_string()).unwrap();
        assert_eq!(grant_body["TTL"].as_u64(), Some(30));

        let txn = server.assert_received("POST", "/v3/kv/txn");
        let txn_body: serde_json::Value = serde_json::from_str(&txn.body_string()).unwrap();
        let ops = txn_body["success"].as_array().unwrap();
        assert_eq!(ops.len(), 2, "both keys must be written in one transaction");
        for op in ops {
            assert_eq!(
                op["requestPut"]["lease"].as_str(),
                Some("7587822721650585186"),
                "every registration key must hang off the granted lease"
            );
        }

        // Only one round trip wrote keys: no second, non-atomic PUT.
        assert!(
            !server
                .requests()
                .iter()
                .any(|r| r.path == "/v3/kv/put" || r.path == "/v3/kv/deleterange")
        );
    }

    #[tokio::test]
    async fn register_spawns_a_keep_alive_that_refreshes_the_lease() {
        let instance = ServiceInstance::new("svc-1", "api", "localhost", 8080);

        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/lease/grant",
                StubResponse::json(200, LEASE_JSON),
            )
            .route("POST", "/v3/kv/txn", StubResponse::json(200, "{}"))
            .route("POST", "/v3/lease/keepalive", StubResponse::json(200, "{}"))
            .start()
            .await;

        // 1s TTL -> ~333ms refresh interval, so a short sleep is enough.
        let etcd = EtcdDiscovery::new(server.url(), "/services")
            .unwrap()
            .with_lease_ttl(Duration::from_secs(1));
        etcd.register(&instance).await.unwrap();

        tokio::time::sleep(Duration::from_millis(800)).await;

        let keepalive = server.assert_received("POST", "/v3/lease/keepalive");
        let body: serde_json::Value = serde_json::from_str(&keepalive.body_string()).unwrap();
        assert_eq!(body["ID"].as_str(), Some("7587822721650585186"));
    }

    #[tokio::test]
    async fn deregister_revokes_the_lease_it_registered_with() {
        let instance = ServiceInstance::new("svc-1", "api", "localhost", 8080);

        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/lease/grant",
                StubResponse::json(200, LEASE_JSON),
            )
            .route("POST", "/v3/kv/txn", StubResponse::json(200, "{}"))
            .route("POST", "/v3/lease/revoke", StubResponse::json(200, "{}"))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        etcd.register(&instance).await.unwrap();
        etcd.deregister("svc-1").await.unwrap();

        let revoke = server.assert_received("POST", "/v3/lease/revoke");
        let body: serde_json::Value = serde_json::from_str(&revoke.body_string()).unwrap();
        assert_eq!(body["ID"].as_str(), Some("7587822721650585186"));

        // Revoking the lease drops both keys, so no key lookup or delete is
        // needed at all.
        assert!(
            !server
                .requests()
                .iter()
                .any(|r| r.path == "/v3/kv/range" || r.path == "/v3/kv/deleterange")
        );
    }

    #[tokio::test]
    async fn register_revokes_the_lease_when_the_write_transaction_fails() {
        let instance = ServiceInstance::new("svc-1", "api", "localhost", 8080);

        let server = StubServer::builder()
            .route(
                "POST",
                "/v3/lease/grant",
                StubResponse::json(200, LEASE_JSON),
            )
            .route("POST", "/v3/kv/txn", StubResponse::json(500, r#"{"e":1}"#))
            .route("POST", "/v3/lease/revoke", StubResponse::json(200, "{}"))
            .start()
            .await;

        let etcd = EtcdDiscovery::new(server.url(), "/services").unwrap();
        let result = etcd.register(&instance).await;

        assert!(matches!(result, Err(DiscoveryError::RegistrationFailed(_))));
        server.assert_received("POST", "/v3/lease/revoke");
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
