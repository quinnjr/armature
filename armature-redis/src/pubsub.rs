//! Redis Pub/Sub support.

#![allow(dead_code)]

use futures::StreamExt;
use redis::Client;
use redis::aio::ConnectionManager;
use tokio::sync::{OnceCell, mpsc};
use tracing::{debug, error, info};

use crate::{RedisConfig, RedisError, Result};

/// A Redis Pub/Sub message.
#[derive(Debug, Clone)]
pub struct Message {
    /// Channel name.
    pub channel: String,
    /// Message payload.
    pub payload: String,
    /// Pattern (for pattern subscriptions).
    pub pattern: Option<String>,
}

/// A subscription handle.
pub struct Subscription {
    /// Receiver for messages.
    receiver: mpsc::Receiver<Message>,
    /// Channel name.
    channel: String,
}

impl Subscription {
    /// Create a new subscription.
    fn new(receiver: mpsc::Receiver<Message>, channel: String) -> Self {
        Self { receiver, channel }
    }

    /// Get the channel name.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Receive the next message.
    pub async fn recv(&mut self) -> Option<Message> {
        self.receiver.recv().await
    }

    /// Try to receive a message without blocking.
    pub fn try_recv(&mut self) -> Option<Message> {
        self.receiver.try_recv().ok()
    }
}

/// Redis Pub/Sub client.
pub struct PubSub {
    config: RedisConfig,
    client: Client,
    /// Lazily-built, cached `ConnectionManager` used by [`Self::publish`].
    ///
    /// `ConnectionManager` is cheap to `Clone` (clones share the same
    /// underlying connection state via an internal handle) and, unlike
    /// `redis::aio::MultiplexedConnection`, automatically reconnects after a
    /// transient connection failure. That auto-reconnect is why it is safe
    /// to build this once and cache it for the lifetime of the `PubSub`
    /// instance instead of opening a fresh connection on every `publish`
    /// call: a single dropped-connection blip self-heals instead of
    /// permanently breaking every subsequent `publish`. This mirrors how
    /// `RedisCache` in `armature-cache/src/redis_cache.rs` uses
    /// `ConnectionManager` for its long-lived connection.
    ///
    /// This is *not* the same pattern as `RedisService::dedicated_client` in
    /// `service.rs`: that caches only the `redis::Client` builder and still
    /// opens a brand-new connection on every `get_dedicated()` call.
    publish_conn: OnceCell<ConnectionManager>,
}

impl PubSub {
    /// Create a new Pub/Sub client.
    pub fn new(config: RedisConfig) -> Result<Self> {
        let url = config.connection_url();
        let client = Client::open(url).map_err(|e| RedisError::Connection(e.to_string()))?;
        Ok(Self {
            config,
            client,
            publish_conn: OnceCell::new(),
        })
    }

    /// Get or lazily build the cached, auto-reconnecting `ConnectionManager`
    /// used by `publish`.
    async fn publish_connection(&self) -> Result<ConnectionManager> {
        let conn = self
            .publish_conn
            .get_or_try_init(|| async {
                ConnectionManager::new(self.client.clone())
                    .await
                    .map_err(|e| RedisError::Connection(e.to_string()))
            })
            .await?;
        Ok(conn.clone())
    }

    /// Subscribe to a channel.
    pub async fn subscribe(&self, channel: &str) -> Result<Subscription> {
        let (tx, rx) = mpsc::channel(100);
        let channel_name = channel.to_string();

        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|e| RedisError::Connection(e.to_string()))?;

        pubsub
            .subscribe(&channel_name)
            .await
            .map_err(|e| RedisError::PubSub(e.to_string()))?;

        info!(channel = %channel_name, "Subscribed to Redis channel");

        // Spawn task to receive messages
        let channel_clone = channel_name.clone();
        tokio::spawn(async move {
            while let Some(msg) = pubsub.on_message().next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        error!(error = %e, "Failed to get message payload");
                        continue;
                    }
                };

                let message = Message {
                    channel: msg.get_channel_name().to_string(),
                    payload,
                    pattern: None,
                };

                debug!(channel = %message.channel, "Received pub/sub message");

                if tx.send(message).await.is_err() {
                    debug!(channel = %channel_clone, "Subscription receiver dropped");
                    break;
                }
            }
        });

        Ok(Subscription::new(rx, channel_name))
    }

    /// Subscribe to a pattern.
    pub async fn psubscribe(&self, pattern: &str) -> Result<Subscription> {
        let (tx, rx) = mpsc::channel(100);
        let pattern_str = pattern.to_string();

        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|e| RedisError::Connection(e.to_string()))?;

        pubsub
            .psubscribe(&pattern_str)
            .await
            .map_err(|e| RedisError::PubSub(e.to_string()))?;

        info!(pattern = %pattern_str, "Subscribed to Redis pattern");

        let pattern_clone = pattern_str.clone();
        tokio::spawn(async move {
            while let Some(msg) = pubsub.on_message().next().await {
                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        error!(error = %e, "Failed to get message payload");
                        continue;
                    }
                };

                let message = Message {
                    channel: msg.get_channel_name().to_string(),
                    payload,
                    pattern: Some(pattern_clone.clone()),
                };

                if tx.send(message).await.is_err() {
                    break;
                }
            }
        });

        Ok(Subscription::new(rx, pattern_str))
    }

    /// Publish a message to a channel.
    ///
    /// Reuses a single cached, auto-reconnecting [`ConnectionManager`]
    /// across calls (see [`Self::publish_connection`]) instead of
    /// establishing a new connection on every invocation.
    pub async fn publish(&self, channel: &str, message: &str) -> Result<u32> {
        let mut conn = self.publish_connection().await?;

        let receivers: u32 = redis::cmd("PUBLISH")
            .arg(channel)
            .arg(message)
            .query_async(&mut conn)
            .await
            .map_err(|e| RedisError::Command(e.to_string()))?;

        debug!(channel = %channel, receivers = receivers, "Published message");

        Ok(receivers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// End-to-end proof (requires Docker) that `PubSub::publish` reuses a
    /// single cached `ConnectionManager` across calls instead of opening a
    /// fresh connection every time.
    ///
    /// `CLIENT ID` returns the ID Redis assigned to the connection that
    /// issued the command. Against the pre-fix code (a fresh
    /// `get_multiplexed_async_connection()` call per `publish`), each call
    /// would observe a different ID; with the connection cached, repeated
    /// calls to `publish_connection` must observe the *same* ID.
    #[tokio::test]
    async fn publish_reuses_cached_connection_across_calls() {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let config = RedisConfig::builder().url(container.url()).build();
        let pubsub = PubSub::new(config).unwrap();

        let mut conn1 = pubsub.publish_connection().await.unwrap();
        let id1: i64 = redis::cmd("CLIENT")
            .arg("ID")
            .query_async(&mut conn1)
            .await
            .unwrap();

        let mut conn2 = pubsub.publish_connection().await.unwrap();
        let id2: i64 = redis::cmd("CLIENT")
            .arg("ID")
            .query_async(&mut conn2)
            .await
            .unwrap();

        assert_eq!(
            id1, id2,
            "publish_connection should return the same cached connection on repeated calls, \
             not open a new one each time"
        );
    }

    /// Poll `publish` on `channel` until it observes at least one
    /// subscriber (`receivers > 0`), or panic after `timeout`.
    ///
    /// `PubSub::subscribe` registers the `SUBSCRIBE` with Redis on a
    /// background task, so there is no guarantee it has completed by the
    /// time `subscribe` returns. A fixed `sleep` before the first publish is
    /// therefore inherently racy under CI load; polling instead only treats
    /// a zero-receiver result as a real failure once `timeout` elapses.
    /// While no subscriber is registered yet, `PUBLISH` simply reports zero
    /// receivers and delivers nothing, so these probe calls are harmless.
    async fn wait_for_subscriber(
        pubsub: &PubSub,
        channel: &str,
        message: &str,
        timeout: Duration,
    ) -> u32 {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let receivers = pubsub.publish(channel, message).await.unwrap();
            if receivers > 0 {
                return receivers;
            }
            if std::time::Instant::now() >= deadline {
                panic!("no subscriber registered for channel {channel:?} within {timeout:?}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// End-to-end proof (requires Docker) of the documented usage pattern
    /// (`service.pubsub()?.publish("channel", "Hello!").await?`): a
    /// subscriber registered via `subscribe` actually receives messages
    /// sent through `publish`, including across repeated `publish` calls
    /// now that they share a cached connection.
    #[tokio::test]
    async fn subscribe_then_publish_round_trips_message() {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let config = RedisConfig::builder().url(container.url()).build();
        let pubsub = PubSub::new(config).unwrap();

        let mut sub = pubsub.subscribe("wf-pubsub-test-channel").await.unwrap();

        // Poll until the background subscribe task has actually registered
        // with Redis, instead of guessing with a fixed sleep.
        let receivers = wait_for_subscriber(
            &pubsub,
            "wf-pubsub-test-channel",
            "Hello!",
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(
            receivers, 1,
            "one subscriber should have received the publish"
        );

        let msg = tokio::time::timeout(Duration::from_secs(5), sub.recv())
            .await
            .expect("timed out waiting for message")
            .expect("subscription closed unexpectedly");
        assert_eq!(msg.channel, "wf-pubsub-test-channel");
        assert_eq!(msg.payload, "Hello!");

        // A second publish must still work via the cached connection.
        let receivers2 = pubsub
            .publish("wf-pubsub-test-channel", "Hello again!")
            .await
            .unwrap();
        assert_eq!(receivers2, 1);

        let msg2 = tokio::time::timeout(Duration::from_secs(5), sub.recv())
            .await
            .expect("timed out waiting for second message")
            .expect("subscription closed unexpectedly");
        assert_eq!(msg2.channel, "wf-pubsub-test-channel");
        assert_eq!(msg2.payload, "Hello again!");
    }
}
