//! Job definition and state management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Job unique identifier.
pub type JobId = Uuid;

/// Job data payload.
pub type JobData = serde_json::Value;

/// Job priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum JobPriority {
    /// Lowest priority
    Low = 0,
    /// Normal priority (default)
    #[default]
    Normal = 1,
    /// High priority
    High = 2,
    /// Critical priority
    Critical = 3,
}

/// Job state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    /// Job is waiting to be processed
    Pending,
    /// Job is currently being processed
    Processing,
    /// Job completed successfully
    Completed,
    /// Job failed and will be retried
    Failed,
    /// Job failed permanently (max retries exceeded)
    Dead,
}

/// Job status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    /// Current state
    pub state: JobState,

    /// Progress percentage (0-100)
    pub progress: u8,

    /// Status message
    pub message: Option<String>,

    /// Error message (if failed)
    pub error: Option<String>,

    /// Last updated timestamp
    pub updated_at: DateTime<Utc>,
}

impl JobStatus {
    /// Create a new pending status.
    pub fn pending() -> Self {
        Self {
            state: JobState::Pending,
            progress: 0,
            message: None,
            error: None,
            updated_at: Utc::now(),
        }
    }

    /// Create a processing status.
    pub fn processing() -> Self {
        Self {
            state: JobState::Processing,
            progress: 0,
            message: None,
            error: None,
            updated_at: Utc::now(),
        }
    }

    /// Create a completed status.
    pub fn completed() -> Self {
        Self {
            state: JobState::Completed,
            progress: 100,
            message: None,
            error: None,
            updated_at: Utc::now(),
        }
    }

    /// Create a failed status.
    pub fn failed(error: String) -> Self {
        Self {
            state: JobState::Failed,
            progress: 0,
            message: None,
            error: Some(error),
            updated_at: Utc::now(),
        }
    }

    /// Create a dead status.
    pub fn dead(error: String) -> Self {
        Self {
            state: JobState::Dead,
            progress: 0,
            message: None,
            error: Some(error),
            updated_at: Utc::now(),
        }
    }

    /// Update progress.
    pub fn with_progress(mut self, progress: u8) -> Self {
        self.progress = progress.min(100);
        self.updated_at = Utc::now();
        self
    }

    /// Update message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self.updated_at = Utc::now();
        self
    }
}

/// A job to be processed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    /// Unique job identifier
    pub id: JobId,

    /// Job type/name
    pub job_type: String,

    /// Job payload data
    pub data: JobData,

    /// Job priority
    pub priority: JobPriority,

    /// Job status
    pub status: JobStatus,

    /// Number of attempts
    pub attempts: u32,

    /// Maximum number of retry attempts
    pub max_attempts: u32,

    /// Queue name
    pub queue: String,

    /// When the job was created
    pub created_at: DateTime<Utc>,

    /// When the job should be processed (for delayed jobs)
    pub scheduled_at: Option<DateTime<Utc>>,

    /// When the job was started
    pub started_at: Option<DateTime<Utc>>,

    /// When the job completed/failed
    pub completed_at: Option<DateTime<Utc>>,

    /// Job metadata
    pub metadata: HashMap<String, String>,
}

impl Job {
    /// Create a new job.
    pub fn new(queue: impl Into<String>, job_type: impl Into<String>, data: JobData) -> Self {
        Self {
            id: Uuid::new_v4(),
            job_type: job_type.into(),
            data,
            priority: JobPriority::default(),
            status: JobStatus::pending(),
            attempts: 0,
            max_attempts: 3,
            queue: queue.into(),
            created_at: Utc::now(),
            scheduled_at: None,
            started_at: None,
            completed_at: None,
            metadata: HashMap::new(),
        }
    }

    /// Set job priority.
    pub fn with_priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set max retry attempts.
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Schedule the job for later.
    pub fn schedule_at(mut self, time: DateTime<Utc>) -> Self {
        self.scheduled_at = Some(time);
        self
    }

    /// Schedule the job after a delay.
    pub fn schedule_after(mut self, duration: chrono::Duration) -> Self {
        self.scheduled_at = Some(Utc::now() + duration);
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Check if the job is ready to be processed.
    pub fn is_ready(&self) -> bool {
        if let Some(scheduled_at) = self.scheduled_at {
            Utc::now() >= scheduled_at
        } else {
            true
        }
    }

    /// Check if the job can be retried.
    pub fn can_retry(&self) -> bool {
        self.attempts < self.max_attempts
    }

    /// Mark job as processing.
    pub fn start_processing(&mut self) {
        self.status = JobStatus::processing();
        self.started_at = Some(Utc::now());
        self.attempts += 1;
    }

    /// Mark job as completed.
    pub fn complete(&mut self) {
        self.status = JobStatus::completed();
        self.completed_at = Some(Utc::now());
    }

    /// Mark job as failed.
    pub fn fail(&mut self, error: String) {
        if self.can_retry() {
            self.status = JobStatus::failed(error);
        } else {
            self.status = JobStatus::dead(error);
            self.completed_at = Some(Utc::now());
        }
    }

    /// Update job progress.
    pub fn update_progress(&mut self, progress: u8, message: Option<String>) {
        self.status.progress = progress.min(100);
        self.status.message = message;
        self.status.updated_at = Utc::now();
    }

    /// Calculate backoff delay for retry.
    pub fn backoff_delay(&self) -> chrono::Duration {
        // Exponential backoff: 2^(attempts-1) seconds, capped at 1 hour.
        //
        // The exponent is clamped to 20 (2^20 == 1_048_576, comfortably above
        // the 3600s cap) before the power is taken, and `saturating_pow` is
        // used besides, so an unbounded `attempts` (e.g. a job retried past
        // `u32::MAX / 2` times) can never overflow `i64` or wrap negative --
        // which would otherwise defeat the cap by making the "backoff" a
        // negative/zero delay and triggering an immediate retry storm.
        let exponent = self.attempts.saturating_sub(1).min(20);
        let seconds = 2_i64.saturating_pow(exponent);
        chrono::Duration::seconds(seconds.min(3600)) // Max 1 hour
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_creation() {
        let job = Job::new(
            "default",
            "send_email",
            serde_json::json!({"to": "test@example.com"}),
        );

        assert_eq!(job.queue, "default");
        assert_eq!(job.job_type, "send_email");
        assert_eq!(job.attempts, 0);
        assert_eq!(job.priority, JobPriority::Normal);
    }

    #[test]
    fn test_job_builder() {
        let job = Job::new("default", "task", serde_json::json!({}))
            .with_priority(JobPriority::High)
            .with_max_attempts(5)
            .with_metadata("user_id", "123");

        assert_eq!(job.priority, JobPriority::High);
        assert_eq!(job.max_attempts, 5);
        assert_eq!(job.metadata.get("user_id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_job_ready() {
        let mut job = Job::new("default", "task", serde_json::json!({}));
        assert!(job.is_ready());

        job = job.schedule_at(Utc::now() + chrono::Duration::hours(1));
        assert!(!job.is_ready());
    }

    #[test]
    fn test_job_retry_logic() {
        let mut job = Job::new("default", "task", serde_json::json!({}));
        job.max_attempts = 3;

        assert!(job.can_retry());

        job.start_processing();
        job.fail("Error 1".to_string());
        assert!(job.can_retry());
        assert_eq!(job.status.state, JobState::Failed);

        job.start_processing();
        job.fail("Error 2".to_string());
        assert!(job.can_retry());

        job.start_processing();
        job.fail("Error 3".to_string());
        assert!(!job.can_retry());
        assert_eq!(job.status.state, JobState::Dead);
    }

    #[test]
    fn test_backoff_delay() {
        let mut job = Job::new("default", "task", serde_json::json!({}));

        job.attempts = 1;
        assert_eq!(job.backoff_delay(), chrono::Duration::seconds(1));

        job.attempts = 2;
        assert_eq!(job.backoff_delay(), chrono::Duration::seconds(2));

        job.attempts = 3;
        assert_eq!(job.backoff_delay(), chrono::Duration::seconds(4));

        job.attempts = 10;
        assert_eq!(job.backoff_delay(), chrono::Duration::seconds(512));
    }

    #[test]
    fn test_backoff_delay_large_attempts_does_not_panic() {
        // Pre-fix this computed 2_i64.pow(attempts - 1) BEFORE the .min(3600)
        // cap, so attempts >= 63 would overflow i64 (panic in debug, wrap to
        // negative in release). Confirm the cap holds even at absurd attempt
        // counts, with no panic.
        let mut job = Job::new("default", "task", serde_json::json!({}));

        job.attempts = 100;
        assert_eq!(job.backoff_delay(), chrono::Duration::seconds(3600));

        job.attempts = u32::MAX;
        assert_eq!(job.backoff_delay(), chrono::Duration::seconds(3600));
    }

    #[test]
    fn test_job_priority_levels() {
        let low =
            Job::new("default", "task", serde_json::json!({})).with_priority(JobPriority::Low);
        let normal =
            Job::new("default", "task", serde_json::json!({})).with_priority(JobPriority::Normal);
        let high =
            Job::new("default", "task", serde_json::json!({})).with_priority(JobPriority::High);
        let critical =
            Job::new("default", "task", serde_json::json!({})).with_priority(JobPriority::Critical);

        assert_eq!(low.priority, JobPriority::Low);
        assert_eq!(normal.priority, JobPriority::Normal);
        assert_eq!(high.priority, JobPriority::High);
        assert_eq!(critical.priority, JobPriority::Critical);
    }

    #[test]
    fn test_job_metadata() {
        let job = Job::new("default", "task", serde_json::json!({}))
            .with_metadata("key1", "value1")
            .with_metadata("key2", "value2");

        assert_eq!(job.metadata.len(), 2);
        assert_eq!(job.metadata.get("key1"), Some(&"value1".to_string()));
        assert_eq!(job.metadata.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_job_schedule_at() {
        let future = Utc::now() + chrono::Duration::hours(2);
        let job = Job::new("default", "task", serde_json::json!({})).schedule_at(future);

        assert!(!job.is_ready());
        assert!(job.scheduled_at.is_some());
    }

    #[test]
    fn test_job_scheduled_at_in_future() {
        let future = Utc::now() + chrono::Duration::minutes(30);
        let job = Job::new("default", "task", serde_json::json!({})).schedule_at(future);

        assert!(!job.is_ready());
        assert!(job.scheduled_at.is_some());
    }

    #[test]
    fn test_job_status_transitions() {
        let mut job = Job::new("default", "task", serde_json::json!({}));

        assert_eq!(job.status.state, JobState::Pending);

        job.start_processing();
        assert_eq!(job.status.state, JobState::Processing);

        job.complete();
        assert_eq!(job.status.state, JobState::Completed);
    }

    #[test]
    fn test_job_failure_tracking() {
        let mut job = Job::new("default", "task", serde_json::json!({}));

        job.start_processing();
        job.fail("First error".to_string());

        assert_eq!(job.status.state, JobState::Failed);
        assert_eq!(job.status.error, Some("First error".to_string()));
        assert_eq!(job.attempts, 1);
    }

    #[test]
    fn test_job_max_attempts() {
        let job = Job::new("default", "task", serde_json::json!({})).with_max_attempts(10);

        assert_eq!(job.max_attempts, 10);
    }

    #[test]
    fn test_job_default_max_attempts() {
        let job = Job::new("default", "task", serde_json::json!({}));
        assert_eq!(job.max_attempts, 3);
    }

    #[test]
    fn test_job_can_retry_with_zero_max_attempts() {
        let mut job = Job::new("default", "task", serde_json::json!({})).with_max_attempts(0);

        job.start_processing();
        job.fail("Error".to_string());

        assert!(!job.can_retry());
    }

    #[test]
    fn test_job_id_uniqueness() {
        let job1 = Job::new("default", "task", serde_json::json!({}));
        let job2 = Job::new("default", "task", serde_json::json!({}));

        assert_ne!(job1.id, job2.id);
    }

    #[test]
    fn test_job_timestamps() {
        let before = Utc::now();
        let job = Job::new("default", "task", serde_json::json!({}));
        let after = Utc::now();

        assert!(job.created_at >= before);
        assert!(job.created_at <= after);
    }

    #[test]
    fn test_job_complete_sets_state() {
        let mut job = Job::new("default", "task", serde_json::json!({}));

        job.start_processing();
        job.complete();

        assert_eq!(job.status.state, JobState::Completed);
    }

    #[test]
    fn test_job_ready_with_past_schedule() {
        let past = Utc::now() - chrono::Duration::hours(1);
        let job = Job::new("default", "task", serde_json::json!({})).schedule_at(past);

        assert!(job.is_ready());
    }

    #[test]
    fn test_job_serialization_data() {
        let data = serde_json::json!({
            "email": "test@example.com",
            "subject": "Test",
            "count": 42
        });

        let job = Job::new("default", "send_email", data.clone());
        assert_eq!(job.data, data);
    }

    #[test]
    fn test_job_priority_ordering() {
        assert!(JobPriority::Low < JobPriority::Normal);
        assert!(JobPriority::Normal < JobPriority::High);
        assert!(JobPriority::High < JobPriority::Critical);
    }

    #[test]
    fn test_backoff_delay_exponential_growth() {
        let mut job = Job::new("default", "task", serde_json::json!({}));

        let delays: Vec<i64> = (1..=5)
            .map(|attempt| {
                job.attempts = attempt;
                job.backoff_delay().num_seconds()
            })
            .collect();

        // Verify exponential growth
        assert!(delays[0] < delays[1]);
        assert!(delays[1] < delays[2]);
        assert!(delays[2] < delays[3]);
        assert!(delays[3] < delays[4]);
    }

    #[test]
    fn test_job_state_dead_after_max_retries() {
        let mut job = Job::new("default", "task", serde_json::json!({})).with_max_attempts(2);

        job.start_processing();
        job.fail("Error 1".to_string());
        assert_eq!(job.status.state, JobState::Failed);

        job.start_processing();
        job.fail("Error 2".to_string());
        assert_eq!(job.status.state, JobState::Dead);
    }

    #[test]
    fn test_job_metadata_overwrite() {
        let job = Job::new("default", "task", serde_json::json!({}))
            .with_metadata("key", "value1")
            .with_metadata("key", "value2");

        assert_eq!(job.metadata.get("key"), Some(&"value2".to_string()));
    }
}
