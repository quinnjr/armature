//! Docker-backed datastore helpers (behind the `containers` feature).

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::redis::Redis;

/// A running Redis container. Stops when dropped.
pub struct RedisContainer {
    container: ContainerAsync<Redis>,
}

impl RedisContainer {
    /// Start a Redis container and wait until it is ready.
    pub async fn start() -> RedisContainer {
        let container = Redis::default()
            .start()
            .await
            .expect("start redis container");
        RedisContainer { container }
    }

    /// A `redis://127.0.0.1:PORT` URL for the mapped port.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(6379.tcp())
            .await
            .expect("redis mapped port");
        format!("redis://127.0.0.1:{port}")
    }
}

/// A running Postgres container. Stops when dropped.
pub struct PostgresContainer {
    container: ContainerAsync<Postgres>,
}

impl PostgresContainer {
    /// Start a Postgres container (default `postgres`/`postgres` credentials).
    pub async fn start() -> PostgresContainer {
        let container = Postgres::default()
            .start()
            .await
            .expect("start postgres container");
        PostgresContainer { container }
    }

    /// A `postgres://postgres:postgres@127.0.0.1:PORT/postgres` connection string.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(5432.tcp())
            .await
            .expect("postgres mapped port");
        format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres")
    }
}

/// A running single-node OpenSearch container (security disabled). Stops when dropped.
pub struct OpenSearchContainer {
    container: ContainerAsync<GenericImage>,
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
        let container = image.start().await.expect("start opensearch container");
        OpenSearchContainer { container }
    }

    /// The REST API base URL for the mapped 9200 port.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(9200.tcp())
            .await
            .expect("opensearch mapped port");
        format!("http://127.0.0.1:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn redis_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let redis = RedisContainer::start().await;
        let url = redis.url().await;
        assert!(url.starts_with("redis://"), "unexpected url: {url}");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn postgres_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let pg = PostgresContainer::start().await;
        let url = pg.url().await;
        assert!(url.starts_with("postgres://"), "unexpected url: {url}");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn opensearch_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let os = OpenSearchContainer::start().await;
        let url = os.url().await;
        assert!(url.starts_with("http://"), "unexpected url: {url}");
    }
}
