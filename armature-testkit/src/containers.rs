//! Docker-backed datastore helpers (behind the `containers` feature).

use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
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
}
