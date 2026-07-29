//! Redis connection pool.

use bb8::{ManageConnection, Pool, PooledConnection};
use bb8_redis::RedisConnectionManager;
use redis::aio::ConnectionLike;
use redis::cluster::ClusterClient;
use redis::cluster_async::ClusterConnection;
use tracing::{info, warn};

use crate::{RedisConfig, RedisError, Result};

/// Issue `CLIENT SETNAME` on a freshly-established physical connection, if a
/// connection name is configured. Shared by
/// `NamedRedisConnectionManager::connect` and
/// `RedisClusterConnectionManager::connect` so the naming logic (and its
/// failure handling) lives in exactly one place.
async fn apply_connection_name(conn: &mut impl ConnectionLike, name: Option<&str>) {
    if let Some(name) = name {
        let result: std::result::Result<(), redis::RedisError> = redis::cmd("CLIENT")
            .arg("SETNAME")
            .arg(name)
            .query_async(conn)
            .await;
        if let Err(e) = result {
            warn!(error = %e, connection_name = %name, "failed to set Redis connection name");
        }
    }
}

/// Upgrade a seed node entry to `rediss://` for TLS.
///
/// Handles three cases:
/// - `redis://host:port` -> `rediss://host:port`
/// - schemeless `host:port` -> `rediss://host:port` (no scheme at all, so
///   there's nothing to preserve except the address itself)
/// - already-`rediss://` (or any other explicit scheme) -> unchanged
///
/// Used to fix a cluster TLS downgrade: `RedisConfig::connection_url()`
/// upgrades the scheme for the single-node/fallback path, but explicit
/// `cluster_nodes` entries bypass that method entirely, so without this an
/// operator setting `.tls(true)` together with `.cluster_nodes([...])` would
/// silently get an unencrypted cluster connection carrying credentials. The
/// schemeless case matters too: a node given as bare `host:6379` (no scheme
/// at all) previously stayed plaintext under `.tls(true)` because the old
/// implementation only matched an explicit `redis://` prefix.
fn upgrade_node_to_tls(node: &str) -> String {
    if let Some(rest) = node.strip_prefix("redis://") {
        format!("rediss://{rest}")
    } else if node.contains("://") {
        // Already has some other explicit scheme (e.g. `rediss://`): leave it.
        node.to_string()
    } else {
        // Schemeless entry, e.g. `host:6379`.
        format!("rediss://{node}")
    }
}

/// Splice percent-encoded credentials (`config.username`/`config.password`)
/// into a cluster seed node's userinfo, unless the node already has one.
///
/// Mirrors the auth-splicing behavior of `RedisConfig::connection_url()`
/// (legacy `:password@` form when no username is set, `user:pass@` form
/// otherwise), but operates on an arbitrary node string that may or may not
/// carry an explicit scheme.
fn splice_node_credentials(node: &str, config: &RedisConfig) -> String {
    let Some(password) = &config.password else {
        return node.to_string();
    };

    // Determine where the scheme ends (if any) and where the authority
    // (userinfo@host[:port]) region ends, so we don't mistake a `@` inside a
    // path/query for existing userinfo.
    let scheme_end = node.find("://").map(|i| i + 3).unwrap_or(0);
    let authority_end = node[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(node.len());

    if node[scheme_end..authority_end].contains('@') {
        // Node already carries its own userinfo; don't override it.
        return node.to_string();
    }

    let encoded_password = urlencoding::encode(password);
    let userinfo = if let Some(username) = &config.username {
        format!("{}:{}@", urlencoding::encode(username), encoded_password)
    } else {
        format!(":{encoded_password}@")
    };

    format!("{}{}{}", &node[..scheme_end], userinfo, &node[scheme_end..])
}

/// Build the list of cluster seed node URLs from `config`.
///
/// Pure (no I/O) so it can be unit-tested directly. `database` is never
/// applied — it's irrelevant in cluster mode (Redis Cluster doesn't support
/// `SELECT`).
///
/// - When `config.cluster_nodes` is empty, falls back to
///   `vec![config.connection_url()]` (which already applies TLS-upgrade and
///   credential-splicing for the single fallback node).
/// - Otherwise, each explicit node is TLS-upgraded (see
///   `upgrade_node_to_tls`) when `config.tls` is set, and has
///   `config.username`/`config.password` spliced in as percent-encoded
///   userinfo (see `splice_node_credentials`) unless the node already has its
///   own userinfo. Without this, explicit `cluster_nodes` previously bypassed
///   both TLS upgrade and credentials entirely — a config with a password
///   set would connect without auth (NOAUTH at runtime) even though the
///   single-node path spliced credentials in.
fn cluster_seed_nodes(config: &RedisConfig) -> Vec<String> {
    if config.cluster_nodes.is_empty() {
        return vec![config.connection_url()];
    }

    config
        .cluster_nodes
        .iter()
        .map(|node| {
            let node = if config.tls {
                upgrade_node_to_tls(node)
            } else {
                node.clone()
            };
            splice_node_credentials(&node, config)
        })
        .collect()
}

/// Issue a scoped checkout + `PING` probe against a freshly-built pool, to
/// fail fast at `build()` time if the pool can't actually reach a server,
/// rather than surfacing the failure on the first real `get()` call later.
///
/// Shared by `RedisPoolBuilder::build_single` and
/// `RedisPoolBuilder::build_cluster` — both built a pool and immediately
/// checked it out to `PING` it in an identical scoped block.
async fn probe_pool<M>(pool: &Pool<M>) -> Result<()>
where
    M: ManageConnection,
    M::Connection: ConnectionLike,
    M::Error: std::error::Error,
{
    let mut conn = pool
        .get()
        .await
        .map_err(|e| RedisError::Pool(e.to_string()))?;
    let _: String = redis::cmd("PING")
        .query_async(&mut *conn)
        .await
        .map_err(|e| RedisError::Connection(e.to_string()))?;
    Ok(())
}

/// Redact userinfo (`user:pass@`) from a node URL for safe logging. Never
/// log a value derived from `RedisConfig::connection_url()` (which may
/// embed credentials) without passing it through this first.
fn redact_node_for_logging(node: &str) -> String {
    if let Some(scheme_end) = node.find("://") {
        let after_scheme = scheme_end + 3;
        // The authority (userinfo@host[:port]) ends at the next `/`, if any.
        // Search for `@` within that span and take the *last* occurrence, so
        // a `@` embedded in (unencoded) userinfo doesn't cause us to stop
        // early and leave part of the credentials in the output.
        let authority_end = node[after_scheme..]
            .find('/')
            .map(|i| after_scheme + i)
            .unwrap_or(node.len());
        if let Some(at) = node[after_scheme..authority_end].rfind('@') {
            let host_start = after_scheme + at + 1;
            return format!("{}{}", &node[..after_scheme], &node[host_start..]);
        }
    }
    node.to_string()
}

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
        apply_connection_name(&mut conn, self.connection_name.as_deref()).await;
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
        apply_connection_name(&mut conn, self.connection_name.as_deref()).await;
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
#[non_exhaustive]
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
#[non_exhaustive]
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

        probe_pool(&pool).await?;

        // Never log `self.config.url` directly: `RedisConfig::password`/
        // `username` are not embedded in `self.config.url` itself, but an
        // operator-provided URL may already carry `user:pass@` inline.
        // Redact userinfo before logging, for parity with the cluster path.
        info!(
            pool_size = self.config.pool_size,
            url = %redact_node_for_logging(&self.config.url),
            "Redis connection pool created"
        );

        Ok(RedisPool::Single(pool))
    }

    async fn build_cluster(self) -> Result<RedisPool> {
        let nodes = cluster_seed_nodes(&self.config);

        let manager = RedisClusterConnectionManager::new(nodes.clone())?
            .with_connection_name(self.config.connection_name.clone());

        let pool = Pool::builder()
            .max_size(self.config.pool_size)
            .min_idle(self.config.min_idle)
            .connection_timeout(self.config.connection_timeout)
            .build(manager)
            .await
            .map_err(|e| RedisError::Pool(e.to_string()))?;

        probe_pool(&pool).await?;

        // Never log `nodes` directly: when `cluster_nodes` was empty, it was
        // built from `connection_url()`, which may embed `user:pass@`
        // credentials. Redact userinfo before logging.
        let redacted_nodes: Vec<String> =
            nodes.iter().map(|n| redact_node_for_logging(n)).collect();
        info!(
            pool_size = self.config.pool_size,
            nodes = ?redacted_nodes,
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
        armature_testkit::skip_if_no_docker!();
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
        // Uses the real `cluster_seed_nodes` helper that `build_cluster`
        // itself calls, so this exercises the actual production path rather
        // than a re-implemented copy.
        let nodes = cluster_seed_nodes(&builder.config);
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

    /// Regression test for the cluster TLS downgrade: `RedisConfig::tls(true)`
    /// combined with explicit `cluster_nodes` must still upgrade each
    /// `redis://` node to `rediss://` before it reaches
    /// `RedisClusterConnectionManager::new`. Before the fix, `build_cluster`
    /// passed `cluster_nodes` through unchanged, so a TLS-enabled cluster
    /// config with plain `redis://` seed nodes silently built an unencrypted
    /// connection. Calls `cluster_seed_nodes` directly — the exact helper
    /// `build_cluster` calls — rather than a re-implemented copy.
    #[test]
    fn cluster_nodes_are_upgraded_to_tls_when_tls_enabled() {
        let config = RedisConfig {
            tls: true,
            cluster: true,
            cluster_nodes: vec![
                "redis://127.0.0.1:7000".to_string(),
                "rediss://127.0.0.1:7001".to_string(),
                "redis://user:pass@127.0.0.1:7002".to_string(),
            ],
            ..Default::default()
        };

        let nodes = cluster_seed_nodes(&config);

        assert_eq!(
            nodes,
            vec![
                "rediss://127.0.0.1:7000".to_string(),
                "rediss://127.0.0.1:7001".to_string(),
                "rediss://user:pass@127.0.0.1:7002".to_string(),
            ]
        );
    }

    #[test]
    fn upgrade_node_to_tls_leaves_already_tls_nodes_unchanged() {
        assert_eq!(
            upgrade_node_to_tls("rediss://h:7000"),
            "rediss://h:7000".to_string()
        );
    }

    #[test]
    fn upgrade_node_to_tls_prepends_scheme_to_schemeless_node() {
        // A cluster node given as bare `host:port` with no scheme at all
        // must still be upgraded to `rediss://` — only matching an explicit
        // `redis://` prefix would leave it plaintext under TLS.
        assert_eq!(upgrade_node_to_tls("h:6379"), "rediss://h:6379".to_string());
    }

    /// `cluster_seed_nodes` is the pure helper `build_cluster` calls to
    /// build its node list. These tests cover it directly (no Docker, no
    /// network) since `build_cluster` itself needs a live cluster to run.

    #[test]
    fn cluster_seed_nodes_falls_back_to_connection_url_when_empty() {
        let config = RedisConfig {
            url: "redis://h:6379".to_string(),
            cluster: true,
            cluster_nodes: Vec::new(),
            database: Some(2),
            ..Default::default()
        };
        assert_eq!(cluster_seed_nodes(&config), vec![config.connection_url()]);
    }

    #[test]
    fn cluster_seed_nodes_upgrades_redis_scheme_to_rediss_under_tls() {
        let config = RedisConfig {
            tls: true,
            cluster: true,
            cluster_nodes: vec!["redis://127.0.0.1:7000".to_string()],
            ..Default::default()
        };
        assert_eq!(
            cluster_seed_nodes(&config),
            vec!["rediss://127.0.0.1:7000".to_string()]
        );
    }

    #[test]
    fn cluster_seed_nodes_upgrades_schemeless_node_to_rediss_under_tls() {
        let config = RedisConfig {
            tls: true,
            cluster: true,
            cluster_nodes: vec!["127.0.0.1:7000".to_string()],
            ..Default::default()
        };
        assert_eq!(
            cluster_seed_nodes(&config),
            vec!["rediss://127.0.0.1:7000".to_string()]
        );
    }

    #[test]
    fn cluster_seed_nodes_leaves_already_rediss_node_unchanged() {
        let config = RedisConfig {
            tls: true,
            cluster: true,
            cluster_nodes: vec!["rediss://127.0.0.1:7001".to_string()],
            ..Default::default()
        };
        assert_eq!(
            cluster_seed_nodes(&config),
            vec!["rediss://127.0.0.1:7001".to_string()]
        );
    }

    #[test]
    fn cluster_seed_nodes_splices_percent_encoded_credentials() {
        // Regression test: explicit `cluster_nodes` previously bypassed
        // `config.username`/`config.password` entirely, so a cluster config
        // with credentials set would connect without auth (NOAUTH at
        // runtime) even though the single-node fallback path spliced them
        // in via `connection_url()`.
        let config = RedisConfig {
            cluster: true,
            cluster_nodes: vec!["redis://127.0.0.1:7000".to_string()],
            username: Some("us/er".to_string()),
            password: Some("pa:ss@word".to_string()),
            ..Default::default()
        };
        assert_eq!(
            cluster_seed_nodes(&config),
            vec!["redis://us%2Fer:pa%3Ass%40word@127.0.0.1:7000".to_string()]
        );
    }

    #[test]
    fn cluster_seed_nodes_does_not_override_existing_userinfo() {
        let config = RedisConfig {
            cluster: true,
            cluster_nodes: vec!["redis://user:pass@127.0.0.1:7002".to_string()],
            username: Some("other".to_string()),
            password: Some("secret".to_string()),
            ..Default::default()
        };
        assert_eq!(
            cluster_seed_nodes(&config),
            vec!["redis://user:pass@127.0.0.1:7002".to_string()]
        );
    }

    #[test]
    fn cluster_seed_nodes_password_only_uses_legacy_userinfo_form() {
        let config = RedisConfig {
            cluster: true,
            cluster_nodes: vec!["redis://127.0.0.1:7000".to_string()],
            password: Some("secret".to_string()),
            ..Default::default()
        };
        assert_eq!(
            cluster_seed_nodes(&config),
            vec!["redis://:secret@127.0.0.1:7000".to_string()]
        );
    }

    /// Docker-free coverage for `#8`: the connection-name field must survive
    /// manager construction unchanged, since `connect()` relies on it being
    /// present at connection-establishment time.
    #[test]
    fn named_manager_retains_configured_connection_name() {
        let inner = RedisConnectionManager::new("redis://127.0.0.1:6379").expect("valid redis url");
        let manager = NamedRedisConnectionManager::new(inner, Some("wf2-test-conn".to_string()));
        assert_eq!(manager.connection_name.as_deref(), Some("wf2-test-conn"));
    }

    #[test]
    fn cluster_manager_retains_configured_connection_name() {
        let manager =
            RedisClusterConnectionManager::new(vec!["redis://127.0.0.1:7000".to_string()])
                .expect("valid seed nodes")
                .with_connection_name(Some("wf2-test-conn".to_string()));
        assert_eq!(manager.connection_name.as_deref(), Some("wf2-test-conn"));
    }

    #[test]
    fn redact_node_for_logging_strips_userinfo() {
        assert_eq!(
            redact_node_for_logging("redis://user:p@ss@h:6379/2"),
            "redis://h:6379/2"
        );
        // No userinfo present: unchanged.
        assert_eq!(redact_node_for_logging("redis://h:6379"), "redis://h:6379");
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
