//! RabbitMQ message broker implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures_util::StreamExt;
use lapin::{
    BasicProperties, Channel, Connection, ConnectionProperties, Consumer, options::*,
    types::FieldTable, uri::AMQPUri,
};
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::{
    AckMode, Message, MessageBroker, MessageHandler, MessagingConfig, MessagingError,
    ProcessingResult, PublishOptions, SubscribeOptions, Subscription, config::RabbitMqConfig,
    dispatch,
};

/// RabbitMQ message broker
pub struct RabbitMqBroker {
    connection: Arc<Connection>,
    /// Consumer channels keyed by subscription id. Inserted on `subscribe` and
    /// removed (and closed) on `unsubscribe`, so the map does not grow
    /// unbounded across subscribe/unsubscribe cycles.
    channels: Arc<RwLock<HashMap<String, Channel>>>,
    /// Channels pre-created at connect time per `RabbitMqConfig::channel_pool_size`,
    /// handed out by `subscribe_with_options` before falling back to creating a
    /// fresh channel. This avoids paying for an extra channel-open round trip
    /// on every subscription when the config asked for a warm pool.
    channel_pool: Arc<Mutex<Vec<Channel>>>,
    publish_channel: Channel,
    connected: Arc<AtomicBool>,
}

/// Parse `config.base.url` into an [`AMQPUri`] and apply `config.vhost`.
///
/// Broken out as a pure function so the vhost-application logic can be
/// unit-tested without needing a live broker connection.
///
/// The error deliberately does not include the parser's message: AMQP URLs
/// embed `user:pass@` credentials and the underlying `AMQPUri` parser echoes
/// the full input URL back in its error string (`Invalid URL: '<url>'`), so
/// forwarding it would leak the credentials into whatever logs the error.
fn build_amqp_uri(config: &RabbitMqConfig) -> Result<AMQPUri, MessagingError> {
    let mut uri: AMQPUri = config.base.url.parse().map_err(|_| {
        MessagingError::Configuration("invalid AMQP URL (could not be parsed)".to_string())
    })?;
    uri.vhost = config.vhost.clone();
    Ok(uri)
}

/// Build a [`RabbitMqConfig`] for the compat `connect(&MessagingConfig)` path,
/// seeding `vhost` from any vhost embedded in the URL (e.g.
/// `amqp://host/production` -> vhost `production`) instead of unconditionally
/// forcing the `"/"` default. Deliberate callers who want to override the vhost
/// use `connect_with_config` with an explicit `RabbitMqConfig::vhost`.
///
/// Broken out as a pure function so it can be unit-tested without a broker.
fn config_from_messaging(config: &MessagingConfig) -> Result<RabbitMqConfig, MessagingError> {
    let parsed: AMQPUri = config.url.parse().map_err(|_| {
        // See `build_amqp_uri`: the parser error echoes the raw URL, which
        // carries credentials, so it must not be forwarded.
        MessagingError::Configuration("invalid AMQP URL (could not be parsed)".to_string())
    })?;
    Ok(RabbitMqConfig {
        base: config.clone(),
        vhost: parsed.vhost,
        ..Default::default()
    })
}

impl RabbitMqBroker {
    /// Connect to RabbitMQ.
    ///
    /// Any vhost embedded in the URL is preserved (`amqp://host/production`
    /// connects to vhost `production`, not the `"/"` default). To set the vhost
    /// explicitly, use [`connect_with_config`](Self::connect_with_config).
    pub async fn connect(config: &MessagingConfig) -> Result<Self, MessagingError> {
        let rabbitmq_config = config_from_messaging(config)?;
        Self::connect_with_config(rabbitmq_config).await
    }

    /// Connect with RabbitMQ-specific configuration, honoring `vhost`,
    /// `publisher_confirms`, and `channel_pool_size`.
    pub async fn connect_with_config(config: RabbitMqConfig) -> Result<Self, MessagingError> {
        let uri = build_amqp_uri(&config)?;

        // Never log the raw URL: it carries `user:pass@` credentials. Log only
        // the non-secret host/port/vhost parsed out of it.
        info!(
            host = %uri.authority.host,
            port = uri.authority.port,
            vhost = %uri.vhost,
            "Connecting to RabbitMQ"
        );

        let connection = Connection::connect_uri(uri, ConnectionProperties::default()).await?;

        let publish_channel = connection.create_channel().await?;

        // Enable publisher confirms only if requested
        if config.publisher_confirms {
            publish_channel
                .confirm_select(ConfirmSelectOptions::default())
                .await?;
        }

        // Pre-warm a pool of channels for subscriptions to draw from (the
        // publish channel above counts toward the configured pool size).
        let pool_size = config.channel_pool_size.saturating_sub(1);
        let mut channel_pool = Vec::with_capacity(pool_size);
        for _ in 0..pool_size {
            channel_pool.push(connection.create_channel().await?);
        }

        info!("Connected to RabbitMQ successfully");

        Ok(Self {
            connection: Arc::new(connection),
            channels: Arc::new(RwLock::new(HashMap::new())),
            channel_pool: Arc::new(Mutex::new(channel_pool)),
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
        // Bounds how many per-message handler invocations may run
        // concurrently (see `consume_messages`). Defaults to 1, which
        // reproduces the previous strictly-sequential dispatch.
        let concurrency = dispatch::concurrency_or_default(options.concurrency);

        // Reject before a channel is drawn from the pool. RabbitMQ *does* have
        // real manual acknowledgment, but `Message` carries no delivery tag, so
        // a caller has no handle to ack with; this backend consequently treated
        // `Manual` exactly like `Auto`. Say so rather than acking on their
        // behalf.
        crate::reject_manual_ack("RabbitMQ", options.ack_mode)?;
        crate::reject_filter("RabbitMQ", options.filter.as_ref())?;

        // Draw a pre-warmed channel from the pool if one is available;
        // otherwise fall back to creating a fresh one on demand.
        let channel = {
            let mut pool = self.channel_pool.lock().await;
            match pool.pop() {
                Some(channel) => channel,
                None => self.connection.create_channel().await?,
            }
        };

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
        let tasks = Arc::new(Mutex::new(JoinSet::new()));
        let subscription_id = uuid::Uuid::new_v4().to_string();
        let subscription = RabbitMqSubscription {
            topic: topic.to_string(),
            consumer_tag: consumer_tag.clone(),
            channel: channel.clone(),
            active: active.clone(),
            tasks: tasks.clone(),
            id: subscription_id.clone(),
            channels: self.channels.clone(),
        };

        // Store channel keyed by subscription id so `unsubscribe` can prune and
        // close it (the map would otherwise grow unbounded).
        self.channels
            .write()
            .await
            .insert(subscription_id, channel.clone());

        // Spawn consumer task
        let topic_owned = topic.to_string();
        let ack_mode = options.ack_mode;
        let consume_config = ConsumeConfig {
            channel,
            ack_mode,
            concurrency,
        };
        tokio::spawn(async move {
            consume_messages(
                consumer,
                handler,
                &topic_owned,
                active,
                tasks,
                consume_config,
            )
            .await;
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
        for channel in channels.values() {
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

/// Bundles `consume_messages`' connection/policy parameters (as opposed to
/// the per-call `handler`/`topic`/`active`/`tasks` parameters) into one value
/// so the function stays under clippy's argument-count limit.
struct ConsumeConfig {
    channel: Channel,
    ack_mode: AckMode,
    concurrency: usize,
}

async fn consume_messages(
    mut consumer: Consumer,
    handler: Arc<dyn MessageHandler>,
    topic: &str,
    active: Arc<AtomicBool>,
    tasks: Arc<Mutex<JoinSet<()>>>,
    consume_config: ConsumeConfig,
) {
    let ConsumeConfig {
        channel,
        ack_mode,
        concurrency,
    } = consume_config;

    // Bounds how many per-message handler invocations may run concurrently.
    // A permit is acquired before spawning each message's task (tracked in
    // `tasks` so `unsubscribe` can drain outstanding handlers on shutdown -
    // see `dispatch::spawn_bounded`) and released when that task finishes, so
    // at most `concurrency` handlers are ever in flight at once. `concurrency
    // == 1` reproduces the previous strictly-sequential dispatch. Acking is
    // per-delivery-tag, so unlike Kafka's watermark offset commits,
    // acks/nacks/rejects completing out of dispatch order is safe.
    let semaphore = Arc::new(Semaphore::new(concurrency));

    while active.load(Ordering::SeqCst) {
        match consumer.next().await {
            Some(Ok(delivery)) => {
                let message = delivery_to_message(&delivery, topic);
                let delivery_tag = delivery.delivery_tag;
                let handler = handler.clone();
                let channel = channel.clone();

                dispatch::spawn_bounded(&semaphore, &tasks, async move {
                    handle_delivery(&handler, &channel, message, delivery_tag, ack_mode).await;
                })
                .await;
            }
            Some(Err(e)) => {
                if e.can_be_recovered() {
                    // Transient/recoverable error (per lapin's own
                    // `Error::can_be_recovered`, e.g. a channel-state hiccup
                    // that doesn't tear down the connection): log and keep
                    // consuming, mirroring Kafka's posture of not ending the
                    // loop for non-fatal stream errors.
                    warn!(error = %e, "Recoverable RabbitMQ consumer error, continuing to consume");
                    continue;
                }
                error!(error = %e, "Fatal RabbitMQ consumer error, stopping consumption");
                active.store(false, Ordering::SeqCst);
                break;
            }
            None => {
                debug!("Consumer stream ended");
                active.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
}

/// Run the handler for a single delivery and ack/nack/reject it based on the
/// result. Broken out of `consume_messages` so it can be spawned as an
/// independent per-message task under the concurrency semaphore.
async fn handle_delivery(
    handler: &Arc<dyn MessageHandler>,
    channel: &Channel,
    message: Message,
    delivery_tag: u64,
    ack_mode: AckMode,
) {
    match handler.handle(message).await {
        Ok(result) => {
            // `Manual` never reaches here: `subscribe_with_options` rejects it,
            // because there is no delivery tag on `Message` for a caller to ack
            // with and this branch would otherwise ack on their behalf while
            // claiming they were in control. `None` consumes with `no_ack`, so
            // the broker considers the delivery settled already.
            if ack_mode == AckMode::Auto {
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
                            .basic_reject(delivery_tag, BasicRejectOptions { requeue: false })
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
    /// In-flight per-message handler tasks, drained (with a bounded timeout)
    /// on `unsubscribe` instead of being abandoned.
    tasks: Arc<Mutex<JoinSet<()>>>,
    /// Id under which this subscription's channel is tracked in
    /// `RabbitMqBroker::channels`.
    id: String,
    /// Handle to the broker's channel registry so `unsubscribe` can prune this
    /// subscription's channel and keep the map bounded.
    channels: Arc<RwLock<HashMap<String, Channel>>>,
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

        // Drain in-flight handler tasks *before* closing the channel below -
        // `handle_delivery` still needs a live channel to ack/nack/reject
        // through while it finishes.
        dispatch::drain_with_timeout(&self.tasks, dispatch::DEFAULT_DRAIN_TIMEOUT, "rabbitmq")
            .await;

        // Drop the tracked channel from the registry and close it so neither
        // the map nor the broker's open-channel count grows unbounded.
        self.channels.write().await.remove(&self.id);
        if let Err(e) = self.channel.close(200, "Unsubscribed".into()).await {
            warn!(error = %e, "Error closing channel on unsubscribe");
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RabbitMqConfig;

    #[test]
    fn build_amqp_uri_applies_configured_vhost() {
        let config = RabbitMqConfig::new("amqp://localhost:5672").with_vhost("/my-vhost");
        let uri = build_amqp_uri(&config).unwrap();
        assert_eq!(uri.vhost, "/my-vhost");
    }

    #[test]
    fn build_amqp_uri_defaults_to_slash_vhost() {
        let config = RabbitMqConfig::new("amqp://localhost:5672");
        assert_eq!(config.vhost, "/");
        let uri = build_amqp_uri(&config).unwrap();
        assert_eq!(uri.vhost, "/");
    }

    #[test]
    fn build_amqp_uri_overrides_vhost_embedded_in_url() {
        // Even if the URL string already encodes a vhost path, the explicit
        // `RabbitMqConfig::vhost` must win, since that's the field users are
        // expected to configure.
        let config = RabbitMqConfig::new("amqp://localhost:5672/original-vhost")
            .with_vhost("override-vhost");
        let uri = build_amqp_uri(&config).unwrap();
        assert_eq!(uri.vhost, "override-vhost");
    }

    #[test]
    fn build_amqp_uri_rejects_invalid_url() {
        let config = RabbitMqConfig::new("not a valid amqp url");
        assert!(build_amqp_uri(&config).is_err());
    }

    #[test]
    fn compat_connect_preserves_url_embedded_vhost() {
        // The compat `connect(&MessagingConfig)` path must not clobber a vhost
        // embedded in the URL with the `"/"` default.
        let base = MessagingConfig::new("amqp://localhost:5672/production");
        let config = config_from_messaging(&base).unwrap();
        assert_eq!(config.vhost, "production");

        // And the final URI carries it through.
        let uri = build_amqp_uri(&config).unwrap();
        assert_eq!(uri.vhost, "production");
    }

    #[test]
    fn compat_connect_defaults_vhost_when_url_has_none() {
        let base = MessagingConfig::new("amqp://localhost:5672");
        let config = config_from_messaging(&base).unwrap();
        assert_eq!(config.vhost, "/");
    }

    #[test]
    fn build_amqp_uri_error_does_not_leak_credentials() {
        // A malformed-but-credential-bearing URL must not have its userinfo
        // echoed into the error message.
        let config = RabbitMqConfig::new("amqp://user:secretpass@ bad host/vh");
        let err = build_amqp_uri(&config).unwrap_err().to_string();
        assert!(
            !err.contains("secretpass"),
            "error message leaked credentials: {err}"
        );
    }
}
