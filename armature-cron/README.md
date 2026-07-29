# armature-cron

Cron job scheduling for the Armature framework.

## Features

- **Cron Expressions** - 6-field cron syntax with seconds precision
- **Named Jobs** - Identify and manage jobs by name
- **Async Tasks** - Non-blocking, concurrent job execution
- **Bounded Concurrency** - Cap simultaneous executions with `max_concurrent_jobs`
- **Job Status** - Track execution counts, last/next run, and failures

All schedules are evaluated in UTC.

## Installation

```toml
[dependencies]
armature-cron = "0.1"
```

## Quick Start

```rust
use armature_cron::{CronResult, CronScheduler};

#[tokio::main]
async fn main() -> CronResult<()> {
    let mut scheduler = CronScheduler::new();

    // Run every minute (fields: sec min hour day-of-month month day-of-week)
    scheduler
        .add_job("cleanup", "0 * * * * *", |_ctx| async {
            println!("Running cleanup...");
            Ok(())
        })
        .await?;

    // Run daily at midnight
    scheduler
        .add_job("daily_report", "0 0 0 * * *", |_ctx| async {
            println!("Generating daily report...");
            Ok(())
        })
        .await?;

    // Run every Monday at 9am
    scheduler
        .add_job("weekly_email", "0 0 9 * * MON", |_ctx| async {
            println!("Sending weekly digest...");
            Ok(())
        })
        .await?;

    scheduler.start().await?;
    Ok(())
}
```

`add_job` registers the job synchronously and returns
`CronError::JobAlreadyExists` if the name is already in use. The job closure
receives a `JobContext` and returns `CronResult<()>`.

## Cron Syntax

This crate uses **6-field** expressions with a leading seconds field:

```
┌───────────── second (0-59)
│ ┌───────────── minute (0-59)
│ │ ┌───────────── hour (0-23)
│ │ │ ┌───────────── day of month (1-31)
│ │ │ │ ┌───────────── month (1-12)
│ │ │ │ │ ┌───────────── day of week (0-6, Sun=0)
│ │ │ │ │ │
* * * * * *
```

## License

MIT OR Apache-2.0

