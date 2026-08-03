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
    SubscribeOptions, Subscription, dispatch,
};
use async_trait::async_trait;
use mq_bridge::CanonicalMessage;
use mq_bridge::endpoints::{create_consumer_from_route, create_publisher_from_route};
use mq_bridge::models::{Endpoint, EndpointType, FileConfig, MemoryConfig, Route, RouteOptions};
use mq_bridge::traits::{Handler, MessageConsumer, MessageDisposition, MessagePublisher};
use mq_bridge::{Handled, HandlerError};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio::task::JoinSet;

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
            MqEndpointType::Memory => Endpoint::new(EndpointType::Memory(MemoryConfig::new(
                self.topic.clone(),
                Some(self.buffer_size),
            ))),
            #[cfg(feature = "mq-bridge-kafka")]
            MqEndpointType::Kafka => {
                use mq_bridge::models::KafkaConfig;
                Endpoint::new(EndpointType::Kafka(KafkaConfig {
                    url: self.url.clone(),
                    topic: Some(self.topic.clone()),
                    group_id: self.options.get("group_id").cloned(),
                    ..Default::default()
                }))
            }
            #[cfg(not(feature = "mq-bridge-kafka"))]
            MqEndpointType::Kafka => {
                panic!("Kafka support requires 'mq-bridge-kafka' feature")
            }
            #[cfg(feature = "mq-bridge-amqp")]
            MqEndpointType::Amqp => {
                use mq_bridge::models::AmqpConfig;
                Endpoint::new(EndpointType::Amqp(AmqpConfig {
                    url: self.url.clone(),
                    queue: Some(self.topic.clone()),
                    exchange: self.options.get("exchange").cloned(),
                    ..Default::default()
                }))
            }
            #[cfg(not(feature = "mq-bridge-amqp"))]
            MqEndpointType::Amqp => {
                panic!("AMQP support requires 'mq-bridge-amqp' feature")
            }
            #[cfg(feature = "mq-bridge-nats")]
            MqEndpointType::Nats => {
                use mq_bridge::models::NatsConfig;
                Endpoint::new(EndpointType::Nats(NatsConfig {
                    url: self.url.clone(),
                    subject: Some(self.topic.clone()),
                    stream: self.options.get("stream").cloned(),
                    ..Default::default()
                }))
            }
            #[cfg(not(feature = "mq-bridge-nats"))]
            MqEndpointType::Nats => {
                panic!("NATS support requires 'mq-bridge-nats' feature")
            }
            #[cfg(feature = "mq-bridge-mqtt")]
            MqEndpointType::Mqtt => {
                use mq_bridge::models::MqttConfig;
                Endpoint::new(EndpointType::Mqtt(MqttConfig {
                    url: self.url.clone(),
                    topic: Some(self.topic.clone()),
                    ..Default::default()
                }))
            }
            #[cfg(not(feature = "mq-bridge-mqtt"))]
            MqEndpointType::Mqtt => {
                panic!("MQTT support requires 'mq-bridge-mqtt' feature")
            }
            #[cfg(feature = "mq-bridge-http")]
            MqEndpointType::Http => {
                use mq_bridge::models::HttpConfig;
                Endpoint::new(EndpointType::Http(HttpConfig {
                    url: self.url.clone(),
                    ..Default::default()
                }))
            }
            #[cfg(not(feature = "mq-bridge-http"))]
            MqEndpointType::Http => {
                panic!("HTTP support requires 'mq-bridge-http' feature")
            }
            MqEndpointType::File => {
                Endpoint::new(EndpointType::File(FileConfig::new(self.topic.clone())))
            }
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

/// Convert an owned armature [`Message`] into an mq-bridge [`CanonicalMessage`],
/// moving the payload, id, topic and headers instead of cloning them.
///
/// This is the by-value counterpart to [`to_canonical`]: `publish` owns its
/// `Message`, so it can hand ownership straight through and avoid cloning the
/// (potentially large) payload on every publish. Use [`to_canonical`] when you
/// only have a `&Message`.
pub fn into_canonical(msg: Message) -> CanonicalMessage {
    let mut canonical = CanonicalMessage::new(msg.payload, None);

    // Store message ID and topic in metadata
    canonical.metadata.insert("armature_id".to_string(), msg.id);
    canonical
        .metadata
        .insert("armature_topic".to_string(), msg.topic);

    // Move headers to metadata
    for (key, value) in msg.headers {
        canonical.metadata.insert(key, value);
    }

    if let Some(ct) = msg.content_type {
        canonical.metadata.insert("content_type".to_string(), ct);
    }
    if let Some(cid) = msg.correlation_id {
        canonical.metadata.insert("correlation_id".to_string(), cid);
    }
    if let Some(rt) = msg.reply_to {
        canonical.metadata.insert("reply_to".to_string(), rt);
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
    config: MqBridgeConfig,
    /// The endpoint built from `config.topic`, kept around for `endpoint()`
    /// and `channel()` (mainly used by the in-memory test broker).
    endpoint: Endpoint,
    connected: AtomicBool,
    /// Publishers are expensive to construct (each one opens a fresh
    /// connection to the backing transport), so they are created lazily per
    /// destination topic and reused across calls to `publish`/
    /// `publish_with_options` instead of being rebuilt on every message.
    publishers: RwLock<HashMap<String, Arc<dyn MessagePublisher>>>,
}

impl MqBridgeBroker {
    /// Create a new mq-bridge broker
    pub async fn new(config: MqBridgeConfig) -> Result<Self, MessagingError> {
        let endpoint = config.build_endpoint();

        Ok(Self {
            config,
            endpoint,
            connected: AtomicBool::new(true),
            publishers: RwLock::new(HashMap::new()),
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

    /// Build the endpoint that should be used to route a message for the
    /// given topic: the endpoint built from `config.topic` when the topic
    /// matches the broker's default (avoiding an extra endpoint build), or a
    /// fresh endpoint pointed at the requested topic otherwise. This is what
    /// makes the per-call topic (the `topic` argument to `subscribe*`, or
    /// `message.topic` for publishing) actually control routing instead of
    /// always going to `config.topic`.
    fn endpoint_for_topic(&self, topic: &str) -> Endpoint {
        if topic == self.config.topic {
            self.endpoint.clone()
        } else {
            let mut config = self.config.clone();
            config.topic = topic.to_string();
            config.build_endpoint()
        }
    }

    /// Get (creating and caching if necessary) the publisher for `topic`.
    /// Publishers are cached per-topic so repeated `publish`/
    /// `publish_with_options` calls reuse the same underlying connection
    /// instead of opening a new one per message.
    async fn get_or_create_publisher(
        &self,
        topic: &str,
    ) -> Result<Arc<dyn MessagePublisher>, MessagingError> {
        if let Some(publisher) = self.publishers.read().await.get(topic) {
            return Ok(publisher.clone());
        }

        let mut publishers = self.publishers.write().await;
        // Re-check after acquiring the write lock in case another task raced
        // us and already created the publisher for this topic.
        if let Some(publisher) = publishers.get(topic) {
            return Ok(publisher.clone());
        }

        let endpoint = self.endpoint_for_topic(topic);
        let route_name = format!("armature-{}", topic);
        let publisher = create_publisher_from_route(&route_name, &endpoint)
            .await
            .map_err(|e| MessagingError::Connection(e.to_string()))?;

        publishers.insert(topic.to_string(), publisher.clone());
        Ok(publisher)
    }

    async fn get_consumer(&self, topic: &str) -> Result<Box<dyn MessageConsumer>, MessagingError> {
        let endpoint = self.endpoint_for_topic(topic);
        let route_name = format!("armature-{}", topic);
        create_consumer_from_route(&route_name, &endpoint)
            .await
            .map_err(|e| MessagingError::Connection(e.to_string()))
    }
}

/// Subscription handle for mq-bridge
pub struct MqBridgeSubscription {
    topic: String,
    active: AtomicBool,
    cancel_token: tokio::sync::watch::Sender<bool>,
    /// In-flight per-message handler tasks, drained (with a bounded timeout)
    /// on `unsubscribe` instead of being abandoned.
    tasks: Arc<Mutex<JoinSet<()>>>,
}

impl MqBridgeSubscription {
    fn new(
        topic: String,
    ) -> (
        Self,
        tokio::sync::watch::Receiver<bool>,
        Arc<Mutex<JoinSet<()>>>,
    ) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let tasks = Arc::new(Mutex::new(JoinSet::new()));
        (
            Self {
                topic,
                active: AtomicBool::new(true),
                cancel_token: tx,
                tasks: tasks.clone(),
            },
            rx,
            tasks,
        )
    }
}

#[async_trait]
impl Subscription for MqBridgeSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        let _ = self.cancel_token.send(true);
        dispatch::drain_with_timeout(&self.tasks, dispatch::DEFAULT_DRAIN_TIMEOUT, "mq-bridge")
            .await;
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

/// Run the adapted handler for a single received mq-bridge message and
/// ack/nack it via `commit` based on the result. Broken out of
/// `subscribe_with_options`'s spawned consumer task so it can be spawned as
/// an independent per-message task under the concurrency semaphore (mirrors
/// `aws.rs`'s `handle_sqs_message` and `rabbitmq.rs`'s `handle_delivery`).
async fn handle_received_message(
    adapter: Arc<HandlerAdapter>,
    msg: CanonicalMessage,
    commit: mq_bridge::traits::CommitFunc,
) {
    match adapter.handle(msg).await {
        Ok(Handled::Ack) => {
            let _ = commit(MessageDisposition::Ack).await;
        }
        Ok(Handled::Publish(response)) => {
            let _ = commit(MessageDisposition::Reply(response)).await;
        }
        Err(_) => {
            // Negative acknowledgement - message will be redelivered
            let _ = commit(MessageDisposition::Nack).await;
        }
    }
}

#[async_trait]
impl MessageBroker for MqBridgeBroker {
    type Subscription = MqBridgeSubscription;

    async fn publish(&self, message: Message) -> Result<(), MessagingError> {
        // Route by the message's own topic, not the broker's default
        // `config.topic` - this is what lets multi-topic use of a single
        // `MqBridgeBroker` actually reach the right destination.
        let publisher = self.get_or_create_publisher(&message.topic).await?;
        // `publish` owns `message`, so move it into the canonical form rather
        // than cloning the payload.
        let canonical = into_canonical(message);

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
        // `persistent`/`routing_key`/`exchange`/`partition_key` have no
        // per-call equivalent in mq-bridge: persistence and AMQP routing are
        // fixed at endpoint/route construction time (see mq-bridge's
        // `AmqpPublisher::send` and `KafkaConfig::partition_key`), and
        // mq-bridge does not expose a per-`CanonicalMessage` override for
        // them. There is nothing meaningful to apply here beyond what
        // `publish` already does, so this intentionally just delegates.
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
        options: SubscribeOptions,
    ) -> Result<Self::Subscription, MessagingError> {
        // Neither field is read anywhere in this bridge, so honoring the
        // request is impossible; failing is the only way the caller finds out.
        crate::reject_manual_ack("MQ bridge", options.ack_mode)?;
        crate::reject_filter("MQ bridge", options.filter.as_ref())?;

        // Bounds how many per-message handler invocations may run
        // concurrently (see the spawned consumer task below). Defaults to 1,
        // which reproduces the previous strictly-sequential dispatch.
        let concurrency = dispatch::concurrency_or_default(options.concurrency);

        let (subscription, mut cancel_rx, tasks) = MqBridgeSubscription::new(topic.to_string());

        // Route by the topic argument, not the broker's default
        // `config.topic` - otherwise every subscription would consume from
        // the same single endpoint regardless of what was requested.
        let mut consumer = self.get_consumer(topic).await?;

        let adapter = Arc::new(HandlerAdapter {
            handler,
            topic: topic.to_string(),
        });

        // Bounds how many `adapter.handle` invocations may run concurrently.
        // A permit is acquired before spawning each message's task (tracked
        // in `tasks` so `unsubscribe` can drain outstanding handlers on
        // shutdown - see `dispatch::spawn_bounded`) and released when that
        // task finishes, so at most `concurrency` handlers are ever in
        // flight at once (concurrency == 1 reproduces the previous
        // strictly-sequential dispatch). `Received::commit` is a `'static +
        // Send + FnOnce` closure fully decoupled from the `consumer` it came
        // from, so committing out of dispatch order from a spawned task is
        // safe and does not block `consumer.receive()` from being called
        // again for the next message.
        let semaphore = Arc::new(Semaphore::new(concurrency));

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
                                let commit = received.commit;
                                let adapter = adapter.clone();

                                dispatch::spawn_bounded(&semaphore, &tasks, async move {
                                    handle_received_message(adapter, msg, commit).await;
                                })
                                .await;
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
        self.input = Some(Endpoint::new(EndpointType::Memory(MemoryConfig::new(
            topic.into(),
            Some(buffer_size),
        ))));
        self
    }

    /// Set output to memory channel
    pub fn to_memory(mut self, topic: impl Into<String>, buffer_size: usize) -> Self {
        self.output = Some(Endpoint::new(EndpointType::Memory(MemoryConfig::new(
            topic.into(),
            Some(buffer_size),
        ))));
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
            options: RouteOptions {
                concurrency: 1,
                batch_size: 128,
                ..Default::default()
            },
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

    #[tokio::test]
    async fn publish_routes_by_message_topic_not_broker_default() {
        let broker = MqBridgeBroker::memory("default-topic").await.unwrap();

        // Publish to a topic different from the broker's configured default;
        // the message must be routed there, not to "default-topic".
        let msg = Message::new("other-topic", b"hello-other".to_vec());
        broker.publish(msg).await.unwrap();

        let other_channel = MqBridgeConfig::memory("other-topic")
            .build_endpoint()
            .channel()
            .expect("memory channel for other-topic");
        let other_msgs = other_channel.drain_messages();
        assert_eq!(other_msgs.len(), 1);
        assert_eq!(other_msgs[0].payload.as_ref(), b"hello-other");

        // Nothing should have landed on the broker's default topic channel.
        if let Some(default_channel) = broker.channel() {
            assert!(default_channel.drain_messages().is_empty());
        }
    }

    #[tokio::test]
    async fn publisher_is_cached_per_topic_across_publishes() {
        let broker = MqBridgeBroker::memory("test-topic").await.unwrap();

        broker
            .publish(Message::new("test-topic", b"one".to_vec()))
            .await
            .unwrap();
        broker
            .publish(Message::new("test-topic", b"two".to_vec()))
            .await
            .unwrap();

        // A second publish to the same topic must reuse the cached
        // publisher rather than constructing a fresh one.
        assert_eq!(broker.publishers.read().await.len(), 1);

        // A publish to a different topic gets its own cache entry.
        broker
            .publish(Message::new("another-topic", b"three".to_vec()))
            .await
            .unwrap();
        assert_eq!(broker.publishers.read().await.len(), 2);

        if let Some(channel) = broker.channel() {
            let msgs = channel.drain_messages();
            assert_eq!(msgs.len(), 2);
        }
    }

    /// Regression test for the finding that the bounded-concurrency dispatch
    /// added across the aws/nats/rabbitmq/mq_bridge backends had zero test
    /// coverage. Uses the in-memory broker (no external dependency) with
    /// `concurrency: Some(2)` and 4 messages whose handlers complete in a
    /// scrambled order (via staggered `tokio::time::sleep`s), and asserts:
    /// (i) an in-flight counter's high-water mark never exceeds 2, proving
    /// the semaphore actually bounds concurrent handlers, and (ii) every
    /// message is still handled (acked) exactly once regardless of
    /// completion order.
    #[tokio::test]
    async fn concurrency_bounds_in_flight_handlers_and_acks_every_message_regardless_of_order() {
        use crate::FnHandler;
        use std::sync::atomic::AtomicUsize;
        use std::time::Duration;
        use tokio::sync::Mutex as AsyncMutex;

        let broker = MqBridgeBroker::memory("concurrency-test").await.unwrap();

        let in_flight = Arc::new(AtomicUsize::new(0));
        let high_water_mark = Arc::new(AtomicUsize::new(0));
        let acked = Arc::new(AsyncMutex::new(Vec::new()));

        let in_flight_clone = in_flight.clone();
        let high_water_mark_clone = high_water_mark.clone();
        let acked_clone = acked.clone();

        let handler = Arc::new(FnHandler(move |msg: Message| {
            let in_flight = in_flight_clone.clone();
            let high_water_mark = high_water_mark_clone.clone();
            let acked = acked_clone.clone();
            async move {
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                high_water_mark.fetch_max(current, Ordering::SeqCst);

                // Stagger completion so handler-completion order is
                // scrambled relative to dispatch order: message 0's handler
                // is the slowest.
                let idx: usize = msg.payload_str().unwrap().parse().unwrap();
                let delay_ms = [80u64, 10, 40, 20][idx];
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;

                in_flight.fetch_sub(1, Ordering::SeqCst);
                acked.lock().await.push(idx);

                Ok::<_, MessagingError>(ProcessingResult::Success)
            }
        }));

        let sub = broker
            .subscribe_with_options(
                "concurrency-test",
                handler,
                SubscribeOptions::default().with_concurrency(2),
            )
            .await
            .unwrap();

        for i in 0..4usize {
            broker
                .publish(Message::new("concurrency-test", i.to_string().into_bytes()))
                .await
                .unwrap();
        }

        // Give the consumer loop time to receive, dispatch, and complete all
        // 4 handlers (longest single delay is 80ms; with concurrency 2 the
        // whole batch completes well within this margin).
        tokio::time::sleep(Duration::from_millis(1000)).await;

        let observed_high_water_mark = high_water_mark.load(Ordering::SeqCst);
        assert!(
            observed_high_water_mark <= 2,
            "at most `concurrency` (2) handlers should ever be in flight at once, saw {}",
            observed_high_water_mark
        );

        let mut acked = acked.lock().await.clone();
        acked.sort_unstable();
        assert_eq!(
            acked,
            vec![0, 1, 2, 3],
            "every message must be handled (acked) exactly once regardless of completion order"
        );

        sub.unsubscribe().await.unwrap();
    }
}
