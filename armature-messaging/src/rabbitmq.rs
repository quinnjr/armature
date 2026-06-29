//! RabbitMQ message broker implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures_util::StreamExt;
use lapin::{
    BasicProperties, Channel, Connection, ConnectionProperties, Consumer, options::*,
    types::FieldTable,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
    AckMode, Message, MessageBroker, MessageHandler, MessagingConfig, MessagingError,
    ProcessingResult, PublishOptions, SubscribeOptions, Subscription,
};

/// RabbitMQ message broker
pub struct RabbitMqBroker {
    connection: Arc<Connection>,
    channels: Arc<RwLock<Vec<Channel>>>,
    publish_channel: Channel,
    connected: Arc<AtomicBool>,
}

impl RabbitMqBroker {
    /// Connect to RabbitMQ
    pub async fn connect(config: &MessagingConfig) -> Result<Self, MessagingError> {
        info!(url = %config.url, "Connecting to RabbitMQ");

        let connection = Connection::connect(&config.url, ConnectionProperties::default()).await?;

        let publish_channel = connection.create_channel().await?;

        // Enable publisher confirms if requested
        publish_channel
            .confirm_select(ConfirmSelectOptions::default())
            .await?;

        info!("Connected to RabbitMQ successfully");

        Ok(Self {
            connection: Arc::new(connection),
            channels: Arc::new(RwLock::new(Vec::new())),
            publish_channel,
            connected: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Declare a queue
    pub async fn declare_queue(
        &self,
        name: &str,
        options: QueueDeclareOptions,
    ) -> Result<(), MessagingError> {
        self.publish_channel
            .queue_declare(name.into(), options, FieldTable::default())
            .await?;
        debug!(queue = name, "Queue declared");
        Ok(())
    }

    /// Declare an exchange
    pub async fn declare_exchange(
        &self,
        name: &str,
        kind: lapin::ExchangeKind,
        options: ExchangeDeclareOptions,
    ) -> Result<(), MessagingError> {
        self.publish_channel
            .exchange_declare(name.into(), kind, options, FieldTable::default())
            .await?;
        debug!(exchange = name, "Exchange declared");
        Ok(())
    }

    /// Bind a queue to an exchange
    pub async fn bind_queue(
        &self,
        queue: &str,
        exchange: &str,
        routing_key: &str,
    ) -> Result<(), MessagingError> {
        self.publish_channel
            .queue_bind(
                queue.into(),
                exchange.into(),
                routing_key.into(),
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await?;
        debug!(
            queue = queue,
            exchange = exchange,
            routing_key = routing_key,
            "Queue bound to exchange"
        );
        Ok(())
    }

    fn build_properties(message: &Message) -> BasicProperties {
        let mut props = BasicProperties::default()
            .with_message_id(message.id.clone().into())
            .with_timestamp(message.timestamp.timestamp() as u64);

        if let Some(ref content_type) = message.content_type {
            props = props.with_content_type(content_type.clone().into());
        }

        if let Some(ref correlation_id) = message.correlation_id {
            props = props.with_correlation_id(correlation_id.clone().into());
        }

        if let Some(ref reply_to) = message.reply_to {
            props = props.with_reply_to(reply_to.clone().into());
        }

        if let Some(priority) = message.priority {
            props = props.with_priority(priority);
        }

        if let Some(ttl) = message.ttl {
            props = props.with_expiration(ttl.to_string().into());
        }

        // Add headers
        if !message.headers.is_empty() {
            let mut headers = FieldTable::default();
            for (key, value) in &message.headers {
                headers.insert(
                    key.clone().into(),
                    lapin::types::AMQPValue::LongString(value.clone().into()),
                );
            }
            props = props.with_headers(headers);
        }

        props
    }
}

#[async_trait]
impl MessageBroker for RabbitMqBroker {
    type Subscription = RabbitMqSubscription;

    async fn publish(&self, message: Message) -> Result<(), MessagingError> {
        self.publish_with_options(message, PublishOptions::default())
            .await
    }

    async fn publish_with_options(
        &self,
        message: Message,
        options: PublishOptions,
    ) -> Result<(), MessagingError> {
        let exchange = options.exchange.as_deref().unwrap_or("");
        let routing_key = options.routing_key.as_deref().unwrap_or(&message.topic);

        let mut props = Self::build_properties(&message);

        if options.persistent {
            props = props.with_delivery_mode(2);
        }

        debug!(
            exchange = exchange,
            routing_key = routing_key,
            message_id = %message.id,
            "Publishing message"
        );

        let confirm = self
            .publish_channel
            .basic_publish(
                exchange.into(),
                routing_key.into(),
                BasicPublishOptions::default(),
                &message.payload,
                props,
            )
            .await?;

        if options.confirm {
            confirm.await.map_err(|e| {
                error!(error = %e, "Publisher confirm failed");
                MessagingError::Publish(format!("Publisher confirm failed: {}", e))
            })?;
        }

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
        let channel = self.connection.create_channel().await?;

        // Set prefetch count if specified
        if let Some(prefetch) = options.prefetch_count {
            channel
                .basic_qos(prefetch, BasicQosOptions::default())
                .await?;
        }

        // Declare the queue
        channel
            .queue_declare(
                topic.into(),
                QueueDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await?;

        let consumer_tag = options
            .consumer_group
            .unwrap_or_else(|| format!("armature-{}", uuid::Uuid::new_v4()));

        let consumer = channel
            .basic_consume(
                topic.into(),
                consumer_tag.as_str().into(),
                BasicConsumeOptions {
                    no_ack: options.ack_mode == AckMode::None,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await?;

        let active = Arc::new(AtomicBool::new(true));
        let subscription = RabbitMqSubscription {
            topic: topic.to_string(),
            consumer_tag: consumer_tag.clone(),
            channel: channel.clone(),
            active: active.clone(),
        };

        // Store channel for cleanup
        self.channels.write().await.push(channel.clone());

        // Spawn consumer task
        let topic_owned = topic.to_string();
        let ack_mode = options.ack_mode;
        tokio::spawn(async move {
            consume_messages(consumer, handler, channel, &topic_owned, ack_mode, active).await;
        });

        info!(queue = topic, consumer_tag = %consumer_tag, "Subscribed to queue");
        Ok(subscription)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst) && self.connection.status().connected()
    }

    async fn close(&self) -> Result<(), MessagingError> {
        info!("Closing RabbitMQ connection");
        self.connected.store(false, Ordering::SeqCst);

        // Close all channels
        let channels = self.channels.read().await;
        for channel in channels.iter() {
            if let Err(e) = channel.close(200, "Normal shutdown".into()).await {
                warn!(error = %e, "Error closing channel");
            }
        }

        // Close connection
        self.connection
            .close(200, "Normal shutdown".into())
            .await
            .map_err(|e| MessagingError::Connection(e.to_string()))?;

        Ok(())
    }
}

async fn consume_messages(
    mut consumer: Consumer,
    handler: Arc<dyn MessageHandler>,
    channel: Channel,
    topic: &str,
    ack_mode: AckMode,
    active: Arc<AtomicBool>,
) {
    while active.load(Ordering::SeqCst) {
        match consumer.next().await {
            Some(Ok(delivery)) => {
                let message = delivery_to_message(&delivery, topic);
                let delivery_tag = delivery.delivery_tag;

                match handler.handle(message).await {
                    Ok(result) => {
                        if ack_mode == AckMode::Auto || ack_mode == AckMode::Manual {
                            match result {
                                ProcessingResult::Success => {
                                    if let Err(e) = channel
                                        .basic_ack(delivery_tag, BasicAckOptions::default())
                                        .await
                                    {
                                        error!(error = %e, "Failed to ack message");
                                    }
                                }
                                ProcessingResult::Retry => {
                                    if let Err(e) = channel
                                        .basic_nack(
                                            delivery_tag,
                                            BasicNackOptions {
                                                requeue: true,
                                                ..Default::default()
                                            },
                                        )
                                        .await
                                    {
                                        error!(error = %e, "Failed to nack message for retry");
                                    }
                                }
                                ProcessingResult::DeadLetter | ProcessingResult::Reject => {
                                    if let Err(e) = channel
                                        .basic_reject(
                                            delivery_tag,
                                            BasicRejectOptions { requeue: false },
                                        )
                                        .await
                                    {
                                        error!(error = %e, "Failed to reject message");
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Message handler error");
                        if ack_mode != AckMode::None {
                            let _ = channel
                                .basic_nack(
                                    delivery_tag,
                                    BasicNackOptions {
                                        requeue: true,
                                        ..Default::default()
                                    },
                                )
                                .await;
                        }
                    }
                }
            }
            Some(Err(e)) => {
                error!(error = %e, "Consumer error");
                break;
            }
            None => {
                debug!("Consumer stream ended");
                break;
            }
        }
    }
}

fn delivery_to_message(delivery: &lapin::message::Delivery, topic: &str) -> Message {
    let props = &delivery.properties;
    let mut headers = HashMap::new();

    if let Some(amqp_headers) = props.headers() {
        for (key, value) in amqp_headers.inner() {
            if let lapin::types::AMQPValue::LongString(s) = value {
                headers.insert(key.to_string(), s.to_string());
            }
        }
    }

    Message {
        id: props
            .message_id()
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        payload: delivery.data.clone(),
        headers,
        topic: topic.to_string(),
        timestamp: props
            .timestamp()
            .map(|ts| {
                chrono::DateTime::from_timestamp(ts as i64, 0).unwrap_or_else(chrono::Utc::now)
            })
            .unwrap_or_else(chrono::Utc::now),
        correlation_id: props.correlation_id().as_ref().map(|s| s.to_string()),
        reply_to: props.reply_to().as_ref().map(|s| s.to_string()),
        content_type: props.content_type().as_ref().map(|s| s.to_string()),
        priority: *props.priority(),
        ttl: props
            .expiration()
            .as_ref()
            .and_then(|s| s.to_string().parse().ok()),
    }
}

/// RabbitMQ subscription handle
pub struct RabbitMqSubscription {
    topic: String,
    consumer_tag: String,
    channel: Channel,
    active: Arc<AtomicBool>,
}

#[async_trait]
impl Subscription for RabbitMqSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        self.channel
            .basic_cancel(
                self.consumer_tag.as_str().into(),
                BasicCancelOptions::default(),
            )
            .await?;
        info!(consumer_tag = %self.consumer_tag, "Unsubscribed from queue");
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    fn topic(&self) -> &str {
        &self.topic
    }
}
