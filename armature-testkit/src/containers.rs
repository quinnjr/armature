//! Docker-backed datastore helpers (behind the `containers` feature).

use std::time::Duration;

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::redis::Redis;

/// Wall-clock bound on container startup so a container that never reaches
/// its readiness probe fails the test instead of hanging it forever.
const START_TIMEOUT: Duration = Duration::from_secs(60);

/// A running Redis container. Stops when dropped.
pub struct RedisContainer {
    // Never read directly; kept alive so the container stops on `Drop`.
    #[allow(dead_code)]
    container: ContainerAsync<Redis>,
    url: String,
}

impl RedisContainer {
    /// Start a Redis container and wait until it is ready.
    pub async fn start() -> RedisContainer {
        let container = tokio::time::timeout(START_TIMEOUT, Redis::default().start())
            .await
            .expect("redis container did not become ready within 60s")
            .expect("start redis container");
        let port = container
            .get_host_port_ipv4(6379.tcp())
            .await
            .expect("redis mapped port");
        let url = format!("redis://127.0.0.1:{port}");
        RedisContainer { container, url }
    }

    /// A `redis://127.0.0.1:PORT` URL for the mapped port.
    pub fn url(&self) -> String {
        self.url.clone()
    }
}

/// A running Postgres container. Stops when dropped.
pub struct PostgresContainer {
    // Never read directly; kept alive so the container stops on `Drop`.
    #[allow(dead_code)]
    container: ContainerAsync<Postgres>,
    url: String,
}

impl PostgresContainer {
    /// Start a Postgres container (default `postgres`/`postgres` credentials).
    pub async fn start() -> PostgresContainer {
        let container = tokio::time::timeout(START_TIMEOUT, Postgres::default().start())
            .await
            .expect("postgres container did not become ready within 60s")
            .expect("start postgres container");
        let port = container
            .get_host_port_ipv4(5432.tcp())
            .await
            .expect("postgres mapped port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        PostgresContainer { container, url }
    }

    /// A `postgres://postgres:postgres@127.0.0.1:PORT/postgres` connection string.
    pub fn url(&self) -> String {
        self.url.clone()
    }
}

/// A running single-node OpenSearch container (security disabled). Stops when dropped.
pub struct OpenSearchContainer {
    // Never read directly; kept alive so the container stops on `Drop`.
    #[allow(dead_code)]
    container: ContainerAsync<GenericImage>,
    url: String,
}

impl OpenSearchContainer {
    /// Start OpenSearch 2.x in single-node mode with the security plugin off.
    pub async fn start() -> OpenSearchContainer {
        let image = GenericImage::new("opensearchproject/opensearch", "2.13.0")
            .with_wait_for(WaitFor::message_on_stdout("Node started"))
            .with_exposed_port(9200.tcp())
            .with_env_var("discovery.type", "single-node")
            .with_env_var("DISABLE_SECURITY_PLUGIN", "true")
            .with_env_var("OPENSEARCH_INITIAL_ADMIN_PASSWORD", "Testkit123!");
        let container = tokio::time::timeout(START_TIMEOUT, image.start())
            .await
            .expect("opensearch container did not become ready within 60s")
            .expect("start opensearch container");
        let port = container
            .get_host_port_ipv4(9200.tcp())
            .await
            .expect("opensearch mapped port");
        let url = format!("http://127.0.0.1:{port}");
        OpenSearchContainer { container, url }
    }

    /// The REST API base URL for the mapped 9200 port.
    pub fn url(&self) -> String {
        self.url.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_nonzero_port(url: &str) {
        let host_port = url
            .split("://")
            .nth(1)
            .expect("url missing scheme separator");
        let host_port = host_port.split('/').next().expect("url missing host part");
        let host_port = host_port.rsplit('@').next().expect("url missing host part");
        let port_str = host_port
            .rsplit(':')
            .next()
            .expect("url missing port separator");
        let port: u16 = port_str
            .parse()
            .unwrap_or_else(|e| panic!("port {port_str:?} did not parse as u16: {e}"));
        assert!(port > 0, "expected non-zero port, got {port} in url: {url}");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn redis_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let redis = RedisContainer::start().await;
        let url = redis.url();
        assert!(url.starts_with("redis://"), "unexpected url: {url}");
        assert_nonzero_port(&url);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn postgres_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let pg = PostgresContainer::start().await;
        let url = pg.url();
        assert!(url.starts_with("postgres://"), "unexpected url: {url}");
        assert_nonzero_port(&url);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn opensearch_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let os = OpenSearchContainer::start().await;
        let url = os.url();
        assert!(url.starts_with("http://"), "unexpected url: {url}");
        assert_nonzero_port(&url);
    }
}
