# Cron Job Scheduling Guide

Armature provides a robust cron job scheduler for running periodic tasks in your application.

## Features

- ✅ Standard cron expression syntax
- ✅ Named jobs with metadata
- ✅ Async job execution
- ✅ Job lifecycle management
- ✅ Error handling and retry logic
- ✅ Job overlap prevention
- ✅ Job enable/disable at runtime
- ✅ Job statistics and monitoring

## Table of Contents

- [Basic Usage](#basic-usage)
- [Cron Expressions](#cron-expressions)
- [Job Context](#job-context)
- [Scheduler Configuration](#scheduler-configuration)
- [Job Management](#job-management)
- [Error Handling](#error-handling)
- [Job Statistics](#job-statistics)
- [Common Patterns](#common-patterns)
- [Best Practices](#best-practices)

## Basic Usage

### Creating a Scheduler

```rust
use armature_cron::*;

#[tokio::main]
async fn main() -> Result<(), CronError> {
    let mut scheduler = CronScheduler::new();

    // Add jobs
    scheduler.add_job(
        "my_job",
        "0 * * * * *", // Every minute
        |ctx| Box::pin(async move {
            println!("Job executed!");
            Ok(())
        })
    )?;

    // Start the scheduler
    scheduler.start().await?;

    // Keep running
    tokio::signal::ctrl_c().await?;

    // Stop the scheduler
    scheduler.stop().await?;

    Ok(())
}
```

### Adding Jobs

```rust
// Simple job
scheduler.add_job(
    "heartbeat",
    "*/5 * * * * *", // Every 5 seconds
    |ctx| Box::pin(async move {
        println!("Heartbeat");
        Ok(())
    })
)?;

// Job with shared state
let counter = Arc::new(AtomicU32::new(0));
let counter_clone = counter.clone();

scheduler.add_job(
    "counter",
    "0 * * * * *", // Every minute
    move |ctx| {
        let counter = counter_clone.clone();
        Box::pin(async move {
            let count = counter.fetch_add(1, Ordering::SeqCst);
            println!("Count: {}", count + 1);
            Ok(())
        })
    }
)?;
```

## Cron Expressions

### Format

Cron expressions consist of 6 fields:

```
┌───────────── second (0-59)
│ ┌───────────── minute (0-59)
│ │ ┌───────────── hour (0-23)
│ │ │ ┌───────────── day of month (1-31)
│ │ │ │ ┌───────────── month (1-12 or JAN-DEC)
│ │ │ │ │ ┌───────────── day of week (0-6 or SUN-SAT, 0=Sunday)
│ │ │ │ │ │
│ │ │ │ │ │
* * * * * *
```

### Special Characters

- `*` - Any value
- `,` - Value list separator (e.g., `1,3,5`)
- `-` - Range (e.g., `1-5`)
- `/` - Step values (e.g., `*/5` = every 5)

### Common Expressions

```rust
use armature_cron::CronPresets;

// Every second
"* * * * * *"

// Every minute
"0 * * * * *"
// or
CronPresets::EVERY_MINUTE

// Every 5 minutes
"0 */5 * * * *"
CronPresets::EVERY_5_MINUTES

// Every hour
"0 0 * * * *"
CronPresets::EVERY_HOUR

// Every day at midnight
"0 0 0 * * *"
CronPresets::DAILY

// Every Monday at 9 AM
"0 0 9 * * MON"

// Weekdays at 9 AM
"0 0 9 * * MON-FRI"
CronPresets::WEEKDAYS_9AM

// First day of every month
"0 0 0 1 * *"
CronPresets::MONTHLY

// Every 15 minutes during business hours (9 AM - 5 PM)
"0 */15 9-17 * * *"

// Every weekend at 10 AM
"0 0 10 * * SAT,SUN"
CronPresets::WEEKENDS_10AM
```

## Job Context

Every job receives a `JobContext` with information about the execution:

```rust
scheduler.add_job(
    "report",
    "0 * * * * *",
    |ctx| Box::pin(async move {
        println!("Job name: {}", ctx.name);
        println!("Scheduled time: {}", ctx.scheduled_time);
        println!("Actual time: {}", ctx.execution_time);
        println!("Execution count: {}", ctx.execution_count);
        println!("Delay: {:?}", ctx.delay());
        Ok(())
    })
)?;
```

### JobContext Fields

```rust
pub struct JobContext {
    /// Job name
    pub name: String,

    /// When the job was scheduled to run
    pub scheduled_time: DateTime<Utc>,

    /// When the job actually started running
    pub execution_time: DateTime<Utc>,

    /// Number of times this job has executed (0-based)
    pub execution_count: u64,
}
```

## Scheduler Configuration

### Custom Configuration

```rust
use std::time::Duration;

let config = SchedulerConfig {
    // How often to check for jobs to run
    tick_interval: Duration::from_secs(1),

    // Whether to run jobs that were missed during downtime
    run_missed_jobs: false,

    // Maximum number of jobs to run concurrently
    max_concurrent_jobs: 10,

    // Whether to log job execution
    log_execution: true,
};

let mut scheduler = CronScheduler::with_config(config);
```

### Default Configuration

```rust
SchedulerConfig {
    tick_interval: Duration::from_secs(1),
    run_missed_jobs: false,
    max_concurrent_jobs: 10,
    log_execution: true,
}
```

## Job Management

### Listing Jobs

```rust
let jobs = scheduler.list_jobs().await;
for job_name in jobs {
    println!("Job: {}", job_name);
}
```

### Enabling/Disabling Jobs

```rust
// Disable a job (it won't run but stays in the scheduler)
scheduler.disable_job("my_job").await?;

// Re-enable a job
scheduler.enable_job("my_job").await?;
```

### Removing Jobs

```rust
// Completely remove a job from the scheduler
scheduler.remove_job("my_job").await?;
```

### Checking Status

```rust
// Check if scheduler is running
if scheduler.is_running().await {
    println!("Scheduler is active");
}
```

## Error Handling

### Handling Job Errors

```rust
scheduler.add_job(
    "api_sync",
    "0 */5 * * * *",
    |ctx| Box::pin(async move {
        match fetch_api_data().await {
            Ok(data) => {
                process_data(data).await?;
                Ok(())
            }
            Err(e) => {
                eprintln!("Failed to fetch API data: {}", e);
                Err(CronError::ExecutionFailed(e.to_string()))
            }
        }
    })
)?;
```

### Error Types

```rust
pub enum CronError {
    /// Invalid cron expression
    InvalidExpression(String),

    /// Job not found
    JobNotFound(String),

    /// Job already exists
    JobAlreadyExists(String),

    /// Job execution failed
    ExecutionFailed(String),

    /// Scheduler not running
    SchedulerNotRunning,

    /// Scheduler already running
    SchedulerAlreadyRunning,

    /// Configuration error
    Config(String),

    /// Generic error
    Other(String),
}
```

## Job Statistics

### Getting Job Stats

```rust
let stats = scheduler.get_stats("my_job").await?;

println!("Job: {}", stats.name);
println!("Enabled: {}", stats.enabled);
println!("Executions: {}", stats.execution_count);

if let Some(last_run) = stats.last_run {
    println!("Last run: {}", last_run);
}

if let Some(next_run) = stats.next_run {
    println!("Next run: {}", next_run);
}

match stats.status {
    JobStatus::Scheduled => println!("Status: Waiting"),
    JobStatus::Running => println!("Status: Running"),
    JobStatus::Completed => println!("Status: Completed"),
    JobStatus::Failed(err) => println!("Status: Failed - {}", err),
}
```

## Common Patterns

### Database Cleanup Job

```rust
scheduler.add_job(
    "cleanup_old_records",
    "0 0 2 * * *", // 2 AM daily
    |ctx| Box::pin(async move {
        let db = get_database_connection().await?;

        let deleted = db.execute(
            "DELETE FROM sessions WHERE expires_at < NOW()"
        ).await?;

        println!("Deleted {} expired sessions", deleted);
        Ok(())
    })
)?;
```

### Report Generation

```rust
scheduler.add_job(
    "daily_report",
    "0 0 8 * * MON-FRI", // 8 AM on weekdays
    |ctx| Box::pin(async move {
        let report = generate_daily_report().await?;
        send_email_report(report).await?;
        println!("Daily report sent");
        Ok(())
    })
)?;
```

### Cache Warming

```rust
scheduler.add_job(
    "warm_cache",
    "0 */15 * * * *", // Every 15 minutes
    |ctx| Box::pin(async move {
        let cache = get_cache().await?;
        let data = fetch_frequently_accessed_data().await?;
        cache.set("popular_items", data, Some(Duration::from_secs(900))).await?;
        Ok(())
    })
)?;
```

### API Rate Limit Reset

```rust
scheduler.add_job(
    "reset_rate_limits",
    "0 0 * * * *", // Every hour
    |ctx| Box::pin(async move {
        let limiter = get_rate_limiter().await?;
        limiter.reset_hourly_limits().await?;
        println!("Rate limits reset");
        Ok(())
    })
)?;
```

### Health Check

```rust
scheduler.add_job(
    "health_check",
    "*/30 * * * * *", // Every 30 seconds
    |ctx| Box::pin(async move {
        let status = check_service_health().await?;

        if !status.is_healthy {
            send_alert("Service unhealthy").await?;
        }

        Ok(())
    })
)?;
```

## Best Practices

### 1. Prevent Job Overlap

Jobs automatically prevent overlapping executions by default:

```rust
// This is enabled by default
job.prevent_overlap = true;
```

If a job is still running when its next scheduled time arrives, it will be skipped.

### 2. Use Appropriate Intervals

Don't schedule jobs too frequently:

```rust
// ❌ Bad: Too frequent, might cause performance issues
"* * * * * *" // Every second

// ✅ Good: Reasonable interval
"0 */5 * * * *" // Every 5 minutes
```

### 3. Handle Errors Gracefully

```rust
scheduler.add_job(
    "resilient_job",
    "0 * * * * *",
    |ctx| Box::pin(async move {
        let result = risky_operation().await;

        match result {
            Ok(data) => {
                process(data).await?;
                Ok(())
            }
            Err(e) => {
                // Log error but don't crash
                eprintln!("Job failed: {}", e);
                // Optionally return error to mark job as failed
                Err(CronError::ExecutionFailed(e.to_string()))
            }
        }
    })
)?;
```

### 4. Use Shared State Carefully

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

let shared_state = Arc::new(RwLock::new(AppState::new()));
let state_clone = shared_state.clone();

scheduler.add_job(
    "state_job",
    "0 * * * * *",
    move |ctx| {
        let state = state_clone.clone();
        Box::pin(async move {
            let mut state = state.write().await;
            state.update().await?;
            Ok(())
        })
    }
)?;
```

### 5. Monitor Job Execution

```rust
// Periodically check job statistics
scheduler.add_job(
    "monitor",
    "0 */10 * * * *", // Every 10 minutes
    |ctx| Box::pin(async move {
        let stats = get_all_job_stats().await?;

        for stat in stats {
            if let JobStatus::Failed(err) = stat.status {
                send_alert(&format!("Job {} failed: {}", stat.name, err)).await?;
            }
        }

        Ok(())
    })
)?;
```

### 6. Graceful Shutdown

```rust
#[tokio::main]
async fn main() -> Result<(), CronError> {
    let mut scheduler = CronScheduler::new();

    // Add jobs...

    scheduler.start().await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    println!("Shutting down scheduler...");
    scheduler.stop().await?;

    // Wait for running jobs to complete
    tokio::time::sleep(Duration::from_secs(5)).await;

    println!("Shutdown complete");
    Ok(())
}
```

### 7. Use Job Metadata

```rust
// Jobs support metadata for additional information
let mut job = Job::new("backup", expression, handler);
job.set_metadata("environment", "production");
job.set_metadata("priority", "high");
job.set_metadata("owner", "ops-team");
```

## Integration with Armature

### Using with Dependency Injection

```rust
use armature_framework::prelude::*;
use armature_cron::*;

#[injectable]
struct CronService {
    scheduler: Arc<RwLock<CronScheduler>>,
}

impl CronService {
    pub fn new() -> Self {
        Self {
            scheduler: Arc::new(RwLock::new(CronScheduler::new())),
        }
    }

    pub async fn setup_jobs(&self) -> CronResult<()> {
        let mut scheduler = self.scheduler.write().await;

        scheduler.add_job(
            "cleanup",
            "0 0 * * * *",
            |ctx| Box::pin(async move {
                // Job logic
                Ok(())
            })
        )?;

        scheduler.start().await?;
        Ok(())
    }
}

#[module]
struct AppModule {
    providers: vec![CronService::provider()],
}
```

### HTTP Endpoints for Job Management

```rust
#[controller("/api/cron")]
struct CronController {
    cron_service: CronService,
}

#[routes]
impl CronController {
    #[get("/jobs")]
    async fn list_jobs(&self) -> Json<Vec<String>> {
        let scheduler = self.cron_service.scheduler.read().await;
        Json(scheduler.list_jobs().await)
    }

    #[post("/jobs/:name/enable")]
    async fn enable_job(&self, #[param] name: String) -> Result<HttpResponse, Error> {
        let scheduler = self.cron_service.scheduler.read().await;
        scheduler.enable_job(&name).await
            .map_err(|e| Error::BadRequest(e.to_string()))?;
        Ok(HttpResponse::ok("Job enabled"))
    }

    #[post("/jobs/:name/disable")]
    async fn disable_job(&self, #[param] name: String) -> Result<HttpResponse, Error> {
        let scheduler = self.cron_service.scheduler.read().await;
        scheduler.disable_job(&name).await
            .map_err(|e| Error::BadRequest(e.to_string()))?;
        Ok(HttpResponse::ok("Job disabled"))
    }

    #[get("/jobs/:name/stats")]
    async fn get_stats(&self, #[param] name: String) -> Result<Json<JobStats>, Error> {
        let scheduler = self.cron_service.scheduler.read().await;
        let stats = scheduler.get_stats(&name).await
            .map_err(|e| Error::NotFound(e.to_string()))?;
        Ok(Json(stats))
    }
}
```

## Summary

The Armature cron system provides:

- ✅ **Standard cron syntax** for familiar scheduling
- ✅ **Async execution** for non-blocking jobs
- ✅ **Job management** with enable/disable/remove operations
- ✅ **Statistics** for monitoring and debugging
- ✅ **Error handling** with detailed error types
- ✅ **Overlap prevention** to avoid concurrent executions
- ✅ **Integration** with Armature's DI system

Perfect for:
- Database maintenance
- Report generation
- Cache warming
- Data synchronization
- Health checks
- Rate limit resets
- Cleanup tasks
- Scheduled notifications

For more examples, see `examples/cron_jobs.rs`.

