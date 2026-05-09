//! Queue implementation with Redis backend.

use crate::error::{QueueError, QueueResult};
use crate::job::{Job, JobData, JobId, JobPriority, JobState};
use armature_log::{debug, info};
use chrono::Utc;
use redis::{AsyncCommands, Client, aio::ConnectionManager};
use std::time::Duration;

/// Queue configuration.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Redis connection URL
    pub redis_url: String,

    /// Queue name
    pub queue_name: String,

    /// Key prefix for Redis keys
    pub key_prefix: String,

    /// Maximum queue size (0 = unlimited)
    pub max_size: usize,

    /// Job retention time for completed jobs
    pub retention_time: Duration,
}

impl QueueConfig {
    /// Create a new queue configuration.
    pub fn new(redis_url: impl Into<String>, queue_name: impl Into<String>) -> Self {
        let queue_name = queue_name.into();
        Self {
            redis_url: redis_url.into(),
            key_prefix: format!("armature:queue:{}", queue_name),
            queue_name,
            max_size: 0,
            retention_time: Duration::from_secs(86400), // 24 hours
        }
    }

    /// Set the key prefix.
    pub fn with_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Set the maximum queue size.
    pub fn with_max_size(mut self, max_size: usize) -> Self {
        self.max_size = max_size;
        self
    }

    /// Set the retention time for completed jobs.
    pub fn with_retention_time(mut self, retention_time: Duration) -> Self {
        self.retention_time = retention_time;
        self
    }

    /// Build Redis key.
    fn key(&self, suffix: &str) -> String {
        format!("{}:{}", self.key_prefix, suffix)
    }
}

/// Job queue backed by Redis.
#[derive(Clone)]
pub struct Queue {
    connection: ConnectionManager,
    config: QueueConfig,
}

impl Queue {
    /// Create a new queue.
    pub async fn new(
        redis_url: impl Into<String>,
        queue_name: impl Into<String>,
    ) -> QueueResult<Self> {
        let config = QueueConfig::new(redis_url, queue_name);
        Self::with_config(config).await
    }

    /// Create a queue with custom configuration.
    pub async fn with_config(config: QueueConfig) -> QueueResult<Self> {
        info!("Initializing job queue: {}", config.queue_name);
        debug!(
            "Queue config - prefix: {}, max_size: {}",
            config.key_prefix, config.max_size
        );

        let client = Client::open(config.redis_url.as_str())
            .map_err(|e| QueueError::Config(e.to_string()))?;

        let connection = ConnectionManager::new(client).await?;

        info!("Job queue '{}' ready", config.queue_name);
        Ok(Self { connection, config })
    }

    /// Enqueue a job.
    pub async fn enqueue(&self, job_type: impl Into<String>, data: JobData) -> QueueResult<JobId> {
        let job_type = job_type.into();
        debug!(
            "Enqueueing job: {} on queue '{}'",
            job_type, self.config.queue_name
        );
        let job = Job::new(&self.config.queue_name, &job_type, data);
        self.enqueue_job(job).await
    }

    /// Enqueue a job with options.
    pub async fn enqueue_job(&self, job: Job) -> QueueResult<JobId> {
        // Check queue size limit
        if self.config.max_size > 0 {
            let size = self.size().await?;
            if size >= self.config.max_size {
                return Err(QueueError::QueueFull);
            }
        }

        let job_id = job.id;
        let mut conn = self.connection.clone();

        // Serialize job
        let job_json =
            serde_json::to_string(&job).map_err(|e| QueueError::Serialization(e.to_string()))?;

        // Store job data
        let job_key = self.config.key(&format!("job:{}", job_id));
        let _: () = conn
            .set_ex(&job_key, job_json, self.config.retention_time.as_secs())
            .await?;

        // Add to appropriate queue based on priority and schedule
        if job.is_ready() {
            let queue_key = self.priority_queue_key(job.priority);
            let score = -(job.priority as i64); // Negative for high priority first
            let _: () = conn.zadd(&queue_key, job_id.to_string(), score).await?;
        } else {
            // Scheduled job
            let delayed_key = self.config.key("delayed");
            let score = job.scheduled_at.unwrap().timestamp();
            let _: () = conn.zadd(&delayed_key, job_id.to_string(), score).await?;
        }

        Ok(job_id)
    }

    /// Dequeue the next job.
    pub async fn dequeue(&self) -> QueueResult<Option<Job>> {
        self.move_delayed_jobs().await?;

        let mut conn = self.connection.clone();

        // Try to get job from priority queues (high to low)
        for priority in [
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ] {
            let queue_key = self.priority_queue_key(priority);

            // Pop the highest priority job
            let result: Option<Vec<String>> = conn.zpopmin(&queue_key, 1).await?;

            if let Some(items) = result
                && let Some(job_id_str) = items.first()
                && let Ok(job_id) = job_id_str.parse::<JobId>()
                && let Some(mut job) = self.get_job(job_id).await?
            {
                job.start_processing();
                self.save_job(&job).await?;

                // Add to processing set
                let processing_key = self.config.key("processing");
                let _: () = conn
                    .zadd(&processing_key, job_id.to_string(), Utc::now().timestamp())
                    .await?;

                return Ok(Some(job));
            }
        }

        Ok(None)
    }

    /// Complete a job.
    pub async fn complete(&self, job_id: JobId) -> QueueResult<()> {
        if let Some(mut job) = self.get_job(job_id).await? {
            job.complete();
            self.save_job(&job).await?;
            self.remove_from_processing(job_id).await?;
        }
        Ok(())
    }

    /// Fail a job.
    pub async fn fail(&self, job_id: JobId, error: String) -> QueueResult<()> {
        if let Some(mut job) = self.get_job(job_id).await? {
            job.fail(error);

            if job.status.state == JobState::Failed && job.can_retry() {
                // Retry with backoff
                let retry_at = Utc::now() + job.backoff_delay();
                job.scheduled_at = Some(retry_at);
                self.save_job(&job).await?;

                // Add to delayed queue
                let mut conn = self.connection.clone();
                let delayed_key = self.config.key("delayed");
                let _: () = conn
                    .zadd(&delayed_key, job_id.to_string(), retry_at.timestamp())
                    .await?;
            } else {
                // Move to dead letter queue
                self.save_job(&job).await?;
                let mut conn = self.connection.clone();
                let dead_key = self.config.key("dead");
                let _: () = conn
                    .zadd(&dead_key, job_id.to_string(), Utc::now().timestamp())
                    .await?;
            }

            self.remove_from_processing(job_id).await?;
        }
        Ok(())
    }

    /// Get a job by ID.
    pub async fn get_job(&self, job_id: JobId) -> QueueResult<Option<Job>> {
        let mut conn = self.connection.clone();
        let job_key = self.config.key(&format!("job:{}", job_id));

        let job_json: Option<String> = conn.get(&job_key).await?;

        if let Some(json) = job_json {
            let job: Job = serde_json::from_str(&json)
                .map_err(|e| QueueError::Deserialization(e.to_string()))?;
            Ok(Some(job))
        } else {
            Ok(None)
        }
    }

    /// Save a job.
    async fn save_job(&self, job: &Job) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let job_key = self.config.key(&format!("job:{}", job.id));
        let job_json =
            serde_json::to_string(job).map_err(|e| QueueError::Serialization(e.to_string()))?;

        let _: () = conn
            .set_ex(&job_key, job_json, self.config.retention_time.as_secs())
            .await?;
        Ok(())
    }

    /// Get queue size.
    pub async fn size(&self) -> QueueResult<usize> {
        let mut conn = self.connection.clone();
        let mut total = 0;

        for priority in [
            JobPriority::Critical,
            JobPriority::High,
            JobPriority::Normal,
            JobPriority::Low,
        ] {
            let queue_key = self.priority_queue_key(priority);
            let count: usize = conn.zcard(&queue_key).await?;
            total += count;
        }

        Ok(total)
    }

    /// Move delayed jobs to ready queue.
    async fn move_delayed_jobs(&self) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let delayed_key = self.config.key("delayed");
        let now = Utc::now().timestamp();

        // Get all jobs that are ready
        let job_ids: Vec<String> = conn.zrangebyscore(&delayed_key, "-inf", now).await?;

        for job_id_str in job_ids {
            if let Ok(job_id) = job_id_str.parse::<JobId>()
                && let Some(job) = self.get_job(job_id).await?
                && job.is_ready()
            {
                // Remove from delayed
                let _: () = conn.zrem(&delayed_key, job_id.to_string()).await?;

                // Add to priority queue
                let queue_key = self.priority_queue_key(job.priority);
                let score = -(job.priority as i64);
                let _: () = conn.zadd(&queue_key, job_id.to_string(), score).await?;
            }
        }

        Ok(())
    }

    /// Remove job from processing set.
    async fn remove_from_processing(&self, job_id: JobId) -> QueueResult<()> {
        let mut conn = self.connection.clone();
        let processing_key = self.config.key("processing");
        let _: () = conn.zrem(&processing_key, job_id.to_string()).await?;
        Ok(())
    }

    /// Get the priority queue key.
    fn priority_queue_key(&self, priority: JobPriority) -> String {
        self.config
            .key(&format!("pending:{:?}", priority).to_lowercase())
    }

    /// Clear all jobs from the queue.
    pub async fn clear(&self) -> QueueResult<()> {
        let mut conn = self.connection.clone();

        let pattern = format!("{}:*", self.config.key_prefix);
        let keys: Vec<String> = conn.keys(&pattern).await?;

        if !keys.is_empty() {
            let _: () = conn.del(keys).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_config() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert_eq!(config.queue_name, "test");
        assert!(config.key_prefix.contains("test"));
    }

    #[test]
    fn test_priority_queue_key() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert!(config.key("pending:high").contains("high"));
    }

    #[test]
    fn test_queue_config_with_custom_prefix() {
        let config = QueueConfig::new("redis://localhost:6379", "myqueue").with_key_prefix("app");
        assert!(config.key_prefix.contains("app"));
    }

    #[test]
    fn test_queue_config_default_retention() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert_eq!(config.retention_time, Duration::from_secs(86400)); // 1 day
    }

    #[test]
    fn test_queue_config_custom_retention() {
        let retention = Duration::from_secs(3600);
        let config =
            QueueConfig::new("redis://localhost:6379", "test").with_retention_time(retention);
        assert_eq!(config.retention_time, retention);
    }

    #[test]
    fn test_queue_config_default_max_size() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        assert_eq!(config.max_size, 0); // 0 means unlimited
    }

    #[test]
    fn test_queue_config_custom_max_size() {
        let config = QueueConfig::new("redis://localhost:6379", "test").with_max_size(1000);
        assert_eq!(config.max_size, 1000);
    }

    #[test]
    fn test_queue_key_generation() {
        let config = QueueConfig::new("redis://localhost:6379", "jobs");

        let pending_key = config.key("pending:normal");
        let processing_key = config.key("processing");
        let completed_key = config.key("completed");

        assert!(pending_key.contains("jobs"));
        assert!(processing_key.contains("jobs"));
        assert!(completed_key.contains("jobs"));
    }

    #[test]
    fn test_queue_config_clone() {
        let config1 = QueueConfig::new("redis://localhost:6379", "test");
        let config2 = config1.clone();

        assert_eq!(config1.queue_name, config2.queue_name);
        assert_eq!(config1.redis_url, config2.redis_url);
    }

    #[test]
    fn test_queue_config_different_queues() {
        let config1 = QueueConfig::new("redis://localhost:6379", "queue1");
        let config2 = QueueConfig::new("redis://localhost:6379", "queue2");

        assert_ne!(config1.key_prefix, config2.key_prefix);
    }

    #[test]
    fn test_queue_config_key_consistency() {
        let config = QueueConfig::new("redis://localhost:6379", "test");

        let key1 = config.key("pending");
        let key2 = config.key("pending");

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_queue_config_builder_pattern() {
        let config = QueueConfig::new("redis://localhost:6379", "test")
            .with_key_prefix("app")
            .with_retention_time(Duration::from_secs(7200))
            .with_max_size(500);

        assert!(config.key_prefix.contains("app"));
        assert_eq!(config.retention_time, Duration::from_secs(7200));
        assert_eq!(config.max_size, 500);
    }

    #[test]
    fn test_queue_config_redis_url() {
        let url = "redis://user:pass@host:6380/2";
        let config = QueueConfig::new(url, "test");
        assert_eq!(config.redis_url, url);
    }

    #[test]
    fn test_queue_config_key_with_empty_suffix() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        let key = config.key("");
        assert!(key.contains("test"));
    }

    #[test]
    fn test_queue_config_key_with_special_characters() {
        let config = QueueConfig::new("redis://localhost:6379", "test");
        let key = config.key("pending:high:priority");
        assert!(key.contains("pending:high:priority"));
    }

    #[test]
    fn test_queue_config_multiple_prefixes() {
        let config1 =
            QueueConfig::new("redis://localhost:6379", "app1").with_key_prefix("production");
        let config2 =
            QueueConfig::new("redis://localhost:6379", "app2").with_key_prefix("development");

        let key1 = config1.key("jobs");
        let key2 = config2.key("jobs");

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_queue_config_unlimited_max_size() {
        let config = QueueConfig::new("redis://localhost:6379", "test").with_max_size(0);
        assert_eq!(config.max_size, 0);
    }

    #[test]
    fn test_queue_config_large_retention() {
        let week = Duration::from_secs(7 * 24 * 3600);
        let config = QueueConfig::new("redis://localhost:6379", "test").with_retention_time(week);
        assert_eq!(config.retention_time, week);
    }
}
