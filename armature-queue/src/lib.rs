//! Job queue and background processing for Armature framework.
//!
//! Provides a robust job queue system with:
//! - 📦 Redis-backed persistence
//! - 🔄 Automatic retries with exponential backoff
//! - ⭐ Job priorities
//! - ⏰ Delayed/scheduled jobs
//! - 💀 Dead letter queue
//! - 📊 Job progress tracking
//! - 🎯 Multiple queues
//! - 👷 Worker pools
//! - ♻️ Stale-claim reclaim for jobs orphaned by a crashed worker
//!
//! ## Delivery semantics
//!
//! Delivery is **at-least-once**, so job handlers must be idempotent. A worker
//! that is SIGKILLed (or whose handler task panics) after its side effects but
//! before `complete()` is indistinguishable from one that died before them, so
//! [`Queue::reclaim_stale`] returns the job to its pending queue and it runs
//! again. [`Worker::start`] runs that reaper in the background on
//! [`WorkerConfig::visibility_timeout`]; without it such a job would sit in the
//! `processing` set forever, never retried and never dead-lettered.
//!
//! ## Quick Start - Job Creation
//!
//! ```
//! use armature_queue::{Job, JobData, JobPriority};
//! use serde_json::json;
//!
//! let job = Job::new(
//!     "emails",
//!     "send_welcome",
//!     json!({"to": "user@example.com"})
//! );
//!
//! assert_eq!(job.queue, "emails");
//! assert_eq!(job.job_type, "send_welcome");
//! assert_eq!(job.priority, JobPriority::Normal);
//! ```
//!
//! ## Job Priorities
//!
//! ```
//! use armature_queue::{Job, JobData, JobPriority};
//! use serde_json::json;
//!
//! // Create high priority job
//! let urgent = Job::new(
//!     "tasks",
//!     "urgent_task",
//!     json!({})
//! ).with_priority(JobPriority::High);
//!
//! // Create low priority job
//! let background = Job::new(
//!     "tasks",
//!     "cleanup",
//!     json!({})
//! ).with_priority(JobPriority::Low);
//!
//! assert_eq!(urgent.priority, JobPriority::High);
//! assert_eq!(background.priority, JobPriority::Low);
//! assert!(urgent.priority > background.priority);
//! ```
//!
//! ## Delayed Jobs
//!
//! ```
//! use armature_queue::Job;
//! use serde_json::json;
//! use chrono::Duration;
//!
//! // Schedule job to run in 1 hour
//! let scheduled = Job::new(
//!     "emails",
//!     "reminder",
//!     json!({"message": "Don't forget!"})
//! ).schedule_after(Duration::hours(1));
//!
//! assert!(scheduled.scheduled_at.is_some());
//! ```
//!
//! ## Queue Configuration
//!
//! ```
//! use armature_queue::QueueConfig;
//! use std::time::Duration;
//!
//! let config = QueueConfig::new("redis://localhost:6379", "emails")
//!     .with_key_prefix("myapp:queue:emails")
//!     .with_max_size(10000)
//!     .with_retention_time(Duration::from_secs(86400));
//!
//! assert_eq!(config.queue_name, "emails");
//! assert_eq!(config.max_size, 10000);
//! assert_eq!(config.retention_time, Duration::from_secs(86400));
//! ```
//!
//! ## Complete Example
//!
//! ```no_run
//! use armature_queue::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), QueueError> {
//!     // Create a queue
//!     let queue = Queue::new("redis://localhost:6379", "default").await?;
//!
//!     // Enqueue a job
//!     let job_id = queue.enqueue(
//!         "send_email",
//!         serde_json::json!({
//!             "to": "user@example.com",
//!             "subject": "Hello"
//!         })
//!     ).await?;
//!
//!     // Process jobs
//!     let mut worker = Worker::new(queue);
//!     worker
//!         .register_handler("send_email", |job| async move {
//!             // Send email logic
//!             Ok(())
//!         })
//!         .await;
//!
//!     worker.start().await?;
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod job;
pub mod queue;
pub mod worker;

pub use error::{QueueError, QueueResult};
pub use job::{Job, JobData, JobId, JobPriority, JobState, JobStatus};
pub use queue::{Queue, QueueConfig};
pub use worker::{JobHandler, Worker, WorkerConfig};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::error::{QueueError, QueueResult};
    pub use crate::job::{Job, JobData, JobId, JobPriority, JobState, JobStatus};
    pub use crate::queue::{Queue, QueueConfig};
    pub use crate::worker::{JobHandler, Worker, WorkerConfig};
}
