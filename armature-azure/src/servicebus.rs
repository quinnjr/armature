//! Azure Service Bus client.
//!
//! The [`azure_messaging_servicebus`] 0.21 SDK predates the `azure_core` 1.0
//! `TokenCredential` model: it authenticates with a Shared Access Signature
//! (SAS) policy name + key and only exposes per-entity clients
//! ([`QueueClient`]/[`TopicClient`]), not an account-level client. This wrapper
//! stores the namespace + SAS policy and vends entity clients on demand, so the
//! `AzureServices::servicebus()` accessor mirrors the other services while
//! matching what the SDK actually supports.

use azure_messaging_servicebus::service_bus::{QueueClient, TopicClient};
use serde::Deserialize;
use std::sync::Arc;

use crate::{AzureError, Result};

/// Per-service configuration for Service Bus, read from
/// `AzureConfig::service_config("servicebus")` when no
/// `servicebus_connection_string` is set.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceBusServiceConfig {
    /// Service Bus namespace (falls back to `AzureConfig::servicebus_namespace`).
    #[serde(default)]
    pub namespace: Option<String>,
    /// Shared Access Signature policy name (e.g. `RootManageSharedAccessKey`).
    pub policy_name: String,
    /// Shared Access Signature key.
    pub shared_access_key: String,
}

/// Namespace-scoped Azure Service Bus client.
///
/// Clone-cheap: it holds an `Arc<dyn HttpClient>` and the SAS material, and
/// builds a fresh [`QueueClient`]/[`TopicClient`] per entity on request.
#[derive(Clone)]
pub struct ServiceBusClient {
    namespace: String,
    policy_name: String,
    signing_key: String,
    http_client: Arc<dyn azure_core_sb::HttpClient>,
}

impl ServiceBusClient {
    /// Create a client from an explicit namespace + SAS policy.
    pub fn new(
        namespace: impl Into<String>,
        policy_name: impl Into<String>,
        signing_key: impl Into<String>,
    ) -> Self {
        Self {
            namespace: namespace.into(),
            policy_name: policy_name.into(),
            signing_key: signing_key.into(),
            http_client: azure_core_sb::new_http_client(),
        }
    }

    /// Parse a Service Bus connection string of the form
    /// `Endpoint=sb://<namespace>.servicebus.windows.net/;SharedAccessKeyName=<policy>;SharedAccessKey=<key>`.
    pub fn from_connection_string(conn: &str) -> Result<Self> {
        let mut endpoint = None;
        let mut policy_name = None;
        let mut signing_key = None;

        for part in conn.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            // Split on the FIRST '=' only: the base64 SAS key itself may contain '='.
            let (key, value) = part.split_once('=').ok_or_else(|| {
                AzureError::Config(format!(
                    "invalid Service Bus connection string segment: '{part}'"
                ))
            })?;
            match key.trim().to_ascii_lowercase().as_str() {
                "endpoint" => endpoint = Some(value.trim().to_string()),
                "sharedaccesskeyname" => policy_name = Some(value.trim().to_string()),
                "sharedaccesskey" => signing_key = Some(value.trim().to_string()),
                _ => {}
            }
        }

        let endpoint = endpoint.ok_or_else(|| {
            AzureError::Config("Service Bus connection string missing 'Endpoint'".to_string())
        })?;
        let namespace = parse_namespace(&endpoint)?;
        let policy_name = policy_name.ok_or_else(|| {
            AzureError::Config(
                "Service Bus connection string missing 'SharedAccessKeyName'".to_string(),
            )
        })?;
        let signing_key = signing_key.ok_or_else(|| {
            AzureError::Config(
                "Service Bus connection string missing 'SharedAccessKey'".to_string(),
            )
        })?;

        Ok(Self::new(namespace, policy_name, signing_key))
    }

    /// The Service Bus namespace this client targets.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Get a [`QueueClient`] for the named queue.
    pub fn queue(&self, queue: impl Into<String>) -> Result<QueueClient> {
        QueueClient::new(
            self.http_client.clone(),
            self.namespace.clone(),
            queue.into(),
            self.policy_name.clone(),
            self.signing_key.clone(),
        )
        .map_err(|e| AzureError::Service(e.to_string()))
    }

    /// Get a [`TopicClient`] for the named topic.
    pub fn topic(&self, topic: impl Into<String>) -> Result<TopicClient> {
        TopicClient::new(
            self.http_client.clone(),
            self.namespace.clone(),
            topic.into(),
            self.policy_name.clone(),
            self.signing_key.clone(),
        )
        .map_err(|e| AzureError::Service(e.to_string()))
    }
}

impl std::fmt::Debug for ServiceBusClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceBusClient")
            .field("namespace", &self.namespace)
            .field("policy_name", &self.policy_name)
            .finish_non_exhaustive()
    }
}

/// Extract the namespace from a Service Bus endpoint such as
/// `sb://mybus.servicebus.windows.net/` -> `mybus`.
fn parse_namespace(endpoint: &str) -> Result<String> {
    let host = endpoint
        .rsplit("://")
        .next()
        .unwrap_or(endpoint)
        .trim_end_matches('/');
    let namespace = host.split('.').next().unwrap_or(host);
    if namespace.is_empty() {
        return Err(AzureError::Config(format!(
            "could not parse namespace from Service Bus endpoint '{endpoint}'"
        )));
    }
    Ok(namespace.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_connection_string_parses_namespace_policy_key() {
        let conn = "Endpoint=sb://mybus.servicebus.windows.net/;\
                    SharedAccessKeyName=RootManageSharedAccessKey;\
                    SharedAccessKey=abc123def456==";
        let client = ServiceBusClient::from_connection_string(conn).expect("parse");
        assert_eq!(client.namespace(), "mybus");
        assert_eq!(client.policy_name, "RootManageSharedAccessKey");
        // The base64 key contains '=' which must survive split_once-on-first-'='.
        assert_eq!(client.signing_key, "abc123def456==");
    }

    #[test]
    fn builds_queue_and_topic_clients() {
        let client = ServiceBusClient::new("mybus", "policy", "a2V5"); // key = base64("key")
        // Constructing entity clients is offline (no request is sent here).
        assert!(client.queue("orders").is_ok());
        assert!(client.topic("events").is_ok());
    }

    #[test]
    fn from_connection_string_rejects_missing_key() {
        let conn = "Endpoint=sb://mybus.servicebus.windows.net/;\
                    SharedAccessKeyName=RootManageSharedAccessKey";
        assert!(ServiceBusClient::from_connection_string(conn).is_err());
    }
}
