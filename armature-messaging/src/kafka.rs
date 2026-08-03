//! Apache Kafka message broker implementation

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::FuturesOrdered;
use rdkafka::Message as KafkaMessage;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{CommitMode, Consumer, StreamConsumer};
use rdkafka::message::{Header, Headers, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::{Offset, TopicPartitionList};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::{
    AckMode, Message, MessageBroker, MessageHandler, MessagingConfig, MessagingError,
    ProcessingResult, PublishOptions, SubscribeOptions, Subscription, config::KafkaConfig,
};

/// A live consumer subscription tracked by [`KafkaBroker`], keyed by
/// subscription id. Holds the consumer alive and the `active` flag used to
/// stop its polling task on `close`/`unsubscribe`.
struct ConsumerEntry {
    /// Kept alive so the spawned consume task's `Arc<StreamConsumer>` is not
    /// the only owner; dropped when the subscription is removed.
    _consumer: Arc<StreamConsumer>,
    /// Shared stop-flag polled by the consume loop.
    active: Arc<AtomicBool>,
}

/// Apache Kafka message broker
pub struct KafkaBroker {
    producer: FutureProducer,
    config: KafkaConfig,
    /// Active consumer subscriptions keyed by subscription id. Entries are
    /// inserted on `subscribe` and removed on `unsubscribe` (so the map does
    /// not grow unbounded), and every entry's `active` flag is flipped to
    /// `false` on `close` so the spawned consume tasks stop polling.
    consumers: Arc<RwLock<HashMap<String, ConsumerEntry>>>,
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
            consumers: Arc::new(RwLock::new(HashMap::new())),
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
        // Kafka honors `ack_mode` (see `derive_enable_auto_commit`) but has no
        // notion of a broker-side subscription filter, so a `filter` would be
        // silently dropped and the consumer would receive the whole topic.
        crate::reject_filter("Kafka", options.filter.as_ref())?;

        // Bounds how many per-message handler invocations may run
        // concurrently (see `consume_messages`). Defaults to 1, which
        // reproduces the previous strictly-sequential dispatch.
        let concurrency = options.concurrency.unwrap_or(1).max(1);

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
        let subscription_id = uuid::Uuid::new_v4().to_string();

        let subscription = KafkaSubscription {
            topic: topic.to_string(),
            group_id: group_id.clone(),
            active: active.clone(),
            id: subscription_id.clone(),
            consumers: self.consumers.clone(),
        };

        // Store consumer + stop-flag keyed by subscription id so `close` can
        // stop every consumer and `unsubscribe` can prune this one.
        self.consumers.write().await.insert(
            subscription_id,
            ConsumerEntry {
                _consumer: consumer.clone(),
                active: active.clone(),
            },
        );

        // Spawn consumer task
        let topic_owned = topic.to_string();
        let ack_mode = options.ack_mode;
        tokio::spawn(async move {
            consume_messages(
                consumer,
                handler,
                &topic_owned,
                ack_mode,
                active,
                concurrency,
            )
            .await;
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
        // Stop every consume task by flipping its stop-flag. Without this the
        // spawned tasks keep polling and invoking handlers after `close`.
        let consumers = self.consumers.read().await;
        for entry in consumers.values() {
            entry.active.store(false, Ordering::SeqCst);
        }
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

/// The outcome of dispatching one message to the handler, carrying enough
/// owned data (topic/partition/offset) to commit its offset without holding
/// on to the borrowed `rdkafka` message.
struct KafkaDispatchOutcome {
    topic: String,
    partition: i32,
    offset: i64,
    result: Result<ProcessingResult, MessagingError>,
}

async fn consume_messages(
    consumer: Arc<StreamConsumer>,
    handler: Arc<dyn MessageHandler>,
    topic: &str,
    ack_mode: AckMode,
    active: Arc<AtomicBool>,
    concurrency: usize,
) {
    use futures_util::StreamExt;

    let mut stream = consumer.stream();

    // Manual-ack at-least-once safety.
    //
    // In `AckMode::Manual` we commit offsets explicitly, only after the handler
    // reports `Success`. The hazard is that Kafka commits a *watermark*: a
    // single committed offset marks everything up to it as consumed. If message
    // N fails (non-`Success` result or handler error) but message N+1 then
    // succeeds, committing N+1 would advance the watermark past N, permanently
    // skipping the failed message on restart (silent data loss).
    //
    // To preserve at-least-once, once any message in this consumer's stream
    // fails to be acknowledged we stop committing entirely (`commit_halted`).
    // The failed message and everything after it are then redelivered when the
    // consumer group next starts from its last committed offset. This is a
    // deliberately coarse, safe choice: it may redeliver already-processed
    // messages (handlers must be idempotent), but it never skips one.
    let mut commit_halted = false;

    // Bounds how many handler invocations run concurrently. Handler futures
    // are pushed onto `in_flight` (capped at `concurrency` entries at a
    // time) and polled together, but `FuturesOrdered` always *yields* them
    // in push order regardless of which finishes first. That is essential
    // here: per rdkafka's own docs, `Consumer::commit`/`commit_message`
    // commit a per-partition *watermark* and must "only [be used] if you are
    // processing messages in order," so the `commit_halted` bookkeeping
    // below is applied strictly in dispatch order even though the handler
    // futures themselves run concurrently underneath.
    let mut in_flight: FuturesOrdered<
        std::pin::Pin<Box<dyn std::future::Future<Output = KafkaDispatchOutcome> + Send>>,
    > = FuturesOrdered::new();
    let mut stream_ended = false;

    loop {
        if in_flight.is_empty() && (stream_ended || !active.load(Ordering::SeqCst)) {
            // Flip `active` false right as the loop is genuinely about to
            // exit for good (not on a transient/recoverable path - a stream
            // error just logs and keeps looping above), so `is_active()`
            // stops reporting `true` once the background task has actually
            // died. Matches RabbitMQ's consume loop, which does the same on
            // its own stream-ended/fatal-error exits.
            active.store(false, Ordering::SeqCst);
            break;
        }

        let can_poll_stream =
            !stream_ended && active.load(Ordering::SeqCst) && in_flight.len() < concurrency;

        tokio::select! {
            item = stream.next(), if can_poll_stream => {
                match item {
                    Some(Ok(borrowed_message)) => {
                        let message = kafka_message_to_message(&borrowed_message, topic);
                        let msg_topic = borrowed_message.topic().to_string();
                        let partition = borrowed_message.partition();
                        let offset = borrowed_message.offset();
                        let handler = handler.clone();

                        in_flight.push_back(Box::pin(async move {
                            let result = handler.handle(message).await;
                            KafkaDispatchOutcome {
                                topic: msg_topic,
                                partition,
                                offset,
                                result,
                            }
                        }));
                    }
                    Some(Err(e)) => {
                        error!(error = %e, "Kafka consumer error");
                    }
                    None => {
                        debug!("Consumer stream ended");
                        stream_ended = true;
                    }
                }
            }
            outcome = in_flight.next(), if !in_flight.is_empty() => {
                if let Some(outcome) = outcome {
                    apply_kafka_outcome(consumer.as_ref(), ack_mode, &mut commit_halted, outcome).await;
                }
            }
        }
    }
}

/// Abstraction over "commit an offset," used so [`apply_kafka_outcome`]'s
/// dispatch-order guarantee can be unit-tested against a fake committer that
/// records the sequence of offsets it was asked to commit, instead of
/// requiring a live Kafka broker (`rdkafka::consumer::Consumer::commit`
/// talks to the broker). `StreamConsumer` implements this by delegating to
/// that real method; production code paths are unaffected.
trait KafkaCommit {
    fn commit(&self, tpl: &TopicPartitionList, mode: CommitMode)
    -> rdkafka::error::KafkaResult<()>;
}

impl KafkaCommit for StreamConsumer {
    fn commit(
        &self,
        tpl: &TopicPartitionList,
        mode: CommitMode,
    ) -> rdkafka::error::KafkaResult<()> {
        Consumer::commit(self, tpl, mode)
    }
}

/// Apply one message's handler outcome: ack/commit on success, or set
/// `commit_halted` on failure so later offsets stop being committed (see the
/// at-least-once safety comment in `consume_messages`). Called strictly in
/// original dispatch order, even when handlers ran concurrently.
async fn apply_kafka_outcome<C: KafkaCommit>(
    consumer: &C,
    ack_mode: AckMode,
    commit_halted: &mut bool,
    outcome: KafkaDispatchOutcome,
) {
    match outcome.result {
        Ok(ProcessingResult::Success) => {
            // Commit the offset explicitly in manual ack mode, since
            // `enable.auto.commit` is disabled for that mode and rdkafka
            // will never advance the consumer group's offset on its own,
            // causing redelivery on restart. Skip once a prior message has
            // failed (see `commit_halted` above).
            if ack_mode == AckMode::Manual && !*commit_halted {
                let mut tpl = TopicPartitionList::new();
                let add_result = tpl.add_partition_offset(
                    &outcome.topic,
                    outcome.partition,
                    Offset::Offset(outcome.offset + 1),
                );
                match add_result {
                    Ok(()) => {
                        if let Err(e) = consumer.commit(&tpl, CommitMode::Async) {
                            error!(error = %e, "Failed to commit Kafka offset");
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Failed to build Kafka commit offset");
                    }
                }
            }
        }
        Ok(ProcessingResult::Retry) => {
            if ack_mode == AckMode::Manual {
                *commit_halted = true;
                warn!(
                    "Kafka message not acknowledged (retry requested); halting further offset commits so it and later messages are redelivered on restart"
                );
            } else {
                warn!("Kafka does not support message retry - message will be lost");
            }
        }
        Ok(ProcessingResult::DeadLetter | ProcessingResult::Reject) => {
            if ack_mode == AckMode::Manual {
                *commit_halted = true;
                debug!(
                    "Message rejected; halting further Kafka offset commits to avoid skipping it"
                );
            } else {
                debug!("Message rejected");
            }
        }
        Err(e) => {
            if ack_mode == AckMode::Manual {
                *commit_halted = true;
            }
            error!(error = %e, "Message handler error");
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
    /// Id under which this subscription is tracked in `KafkaBroker::consumers`.
    id: String,
    /// Handle to the broker's consumer registry so `unsubscribe` can prune this
    /// subscription's entry (preventing unbounded growth of the map).
    consumers: Arc<RwLock<HashMap<String, ConsumerEntry>>>,
}

#[async_trait]
impl Subscription for KafkaSubscription {
    async fn unsubscribe(&self) -> Result<(), MessagingError> {
        self.active.store(false, Ordering::SeqCst);
        // Drop the tracked consumer so the map does not grow unbounded across
        // subscribe/unsubscribe cycles.
        self.consumers.write().await.remove(&self.id);
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
    use futures_util::StreamExt;
    use std::sync::Mutex as StdMutex;

    /// Records the sequence of offsets it is asked to commit, so tests can
    /// assert commit ordering without a live Kafka broker.
    #[derive(Default)]
    struct FakeCommitter {
        committed: StdMutex<Vec<i64>>,
    }

    impl KafkaCommit for FakeCommitter {
        fn commit(
            &self,
            tpl: &TopicPartitionList,
            _mode: CommitMode,
        ) -> rdkafka::error::KafkaResult<()> {
            for elem in tpl.elements() {
                if let Offset::Offset(offset) = elem.offset() {
                    self.committed.lock().unwrap().push(offset);
                }
            }
            Ok(())
        }
    }

    /// Regression test for the finding that Kafka's `concurrency > 1` path
    /// (handlers dispatched concurrently via `FuturesOrdered`) had zero test
    /// coverage of the ordering guarantee offset-commit correctness depends
    /// on. This hand-builds a `FuturesOrdered` exactly like
    /// `consume_messages` does, with handler completion order intentionally
    /// scrambled (message 0's handler is the slowest, so it finishes last
    /// even though it was dispatched first), and asserts both that
    /// `FuturesOrdered::next()` still yields outcomes in original dispatch
    /// order and that `apply_kafka_outcome` commits offsets in that same
    /// order.
    #[tokio::test]
    async fn concurrent_handlers_commit_offsets_in_original_dispatch_order() {
        let delays_ms = [300u64, 10, 100, 50];
        let mut in_flight: FuturesOrdered<
            std::pin::Pin<Box<dyn std::future::Future<Output = KafkaDispatchOutcome> + Send>>,
        > = FuturesOrdered::new();

        for (i, delay) in delays_ms.into_iter().enumerate() {
            let offset = i as i64;
            in_flight.push_back(Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                KafkaDispatchOutcome {
                    topic: "test-topic".to_string(),
                    partition: 0,
                    offset,
                    result: Ok(ProcessingResult::Success),
                }
            }));
        }

        let committer = FakeCommitter::default();
        let mut commit_halted = false;
        let mut observed_order = Vec::new();

        while let Some(outcome) = in_flight.next().await {
            observed_order.push(outcome.offset);
            apply_kafka_outcome(&committer, AckMode::Manual, &mut commit_halted, outcome).await;
        }

        assert_eq!(
            observed_order,
            vec![0, 1, 2, 3],
            "FuturesOrdered must yield outcomes in original dispatch order even when \
             handler completion order is scrambled"
        );
        assert_eq!(
            *committer.committed.lock().unwrap(),
            vec![1, 2, 3, 4],
            "commits must be applied strictly in dispatch order (offset+1 per outcome)"
        );
    }

    /// Companion to the above: an early message that fails (here, requests
    /// retry) must halt commits for *later* messages even when those later
    /// messages' handlers finish first. If completion order (rather than
    /// dispatch order) controlled commit application, message 1's offset
    /// would be committed before message 0's failure was observed, silently
    /// skipping message 0 on redelivery - exactly the data-loss hazard
    /// `commit_halted` exists to prevent.
    #[tokio::test]
    async fn early_failure_halts_commits_for_later_successful_messages_despite_finishing_first() {
        // Message 0 fails but is slow (finishes second); message 1 succeeds
        // but is fast (finishes first).
        let specs: Vec<(u64, Result<ProcessingResult, MessagingError>)> = vec![
            (200, Ok(ProcessingResult::Retry)),
            (10, Ok(ProcessingResult::Success)),
        ];

        let mut in_flight: FuturesOrdered<
            std::pin::Pin<Box<dyn std::future::Future<Output = KafkaDispatchOutcome> + Send>>,
        > = FuturesOrdered::new();

        for (i, (delay, result)) in specs.into_iter().enumerate() {
            let offset = i as i64;
            in_flight.push_back(Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                KafkaDispatchOutcome {
                    topic: "test-topic".to_string(),
                    partition: 0,
                    offset,
                    result,
                }
            }));
        }

        let committer = FakeCommitter::default();
        let mut commit_halted = false;

        while let Some(outcome) = in_flight.next().await {
            apply_kafka_outcome(&committer, AckMode::Manual, &mut commit_halted, outcome).await;
        }

        assert!(
            commit_halted,
            "the failed message must halt further commits"
        );
        assert!(
            committer.committed.lock().unwrap().is_empty(),
            "no offsets should be committed once an earlier-dispatched message failed, \
             even though the later message's handler finished first"
        );
    }

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
