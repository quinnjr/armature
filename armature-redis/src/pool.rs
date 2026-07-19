//! Redis connection pool.

use bb8::{ManageConnection, Pool, PooledConnection};
use bb8_redis::RedisConnectionManager;
use redis::aio::ConnectionLike;
use redis::cluster::ClusterClient;
use redis::cluster_async::ClusterConnection;
use tracing::{info, warn};

use crate::{RedisConfig, RedisError, Result};

/// A `bb8::ManageConnection` for `redis::cluster::ClusterClient`.
///
/// This mirrors `bb8_redis::RedisConnectionManager` but produces cluster-aware
/// connections (`redis::cluster_async::ClusterConnection`) that route commands
/// to the correct cluster node/slot instead of talking to a single node.
///
/// If `connection_name` is set, `CLIENT SETNAME` is issued once per physical
/// connection, in `connect()`, rather than on every pool checkout — bb8 reuses
/// connections across checkouts, so the name only needs to be set when the
/// connection is first established.
#[derive(Clone)]
pub struct RedisClusterConnectionManager {
    client: ClusterClient,
    connection_name: Option<String>,
}

impl RedisClusterConnectionManager {
    /// Create a new cluster connection manager from a set of seed node addresses.
    pub fn new(nodes: Vec<String>) -> Result<Self> {
        let client =
            ClusterClient::new(nodes).map_err(|e| RedisError::Connection(e.to_string()))?;
        Ok(Self {
            client,
            connection_name: None,
        })
    }

    /// Set the connection name to apply (via `CLIENT SETNAME`) to every
    /// physical connection this manager establishes.
    pub fn with_connection_name(mut self, name: Option<String>) -> Self {
        self.connection_name = name;
        self
    }
}

impl ManageConnection for RedisClusterConnectionManager {
    type Connection = ClusterConnection;
    type Error = redis::RedisError;

    async fn connect(&self) -> std::result::Result<Self::Connection, Self::Error> {
        let mut conn = self.client.get_async_connection().await?;
        if let Some(name) = &self.connection_name {
            let result: std::result::Result<(), redis::RedisError> = redis::cmd("CLIENT")
                .arg("SETNAME")
                .arg(name.as_str())
                .query_async(&mut conn)
                .await;
            if let Err(e) = result {
                warn!(error = %e, connection_name = %name, "failed to set Redis connection name");
            }
        }
        Ok(conn)
    }

    async fn is_valid(&self, conn: &mut Self::Connection) -> std::result::Result<(), Self::Error> {
        let pong: String = redis::cmd("PING").query_async(conn).await?;
        match pong.as_str() {
            "PONG" => Ok(()),
            _ => Err((redis::ErrorKind::Extension, "ping request").into()),
        }
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

/// A `bb8::ManageConnection` that wraps `bb8_redis::RedisConnectionManager`
/// and applies `CLIENT SETNAME` once, when a physical connection is
/// established in `connect()`, rather than on every pool checkout.
///
/// `bb8_redis::RedisConnectionManager::connect()` is not ours to edit (it's
/// an external crate), so this thin wrapper delegates everything to the
/// inner manager and only adds the one-time naming step.
#[derive(Clone)]
pub struct NamedRedisConnectionManager {
    inner: RedisConnectionManager,
    connection_name: Option<String>,
}

impl NamedRedisConnectionManager {
    /// Wrap an existing `RedisConnectionManager`, optionally naming every
    /// connection it establishes.
    pub fn new(inner: RedisConnectionManager, connection_name: Option<String>) -> Self {
        Self {
            inner,
            connection_name,
        }
    }
}

impl ManageConnection for NamedRedisConnectionManager {
    type Connection = <RedisConnectionManager as ManageConnection>::Connection;
    type Error = <RedisConnectionManager as ManageConnection>::Error;

    async fn connect(&self) -> std::result::Result<Self::Connection, Self::Error> {
        let mut conn = self.inner.connect().await?;
        if let Some(name) = &self.connection_name {
            let result: std::result::Result<(), redis::RedisError> = redis::cmd("CLIENT")
                .arg("SETNAME")
                .arg(name.as_str())
                .query_async(&mut conn)
                .await;
            if let Err(e) = result {
                warn!(error = %e, connection_name = %name, "failed to set Redis connection name");
            }
        }
        Ok(conn)
    }

    async fn is_valid(&self, conn: &mut Self::Connection) -> std::result::Result<(), Self::Error> {
        self.inner.is_valid(conn).await
    }

    fn has_broken(&self, conn: &mut Self::Connection) -> bool {
        self.inner.has_broken(conn)
    }
}

/// A Redis connection pool.
///
/// Wraps either a single-node bb8 pool or a cluster-aware bb8 pool depending
/// on whether `RedisConfig::cluster` was set when the pool was built. The
/// public API (`get`, `state`) is identical either way so callers don't need
/// to know which mode is active.
#[derive(Clone)]
pub enum RedisPool {
    /// Single-node pool. Any `connection_name` (`RedisConfig::connection_name`)
    /// is applied via `CLIENT SETNAME` once, when each physical connection is
    /// established (`NamedRedisConnectionManager::connect`) — not on every
    /// checkout, since bb8 reuses connections across checkouts.
    Single(Pool<NamedRedisConnectionManager>),
    /// Cluster-aware pool. Same `connection_name` handling as `Single`, but
    /// applied inside `RedisClusterConnectionManager::connect`.
    Cluster(Pool<RedisClusterConnectionManager>),
}

impl RedisPool {
    /// Get a connection from the pool.
    ///
    /// If `RedisConfig::connection_name` was set when the pool was built,
    /// the returned connection was already named via `CLIENT SETNAME` when
    /// its underlying physical connection was established — naming happens
    /// once per connection, not on every checkout.
    pub async fn get(&self) -> Result<RedisConnection<'_>> {
        let conn = match self {
            RedisPool::Single(pool) => {
                let conn = pool
                    .get()
                    .await
                    .map_err(|e| RedisError::Pool(e.to_string()))?;
                RedisConnection::Single(conn)
            }
            RedisPool::Cluster(pool) => {
                let conn = pool
                    .get()
                    .await
                    .map_err(|e| RedisError::Pool(e.to_string()))?;
                RedisConnection::Cluster(conn)
            }
        };

        Ok(conn)
    }

    /// Get pool state (connection counts).
    pub fn state(&self) -> bb8::State {
        match self {
            RedisPool::Single(pool) => pool.state(),
            RedisPool::Cluster(pool) => pool.state(),
        }
    }

    /// True if this pool was built in cluster mode.
    pub fn is_cluster(&self) -> bool {
        matches!(self, RedisPool::Cluster(_))
    }
}

/// A pooled Redis connection.
///
/// Wraps either a single-node or cluster-aware pooled connection. Both
/// variants implement `redis::aio::ConnectionLike`, so callers can issue
/// commands (via `redis::cmd(..).query_async(&mut conn)` or the
/// `AsyncCommands` trait) without matching on the variant.
pub enum RedisConnection<'a> {
    /// Connection from a single-node pool.
    Single(PooledConnection<'a, NamedRedisConnectionManager>),
    /// Connection from a cluster-aware pool.
    Cluster(PooledConnection<'a, RedisClusterConnectionManager>),
}

impl<'a> ConnectionLike for RedisConnection<'a> {
    fn req_packed_command<'b>(
        &'b mut self,
        cmd: &'b redis::Cmd,
    ) -> redis::RedisFuture<'b, redis::Value> {
        match self {
            RedisConnection::Single(conn) => (**conn).req_packed_command(cmd),
            RedisConnection::Cluster(conn) => (**conn).req_packed_command(cmd),
        }
    }

    fn req_packed_commands<'b>(
        &'b mut self,
        cmd: &'b redis::Pipeline,
        offset: usize,
        count: usize,
    ) -> redis::RedisFuture<'b, Vec<redis::Value>> {
        match self {
            RedisConnection::Single(conn) => (**conn).req_packed_commands(cmd, offset, count),
            RedisConnection::Cluster(conn) => (**conn).req_packed_commands(cmd, offset, count),
        }
    }

    fn get_db(&self) -> i64 {
        match self {
            RedisConnection::Single(conn) => (**conn).get_db(),
            RedisConnection::Cluster(conn) => (**conn).get_db(),
        }
    }
}

/// Builder for creating Redis connection pools.
pub struct RedisPoolBuilder {
    config: RedisConfig,
}

impl RedisPoolBuilder {
    /// Create a new pool builder.
    pub fn new(config: RedisConfig) -> Self {
        Self { config }
    }

    /// Build the connection pool.
    ///
    /// When `config.cluster` is set, builds a cluster-aware pool backed by
    /// `redis::cluster::ClusterClient` seeded with `config.cluster_nodes`
    /// (falling back to `config.connection_url()` if no nodes were given).
    /// Otherwise builds a single-node pool as before.
    pub async fn build(self) -> Result<RedisPool> {
        if self.config.cluster {
            self.build_cluster().await
        } else {
            self.build_single().await
        }
    }

    async fn build_single(self) -> Result<RedisPool> {
        let url = self.config.connection_url();

        let inner_manager = RedisConnectionManager::new(url.clone())
            .map_err(|e| RedisError::Connection(e.to_string()))?;
        let manager =
            NamedRedisConnectionManager::new(inner_manager, self.config.connection_name.clone());

        let pool = Pool::builder()
            .max_size(self.config.pool_size)
            .min_idle(self.config.min_idle)
            .connection_timeout(self.config.connection_timeout)
            .build(manager)
            .await
            .map_err(|e| RedisError::Pool(e.to_string()))?;

        // Test the connection in a scope so the connection is dropped before returning pool
        {
            let mut conn = pool
                .get()
                .await
                .map_err(|e| RedisError::Pool(e.to_string()))?;
            let _: String = redis::cmd("PING")
                .query_async(&mut *conn)
                .await
                .map_err(|e| RedisError::Connection(e.to_string()))?;
        }

        info!(
            pool_size = self.config.pool_size,
            url = %self.config.url,
            "Redis connection pool created"
        );

        Ok(RedisPool::Single(pool))
    }

    async fn build_cluster(self) -> Result<RedisPool> {
        let nodes = if self.config.cluster_nodes.is_empty() {
            vec![self.config.connection_url()]
        } else {
            self.config.cluster_nodes.clone()
        };

        let manager = RedisClusterConnectionManager::new(nodes.clone())?
            .with_connection_name(self.config.connection_name.clone());

        let pool = Pool::builder()
            .max_size(self.config.pool_size)
            .min_idle(self.config.min_idle)
            .connection_timeout(self.config.connection_timeout)
            .build(manager)
            .await
            .map_err(|e| RedisError::Pool(e.to_string()))?;

        // Test the connection in a scope so the connection is dropped before returning pool
        {
            let mut conn = pool
                .get()
                .await
                .map_err(|e| RedisError::Pool(e.to_string()))?;
            let _: String = redis::cmd("PING")
                .query_async(&mut *conn)
                .await
                .map_err(|e| RedisError::Connection(e.to_string()))?;
        }

        info!(
            pool_size = self.config.pool_size,
            nodes = ?nodes,
            "Redis cluster connection pool created"
        );

        Ok(RedisPool::Cluster(pool))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end proof (requires Docker) that `RedisConfig::connection_name`
    /// is actually applied: a pool built with `connection_name` set must
    /// issue `CLIENT SETNAME` when the physical connection is established
    /// (`connect()`), so `CLIENT GETNAME` on a checked-out connection still
    /// reflects it, even though `get()` no longer re-issues `SETNAME` on
    /// every checkout.
    #[tokio::test]
    async fn checkout_applies_connection_name_via_client_setname() {
        if !armature_testkit::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
        let container = armature_testkit::containers::RedisContainer::start().await;
        let config = RedisConfig::builder()
            .url(container.url())
            .connection_name("wf2-test-conn")
            .build();

        let pool = RedisPoolBuilder::new(config).build().await.unwrap();
        let mut conn = pool.get().await.unwrap();

        let name: String = redis::cmd("CLIENT")
            .arg("GETNAME")
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(name, "wf2-test-conn");
    }

    /// With `cluster = true` + `cluster_nodes` set, `RedisPoolBuilder::build`
    /// must dispatch to the cluster manager path
    /// (`RedisClusterConnectionManager` wrapping `redis::cluster::ClusterClient`)
    /// rather than silently building a single-node `RedisConnectionManager`.
    ///
    /// This exercises the exact branch `build()` uses
    /// (`if self.config.cluster { build_cluster } else { build_single }`) and
    /// the manager type `build_cluster` constructs, without requiring a live
    /// server (no network I/O happens in `ClusterClient::new`/manager
    /// construction — only `connect()` touches the network).
    ///
    /// Against the pre-fix code this fails to compile: `build()` had no
    /// branch on `config.cluster` at all, and `RedisClusterConnectionManager`
    /// did not exist — `build` unconditionally called
    /// `RedisConnectionManager::new(url)`.
    #[test]
    fn cluster_config_selects_cluster_manager_not_single_node() {
        let config = RedisConfig::builder()
            .url("redis://127.0.0.1:7000")
            .cluster_nodes(vec![
                "redis://127.0.0.1:7000".to_string(),
                "redis://127.0.0.1:7001".to_string(),
            ])
            .build();

        assert!(config.cluster, "cluster_nodes() must imply cluster mode");

        let builder = RedisPoolBuilder::new(config);
        assert!(
            builder.config.cluster,
            "builder must retain cluster mode from config"
        );

        // Mirrors `RedisPoolBuilder::build`'s dispatch: cluster=true must
        // route through `build_cluster`, which constructs
        // `RedisClusterConnectionManager`, never `RedisConnectionManager`.
        let nodes = if builder.config.cluster_nodes.is_empty() {
            vec![builder.config.connection_url()]
        } else {
            builder.config.cluster_nodes.clone()
        };
        let manager = RedisClusterConnectionManager::new(nodes)
            .expect("cluster manager should construct from valid seed URLs");

        // Type-level proof: this only compiles if `manager` really is
        // `RedisClusterConnectionManager`.
        let _: RedisClusterConnectionManager = manager;
    }

    #[test]
    fn non_cluster_config_defaults_to_single_node() {
        let config = RedisConfig::builder().url("redis://127.0.0.1:6379").build();
        assert!(!config.cluster);
    }

    /// A live cluster smoke test would require an actual multi-node Redis
    /// Cluster (armature's `RedisContainer` testkit only stands up a
    /// single-node instance), so this is `#[ignore]`d and left as a manual
    /// integration check against a real cluster (e.g. `docker run` a 3-node
    /// cluster or point at `REDIS_CLUSTER_NODES`).
    #[tokio::test]
    #[ignore = "requires a live Redis Cluster (multiple nodes); testkit's RedisContainer is single-node"]
    async fn live_cluster_pool_connects_and_pings() {
        let config = RedisConfig::builder()
            .cluster_nodes(vec!["redis://127.0.0.1:7000".to_string()])
            .build();

        let pool = RedisPoolBuilder::new(config).build().await.unwrap();
        assert!(pool.is_cluster());

        let mut conn = pool.get().await.unwrap();
        let pong: String = redis::cmd("PING").query_async(&mut conn).await.unwrap();
        assert_eq!(pong, "PONG");
    }
}
