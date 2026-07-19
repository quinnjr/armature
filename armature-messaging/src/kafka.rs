//! Apache Kafka message broker implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use rdkafka::Message as KafkaMessage;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{Header, Headers, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
    AckMode, Message, MessageBroker, MessageHandler, MessagingConfig, MessagingError,
    ProcessingResult, PublishOptions, SubscribeOptions, Subscription, config::KafkaConfig,
};

/// Apache Kafka message broker
pub struct KafkaBroker {
    producer: FutureProducer,
    config: KafkaConfig,
    consumers: Arc<RwLock<Vec<Arc<StreamConsumer>>>>,
    connected: Arc<AtomicBool>,
}

impl KafkaBroker {
    /// Connect to Kafka
    pub async fn connect(config: &MessagingConfig) -> Result<Self, MessagingError> {
        let kafka_config = KafkaConfig {
            base: config.clone(),
            ..Default::default()
        };
        Self::connect_with_config(kafka_config).await
    }

    /// Connect with Kafka-specific configuration
    pub async fn connect_with_config(config: KafkaConfig) -> Result<Self, MessagingError> {
        info!(brokers = %config.base.url, "Connecting to Kafka");

        let mut client_config = ClientConfig::new();
        client_config.set("bootstrap.servers", &config.base.url);

        if let Some(ref client_id) = config.client_id {
            client_config.set("client.id", client_id);
        }

        // TLS configuration
        if config.base.tls {
            client_config.set("security.protocol", "SSL");
            if let Some(ref tls_config) = config.base.tls_config {
                if let Some(ref ca_cert) = tls_config.ca_cert {
                    client_config.set("ssl.ca.location", ca_cert);
                }
                if let Some(ref client_cert) = tls_config.client_cert {
                    client_config.set("ssl.certificate.location", client_cert);
                }
                if let Some(ref client_key) = tls_config.client_key {
                    client_config.set("ssl.key.location", client_key);
                }
            }
        }

        // SASL authentication
        if let Some(ref mechanism) = config.sasl_mechanism {
            client_config.set(
                "security.protocol",
                if config.base.tls {
                    "SASL_SSL"
                } else {
                    "SASL_PLAINTEXT"
                },
            );
            client_config.set("sasl.mechanism", mechanism);
            if let Some(ref username) = config.sasl_username {
                client_config.set("sasl.username", username);
            }
            if let Some(ref password) = config.sasl_password {
                client_config.set("sasl.password", password);
            }
        }

        let producer: FutureProducer = client_config
            .create()
            .map_err(|e| MessagingError::Connection(e.to_string()))?;

        info!("Connected to Kafka successfully");

        Ok(Self {
            producer,
            config,
            consumers: Arc::new(RwLock::new(Vec::new())),
            connected: Arc::new(AtomicBool::new(true)),
        })
    }

    fn build_headers(message: &Message) -> OwnedHeaders {
        let mut headers = OwnedHeaders::new();

        headers = headers.insert(Header {
            key: "message_id",
            value: Some(message.id.as_bytes()),
        });

        headers = headers.insert(Header {
            key: "timestamp",
            value: Some(message.timestamp.to_rfc3339().as_bytes()),
        });

        if let Some(ref correlation_id) = message.correlation_id {
            headers = headers.insert(Header {
                key: "correlation_id",
                value: Some(correlation_id.as_bytes()),
            });
        }

        if let Some(ref content_type) = message.content_type {
            headers = headers.insert(Header {
                key: "content_type",
                value: Some(content_type.as_bytes()),
            });
        }

        for (key, value) in &message.headers {
            headers = headers.insert(Header {
                key,
                value: Some(value.as_bytes()),
            });
        }

        headers
    }
}

#[async_trait]
impl MessageBroker for KafkaBroker {
    type Subscription = KafkaSubscription;

    async fn publish(&self, message: Message) -> Result<(), MessagingError> {
        self.publish_with_options(message, PublishOptions::default())
            .await
    }

    async fn publish_with_options(
        &self,
        message: Message,
        options: PublishOptions,
    ) -> Result<(), MessagingError> {
        let topic = &message.topic;
        let headers = Self::build_headers(&message);

        let mut record = FutureRecord::to(topic)
            .payload(&message.payload)
            .headers(headers);

        // Use partition key if provided
        if let Some(ref key) = options.partition_key {
            record = record.key(key);
        }

        debug!(topic = topic, message_id = %message.id, "Publishing message to Kafka");

        let timeout = options.timeout.unwrap_or(Duration::from_secs(5));

        self.producer
            .send(record, timeout)
            .await
            .map_err(|(e, _)| MessagingError::Publish(e.to_string()))?;

        Ok(())
    }

    async fn subscribe(
        &self,
        topic: &str,
        handler: Arc<dyn MessageHandler>,
    ) -> Result<Self::Subscription, MessagingError> {
        self.subscribe_with_options(topic, handler, SubscribeOptions::default())
            .await
    }

    async fn subscribe_with_options(
        &self,
        topic: &str,
        handler: Arc<dyn MessageHandler>,
        options: SubscribeOptions,
    ) -> Result<Self::Subscription, MessagingError> {
        let mut client_config = ClientConfig::new();
        client_config.set("bootstrap.servers", &self.config.base.url);

        let group_id = options
            .consumer_group
            .or_else(|| self.config.group_id.clone())
            .unwrap_or_else(|| format!("armature-{}", uuid::Uuid::new_v4()));

        client_config.set("group.id", &group_id);

        if let Some(ref client_id) = self.config.client_id {
            client_config.set("client.id", client_id);
        }

        let offset_reset = if options.from_beginning {
            "earliest"
        } else {
            &self.config.auto_offset_reset
        };
        client_config.set("auto.offset.reset", offset_reset);

        // Auto commit based on ack mode, combined with the configured preference.
        // Manual ack mode always disables auto-commit since offsets are committed
        // explicitly after the handler succeeds (see `consume_messages`).
        let enable_auto_commit = derive_enable_auto_commit(options.ack_mode, &self.config);
        client_config.set(
            "enable.auto.commit",
            if enable_auto_commit { "true" } else { "false" },
        );

        if enable_auto_commit {
            client_config.set(
                "auto.commit.interval.ms",
                self.config.auto_commit_interval_ms.to_string(),
            );
        }

        client_config.set(
            "session.timeout.ms",
            self.config.session_timeout_ms.to_string(),
        );

        // TLS configuration
        if self.config.base.tls {
            client_config.set("security.protocol", "SSL");
            if let Some(ref tls_config) = self.config.base.tls_config {
                if let Some(ref ca_cert) = tls_config.ca_cert {
                    client_config.set("ssl.ca.location", ca_cert);
                }
                if let Some(ref client_cert) = tls_config.client_cert {
                    client_config.set("ssl.certificate.location", client_cert);
                }
                if let Some(ref client_key) = tls_config.client_key {
                    client_config.set("ssl.key.location", client_key);
                }
            }
        }

        // SASL
        if let Some(ref mechanism) = self.config.sasl_mechanism {
            client_config.set(
                "security.protocol",
                if self.config.base.tls {
                    "SASL_SSL"
                } else {
                    "SASL_PLAINTEXT"
                },
            );
            client_config.set("sasl.mechanism", mechanism);
            if let Some(ref username) = self.config.sasl_username {
                client_config.set("sasl.username", username);
            }
            if let Some(ref password) = self.config.sasl_password {
                client_config.set("sasl.password", password);
            }
        }

        let consumer: StreamConsumer = client_config
            .create()
            .map_err(|e| MessagingError::Subscribe(e.to_string()))?;

        consumer
            .subscribe(&[topic])
            .map_err(|e| MessagingError::Subscribe(e.to_string()))?;

        let consumer = Arc::new(consumer);
        let active = Arc::new(AtomicBool::new(true));

        let subscription = KafkaSubscription {
            topic: topic.to_string(),
            group_id: group_id.clone(),
            active: active.clone(),
        };

        // Store consumer for cleanup
        self.consumers.write().await.push(consumer.clone());

        // Spawn consumer task
        let topic_owned = topic.to_string();
        let ack_mode = options.ack_mode;
        tokio::spawn(async move {
            consume_messages(consumer, handler, &topic_owned, ack_mode, active).await;
        });

        info!(topic = topic, group_id = %group_id, "Subscribed to Kafka topic");
        Ok(subscription)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), MessagingError> {
        info!("Closing Kafka connections");
        self.connected.store(false, Ordering::SeqCst);
        // Consumers will stop when active flag is set to false
        Ok(())
    }
}

/// Derive the `enable.auto.commit` rdkafka property from the subscribe-time
/// ack mode and the configured `enable_auto_commit` preference.
///
/// `AckMode::Manual` always disables auto-commit: offsets are committed
/// explicitly in `consume_messages` after the handler reports success.
/// For `Auto`/`None` ack modes, the configured preference is honored so
/// `KafkaConfig::without_auto_commit` actually has an effect.
fn derive_enable_auto_commit(ack_mode: AckMode, config: &KafkaConfig) -> bool {
    match ack_mode {
        AckMode::Manual => false,
        AckMode::Auto | AckMode::None => config.enable_auto_commit,
    }
}

async fn consume_messages(
    consumer: Arc<StreamConsumer>,
    handler: Arc<dyn MessageHandler>,
    topic: &str,
    ack_mode: AckMode,
    active: Arc<AtomicBool>,
) {
    use futures_util::StreamExt;

    let mut stream = consumer.stream();

    while active.load(Ordering::SeqCst) {
        match stream.next().await {
            Some(Ok(borrowed_message)) => {
                let message = kafka_message_to_message(&borrowed_message, topic);

                match handler.handle(message).await {
                    Ok(result) => {
                        match result {
                            ProcessingResult::Success => {
                                // Commit the offset explicitly in manual ack mode, since
                                // `enable.auto.commit` is disabled for that mode and
                                // rdkafka will never advance the consumer group's offset
                                // on its own, causing redelivery on restart.
                                if ack_mode == AckMode::Manual
                                    && let Err(e) = consumer
                                        .commit_message(&borrowed_message, CommitMode::Async)
                                {
                                    error!(error = %e, "Failed to commit Kafka offset");
                                }
                            }
                            ProcessingResult::Retry => {
                                warn!(
                                    "Kafka does not support message retry - message will be lost"
                                );
                            }
                            ProcessingResult::DeadLetter | ProcessingResult::Reject => {
                                debug!("Message rejected");
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Message handler error");
                    }
                }
            }
            Some(Err(e)) => {
                error!(error = %e, "Kafka consumer error");
            }
            None => {
                debug!("Consumer stream ended");
                break;
            }
        }
    }
}

fn kafka_message_to_message<M: KafkaMessage>(kafka_msg: &M, topic: &str) -> Message {
    let payload = kafka_msg.payload().map(|p| p.to_vec()).unwrap_or_default();

    let mut headers = HashMap::new();
    let mut message_id = None;
    let mut correlation_id = None;
    let mut content_type = None;
    let mut timestamp = chrono::Utc::now();

    if let Some(kafka_headers) = kafka_msg.headers() {
        for header in kafka_headers.iter() {
            if let Some(value) = header.value {
                let value_str = String::from_utf8_lossy(value).to_string();
                match header.key {
                    "message_id" => message_id = Some(value_str),
                    "correlation_id" => correlation_id = Some(value_str),
                    "content_type" => content_type = Some(value_str),
                    "timestamp" => {
                        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&value_str) {
                            timestamp = ts.with_timezone(&chrono::Utc);
                        }
                    }
                    _ => {
                        headers.insert(header.key.to_string(), value_str);
                    }
                }
            }
        }
    }

    // Use Kafka timestamp if available
    if let Some(ts) = kafka_msg.timestamp().to_millis()
        && let Some(dt) = chrono::DateTime::from_timestamp_millis(ts)
    {
        timestamp = dt;
    }

    Message {
        id: message_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        payload,
        headers,
        topic: topic.to_string(),
        timestamp,
        correlation_id,
        reply_to: None,
        content_type,
        priority: None,
        ttl: None,
    }
}

/// Kafka subscription handle
pub struct KafkaSubscription {
    topic: String,
    group_id: String,
    active: Arc<AtomicBool>,
}

#[async_trait]
impl Subscription for KafkaSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        info!(topic = %self.topic, group_id = %self.group_id, "Unsubscribed from Kafka topic");
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn topic(&self) -> &str {
        &self.topic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_ack_mode_always_disables_auto_commit() {
        let mut config = KafkaConfig::new("localhost:9092");
        assert!(config.enable_auto_commit, "default config should be true");
        assert!(!derive_enable_auto_commit(AckMode::Manual, &config));

        config.enable_auto_commit = false;
        assert!(!derive_enable_auto_commit(AckMode::Manual, &config));
    }

    #[test]
    fn auto_and_none_ack_modes_respect_configured_preference() {
        let mut config = KafkaConfig::new("localhost:9092");

        // Config says auto-commit is enabled (the default).
        assert!(derive_enable_auto_commit(AckMode::Auto, &config));
        assert!(derive_enable_auto_commit(AckMode::None, &config));

        // `without_auto_commit()` must actually take effect for Auto/None.
        config = config.without_auto_commit();
        assert!(!derive_enable_auto_commit(AckMode::Auto, &config));
        assert!(!derive_enable_auto_commit(AckMode::None, &config));
    }

    #[test]
    fn build_headers_includes_custom_and_reserved_headers() {
        let message = Message::new("topic", b"payload".to_vec())
            .with_header("x-custom", "value")
            .with_correlation_id("corr-1")
            .with_content_type("application/json");

        let headers = KafkaBroker::build_headers(&message);
        let mut seen = std::collections::HashSet::new();
        for header in headers.iter() {
            seen.insert(header.key.to_string());
        }

        assert!(seen.contains("message_id"));
        assert!(seen.contains("timestamp"));
        assert!(seen.contains("correlation_id"));
        assert!(seen.contains("content_type"));
        assert!(seen.contains("x-custom"));
    }
}
