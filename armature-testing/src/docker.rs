//! Docker Test Containers
//!
//! Provides Docker-based testing with automatic container lifecycle management.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use thiserror::Error;

/// Docker test container errors
#[derive(Debug, Error)]
pub enum DockerError {
    #[error("Docker not available: {0}")]
    NotAvailable(String),

    #[error("Container start failed: {0}")]
    StartFailed(String),

    #[error("Container stop failed: {0}")]
    StopFailed(String),

    #[error("Image pull failed: {0}")]
    PullFailed(String),

    #[error("Container not found: {0}")]
    NotFound(String),
}

/// Docker container configuration
#[derive(Debug, Clone)]
pub struct ContainerConfig {
    /// Docker image
    pub image: String,

    /// Image tag
    pub tag: String,

    /// Container name
    pub name: Option<String>,

    /// Environment variables
    pub env: HashMap<String, String>,

    /// Port mappings (host_port -> container_port)
    pub ports: HashMap<u16, u16>,

    /// Wait for container to be ready
    pub wait_timeout_secs: u64,
}

impl ContainerConfig {
    /// Create new container config
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::docker::ContainerConfig;
    ///
    /// let config = ContainerConfig::new("postgres", "15");
    /// ```
    pub fn new(image: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            tag: tag.into(),
            name: None,
            env: HashMap::new(),
            ports: HashMap::new(),
            wait_timeout_secs: 30,
        }
    }

    /// Set container name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Add environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Add port mapping
    pub fn with_port(mut self, host_port: u16, container_port: u16) -> Self {
        self.ports.insert(host_port, container_port);
        self
    }

    /// Set wait timeout
    pub fn with_wait_timeout(mut self, seconds: u64) -> Self {
        self.wait_timeout_secs = seconds;
        self
    }

    /// Get full image name
    pub fn image_name(&self) -> String {
        format!("{}:{}", self.image, self.tag)
    }
}

/// Docker test container
pub struct DockerContainer {
    config: ContainerConfig,
    container_id: Option<String>,
}

impl DockerContainer {
    /// Create new Docker container
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::docker::*;
    ///
    /// let config = ContainerConfig::new("postgres", "15")
    ///     .with_env("POSTGRES_PASSWORD", "test")
    ///     .with_port(5432, 5432);
    ///
    /// let container = DockerContainer::new(config);
    /// ```
    pub fn new(config: ContainerConfig) -> Self {
        Self {
            config,
            container_id: None,
        }
    }

    /// Check if Docker is available
    pub fn is_docker_available() -> bool {
        Command::new("docker")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Start container
    pub async fn start(&mut self) -> Result<(), DockerError> {
        if !Self::is_docker_available() {
            return Err(DockerError::NotAvailable(
                "Docker not found. Please install Docker.".to_string(),
            ));
        }

        // Pull image if needed
        self.pull_image()?;

        // Build docker run command
        let mut cmd = Command::new("docker");
        cmd.arg("run")
            .arg("-d") // Detached
            .arg("--rm"); // Auto-remove on stop

        // Container name
        if let Some(ref name) = self.config.name {
            cmd.arg("--name").arg(name);
        }

        // Environment variables
        for (key, value) in &self.config.env {
            cmd.arg("-e").arg(format!("{}={}", key, value));
        }

        // Port mappings
        for (host_port, container_port) in &self.config.ports {
            cmd.arg("-p")
                .arg(format!("{}:{}", host_port, container_port));
        }

        // Image
        cmd.arg(self.config.image_name());

        // Execute
        let output = cmd
            .output()
            .map_err(|e| DockerError::StartFailed(e.to_string()))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::StartFailed(error.to_string()));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        self.container_id = Some(container_id);

        // Wait for the container to report itself running, polling up to
        // the configured `wait_timeout_secs` instead of a fixed sleep.
        let ready = Self::poll_until_ready(
            std::time::Duration::from_secs(self.config.wait_timeout_secs.max(1)),
            std::time::Duration::from_millis(200),
            || self.is_running(),
        )
        .await;

        if !ready {
            return Err(DockerError::StartFailed(format!(
                "container did not report running within wait_timeout_secs ({}s)",
                self.config.wait_timeout_secs
            )));
        }

        Ok(())
    }

    /// Poll `ready` at `poll_interval` until it returns `true` or `timeout`
    /// elapses. Returns `true` as soon as `ready` succeeds, `false` if the
    /// deadline passes first. Factored out of `start` so the readiness-wait
    /// behavior (honoring a configurable timeout instead of a fixed sleep)
    /// is testable without a real Docker daemon.
    async fn poll_until_ready<F>(
        timeout: std::time::Duration,
        poll_interval: std::time::Duration,
        mut ready: F,
    ) -> bool
    where
        F: FnMut() -> bool,
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if ready() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(poll_interval).await;
        }
    }

    /// Stop container
    pub async fn stop(&mut self) -> Result<(), DockerError> {
        if let Some(ref container_id) = self.container_id {
            let output = Command::new("docker")
                .arg("stop")
                .arg(container_id)
                .output()
                .map_err(|e| DockerError::StopFailed(e.to_string()))?;

            if !output.status.success() {
                let error = String::from_utf8_lossy(&output.stderr);
                return Err(DockerError::StopFailed(error.to_string()));
            }

            self.container_id = None;
        }

        Ok(())
    }

    /// Pull Docker image
    fn pull_image(&self) -> Result<(), DockerError> {
        let output = Command::new("docker")
            .arg("pull")
            .arg(self.config.image_name())
            .output()
            .map_err(|e| DockerError::PullFailed(e.to_string()))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(DockerError::PullFailed(error.to_string()));
        }

        Ok(())
    }

    /// Get container ID
    pub fn container_id(&self) -> Option<&str> {
        self.container_id.as_deref()
    }

    /// Check if container is running
    pub fn is_running(&self) -> bool {
        if let Some(ref container_id) = self.container_id {
            Command::new("docker")
                .arg("inspect")
                .arg("-f")
                .arg("{{.State.Running}}")
                .arg(container_id)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim() == "true")
                .unwrap_or(false)
        } else {
            false
        }
    }
}

impl Drop for DockerContainer {
    fn drop(&mut self) {
        // Stop the container synchronously via a blocking `docker stop`
        // command, never by spinning up a nested Tokio runtime: the
        // documented usage is dropping a `DockerContainer` inside an async
        // test (`#[tokio::test]`), and `Runtime::new().unwrap().block_on(..)`
        // there panics with "Cannot start a runtime from within a runtime"
        // instead of cleaning up.
        if let Some(container_id) = self.container_id.take() {
            let _ = Command::new("docker")
                .arg("stop")
                .arg(&container_id)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Postgres test container helper
pub struct PostgresContainer;

impl PostgresContainer {
    /// Create Postgres test container
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::docker::PostgresContainer;
    ///
    /// let config = PostgresContainer::config("testdb", "testuser", "testpass");
    /// ```
    pub fn config(database: &str, username: &str, password: &str) -> ContainerConfig {
        ContainerConfig::new("postgres", "15")
            .with_name(format!("armature-test-postgres-{}", uuid::Uuid::new_v4()))
            .with_env("POSTGRES_DB", database)
            .with_env("POSTGRES_USER", username)
            .with_env("POSTGRES_PASSWORD", password)
            .with_port(5432, 5432)
            .with_wait_timeout(10)
    }
}

/// Redis test container helper
pub struct RedisContainer;

impl RedisContainer {
    /// Create Redis test container
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::docker::RedisContainer;
    ///
    /// let config = RedisContainer::config();
    /// ```
    pub fn config() -> ContainerConfig {
        ContainerConfig::new("redis", "7")
            .with_name(format!("armature-test-redis-{}", uuid::Uuid::new_v4()))
            .with_port(6379, 6379)
            .with_wait_timeout(5)
    }
}

/// MongoDB test container helper
pub struct MongoContainer;

impl MongoContainer {
    /// Create MongoDB test container
    pub fn config(database: &str) -> ContainerConfig {
        ContainerConfig::new("mongo", "7")
            .with_name(format!("armature-test-mongo-{}", uuid::Uuid::new_v4()))
            .with_env("MONGO_INITDB_DATABASE", database)
            .with_port(27017, 27017)
            .with_wait_timeout(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_config() {
        let config = ContainerConfig::new("postgres", "15")
            .with_name("test-db")
            .with_env("POSTGRES_PASSWORD", "test")
            .with_port(5432, 5432);

        assert_eq!(config.image, "postgres");
        assert_eq!(config.tag, "15");
        assert_eq!(config.name, Some("test-db".to_string()));
        assert_eq!(
            config.env.get("POSTGRES_PASSWORD"),
            Some(&"test".to_string())
        );
        assert_eq!(config.ports.get(&5432), Some(&5432));
    }

    #[test]
    fn test_postgres_container_config() {
        let config = PostgresContainer::config("testdb", "user", "pass");
        assert_eq!(config.image, "postgres");
        assert_eq!(config.env.get("POSTGRES_DB"), Some(&"testdb".to_string()));
    }

    #[test]
    fn test_redis_container_config() {
        let config = RedisContainer::config();
        assert_eq!(config.image, "redis");
        assert_eq!(config.tag, "7");
    }

    #[test]
    fn test_docker_available() {
        // This will vary by system
        let _ = DockerContainer::is_docker_available();
    }

    /// Docker-free: proves the readiness wait honors a configurable timeout
    /// (rather than a fixed 2s sleep) by never reporting ready and checking
    /// that `poll_until_ready` actually waited out the deadline.
    #[tokio::test]
    async fn poll_until_ready_honors_timeout_when_never_ready() {
        let start = std::time::Instant::now();
        let ready = DockerContainer::poll_until_ready(
            std::time::Duration::from_millis(150),
            std::time::Duration::from_millis(20),
            || false,
        )
        .await;

        assert!(!ready);
        assert!(start.elapsed() >= std::time::Duration::from_millis(150));
    }

    /// Docker-free: proves readiness is detected as soon as it flips true,
    /// rather than always sleeping the full window.
    #[tokio::test]
    async fn poll_until_ready_returns_as_soon_as_ready() {
        let mut calls = 0u32;
        let start = std::time::Instant::now();

        let ready = DockerContainer::poll_until_ready(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(10),
            || {
                calls += 1;
                calls >= 3
            },
        )
        .await;

        assert!(ready);
        assert!(calls >= 3);
        // Should return well before the 5s timeout.
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }

    /// Docker-free: reproduces the exact scenario the old `Drop` impl
    /// broke — dropping a `DockerContainer` with a live `container_id`
    /// while already inside a Tokio runtime (`#[tokio::test]`). The old
    /// `tokio::runtime::Runtime::new().unwrap().block_on(self.stop())`
    /// panicked with "Cannot start a runtime from within a runtime" here,
    /// regardless of whether a real Docker daemon or container is present
    /// (the panic fires before any `docker` command even runs). No Docker
    /// installation is required: the synchronous `docker stop` the new Drop
    /// issues is allowed to fail (ignored) for a nonexistent container id.
    #[tokio::test]
    async fn drop_inside_tokio_runtime_does_not_panic() {
        let container = DockerContainer {
            config: ContainerConfig::new("redis", "7"),
            container_id: Some("armature-testing-nonexistent-container".to_string()),
        };

        // Must not panic. If it does, the test process aborts here.
        drop(container);
    }

    /// Docker-gated: exercises the real `start()` → readiness poll →
    /// (implicit) `Drop` → stop lifecycle end-to-end against an actual
    /// Docker daemon, inside a `#[tokio::test]` runtime (the documented
    /// usage). Self-skips when Docker is unavailable so the default
    /// `cargo test` stays Docker-free; set `ARMATURE_REQUIRE_DOCKER=1` to
    /// turn the skip into a hard failure (e.g. in CI, where Docker should
    /// be present).
    #[tokio::test]
    async fn start_and_drop_a_real_container_when_docker_is_available() {
        if !DockerContainer::is_docker_available() {
            if std::env::var("ARMATURE_REQUIRE_DOCKER").as_deref() == Ok("1") {
                panic!(
                    "ARMATURE_REQUIRE_DOCKER=1 is set but no Docker daemon is reachable: \
                     this container-gated test must run, not skip."
                );
            }
            eprintln!(
                "skipping: Docker not available (set ARMATURE_REQUIRE_DOCKER=1 to make this a failure)"
            );
            return;
        }

        let config = RedisContainer::config().with_wait_timeout(15);
        let mut container = DockerContainer::new(config);

        container
            .start()
            .await
            .expect("container should start and become ready within wait_timeout_secs");
        assert!(container.is_running());

        // Drop runs here, inside this test's Tokio runtime — must not
        // panic, and must actually stop the container synchronously.
        let container_id = container.container_id().unwrap().to_string();
        drop(container);

        let still_running = Command::new("docker")
            .arg("inspect")
            .arg("-f")
            .arg("{{.State.Running}}")
            .arg(&container_id)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "true")
            .unwrap_or(false);
        assert!(!still_running, "Drop should have stopped the container");
    }
}
