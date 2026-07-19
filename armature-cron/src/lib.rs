//! Cron job scheduling for Armature framework.
//!
//! Provides a robust cron job scheduler with support for:
//! - ⏰ Standard 6-field cron expressions (with seconds)
//! - 📛 Named jobs with metadata
//! - 🚀 Async job execution
//! - 🔒 Job overlap prevention
//! - 🎚️ Bounded concurrency via `max_concurrent_jobs`
//!
//! ## Quick Start - Cron Expressions
//!
//! ```
//! use armature_cron::CronExpression;
//!
//! // Parse a cron expression for "every hour"
//! let expr = CronExpression::parse("0 0 * * * *").unwrap();
//!
//! // Get next execution time
//! let now = chrono::Utc::now();
//! let next = expr.next_after(now);
//!
//! assert!(next.is_some());
//! assert!(next.unwrap() > now);
//! ```
//!
//! ## Cron Expression Presets
//!
//! ```
//! use armature_cron::expression::{CronExpression, CronPresets};
//!
//! // Use preset expressions
//! let every_minute = CronExpression::parse(CronPresets::EVERY_MINUTE).unwrap();
//! let every_hour = CronExpression::parse(CronPresets::EVERY_HOUR).unwrap();
//! let daily = CronExpression::parse(CronPresets::DAILY).unwrap();
//!
//! // Verify they parse correctly
//! let now = chrono::Utc::now();
//! assert!(every_minute.next_after(now).unwrap() > now);
//! assert!(every_hour.next_after(now).unwrap() > now);
//! assert!(daily.next_after(now).unwrap() > now);
//! ```
//!
//! ## Complete Example
//!
//! ```no_run
//! use armature_cron::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), CronError> {
//!     let mut scheduler = CronScheduler::new();
//!
//!     // Schedule a job to run every minute
//!     scheduler.add_job(
//!         "cleanup",
//!         "0 * * * * *",
//!         |context| Box::pin(async move {
//!             println!("Running cleanup job");
//!             Ok(())
//!         })
//!     ).await?;
//!
//!     // Start the scheduler
//!     scheduler.start().await?;
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod expression;
pub mod job;
pub mod scheduler;

pub use error::{CronError, CronResult};
pub use expression::CronExpression;
pub use job::{Job, JobContext, JobFn, JobStatus};
pub use scheduler::{CronScheduler, SchedulerConfig};

/// Re-export commonly used types
pub mod prelude {
    pub use crate::error::{CronError, CronResult};
    pub use crate::expression::CronExpression;
    pub use crate::job::{Job, JobContext, JobFn, JobStatus};
    pub use crate::scheduler::{CronScheduler, SchedulerConfig};
}
