//! # Armature Messaging
//!
//! Message broker integrations for the Armature framework.
//!
//! This crate provides a unified interface for working with various message brokers:
//! - **RabbitMQ** - AMQP message broker
//! - **Kafka** - Distributed event streaming
//! - **NATS** - Cloud-native messaging
//! - **AWS SQS/SNS** - AWS messaging services
//!
//! ## Features
//!
//! Enable specific backends via Cargo features:
//! - `rabbitmq` - RabbitMQ/AMQP support
//! - `kafka` - Apache Kafka support
//! - `nats` - NATS support
//! - `aws` - AWS SQS/SNS support
//! - `full` - All backends
//!
//! ## Example
//!
//! ```rust,ignore
//! use armature_messaging::{Message, MessageBroker, MessageHandler};
//!
//! // Define a message handler
//! struct MyHandler;
//!
//! #[async_trait::async_trait]
//! impl MessageHandler for MyHandler {
//!     async fn handle(&self, message: Message) -> Result<(), MessagingError> {
//!         println!("Received: {:?}", message.payload);
//!         Ok(())
//!     }
//! }
//!
//! // Connect and subscribe
//! #[cfg(feature = "rabbitmq")]
//! async fn example() -> Result<(), MessagingError> {
//!     let broker = RabbitMqBroker::connect("amqp://localhost:5672").await?;
//!     broker.subscribe("my-queue", MyHandler).await?;
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod config;
pub mod error;

#[cfg(feature = "rabbitmq")]
pub mod rabbitmq;

#[cfg(feature = "kafka")]
pub mod kafka;

#[cfg(feature = "nats")]
pub mod nats;

#[cfg(feature = "aws")]
pub mod aws;

#[cfg(feature = "mq-bridge")]
pub mod mq_bridge;

pub use config::*;
pub use error::*;

/// A message to be sent or received from a message broker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message identifier
    pub id: String,
    /// Message payload as bytes
    pub payload: Vec<u8>,
    /// Message headers/properties
    pub headers: HashMap<String, String>,
    /// Topic/queue/subject the message belongs to
    pub topic: String,
    /// Timestamp when the message was created
    pub timestamp: DateTime<Utc>,
    /// Optional correlation ID for request-response patterns
    pub correlation_id: Option<String>,
    /// Optional reply-to address
    pub reply_to: Option<String>,
    /// Message content type (e.g., "application/json")
    pub content_type: Option<String>,
    /// Message priority (0-9, where 9 is highest)
    pub priority: Option<u8>,
    /// Time-to-live in milliseconds
    pub ttl: Option<u64>,
}

impl Message {
    /// Create a new message with the given payload
    pub fn new<T: Into<Vec<u8>>>(topic: impl Into<String>, payload: T) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            payload: payload.into(),
            headers: HashMap::new(),
            topic: topic.into(),
            timestamp: Utc::now(),
            correlation_id: None,
            reply_to: None,
            content_type: None,
            priority: None,
            ttl: None,
        }
    }

    /// Create a message from a JSON-serializable value
    pub fn json<T: Serialize>(topic: impl Into<String>, value: &T) -> Result<Self, MessagingError> {
        let payload =
            serde_json::to_vec(value).map_err(|e| MessagingError::Serialization(e.to_string()))?;
        let mut msg = Self::new(topic, payload);
        msg.content_type = Some("application/json".to_string());
        Ok(msg)
    }

    /// Parse the payload as JSON
    pub fn parse_json<T: for<'de> Deserialize<'de>>(&self) -> Result<T, MessagingError> {
        serde_json::from_slice(&self.payload)
            .map_err(|e| MessagingError::Deserialization(e.to_string()))
    }

    /// Get the payload as a UTF-8 string
    pub fn payload_str(&self) -> Result<&str, MessagingError> {
        std::str::from_utf8(&self.payload)
            .map_err(|e| MessagingError::Deserialization(e.to_string()))
    }

    /// Add a header to the message
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set the correlation ID
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Set the reply-to address
    pub fn with_reply_to(mut self, reply_to: impl Into<String>) -> Self {
        self.reply_to = Some(reply_to.into());
        self
    }

    /// Set the content type
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Set the priority (0-9)
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = Some(priority.min(9));
        self
    }

    /// Set the time-to-live in milliseconds
    pub fn with_ttl(mut self, ttl_ms: u64) -> Self {
        self.ttl = Some(ttl_ms);
        self
    }

    /// Set the time-to-live from a Duration
    pub fn with_ttl_duration(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl.as_millis() as u64);
        self
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Message {{ id: {}, topic: {}, size: {} bytes }}",
            self.id,
            self.topic,
            self.payload.len()
        )
    }
}

/// Acknowledgment behavior for received messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AckMode {
    /// Automatically acknowledge messages after successful processing
    #[default]
    Auto,
    /// Manually acknowledge messages
    Manual,
    /// No acknowledgment required
    None,
}

/// Result of processing a message
#[derive(Debug, Clone)]
pub enum ProcessingResult {
    /// Message was processed successfully
    Success,
    /// Message processing failed, should be retried
    Retry,
    /// Message processing failed, should be dead-lettered
    DeadLetter,
    /// Message should be rejected and discarded
    Reject,
}

/// Trait for handling received messages
#[async_trait]
pub trait MessageHandler: Send + Sync + 'static {
    /// Handle a received message
    async fn handle(&self, message: Message) -> Result<ProcessingResult, MessagingError>;

    /// Called when a message cannot be deserialized
    async fn on_deserialize_error(&self, _error: &MessagingError) -> ProcessingResult {
        ProcessingResult::DeadLetter
    }
}

/// Function-based message handler
pub struct FnHandler<F>(pub F);

#[async_trait]
impl<F, Fut> MessageHandler for FnHandler<F>
where
    F: Fn(Message) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ProcessingResult, MessagingError>> + Send,
{
    async fn handle(&self, message: Message) -> Result<ProcessingResult, MessagingError> {
        (self.0)(message).await
    }
}

/// Options for publishing a message
#[derive(Debug, Clone, Default)]
pub struct PublishOptions {
    /// Whether to wait for confirmation from the broker
    pub confirm: bool,
    /// Timeout for confirmation
    pub timeout: Option<Duration>,
    /// Delivery mode (persistent or transient)
    pub persistent: bool,
    /// Routing key (for topic-based routing)
    pub routing_key: Option<String>,
    /// Exchange name (for RabbitMQ)
    pub exchange: Option<String>,
    /// Partition key (for Kafka)
    pub partition_key: Option<String>,
}

impl PublishOptions {
    /// Create options for persistent delivery
    pub fn persistent() -> Self {
        Self {
            persistent: true,
            confirm: true,
            ..Default::default()
        }
    }

    /// Set the routing key
    pub fn with_routing_key(mut self, key: impl Into<String>) -> Self {
        self.routing_key = Some(key.into());
        self
    }

    /// Set the exchange
    pub fn with_exchange(mut self, exchange: impl Into<String>) -> Self {
        self.exchange = Some(exchange.into());
        self
    }

    /// Set the partition key
    pub fn with_partition_key(mut self, key: impl Into<String>) -> Self {
        self.partition_key = Some(key.into());
        self
    }

    /// Enable confirmation
    pub fn with_confirm(mut self, timeout: Duration) -> Self {
        self.confirm = true;
        self.timeout = Some(timeout);
        self
    }
}

/// Options for subscribing to messages
#[derive(Debug, Clone, Default)]
pub struct SubscribeOptions {
    /// Consumer group/tag
    pub consumer_group: Option<String>,
    /// Prefetch count (how many messages to buffer)
    pub prefetch_count: Option<u16>,
    /// Acknowledgment mode
    pub ack_mode: AckMode,
    /// Whether to start from the beginning (for Kafka)
    pub from_beginning: bool,
    /// Filter expression (for some brokers)
    pub filter: Option<String>,
    /// Maximum concurrent handlers
    pub concurrency: Option<usize>,
}

impl SubscribeOptions {
    /// Set the consumer group
    pub fn with_consumer_group(mut self, group: impl Into<String>) -> Self {
        self.consumer_group = Some(group.into());
        self
    }

    /// Set the prefetch count
    pub fn with_prefetch(mut self, count: u16) -> Self {
        self.prefetch_count = Some(count);
        self
    }

    /// Set the acknowledgment mode
    pub fn with_ack_mode(mut self, mode: AckMode) -> Self {
        self.ack_mode = mode;
        self
    }

    /// Start from the beginning (Kafka)
    pub fn from_beginning(mut self) -> Self {
        self.from_beginning = true;
        self
    }

    /// Set the concurrency level
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = Some(concurrency);
        self
    }
}

/// Subscription handle for managing a subscription
#[async_trait]
pub trait Subscription: Send + Sync {
    /// Stop the subscription
    async fn unsubscribe(&self) -> Result<(), MessagingError>;

    /// Check if the subscription is active
    fn is_active(&self) -> bool;

    /// Get the topic/queue name
    fn topic(&self) -> &str;
}

/// Core trait for message brokers
#[async_trait]
pub trait MessageBroker: Send + Sync {
    /// The subscription handle type
    type Subscription: Subscription;

    /// Publish a message
    async fn publish(&self, message: Message) -> Result<(), MessagingError>;

    /// Publish a message with options
    async fn publish_with_options(
        &self,
        message: Message,
        options: PublishOptions,
    ) -> Result<(), MessagingError>;

    /// Subscribe to a topic/queue
    async fn subscribe(
        &self,
        topic: &str,
        handler: Arc<dyn MessageHandler>,
    ) -> Result<Self::Subscription, MessagingError>;

    /// Subscribe with options
    async fn subscribe_with_options(
        &self,
        topic: &str,
        handler: Arc<dyn MessageHandler>,
        options: SubscribeOptions,
    ) -> Result<Self::Subscription, MessagingError>;

    /// Check if connected to the broker
    fn is_connected(&self) -> bool;

    /// Close the connection
    async fn close(&self) -> Result<(), MessagingError>;
}

/// Builder for creating message broker connections
pub struct MessagingBuilder {
    /// Configuration for the message broker
    pub config: MessagingConfig,
}

impl MessagingBuilder {
    /// Create a new builder with the given configuration
    pub fn new(config: MessagingConfig) -> Self {
        Self { config }
    }

    /// Build a RabbitMQ broker
    #[cfg(feature = "rabbitmq")]
    pub async fn build_rabbitmq(self) -> Result<rabbitmq::RabbitMqBroker, MessagingError> {
        rabbitmq::RabbitMqBroker::connect(&self.config).await
    }

    /// Build a Kafka broker
    #[cfg(feature = "kafka")]
    pub async fn build_kafka(self) -> Result<kafka::KafkaBroker, MessagingError> {
        kafka::KafkaBroker::connect(&self.config).await
    }

    /// Build a NATS broker
    #[cfg(feature = "nats")]
    pub async fn build_nats(self) -> Result<nats::NatsBroker, MessagingError> {
        nats::NatsBroker::connect(&self.config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let msg = Message::new("test-topic", b"hello world".to_vec());
        assert_eq!(msg.topic, "test-topic");
        assert_eq!(msg.payload, b"hello world");
        assert!(!msg.id.is_empty());
    }

    #[test]
    fn test_message_json() {
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct TestData {
            name: String,
            value: i32,
        }

        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let msg = Message::json("test-topic", &data).unwrap();
        assert_eq!(msg.content_type, Some("application/json".to_string()));

        let parsed: TestData = msg.parse_json().unwrap();
        assert_eq!(parsed, data);
    }

    #[test]
    fn test_message_builder() {
        let msg = Message::new("topic", b"data".to_vec())
            .with_header("key", "value")
            .with_correlation_id("corr-123")
            .with_reply_to("reply-queue")
            .with_priority(5)
            .with_ttl(60000);

        assert_eq!(msg.headers.get("key"), Some(&"value".to_string()));
        assert_eq!(msg.correlation_id, Some("corr-123".to_string()));
        assert_eq!(msg.reply_to, Some("reply-queue".to_string()));
        assert_eq!(msg.priority, Some(5));
        assert_eq!(msg.ttl, Some(60000));
    }

    #[test]
    fn test_publish_options() {
        let opts = PublishOptions::persistent()
            .with_routing_key("my.routing.key")
            .with_exchange("my-exchange")
            .with_confirm(Duration::from_secs(5));

        assert!(opts.persistent);
        assert!(opts.confirm);
        assert_eq!(opts.routing_key, Some("my.routing.key".to_string()));
        assert_eq!(opts.exchange, Some("my-exchange".to_string()));
        assert_eq!(opts.timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn test_subscribe_options() {
        let opts = SubscribeOptions::default()
            .with_consumer_group("my-group")
            .with_prefetch(10)
            .with_ack_mode(AckMode::Manual)
            .with_concurrency(4);

        assert_eq!(opts.consumer_group, Some("my-group".to_string()));
        assert_eq!(opts.prefetch_count, Some(10));
        assert_eq!(opts.ack_mode, AckMode::Manual);
        assert_eq!(opts.concurrency, Some(4));
    }
}
