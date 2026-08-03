//! NATS message broker implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_nats::Client;
use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::{Mutex, RwLock, Semaphore};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};

use crate::{
    Message, MessageBroker, MessageHandler, MessagingConfig, MessagingError, ProcessingResult,
    PublishOptions, SubscribeOptions, Subscription, config::NatsConfig, dispatch,
};

/// Default timeout applied to the initial JetStream publish send itself
/// (separate from - and always applied regardless of - the optional
/// publish-acknowledgment timeout controlled by `PublishOptions`). Used only
/// when the caller didn't set `PublishOptions::timeout`, so a
/// blocked/backpressured send is never left completely unbounded just
/// because `confirm` wasn't requested.
const DEFAULT_JETSTREAM_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// One subscription's stop-flag and shared handler-task registry, tracked by
/// [`NatsBroker`] so `close` can stop every consumer and drain its in-flight
/// handler tasks.
struct ConsumerHandle {
    active: Arc<AtomicBool>,
    tasks: Arc<Mutex<JoinSet<()>>>,
}

/// NATS message broker
pub struct NatsBroker {
    client: Client,
    config: NatsConfig,
    /// JetStream context, present when `NatsConfig::jetstream` is enabled.
    jetstream: Option<async_nats::jetstream::Context>,
    active_consumers: Arc<RwLock<Vec<ConsumerHandle>>>,
    connected: Arc<AtomicBool>,
}

impl NatsBroker {
    /// Connect to NATS
    pub async fn connect(config: &MessagingConfig) -> Result<Self, MessagingError> {
        let nats_config = NatsConfig {
            base: config.clone(),
            ..Default::default()
        };
        Self::connect_with_config(nats_config).await
    }

    /// Connect with NATS-specific configuration
    pub async fn connect_with_config(config: NatsConfig) -> Result<Self, MessagingError> {
        info!(url = %config.base.url, "Connecting to NATS");

        let mut options = async_nats::ConnectOptions::new();

        if let Some(ref name) = config.name {
            options = options.name(name);
        }

        if let Some(ref username) = config.base.username
            && let Some(ref password) = config.base.password
        {
            options = options.user_and_password(username.clone(), password.clone());
        }

        if let Some(ref creds_file) = config.credentials_file {
            options = options
                .credentials_file(creds_file)
                .await
                .map_err(|e| MessagingError::Configuration(e.to_string()))?;
        }

        // JWT + NKey authentication: sign the server-issued nonce with the
        // NKey seed's key pair, as required by NATS decentralized auth.
        match (&config.jwt, &config.nkey_seed) {
            (Some(jwt), Some(nkey_seed)) => {
                let key_pair = Arc::new(nkeys::KeyPair::from_seed(nkey_seed).map_err(|e| {
                    MessagingError::Configuration(format!("invalid NATS NKey seed: {e}"))
                })?);
                let jwt = jwt.clone();
                options = options.jwt(jwt, move |nonce: Vec<u8>| {
                    let key_pair = key_pair.clone();
                    async move { key_pair.sign(&nonce).map_err(async_nats::AuthError::new) }
                });
            }
            (Some(_), None) | (None, Some(_)) => {
                warn!(
                    "NATS JWT+NKey authentication requires both `jwt` and `nkey_seed` to be set; ignoring the partial configuration"
                );
            }
            (None, None) => {}
        }

        // Reconnection tuning
        options = options.max_reconnects(config.max_reconnects);
        let reconnect_wait = config.reconnect_wait;
        options = options.reconnect_delay_callback(move |_attempts| reconnect_wait);

        let client = options
            .connect(&config.base.url)
            .await
            .map_err(MessagingError::from)?;

        // JetStream context, if enabled
        let jetstream = if config.jetstream {
            Some(async_nats::jetstream::new(client.clone()))
        } else {
            None
        };

        info!("Connected to NATS successfully");

        Ok(Self {
            client,
            config,
            jetstream,
            active_consumers: Arc::new(RwLock::new(Vec::new())),
            connected: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Get the JetStream context, if `NatsConfig::jetstream` was enabled when
    /// this broker was created.
    pub fn jetstream(&self) -> Option<&async_nats::jetstream::Context> {
        self.jetstream.as_ref()
    }

    /// Get the NATS-specific configuration this broker was created with.
    pub fn config(&self) -> &NatsConfig {
        &self.config
    }

    fn build_headers(message: &Message) -> async_nats::HeaderMap {
        let mut headers = async_nats::HeaderMap::new();

        headers.insert("Nats-Msg-Id", message.id.as_str());
        headers.insert("timestamp", message.timestamp.to_rfc3339().as_str());

        if let Some(ref correlation_id) = message.correlation_id {
            headers.insert("correlation-id", correlation_id.as_str());
        }

        if let Some(ref content_type) = message.content_type {
            headers.insert("content-type", content_type.as_str());
        }

        if let Some(ref reply_to) = message.reply_to {
            headers.insert("reply-to", reply_to.as_str());
        }

        for (key, value) in &message.headers {
            headers.insert(key.as_str(), value.as_str());
        }

        headers
    }
}

#[async_trait]
impl MessageBroker for NatsBroker {
    type Subscription = NatsSubscription;

    async fn publish(&self, message: Message) -> Result<(), MessagingError> {
        self.publish_with_options(message, PublishOptions::default())
            .await
    }

    async fn publish_with_options(
        &self,
        message: Message,
        options: PublishOptions,
    ) -> Result<(), MessagingError> {
        let subject = &message.topic;
        let headers = Self::build_headers(&message);

        debug!(subject = subject, message_id = %message.id, "Publishing message to NATS");

        // `persistent`/`routing_key`/`exchange`/`partition_key` have no
        // equivalent to apply on either path below: core NATS `publish` is a
        // fire-and-forget send over a subject with no broker-side exchange,
        // routing key, or partition concept, and JetStream's persistence is
        // inherent to the stream (not a per-publish flag) with routing done
        // by subject alone.
        if let Some(ref js) = self.jetstream {
            // Route through JetStream: the send happens synchronously inside
            // `publish_with_headers` (it only returns a `PublishAckFuture`
            // for the *acknowledgment*), so the message reaches the stream
            // regardless of whether `confirm` is set below. This initial
            // send is itself bounded by a timeout - not just the optional
            // ack wait further down - so a blocked/backpressured send can't
            // hang forever when the caller set `confirm: false` (and so has
            // no protection from the ack-wait timeout below at all).
            // `PublishOptions::timeout` is reused when set; otherwise falls
            // back to `DEFAULT_JETSTREAM_SEND_TIMEOUT`.
            let send_timeout = options.timeout.unwrap_or(DEFAULT_JETSTREAM_SEND_TIMEOUT);
            let ack_future = tokio::time::timeout(
                send_timeout,
                js.publish_with_headers(subject.clone(), headers, message.payload.into()),
            )
            .await
            .map_err(|_| {
                MessagingError::Publish("Timed out sending message to NATS JetStream".to_string())
            })?
            .map_err(|e| MessagingError::Publish(e.to_string()))?;

            // `confirm`/`timeout` mirror the core-NATS branch below, except
            // the JetStream ack is a stronger guarantee: it confirms the
            // broker durably stored the message on the stream, not just
            // that it left the client's outbound buffer.
            if options.confirm {
                match options.timeout {
                    Some(timeout) => tokio::time::timeout(timeout, ack_future)
                        .await
                        .map_err(|_| {
                            MessagingError::Publish(
                                "Timed out waiting for NATS JetStream publish acknowledgment"
                                    .to_string(),
                            )
                        })?
                        .map_err(|e| MessagingError::Publish(e.to_string()))?,
                    None => ack_future
                        .await
                        .map_err(|e| MessagingError::Publish(e.to_string()))?,
                };
            }

            return Ok(());
        }

        self.client
            .publish_with_headers(subject.clone(), headers, message.payload.into())
            .await
            .map_err(MessagingError::from)?;

        // `confirm`/`timeout` are approximated by flushing the client's
        // outbound buffer, which is the closest thing core NATS offers to a
        // publish acknowledgement.
        if options.confirm {
            let flush = self.client.flush();
            match options.timeout {
                Some(timeout) => tokio::time::timeout(timeout, flush)
                    .await
                    .map_err(|_| {
                        MessagingError::Publish(
                            "Timed out waiting for NATS publish confirmation".to_string(),
                        )
                    })?
                    .map_err(|e| MessagingError::Publish(e.to_string()))?,
                None => flush
                    .await
                    .map_err(|e| MessagingError::Publish(e.to_string()))?,
            }
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
        // Core NATS pub/sub has no acknowledgment at all, and this backend has
        // never read `ack_mode`. Rejecting up front is the only way a caller
        // asking for manual acknowledgment learns they will not get it.
        crate::reject_manual_ack("NATS", options.ack_mode)?;
        crate::reject_filter("NATS", options.filter.as_ref())?;

        // Bounds how many per-message handler invocations may run
        // concurrently (see `consume_messages`). Defaults to 1, which
        // reproduces the previous strictly-sequential dispatch. Note this
        // always uses core NATS pub/sub, even when `NatsConfig::jetstream`
        // is enabled - see the doc comment on that field for why
        // subscriptions don't yet route through JetStream.
        let concurrency = dispatch::concurrency_or_default(options.concurrency);

        if self.jetstream.is_some() {
            warn!(
                subject = topic,
                "NatsConfig::jetstream is enabled but subscribe() still uses core NATS \
                 pub/sub with no persistence or redelivery guarantees; use \
                 NatsBroker::jetstream() to drive the async-nats JetStream consumer APIs \
                 directly for JetStream-backed consumption"
            );
        }

        let subscriber = if let Some(ref group) = options.consumer_group {
            // Queue group subscription for load balancing
            self.client
                .queue_subscribe(topic.to_string(), group.clone())
                .await
                .map_err(MessagingError::from)?
        } else {
            self.client
                .subscribe(topic.to_string())
                .await
                .map_err(MessagingError::from)?
        };

        let active = Arc::new(AtomicBool::new(true));
        let tasks = Arc::new(Mutex::new(JoinSet::new()));

        let subscription = NatsSubscription {
            topic: topic.to_string(),
            active: active.clone(),
            tasks: tasks.clone(),
        };

        // Store active flag + task registry for cleanup, pruning entries
        // whose consumer task has already stopped so the vec does not grow
        // unbounded.
        {
            let mut consumers = self.active_consumers.write().await;
            consumers.retain(|c| c.active.load(Ordering::SeqCst));
            consumers.push(ConsumerHandle {
                active: active.clone(),
                tasks: tasks.clone(),
            });
        }

        // Spawn consumer task
        let topic_owned = topic.to_string();
        tokio::spawn(async move {
            consume_messages(
                subscriber,
                handler,
                &topic_owned,
                active,
                tasks,
                concurrency,
            )
            .await;
        });

        info!(subject = topic, "Subscribed to NATS subject");
        Ok(subscription)
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<(), MessagingError> {
        info!("Closing NATS connection");
        self.connected.store(false, Ordering::SeqCst);

        // Stop all subscriptions by setting active flags to false first, then
        // drain each's in-flight handler tasks - draining one consumer must
        // not delay flipping the flag for the others.
        let consumers = self.active_consumers.read().await;
        for consumer in consumers.iter() {
            consumer.active.store(false, Ordering::SeqCst);
        }
        for consumer in consumers.iter() {
            dispatch::drain_with_timeout(&consumer.tasks, dispatch::DEFAULT_DRAIN_TIMEOUT, "nats")
                .await;
        }

        // Flush and close client
        self.client
            .flush()
            .await
            .map_err(|e| MessagingError::Connection(e.to_string()))?;

        Ok(())
    }
}

async fn consume_messages(
    mut subscriber: async_nats::Subscriber,
    handler: Arc<dyn MessageHandler>,
    topic: &str,
    active: Arc<AtomicBool>,
    tasks: Arc<Mutex<JoinSet<()>>>,
    concurrency: usize,
) {
    // Bounds how many per-message handler invocations may run concurrently.
    // A permit is acquired before spawning each message's task (tracked in
    // `tasks` so `unsubscribe` can drain outstanding handlers on shutdown -
    // see `dispatch::spawn_bounded`) and released when that task finishes, so
    // at most `concurrency` handlers are ever in flight at once. `concurrency
    // == 1` reproduces the previous strictly sequential dispatch. Core NATS
    // has no per-message acknowledgment, so unlike Kafka there is no
    // ordering constraint on completion order.
    let semaphore = Arc::new(Semaphore::new(concurrency));

    while active.load(Ordering::SeqCst) {
        match subscriber.next().await {
            Some(nats_msg) => {
                let message = nats_message_to_message(&nats_msg, topic);
                let handler = handler.clone();

                dispatch::spawn_bounded(&semaphore, &tasks, async move {
                    handle_nats_message(&handler, message).await;
                })
                .await;
            }
            None => {
                debug!("Subscriber stream ended");
                // Flip `active` false right as the loop is genuinely about to
                // exit for good (a stream error isn't possible here - core
                // NATS subscribers only ever yield `None` once, when the
                // subscription is unsubscribed/the connection is closed), so
                // `is_active()` stops reporting `true` once the background
                // task has actually died. Matches RabbitMQ's/Kafka's consume
                // loops, which do the same on their own terminal exits.
                active.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
}

/// Run the handler for a single NATS message. Broken out of
/// `consume_messages` so it can be spawned as an independent per-message task
/// under the concurrency semaphore (mirrors `aws.rs`'s `handle_sqs_message`
/// and `rabbitmq.rs`'s `handle_delivery`).
async fn handle_nats_message(handler: &Arc<dyn MessageHandler>, message: Message) {
    match handler.handle(message).await {
        Ok(result) => match result {
            ProcessingResult::Success => {
                debug!("Message processed successfully");
            }
            ProcessingResult::Retry => {
                debug!("Message retry requested (NATS does not support built-in retry)");
            }
            ProcessingResult::DeadLetter | ProcessingResult::Reject => {
                debug!("Message rejected");
            }
        },
        Err(e) => {
            error!(error = %e, "Message handler error");
        }
    }
}

/// Header keys that carry reserved `Message` fields rather than user headers.
/// Mirrors the Kafka side's reserved-key handling so a custom header set via
/// `Message::with_header` survives a NATS publish/receive round trip instead
/// of being silently dropped.
const RESERVED_NATS_HEADERS: [&str; 5] = [
    "Nats-Msg-Id",
    "correlation-id",
    "content-type",
    "reply-to",
    "timestamp",
];

fn nats_message_to_message(nats_msg: &async_nats::Message, topic: &str) -> Message {
    let mut headers = HashMap::new();
    let mut message_id = None;
    let mut correlation_id = None;
    let mut content_type = None;
    let mut reply_to = None;
    let mut timestamp = chrono::Utc::now();

    if let Some(nats_headers) = nats_msg.headers.as_ref() {
        // Get specific headers we care about
        if let Some(value) = nats_headers.get("Nats-Msg-Id") {
            message_id = Some(AsRef::<str>::as_ref(&value).to_string());
        }
        if let Some(value) = nats_headers.get("correlation-id") {
            correlation_id = Some(AsRef::<str>::as_ref(&value).to_string());
        }
        if let Some(value) = nats_headers.get("content-type") {
            content_type = Some(AsRef::<str>::as_ref(&value).to_string());
        }
        if let Some(value) = nats_headers.get("reply-to") {
            reply_to = Some(AsRef::<str>::as_ref(&value).to_string());
        }
        if let Some(value) = nats_headers.get("timestamp") {
            let ts_str: &str = AsRef::<str>::as_ref(&value);
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                timestamp = ts.with_timezone(&chrono::Utc);
            }
        }

        // Preserve any remaining (non-reserved) headers, e.g. ones set via
        // `Message::with_header`/`with_headers` on the publish side.
        for (name, value) in nats_headers.iter() {
            let name_str: &str = AsRef::<str>::as_ref(name);
            if RESERVED_NATS_HEADERS.contains(&name_str) {
                continue;
            }
            if let Some(first_value) = value.iter().next() {
                headers.insert(
                    name_str.to_string(),
                    AsRef::<str>::as_ref(first_value).to_string(),
                );
            }
        }
    }

    // Use NATS reply if available
    if reply_to.is_none()
        && let Some(ref nats_reply) = nats_msg.reply
    {
        reply_to = Some(nats_reply.to_string());
    }

    Message {
        id: message_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        payload: nats_msg.payload.to_vec(),
        headers,
        topic: topic.to_string(),
        timestamp,
        correlation_id,
        reply_to,
        content_type,
        priority: None,
        ttl: None,
    }
}

/// NATS subscription handle
pub struct NatsSubscription {
    topic: String,
    active: Arc<AtomicBool>,
    /// In-flight per-message handler tasks, drained (with a bounded timeout)
    /// on `unsubscribe` instead of being abandoned.
    tasks: Arc<Mutex<JoinSet<()>>>,
}

#[async_trait]
impl Subscription for NatsSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        dispatch::drain_with_timeout(&self.tasks, dispatch::DEFAULT_DRAIN_TIMEOUT, "nats").await;
        info!(subject = %self.topic, "Unsubscribed from NATS subject");
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
    fn test_nats_config() {
        let config = NatsConfig::new("nats://localhost:4222")
            .with_name("test-client")
            .with_jetstream();

        assert_eq!(config.base.url, "nats://localhost:4222");
        assert_eq!(config.name, Some("test-client".to_string()));
        assert!(config.jetstream);
    }

    /// A custom header set via `Message::with_header` must survive a
    /// publish -> receive round trip, not just the reserved keys
    /// (`Nats-Msg-Id`, `correlation-id`, `content-type`, `reply-to`,
    /// `timestamp`).
    #[test]
    fn custom_header_survives_publish_receive_round_trip() {
        let message = Message::new("subject", b"payload".to_vec())
            .with_header("x-custom", "custom-value")
            .with_correlation_id("corr-1");

        let built_headers = NatsBroker::build_headers(&message);

        let nats_msg = async_nats::Message {
            subject: async_nats::Subject::from("subject"),
            reply: None,
            payload: message.payload.clone().into(),
            headers: Some(built_headers),
            status: None,
            description: None,
            length: 0,
        };

        let round_tripped = nats_message_to_message(&nats_msg, "subject");

        assert_eq!(
            round_tripped.headers.get("x-custom"),
            Some(&"custom-value".to_string()),
            "custom header must survive the round trip"
        );
        assert_eq!(round_tripped.correlation_id, Some("corr-1".to_string()));
        // Reserved keys must not leak into the generic headers map.
        assert!(!round_tripped.headers.contains_key("correlation-id"));
    }
}
