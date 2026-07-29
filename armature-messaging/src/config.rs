//! Configuration types for messaging backends

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for messaging backends
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagingConfig {
    /// Connection URL or comma-separated list of URLs
    pub url: String,
    /// Optional username for authentication
    pub username: Option<String>,
    /// Optional password for authentication
    pub password: Option<String>,
    /// Connection timeout
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout: Duration,
    /// Heartbeat interval
    #[serde(default = "default_heartbeat")]
    pub heartbeat: Duration,
    /// Whether to use TLS
    #[serde(default)]
    pub tls: bool,
    /// TLS configuration
    pub tls_config: Option<TlsConfig>,
    /// Backend-specific options
    #[serde(default)]
    pub options: HashMap<String, String>,
}

fn default_connection_timeout() -> Duration {
    Duration::from_secs(30)
}

fn default_heartbeat() -> Duration {
    Duration::from_secs(60)
}

impl Default for MessagingConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            username: None,
            password: None,
            connection_timeout: default_connection_timeout(),
            heartbeat: default_heartbeat(),
            tls: false,
            tls_config: None,
            options: HashMap::new(),
        }
    }
}

impl MessagingConfig {
    /// Create a new configuration with the given URL
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Default::default()
        }
    }

    /// Set the username
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Set the password
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set credentials
    pub fn with_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Set the connection timeout
    pub fn with_connection_timeout(mut self, timeout: Duration) -> Self {
        self.connection_timeout = timeout;
        self
    }

    /// Set the heartbeat interval
    pub fn with_heartbeat(mut self, heartbeat: Duration) -> Self {
        self.heartbeat = heartbeat;
        self
    }

    /// Enable TLS
    pub fn with_tls(mut self) -> Self {
        self.tls = true;
        self
    }

    /// Set TLS configuration
    pub fn with_tls_config(mut self, config: TlsConfig) -> Self {
        self.tls = true;
        self.tls_config = Some(config);
        self
    }

    /// Set a custom option
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// Get an option value
    pub fn get_option(&self, key: &str) -> Option<&String> {
        self.options.get(key)
    }
}

/// TLS configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to CA certificate file
    pub ca_cert: Option<String>,
    /// Path to client certificate file
    pub client_cert: Option<String>,
    /// Path to client key file
    pub client_key: Option<String>,
    /// Whether to skip certificate verification (not recommended for production)
    #[serde(default)]
    pub insecure_skip_verify: bool,
    /// Server name for SNI
    pub server_name: Option<String>,
}

impl TlsConfig {
    /// Create a new TLS configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the CA certificate path
    pub fn with_ca_cert(mut self, path: impl Into<String>) -> Self {
        self.ca_cert = Some(path.into());
        self
    }

    /// Set client certificate and key paths
    pub fn with_client_cert(
        mut self,
        cert_path: impl Into<String>,
        key_path: impl Into<String>,
    ) -> Self {
        self.client_cert = Some(cert_path.into());
        self.client_key = Some(key_path.into());
        self
    }

    /// Skip certificate verification (insecure!)
    pub fn insecure(mut self) -> Self {
        self.insecure_skip_verify = true;
        self
    }

    /// Set the server name for SNI
    pub fn with_server_name(mut self, name: impl Into<String>) -> Self {
        self.server_name = Some(name.into());
        self
    }
}

/// RabbitMQ-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RabbitMqConfig {
    /// Base messaging config
    #[serde(flatten)]
    pub base: MessagingConfig,
    /// Virtual host
    #[serde(default = "default_vhost")]
    pub vhost: String,
    /// Channel pool size
    #[serde(default = "default_channel_pool_size")]
    pub channel_pool_size: usize,
    /// Publisher confirms enabled
    #[serde(default = "default_true")]
    pub publisher_confirms: bool,
}

fn default_vhost() -> String {
    "/".to_string()
}

fn default_channel_pool_size() -> usize {
    10
}

fn default_true() -> bool {
    true
}

impl Default for RabbitMqConfig {
    fn default() -> Self {
        Self {
            base: MessagingConfig::new("amqp://localhost:5672"),
            vhost: default_vhost(),
            channel_pool_size: default_channel_pool_size(),
            publisher_confirms: true,
        }
    }
}

impl RabbitMqConfig {
    /// Create a new RabbitMQ configuration
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            base: MessagingConfig::new(url),
            ..Default::default()
        }
    }

    /// Set the virtual host
    pub fn with_vhost(mut self, vhost: impl Into<String>) -> Self {
        self.vhost = vhost.into();
        self
    }

    /// Set the channel pool size
    pub fn with_channel_pool_size(mut self, size: usize) -> Self {
        self.channel_pool_size = size;
        self
    }

    /// Disable publisher confirms
    pub fn without_publisher_confirms(mut self) -> Self {
        self.publisher_confirms = false;
        self
    }
}

/// Kafka-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KafkaConfig {
    /// Base messaging config (URL is comma-separated broker list)
    #[serde(flatten)]
    pub base: MessagingConfig,
    /// Consumer group ID
    pub group_id: Option<String>,
    /// Client ID
    pub client_id: Option<String>,
    /// Auto offset reset behavior
    #[serde(default = "default_offset_reset")]
    pub auto_offset_reset: String,
    /// Enable auto commit
    #[serde(default = "default_true")]
    pub enable_auto_commit: bool,
    /// Auto commit interval in milliseconds
    #[serde(default = "default_auto_commit_interval")]
    pub auto_commit_interval_ms: u32,
    /// Session timeout in milliseconds
    #[serde(default = "default_session_timeout")]
    pub session_timeout_ms: u32,
    /// SASL mechanism
    pub sasl_mechanism: Option<String>,
    /// SASL username
    pub sasl_username: Option<String>,
    /// SASL password
    pub sasl_password: Option<String>,
}

fn default_offset_reset() -> String {
    "latest".to_string()
}

fn default_auto_commit_interval() -> u32 {
    5000
}

fn default_session_timeout() -> u32 {
    30000
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            base: MessagingConfig::new("localhost:9092"),
            group_id: None,
            client_id: None,
            auto_offset_reset: default_offset_reset(),
            enable_auto_commit: true,
            auto_commit_interval_ms: default_auto_commit_interval(),
            session_timeout_ms: default_session_timeout(),
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
        }
    }
}

impl KafkaConfig {
    /// Create a new Kafka configuration
    pub fn new(brokers: impl Into<String>) -> Self {
        Self {
            base: MessagingConfig::new(brokers),
            ..Default::default()
        }
    }

    /// Set the consumer group ID
    pub fn with_group_id(mut self, group_id: impl Into<String>) -> Self {
        self.group_id = Some(group_id.into());
        self
    }

    /// Set the client ID
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = Some(client_id.into());
        self
    }

    /// Set auto offset reset to "earliest"
    pub fn from_earliest(mut self) -> Self {
        self.auto_offset_reset = "earliest".to_string();
        self
    }

    /// Disable auto commit
    pub fn without_auto_commit(mut self) -> Self {
        self.enable_auto_commit = false;
        self
    }

    /// Set SASL authentication
    pub fn with_sasl(
        mut self,
        mechanism: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.sasl_mechanism = Some(mechanism.into());
        self.sasl_username = Some(username.into());
        self.sasl_password = Some(password.into());
        self
    }
}

/// NATS-specific configuration
///
/// `credentials_file`, `jwt`, and `nkey_seed` are authentication secrets: they
/// are redacted from the [`Debug`] output (custom `impl` below) and skipped
/// when serializing (`#[serde(skip_serializing)]`), so a `{:?}` log line or a
/// serialized config snapshot cannot leak the signing material. They still
/// deserialize normally, so loading a config from a file is unaffected.
#[derive(Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    /// Base messaging config
    #[serde(flatten)]
    pub base: MessagingConfig,
    /// Connection name
    pub name: Option<String>,
    /// Maximum reconnection attempts
    #[serde(default = "default_max_reconnects")]
    pub max_reconnects: usize,
    /// Reconnection wait time
    #[serde(default = "default_reconnect_wait")]
    pub reconnect_wait: Duration,
    /// Enable JetStream persistence.
    ///
    /// When set, [`NatsBroker::publish`]/`publish_with_options` route
    /// through the JetStream context instead of core NATS, giving published
    /// messages stream persistence and a server ack (honored via
    /// [`PublishOptions::confirm`]/`timeout`, mirroring the core-NATS
    /// behavior).
    ///
    /// [`NatsBroker::subscribe`]/`subscribe_with_options` do **not**
    /// currently route through JetStream even when this is enabled:
    /// consuming from a stream requires provisioning decisions (stream
    /// name, durable vs. ephemeral consumer, ack policy) that don't yet
    /// have a place in this config or in [`SubscribeOptions`], so
    /// subscriptions still use core NATS pub/sub with no persistence or
    /// redelivery guarantees. Use [`NatsBroker::jetstream`] to drive the
    /// `async-nats` JetStream consumer APIs directly if you need
    /// JetStream-backed consumption today.
    ///
    /// [`NatsBroker::publish`]: crate::nats::NatsBroker::publish
    /// [`NatsBroker::subscribe`]: crate::nats::NatsBroker::subscribe
    /// [`NatsBroker::jetstream`]: crate::nats::NatsBroker::jetstream
    /// [`PublishOptions::confirm`]: crate::PublishOptions::confirm
    /// [`SubscribeOptions`]: crate::SubscribeOptions
    #[serde(default)]
    pub jetstream: bool,
    /// NATS credentials file path (secret: redacted from `Debug`, not serialized)
    #[serde(skip_serializing)]
    pub credentials_file: Option<String>,
    /// JWT token (secret: redacted from `Debug`, not serialized)
    #[serde(skip_serializing)]
    pub jwt: Option<String>,
    /// NKey seed (secret: redacted from `Debug`, not serialized)
    #[serde(skip_serializing)]
    pub nkey_seed: Option<String>,
}

impl std::fmt::Debug for NatsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact the secret fields: show only whether each is set, never its
        // value, so a `{:?}` of this config cannot leak credentials.
        let redact = |opt: &Option<String>| opt.as_ref().map(|_| "<redacted>");
        f.debug_struct("NatsConfig")
            .field("base", &self.base)
            .field("name", &self.name)
            .field("max_reconnects", &self.max_reconnects)
            .field("reconnect_wait", &self.reconnect_wait)
            .field("jetstream", &self.jetstream)
            .field("credentials_file", &redact(&self.credentials_file))
            .field("jwt", &redact(&self.jwt))
            .field("nkey_seed", &redact(&self.nkey_seed))
            .finish()
    }
}

fn default_max_reconnects() -> usize {
    60
}

fn default_reconnect_wait() -> Duration {
    Duration::from_secs(2)
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            base: MessagingConfig::new("nats://localhost:4222"),
            name: None,
            max_reconnects: default_max_reconnects(),
            reconnect_wait: default_reconnect_wait(),
            jetstream: false,
            credentials_file: None,
            jwt: None,
            nkey_seed: None,
        }
    }
}

impl NatsConfig {
    /// Create a new NATS configuration
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            base: MessagingConfig::new(url),
            ..Default::default()
        }
    }

    /// Set the connection name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Enable JetStream persistence for publishing (see the `jetstream`
    /// field doc for exactly what this does and does not affect - notably,
    /// subscriptions do not yet route through JetStream).
    pub fn with_jetstream(mut self) -> Self {
        self.jetstream = true;
        self
    }

    /// Set credentials file
    pub fn with_credentials_file(mut self, path: impl Into<String>) -> Self {
        self.credentials_file = Some(path.into());
        self
    }

    /// Set JWT and NKey authentication
    pub fn with_jwt_nkey(mut self, jwt: impl Into<String>, nkey_seed: impl Into<String>) -> Self {
        self.jwt = Some(jwt.into());
        self.nkey_seed = Some(nkey_seed.into());
        self
    }
}

/// AWS SQS/SNS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsConfig {
    /// AWS region
    pub region: String,
    /// AWS access key ID (optional, uses environment/IAM role if not set)
    pub access_key_id: Option<String>,
    /// AWS secret access key
    pub secret_access_key: Option<String>,
    /// Session token (for temporary credentials)
    pub session_token: Option<String>,
    /// Custom endpoint URL (for LocalStack, etc.)
    pub endpoint_url: Option<String>,
    /// SQS queue URL prefix
    pub sqs_queue_url_prefix: Option<String>,
    /// SNS topic ARN prefix
    pub sns_topic_arn_prefix: Option<String>,
    /// Long polling wait time in seconds
    #[serde(default = "default_long_poll_wait")]
    pub long_poll_wait_seconds: i32,
    /// Maximum number of messages to receive at once
    #[serde(default = "default_max_messages")]
    pub max_number_of_messages: i32,
    /// Visibility timeout in seconds
    #[serde(default = "default_visibility_timeout")]
    pub visibility_timeout: i32,
}

fn default_long_poll_wait() -> i32 {
    20
}

fn default_max_messages() -> i32 {
    10
}

fn default_visibility_timeout() -> i32 {
    30
}

impl Default for AwsConfig {
    fn default() -> Self {
        Self {
            region: "us-east-1".to_string(),
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            endpoint_url: None,
            sqs_queue_url_prefix: None,
            sns_topic_arn_prefix: None,
            long_poll_wait_seconds: default_long_poll_wait(),
            max_number_of_messages: default_max_messages(),
            visibility_timeout: default_visibility_timeout(),
        }
    }
}

impl AwsConfig {
    /// Create a new AWS configuration
    pub fn new(region: impl Into<String>) -> Self {
        Self {
            region: region.into(),
            ..Default::default()
        }
    }

    /// Set explicit credentials
    pub fn with_credentials(
        mut self,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Self {
        self.access_key_id = Some(access_key_id.into());
        self.secret_access_key = Some(secret_access_key.into());
        self
    }

    /// Set session token
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    /// Set custom endpoint (for LocalStack)
    pub fn with_endpoint(mut self, url: impl Into<String>) -> Self {
        self.endpoint_url = Some(url.into());
        self
    }

    /// Configure for LocalStack
    pub fn localstack() -> Self {
        Self::new("us-east-1")
            .with_endpoint("http://localhost:4566")
            .with_credentials("test", "test")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nats_config_debug_redacts_secrets() {
        let config = NatsConfig::new("nats://localhost:4222")
            .with_jwt_nkey(
                "super-secret-jwt",
                "SUAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
            .with_credentials_file("/etc/nats/app.creds");

        let debug = format!("{config:?}");
        assert!(
            !debug.contains("super-secret-jwt"),
            "Debug output leaked the JWT: {debug}"
        );
        assert!(
            !debug.contains("SUAAAA"),
            "Debug output leaked the NKey seed: {debug}"
        );
        assert!(
            !debug.contains("/etc/nats/app.creds"),
            "Debug output leaked the credentials file path: {debug}"
        );
        // The presence of a secret is still visible (Some vs None), just redacted.
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn nats_config_serialize_omits_secrets() {
        let config = NatsConfig::new("nats://localhost:4222")
            .with_jwt_nkey("super-secret-jwt", "super-secret-seed")
            .with_credentials_file("/etc/nats/app.creds");

        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("super-secret-jwt"), "serialized JWT leaked");
        assert!(
            !json.contains("super-secret-seed"),
            "serialized seed leaked"
        );
        assert!(
            !json.contains("/etc/nats/app.creds"),
            "serialized credentials file path leaked"
        );
    }

    #[test]
    fn nats_config_deserialize_still_reads_secrets() {
        // `skip_serializing` must not disable deserialization: loading a config
        // that supplies these fields must still populate them.
        let json = r#"{
            "url": "nats://localhost:4222",
            "jwt": "the-jwt",
            "nkey_seed": "the-seed",
            "credentials_file": "/path/to.creds"
        }"#;
        let config: NatsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.jwt.as_deref(), Some("the-jwt"));
        assert_eq!(config.nkey_seed.as_deref(), Some("the-seed"));
        assert_eq!(config.credentials_file.as_deref(), Some("/path/to.creds"));
    }
}
