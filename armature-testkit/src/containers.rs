//! Docker-backed datastore helpers (behind the `containers` feature).

use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::redis::Redis;

/// A running Redis container. Stops when dropped.
pub struct RedisContainer {
    container: ContainerAsync<Redis>,
}

impl RedisContainer {
    /// Start a Redis container and wait until it is ready.
    pub async fn start() -> RedisContainer {
        let container = Redis::default().start().await.expect("start redis container");
        RedisContainer { container }
    }

    /// A `redis://127.0.0.1:PORT` URL for the mapped port.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(6379)
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
        let container = Postgres::default().start().await.expect("start postgres container");
        PostgresContainer { container }
    }

    /// A `postgres://postgres:postgres@127.0.0.1:PORT/postgres` connection string.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(5432)
            .await
            .expect("postgres mapped port");
        format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres")
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
}
