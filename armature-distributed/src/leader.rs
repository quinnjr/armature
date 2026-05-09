//! Distributed leader election using Redis

use redis::AsyncCommands;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Leader election errors
#[derive(Debug, Error)]
pub enum LeaderError {
    #[error("Election failed: {0}")]
    ElectionFailed(String),

    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),

    #[error("Not the leader")]
    NotLeader,
}

/// Leader election callback
pub type LeaderCallback =
    Arc<dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send + Sync>;

/// Leader election coordinator
pub struct LeaderElection {
    /// Election key in Redis
    key: String,

    /// Unique node ID
    node_id: String,

    /// TTL for leadership
    ttl: Duration,

    /// Refresh interval (should be less than TTL)
    refresh_interval: Duration,

    /// Redis connection
    conn: Arc<RwLock<redis::aio::ConnectionManager>>,

    /// Is this node the leader?
    is_leader: Arc<AtomicBool>,

    /// Callback when becoming leader
    on_elected: Option<LeaderCallback>,

    /// Callback when losing leadership
    on_revoked: Option<LeaderCallback>,

    /// Running flag
    running: Arc<AtomicBool>,
}

impl LeaderElection {
    /// Create new leader election coordinator
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_distributed::LeaderElection;
    /// use std::time::Duration;
    ///
    /// let client = redis::Client::open("redis://127.0.0.1/")?;
    /// let conn = client.get_connection_manager().await?;
    ///
    /// let election = LeaderElection::new(
    ///     "my-service-leader",
    ///     Duration::from_secs(30),
    ///     conn,
    /// );
    /// ```
    pub fn new(key: impl Into<String>, ttl: Duration, conn: redis::aio::ConnectionManager) -> Self {
        let refresh_interval = Duration::from_millis((ttl.as_millis() / 3) as u64);

        Self {
            key: key.into(),
            node_id: Uuid::new_v4().to_string(),
            ttl,
            refresh_interval,
            conn: Arc::new(RwLock::new(conn)),
            is_leader: Arc::new(AtomicBool::new(false)),
            on_elected: None,
            on_revoked: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set callback for when this node becomes leader
    pub fn on_elected<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_elected = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Set callback for when this node loses leadership
    pub fn on_revoked<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_revoked = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Check if this node is the leader
    pub fn is_leader(&self) -> bool {
        self.is_leader.load(Ordering::Acquire)
    }

    /// Get the node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Start participating in leader election
    pub async fn start(self: Arc<Self>) -> Result<(), LeaderError> {
        self.running.store(true, Ordering::Release);

        info!(
            "Starting leader election for key: {} (node: {})",
            self.key, self.node_id
        );

        loop {
            if !self.running.load(Ordering::Acquire) {
                break;
            }

            // Try to become leader
            match self.try_become_leader().await {
                Ok(became_leader) => {
                    let was_leader = self.is_leader.load(Ordering::Acquire);

                    if became_leader && !was_leader {
                        // Newly elected
                        self.is_leader.store(true, Ordering::Release);
                        info!("Node {} became leader for {}", self.node_id, self.key);

                        if let Some(callback) = &self.on_elected {
                            callback().await;
                        }
                    } else if !became_leader && was_leader {
                        // Lost leadership
                        self.is_leader.store(false, Ordering::Release);
                        warn!("Node {} lost leadership for {}", self.node_id, self.key);

                        if let Some(callback) = &self.on_revoked {
                            callback().await;
                        }
                    } else if became_leader {
                        // Still leader, just refreshed
                        debug!(
                            "Node {} refreshed leadership for {}",
                            self.node_id, self.key
                        );
                    }
                }
                Err(e) => {
                    error!("Leader election error: {}", e);

                    // If we were leader but encountered an error, we're no longer leader
                    if self.is_leader.swap(false, Ordering::Release)
                        && let Some(callback) = &self.on_revoked
                    {
                        callback().await;
                    }
                }
            }

            // Wait before next attempt
            tokio::time::sleep(self.refresh_interval).await;
        }

        // Clean up on stop
        if self.is_leader.load(Ordering::Acquire) {
            let _ = self.resign().await;
        }

        Ok(())
    }

    /// Stop participating in leader election
    pub async fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    /// Try to become or maintain leadership
    async fn try_become_leader(&self) -> Result<bool, LeaderError> {
        let mut conn = self.conn.write().await;
        let ttl_ms = self.ttl.as_millis() as usize;

        // Use Lua script for atomic operation
        let script = r#"
            local current = redis.call("get", KEYS[1])
            if current == false or current == ARGV[1] then
                redis.call("set", KEYS[1], ARGV[1], "PX", ARGV[2])
                return 1
            else
                return 0
            end
        "#;

        let result: i32 = redis::Script::new(script)
            .key(&self.key)
            .arg(&self.node_id)
            .arg(ttl_ms)
            .invoke_async(&mut *conn)
            .await?;

        Ok(result == 1)
    }

    /// Resign from leadership
    async fn resign(&self) -> Result<(), LeaderError> {
        let mut conn = self.conn.write().await;

        // Only delete if we're still the leader
        let script = r#"
            if redis.call("get", KEYS[1]) == ARGV[1] then
                return redis.call("del", KEYS[1])
            else
                return 0
            end
        "#;

        let _: i32 = redis::Script::new(script)
            .key(&self.key)
            .arg(&self.node_id)
            .invoke_async(&mut *conn)
            .await?;

        self.is_leader.store(false, Ordering::Release);
        info!("Node {} resigned from leadership", self.node_id);

        Ok(())
    }

    /// Get current leader node ID
    pub async fn get_leader(&self) -> Result<Option<String>, LeaderError> {
        let mut conn = self.conn.write().await;
        let leader: Option<String> = conn.get(&self.key).await?;
        Ok(leader)
    }
}

/// Leader election builder
pub struct LeaderElectionBuilder {
    key: String,
    ttl: Duration,
    on_elected: Option<LeaderCallback>,
    on_revoked: Option<LeaderCallback>,
}

impl LeaderElectionBuilder {
    /// Create new builder
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ttl: Duration::from_secs(30),
            on_elected: None,
            on_revoked: None,
        }
    }

    /// Set TTL
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Set elected callback
    pub fn on_elected<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_elected = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Set revoked callback
    pub fn on_revoked<F, Fut>(mut self, callback: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.on_revoked = Some(Arc::new(move || Box::pin(callback())));
        self
    }

    /// Build the leader election coordinator
    pub fn build(self, conn: redis::aio::ConnectionManager) -> LeaderElection {
        let mut election = LeaderElection::new(self.key, self.ttl, conn);
        election.on_elected = self.on_elected;
        election.on_revoked = self.on_revoked;
        election
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leader_election_builder() {
        let builder = LeaderElectionBuilder::new("test-leader").with_ttl(Duration::from_secs(60));

        assert_eq!(builder.key, "test-leader");
        assert_eq!(builder.ttl, Duration::from_secs(60));
    }
}
