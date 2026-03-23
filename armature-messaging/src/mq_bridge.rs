//! mq-bridge integration for armature-messaging
//!
//! This module provides an adapter to use [mq-bridge](https://github.com/marcomq/mq-bridge)
//! as a backend for armature-messaging. mq-bridge is a lower-level messaging library
//! that focuses on data transport and provides unified access to Kafka, AMQP, NATS,
//! MQTT, MongoDB, HTTP, and more.
//!
//! # Features
//!
//! The mq-bridge integration provides:
//! - **Unified Transport Layer**: Use mq-bridge's `CanonicalMessage` for cross-protocol messaging
//! - **Middleware Support**: Leverage mq-bridge's retry, DLQ, and deduplication middleware
//! - **Route-based Architecture**: Define message routes with handlers and transformations
//! - **Protocol Bridging**: Connect systems speaking different protocols (e.g., MQTT to Kafka)
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_messaging::mq_bridge::*;
//!
//! // Create a memory-based broker for testing
//! let broker = MqBridgeBroker::memory("test-channel").await?;
//! broker.publish(Message::new("test", b"hello")).await?;
//! ```
//!
//! # Integration with Armature Messaging
//!
//! The mq-bridge adapter implements the `MessageBroker` trait, allowing you to use
//! mq-bridge endpoints seamlessly with the rest of armature-messaging:
//!
//! ```rust,ignore
//! use armature_messaging::{MessageBroker, Message};
//! use armature_messaging::mq_bridge::MqBridgeBroker;
//!
//! let broker = MqBridgeBroker::memory("test-channel").await?;
//! broker.publish(Message::new("test", b"hello")).await?;
//! ```

use crate::{
    Message, MessageBroker, MessageHandler, MessagingError, ProcessingResult, PublishOptions,
    SubscribeOptions, Subscription,
};
use async_trait::async_trait;
use mq_bridge::CanonicalMessage;
use mq_bridge::endpoints::{create_consumer_from_route, create_publisher_from_route};
use mq_bridge::models::{Endpoint, EndpointType, MemoryConfig, Route};
use mq_bridge::traits::{Handler, MessageConsumer, MessagePublisher};
use mq_bridge::{Handled, HandlerError};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Configuration for mq-bridge endpoints
#[derive(Debug, Clone)]
pub struct MqBridgeConfig {
    /// Endpoint type (kafka, amqp, nats, mqtt, http, memory, file)
    pub endpoint_type: MqEndpointType,
    /// Connection URL or configuration
    pub url: String,
    /// Topic/queue/subject name
    pub topic: String,
    /// Additional options
    pub options: HashMap<String, String>,
    /// Buffer size for memory endpoints
    pub buffer_size: usize,
}

/// Supported mq-bridge endpoint types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqEndpointType {
    /// In-memory channel (for testing)
    Memory,
    /// Apache Kafka
    Kafka,
    /// AMQP (RabbitMQ)
    Amqp,
    /// NATS
    Nats,
    /// MQTT
    Mqtt,
    /// HTTP
    Http,
    /// File-based
    File,
}

impl Default for MqBridgeConfig {
    fn default() -> Self {
        Self {
            endpoint_type: MqEndpointType::Memory,
            url: String::new(),
            topic: "default".to_string(),
            options: HashMap::new(),
            buffer_size: 1000,
        }
    }
}

impl MqBridgeConfig {
    /// Create a new config for memory endpoint
    pub fn memory(topic: impl Into<String>) -> Self {
        Self {
            endpoint_type: MqEndpointType::Memory,
            topic: topic.into(),
            ..Default::default()
        }
    }

    /// Create a new config for Kafka endpoint
    #[cfg(feature = "mq-bridge-kafka")]
    pub fn kafka(brokers: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            endpoint_type: MqEndpointType::Kafka,
            url: brokers.into(),
            topic: topic.into(),
            ..Default::default()
        }
    }

    /// Create a new config for AMQP endpoint
    #[cfg(feature = "mq-bridge-amqp")]
    pub fn amqp(url: impl Into<String>, queue: impl Into<String>) -> Self {
        Self {
            endpoint_type: MqEndpointType::Amqp,
            url: url.into(),
            topic: queue.into(),
            ..Default::default()
        }
    }

    /// Create a new config for NATS endpoint
    #[cfg(feature = "mq-bridge-nats")]
    pub fn nats(url: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            endpoint_type: MqEndpointType::Nats,
            url: url.into(),
            topic: subject.into(),
            ..Default::default()
        }
    }

    /// Create a new config for MQTT endpoint
    #[cfg(feature = "mq-bridge-mqtt")]
    pub fn mqtt(url: impl Into<String>, topic: impl Into<String>) -> Self {
        Self {
            endpoint_type: MqEndpointType::Mqtt,
            url: url.into(),
            topic: topic.into(),
            ..Default::default()
        }
    }

    /// Set buffer size (for memory endpoints)
    pub fn with_buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Set a custom option
    pub fn with_option(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(key.into(), value.into());
        self
    }

    /// Build an mq-bridge Endpoint from this config
    pub fn build_endpoint(&self) -> Endpoint {
        match self.endpoint_type {
            MqEndpointType::Memory => Endpoint::new(EndpointType::Memory(MemoryConfig {
                topic: self.topic.clone(),
                capacity: Some(self.buffer_size),
            })),
            #[cfg(feature = "mq-bridge-kafka")]
            MqEndpointType::Kafka => {
                use mq_bridge::models::{KafkaConfig, KafkaEndpoint};
                Endpoint::new(EndpointType::Kafka(KafkaEndpoint {
                    topic: Some(self.topic.clone()),
                    config: KafkaConfig {
                        brokers: self.url.clone(),
                        group_id: self.options.get("group_id").cloned(),
                        ..Default::default()
                    },
                }))
            }
            #[cfg(not(feature = "mq-bridge-kafka"))]
            MqEndpointType::Kafka => {
                panic!("Kafka support requires 'mq-bridge-kafka' feature")
            }
            #[cfg(feature = "mq-bridge-amqp")]
            MqEndpointType::Amqp => {
                use mq_bridge::models::{AmqpConfig, AmqpEndpoint};
                Endpoint::new(EndpointType::Amqp(AmqpEndpoint {
                    queue: Some(self.topic.clone()),
                    config: AmqpConfig {
                        url: self.url.clone(),
                        exchange: self.options.get("exchange").cloned(),
                        ..Default::default()
                    },
                }))
            }
            #[cfg(not(feature = "mq-bridge-amqp"))]
            MqEndpointType::Amqp => {
                panic!("AMQP support requires 'mq-bridge-amqp' feature")
            }
            #[cfg(feature = "mq-bridge-nats")]
            MqEndpointType::Nats => {
                use mq_bridge::models::{NatsConfig, NatsEndpoint};
                Endpoint::new(EndpointType::Nats(NatsEndpoint {
                    subject: Some(self.topic.clone()),
                    stream: self.options.get("stream").cloned(),
                    config: NatsConfig {
                        url: self.url.clone(),
                        ..Default::default()
                    },
                }))
            }
            #[cfg(not(feature = "mq-bridge-nats"))]
            MqEndpointType::Nats => {
                panic!("NATS support requires 'mq-bridge-nats' feature")
            }
            #[cfg(feature = "mq-bridge-mqtt")]
            MqEndpointType::Mqtt => {
                use mq_bridge::models::{MqttConfig, MqttEndpoint};
                Endpoint::new(EndpointType::Mqtt(MqttEndpoint {
                    topic: Some(self.topic.clone()),
                    config: MqttConfig {
                        url: self.url.clone(),
                        ..Default::default()
                    },
                }))
            }
            #[cfg(not(feature = "mq-bridge-mqtt"))]
            MqEndpointType::Mqtt => {
                panic!("MQTT support requires 'mq-bridge-mqtt' feature")
            }
            #[cfg(feature = "mq-bridge-http")]
            MqEndpointType::Http => {
                use mq_bridge::models::{HttpConfig, HttpEndpoint};
                Endpoint::new(EndpointType::Http(HttpEndpoint {
                    config: HttpConfig {
                        url: Some(self.url.clone()),
                        ..Default::default()
                    },
                }))
            }
            #[cfg(not(feature = "mq-bridge-http"))]
            MqEndpointType::Http => {
                panic!("HTTP support requires 'mq-bridge-http' feature")
            }
            MqEndpointType::File => Endpoint::new(EndpointType::File(self.topic.clone())),
        }
    }
}

/// Convert armature Message to mq-bridge CanonicalMessage
pub fn to_canonical(msg: &Message) -> CanonicalMessage {
    let mut canonical = CanonicalMessage::new(msg.payload.clone(), None);

    // Store message ID in metadata
    canonical
        .metadata
        .insert("armature_id".to_string(), msg.id.clone());
    canonical
        .metadata
        .insert("armature_topic".to_string(), msg.topic.clone());

    // Copy headers to metadata
    for (key, value) in &msg.headers {
        canonical.metadata.insert(key.clone(), value.clone());
    }

    if let Some(ref ct) = msg.content_type {
        canonical
            .metadata
            .insert("content_type".to_string(), ct.clone());
    }
    if let Some(ref cid) = msg.correlation_id {
        canonical
            .metadata
            .insert("correlation_id".to_string(), cid.clone());
    }
    if let Some(ref rt) = msg.reply_to {
        canonical
            .metadata
            .insert("reply_to".to_string(), rt.clone());
    }
    if let Some(pri) = msg.priority {
        canonical
            .metadata
            .insert("priority".to_string(), pri.to_string());
    }
    if let Some(ttl) = msg.ttl {
        canonical
            .metadata
            .insert("ttl".to_string(), ttl.to_string());
    }

    canonical
}

/// Convert mq-bridge CanonicalMessage to armature Message
pub fn from_canonical(canonical: CanonicalMessage, default_topic: &str) -> Message {
    let topic = canonical
        .metadata
        .get("armature_topic")
        .cloned()
        .unwrap_or_else(|| default_topic.to_string());

    let mut msg = Message::new(topic, canonical.payload.to_vec());

    // Restore message ID if present
    if let Some(id) = canonical.metadata.get("armature_id") {
        msg.id = id.clone();
    }

    // Restore optional fields from metadata
    if let Some(ct) = canonical.metadata.get("content_type") {
        msg.content_type = Some(ct.clone());
    }
    if let Some(cid) = canonical.metadata.get("correlation_id") {
        msg.correlation_id = Some(cid.clone());
    }
    if let Some(rt) = canonical.metadata.get("reply_to") {
        msg.reply_to = Some(rt.clone());
    }
    if let Some(pri) = canonical.metadata.get("priority") {
        msg.priority = pri.parse().ok();
    }
    if let Some(ttl) = canonical.metadata.get("ttl") {
        msg.ttl = ttl.parse().ok();
    }

    // Copy remaining metadata to headers (excluding reserved keys)
    let reserved_keys = [
        "armature_id",
        "armature_topic",
        "content_type",
        "correlation_id",
        "reply_to",
        "priority",
        "ttl",
    ];
    for (key, value) in &canonical.metadata {
        if !reserved_keys.contains(&key.as_str()) {
            msg.headers.insert(key.clone(), value.clone());
        }
    }

    msg
}

/// mq-bridge based message broker
///
/// This broker uses mq-bridge endpoints for message transport, providing
/// access to Kafka, AMQP, NATS, MQTT, and more through a unified interface.
pub struct MqBridgeBroker {
    #[allow(dead_code)]
    config: MqBridgeConfig,
    endpoint: Endpoint,
    route_name: String,
    connected: AtomicBool,
}

impl MqBridgeBroker {
    /// Create a new mq-bridge broker
    pub async fn new(config: MqBridgeConfig) -> Result<Self, MessagingError> {
        let endpoint = config.build_endpoint();
        let route_name = format!("armature-{}", config.topic);

        Ok(Self {
            config,
            endpoint,
            route_name,
            connected: AtomicBool::new(true),
        })
    }

    /// Create a memory-based broker (for testing)
    pub async fn memory(topic: impl Into<String>) -> Result<Self, MessagingError> {
        Self::new(MqBridgeConfig::memory(topic)).await
    }

    /// Create a Kafka broker
    #[cfg(feature = "mq-bridge-kafka")]
    pub async fn kafka(
        brokers: impl Into<String>,
        topic: impl Into<String>,
    ) -> Result<Self, MessagingError> {
        Self::new(MqBridgeConfig::kafka(brokers, topic)).await
    }

    /// Create an AMQP broker
    #[cfg(feature = "mq-bridge-amqp")]
    pub async fn amqp(
        url: impl Into<String>,
        queue: impl Into<String>,
    ) -> Result<Self, MessagingError> {
        Self::new(MqBridgeConfig::amqp(url, queue)).await
    }

    /// Create a NATS broker
    #[cfg(feature = "mq-bridge-nats")]
    pub async fn nats(
        url: impl Into<String>,
        subject: impl Into<String>,
    ) -> Result<Self, MessagingError> {
        Self::new(MqBridgeConfig::nats(url, subject)).await
    }

    /// Get the underlying endpoint
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Get the channel for memory endpoints (for testing)
    pub fn channel(&self) -> Option<mq_bridge::endpoints::memory::MemoryChannel> {
        self.endpoint.channel().ok()
    }

    async fn get_publisher(&self) -> Result<Arc<dyn MessagePublisher>, MessagingError> {
        create_publisher_from_route(&self.route_name, &self.endpoint)
            .await
            .map_err(|e| MessagingError::Connection(e.to_string()))
    }

    async fn get_consumer(&self) -> Result<Box<dyn MessageConsumer>, MessagingError> {
        create_consumer_from_route(&self.route_name, &self.endpoint)
            .await
            .map_err(|e| MessagingError::Connection(e.to_string()))
    }
}

/// Subscription handle for mq-bridge
pub struct MqBridgeSubscription {
    topic: String,
    active: AtomicBool,
    cancel_token: tokio::sync::watch::Sender<bool>,
}

impl MqBridgeSubscription {
    fn new(topic: String) -> (Self, tokio::sync::watch::Receiver<bool>) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        (
            Self {
                topic,
                active: AtomicBool::new(true),
                cancel_token: tx,
            },
            rx,
        )
    }
}

#[async_trait]
impl Subscription for MqBridgeSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        let _ = self.cancel_token.send(true);
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn topic(&self) -> &str {
        &self.topic
    }
}

/// Wrapper to adapt armature MessageHandler to mq-bridge Handler
struct HandlerAdapter {
    handler: Arc<dyn MessageHandler>,
    topic: String,
}

#[async_trait]
impl Handler for HandlerAdapter {
    async fn handle(&self, msg: CanonicalMessage) -> Result<Handled, HandlerError> {
        let armature_msg = from_canonical(msg, &self.topic);

        match self.handler.handle(armature_msg).await {
            Ok(ProcessingResult::Success) => Ok(Handled::Ack),
            Ok(ProcessingResult::Retry) => Err(HandlerError::Retryable(anyhow::anyhow!(
                "Handler requested retry"
            ))),
            Ok(ProcessingResult::DeadLetter) => Err(HandlerError::NonRetryable(anyhow::anyhow!(
                "Handler requested dead-letter"
            ))),
            Ok(ProcessingResult::Reject) => Err(HandlerError::NonRetryable(anyhow::anyhow!(
                "Handler rejected message"
            ))),
            Err(e) => Err(HandlerError::NonRetryable(anyhow::anyhow!(
                "Handler error: {}",
                e
            ))),
        }
    }
}

#[async_trait]
impl MessageBroker for MqBridgeBroker {
    type Subscription = MqBridgeSubscription;

    async fn publish(&self, message: Message) -> Result<(), MessagingError> {
        let publisher = self.get_publisher().await?;
        let canonical = to_canonical(&message);

        publisher
            .send(canonical)
            .await
            .map_err(|e| MessagingError::Publish(e.to_string()))?;

        Ok(())
    }

    async fn publish_with_options(
        &self,
        message: Message,
        _options: PublishOptions,
    ) -> Result<(), MessagingError> {
        // mq-bridge handles persistence and routing internally
        self.publish(message).await
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
        _options: SubscribeOptions,
    ) -> Result<Self::Subscription, MessagingError> {
        let (subscription, mut cancel_rx) = MqBridgeSubscription::new(topic.to_string());

        let mut consumer = self.get_consumer().await?;

        let adapter = Arc::new(HandlerAdapter {
            handler,
            topic: topic.to_string(),
        });

        // Spawn consumer task
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_rx.changed() => {
                        if *cancel_rx.borrow() {
                            break;
                        }
                    }
                    result = consumer.receive() => {
                        match result {
                            Ok(received) => {
                                let msg = received.message;
                                match adapter.handle(msg).await {
                                    Ok(Handled::Ack) => {
                                        (received.commit)(None).await;
                                    }
                                    Ok(Handled::Publish(response)) => {
                                        (received.commit)(Some(response)).await;
                                    }
                                    Err(_) => {
                                        // Error - don't commit, message will be redelivered
                                    }
                                }
                            }
                            Err(mq_bridge::errors::ConsumerError::EndOfStream) => {
                                // Channel closed
                                break;
                            }
                            Err(_) => {
                                // Error - continue trying
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
        });

        Ok(subscription)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), MessagingError> {
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// Helper to create message routes using mq-bridge
///
/// Routes define a pipeline from an input endpoint to an output endpoint
/// with optional handlers and middleware.
pub struct MqBridgeRoute {
    input: Option<Endpoint>,
    output: Option<Endpoint>,
    handler: Option<Arc<dyn Handler>>,
}

impl MqBridgeRoute {
    /// Create a new empty route
    pub fn new() -> Self {
        Self {
            input: None,
            output: None,
            handler: None,
        }
    }

    /// Set the input endpoint from config
    pub fn from_config(mut self, config: MqBridgeConfig) -> Self {
        self.input = Some(config.build_endpoint());
        self
    }

    /// Set the output endpoint from config
    pub fn to_config(mut self, config: MqBridgeConfig) -> Self {
        self.output = Some(config.build_endpoint());
        self
    }

    /// Set input from memory channel
    pub fn from_memory(mut self, topic: impl Into<String>, buffer_size: usize) -> Self {
        self.input = Some(Endpoint::new(EndpointType::Memory(MemoryConfig {
            topic: topic.into(),
            capacity: Some(buffer_size),
        })));
        self
    }

    /// Set output to memory channel
    pub fn to_memory(mut self, topic: impl Into<String>, buffer_size: usize) -> Self {
        self.output = Some(Endpoint::new(EndpointType::Memory(MemoryConfig {
            topic: topic.into(),
            capacity: Some(buffer_size),
        })));
        self
    }

    /// Set a handler function
    pub fn with_handler<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(CanonicalMessage) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Handled, HandlerError>> + Send + 'static,
    {
        self.handler = Some(Arc::new(FnHandlerWrapper(f)));
        self
    }

    /// Build the mq-bridge Route
    pub fn build(self) -> Result<Route, MessagingError> {
        let input = self
            .input
            .ok_or_else(|| MessagingError::Configuration("Input endpoint not set".to_string()))?;
        let mut output = self
            .output
            .ok_or_else(|| MessagingError::Configuration("Output endpoint not set".to_string()))?;

        if let Some(handler) = self.handler {
            output.handler = Some(handler);
        }

        Ok(Route {
            input,
            output,
            concurrency: 1,
            batch_size: 128,
        })
    }

    /// Build and run the route
    pub async fn run(self, name: &str) -> Result<(), MessagingError> {
        let route = self.build()?;
        route
            .run_until_err(name, None, None)
            .await
            .map(|_| ())
            .map_err(|e| MessagingError::Other(e.to_string()))
    }
}

impl Default for MqBridgeRoute {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper for function handlers
struct FnHandlerWrapper<F>(F);

#[async_trait]
impl<F, Fut> Handler for FnHandlerWrapper<F>
where
    F: Fn(CanonicalMessage) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Handled, HandlerError>> + Send + 'static,
{
    async fn handle(&self, msg: CanonicalMessage) -> Result<Handled, HandlerError> {
        (self.0)(msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_memory() {
        let config = MqBridgeConfig::memory("test-topic");
        assert_eq!(config.endpoint_type, MqEndpointType::Memory);
        assert_eq!(config.topic, "test-topic");
    }

    #[test]
    fn test_message_conversion() {
        let msg = Message::new("test-topic", b"hello world".to_vec())
            .with_header("key", "value")
            .with_correlation_id("corr-123");

        let canonical = to_canonical(&msg);
        assert_eq!(canonical.payload.as_ref(), b"hello world");
        assert_eq!(
            canonical.metadata.get("armature_topic"),
            Some(&"test-topic".to_string())
        );
        assert_eq!(canonical.metadata.get("key"), Some(&"value".to_string()));
        assert_eq!(
            canonical.metadata.get("correlation_id"),
            Some(&"corr-123".to_string())
        );

        let back = from_canonical(canonical, "default");
        assert_eq!(back.topic, "test-topic");
        assert_eq!(back.payload, b"hello world");
        assert_eq!(back.headers.get("key"), Some(&"value".to_string()));
        assert_eq!(back.correlation_id, Some("corr-123".to_string()));
    }

    #[tokio::test]
    async fn test_memory_broker() {
        let broker = MqBridgeBroker::memory("test").await.unwrap();
        assert!(broker.is_connected());

        // Publish a message
        let msg = Message::new("test", b"hello".to_vec());
        broker.publish(msg).await.unwrap();

        // Verify it was sent via the channel
        if let Some(channel) = broker.channel() {
            let msgs = channel.drain_messages();
            assert_eq!(msgs.len(), 1);
            assert_eq!(msgs[0].payload.as_ref(), b"hello");
        }
    }
}
