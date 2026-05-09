# Job Queue Guide

Armature provides a robust job queue system for background processing of asynchronous tasks using Redis as the backend.

## Features

- ✅ Redis-backed persistence
- ✅ Automatic retries with exponential backoff
- ✅ Job priorities (Low, Normal, High, Critical)
- ✅ Delayed/scheduled jobs
- ✅ Dead letter queue for failed jobs
- ✅ Job progress tracking
- ✅ Multiple queues support
- ✅ Concurrent worker pools
- ✅ Job timeouts
- ✅ Type-safe job data with JSON

## Table of Contents

- [Basic Usage](#basic-usage)
- [Job Lifecycle](#job-lifecycle)
- [Job Priorities](#job-priorities)
- [Delayed Jobs](#delayed-jobs)
- [Retries and Error Handling](#retries-and-error-handling)
- [Worker Configuration](#worker-configuration)
- [Multiple Queues](#multiple-queues)
- [Job Progress Tracking](#job-progress-tracking)
- [Best Practices](#best-practices)
- [Integration with Armature](#integration-with-armature)

## Basic Usage

### Creating a Queue

```rust
use armature_queue::*;

#[tokio::main]
async fn main() -> Result<(), QueueError> {
    // Connect to Redis
    let queue = Queue::new("redis://localhost:6379", "default").await?;

    Ok(())
}
```

### Enqueuing Jobs

```rust
// Simple job
let job_id = queue.enqueue(
    "send_email",
    serde_json::json!({
        "to": "user@example.com",
        "subject": "Welcome!",
        "body": "Thanks for signing up"
    })
).await?;

println!("Enqueued job: {}", job_id);
```

### Processing Jobs with Workers

```rust
// Create a worker
let mut worker = Worker::new(queue.clone());

// Register job handlers
worker.register_handler("send_email", |job| {
    Box::pin(async move {
        let to = job.data["to"].as_str().unwrap();
        let subject = job.data["subject"].as_str().unwrap();

        // Send email logic here
        println!("Sending email to {}: {}", to, subject);

        Ok(())
    })
});

// Start processing
worker.start().await?;

// Keep running
tokio::signal::ctrl_c().await?;

// Graceful shutdown
worker.stop().await?;
```

## Job Lifecycle

### Job States

```rust
pub enum JobState {
    Pending,    // Waiting to be processed
    Processing, // Currently being processed
    Completed,  // Successfully completed
    Failed,     // Failed but will retry
    Dead,       // Failed permanently (max retries exceeded)
}
```

### Job Flow

```
┌──────────┐
│ Enqueue  │
└────┬─────┘
     ▼
┌──────────┐
│ Pending  │ ◄─────────┐
└────┬─────┘           │
     ▼                 │ Retry
┌──────────┐           │ (with backoff)
│Processing│           │
└────┬─────┘           │
     ▼                 │
┌──────────┐    ┌──────┴───┐
│Completed │    │  Failed  │
└──────────┘    └────┬─────┘
                     ▼
                ┌─────────┐
                │  Dead   │
                │  (DLQ)  │
                └─────────┘
```

## Job Priorities

### Priority Levels

```rust
pub enum JobPriority {
    Low = 0,       // Lowest priority
    Normal = 1,    // Default priority
    High = 2,      // High priority
    Critical = 3,  // Highest priority
}
```

### Using Priorities

```rust
// Create a high-priority job
let urgent_job = Job::new(
    "default",
    "send_alert",
    serde_json::json!({
        "message": "System critical alert"
    })
).with_priority(JobPriority::Critical);

queue.enqueue_job(urgent_job).await?;
```

### Priority Behavior

- Workers process higher priority jobs first
- Jobs within the same priority are processed in FIFO order
- Critical jobs are processed before all others

## Delayed Jobs

### Schedule for Later

```rust
use chrono::{Utc, Duration};

// Schedule for a specific time
let scheduled_time = Utc::now() + Duration::hours(2);
let job = Job::new(
    "default",
    "send_reminder",
    serde_json::json!({"user_id": 123})
).schedule_at(scheduled_time);

queue.enqueue_job(job).await?;

// Schedule after a delay
let job = Job::new(
    "default",
    "cleanup_temp_files",
    serde_json::json!({})
).schedule_after(Duration::minutes(30));

queue.enqueue_job(job).await?;
```

### How Delayed Jobs Work

- Delayed jobs are stored separately until their scheduled time
- The queue automatically moves ready jobs to the pending queue
- Workers poll for ready jobs at regular intervals

## Retries and Error Handling

### Automatic Retries

```rust
// Configure max retry attempts
let job = Job::new(
    "default",
    "fetch_api_data",
    serde_json::json!({"url": "https://api.example.com/data"})
).with_max_attempts(5);

queue.enqueue_job(job).await?;
```

### Exponential Backoff

Jobs are retried with exponential backoff:

- 1st retry: 1 second delay
- 2nd retry: 2 seconds delay
- 3rd retry: 4 seconds delay
- 4th retry: 8 seconds delay
- And so on (max 1 hour)

### Handling Failures in Handlers

```rust
worker.register_handler("risky_operation", |job| {
    Box::pin(async move {
        match perform_operation().await {
            Ok(result) => {
                // Success
                Ok(())
            }
            Err(e) if e.is_retryable() => {
                // Temporary error, allow retry
                Err(QueueError::ExecutionFailed(e.to_string()))
            }
            Err(e) => {
                // Permanent error, mark as dead
                Err(QueueError::ExecutionFailed(
                    format!("Permanent failure: {}", e)
                ))
            }
        }
    })
});
```

### Dead Letter Queue

Failed jobs that exceed max retries are moved to the dead letter queue:

```rust
// Jobs in DLQ can be inspected and manually requeued if needed
// They are kept for debugging purposes
```

## Worker Configuration

### Custom Worker Config

```rust
use std::time::Duration;

let config = WorkerConfig {
    // Number of jobs to process concurrently
    concurrency: 5,

    // How often to poll for new jobs
    poll_interval: Duration::from_secs(1),

    // Maximum time a job can run
    job_timeout: Duration::from_secs(300), // 5 minutes

    // Whether to log job execution
    log_execution: true,
};

let worker = Worker::with_config(queue, config);
```

### Concurrency

- Each worker can process multiple jobs concurrently
- Set concurrency based on your workload and resources
- Multiple workers can process the same queue

### Timeouts

- Jobs that exceed the timeout are marked as failed
- The timeout applies to the entire job execution
- Configure based on your job characteristics

## Multiple Queues

### Creating Separate Queues

```rust
// High-priority queue for critical tasks
let critical_queue = Queue::new("redis://localhost:6379", "critical").await?;

// Default queue for normal tasks
let default_queue = Queue::new("redis://localhost:6379", "default").await?;

// Background queue for low-priority tasks
let background_queue = Queue::new("redis://localhost:6379", "background").await?;
```

### Dedicated Workers

```rust
// Worker for critical jobs
let mut critical_worker = Worker::with_config(
    critical_queue,
    WorkerConfig {
        concurrency: 10,
        ..Default::default()
    }
);

// Worker for background jobs
let mut background_worker = Worker::with_config(
    background_queue,
    WorkerConfig {
        concurrency: 2,
        ..Default::default()
    }
);

critical_worker.start().await?;
background_worker.start().await?;
```

## Job Progress Tracking

### Updating Progress

```rust
worker.register_handler("long_running_task", |mut job| {
    Box::pin(async move {
        // Update progress
        job.update_progress(25, Some("Processing step 1".to_string()));

        // Do work...
        step_1().await?;

        job.update_progress(50, Some("Processing step 2".to_string()));
        step_2().await?;

        job.update_progress(75, Some("Processing step 3".to_string()));
        step_3().await?;

        job.update_progress(100, Some("Complete".to_string()));

        Ok(())
    })
});
```

### Checking Job Status

```rust
// Get job status
if let Some(job) = queue.get_job(job_id).await? {
    println!("Job state: {:?}", job.status.state);
    println!("Progress: {}%", job.status.progress);

    if let Some(msg) = job.status.message {
        println!("Message: {}", msg);
    }
}
```

## Best Practices

### 1. Idempotent Handlers

Make job handlers idempotent (safe to retry):

```rust
worker.register_handler("create_user", |job| {
    Box::pin(async move {
        let email = job.data["email"].as_str().unwrap();

        // Check if already exists
        if user_exists(email).await? {
            println!("User already exists, skipping");
            return Ok(());
        }

        // Create user
        create_user(email).await?;

        Ok(())
    })
});
```

### 2. Small, Focused Jobs

```rust
// ❌ Bad: Monolithic job
queue.enqueue("process_order", serde_json::json!({
    "order_id": 123,
    "tasks": ["validate", "charge", "ship", "email", "sms"]
})).await?;

// ✅ Good: Separate jobs
queue.enqueue("validate_order", serde_json::json!({"order_id": 123})).await?;
queue.enqueue("charge_order", serde_json::json!({"order_id": 123})).await?;
queue.enqueue("ship_order", serde_json::json!({"order_id": 123})).await?;
queue.enqueue("send_confirmation", serde_json::json!({"order_id": 123})).await?;
```

### 3. Store Minimal Data

```rust
// ❌ Bad: Storing large data in job
queue.enqueue("process_image", serde_json::json!({
    "image_data": base64_encoded_image // Large!
})).await?;

// ✅ Good: Store reference
queue.enqueue("process_image", serde_json::json!({
    "image_url": "s3://bucket/image.jpg"
})).await?;
```

### 4. Handle Partial Failures

```rust
worker.register_handler("batch_process", |job| {
    Box::pin(async move {
        let items = job.data["items"].as_array().unwrap();
        let mut errors = Vec::new();

        for item in items {
            if let Err(e) = process_item(item).await {
                errors.push(e);
                // Continue processing other items
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(QueueError::ExecutionFailed(
                format!("Partial failure: {:?}", errors)
            ))
        }
    })
});
```

### 5. Monitor Queue Metrics

```rust
// Periodically check queue health
tokio::spawn(async move {
    loop {
        let size = queue.size().await.unwrap_or(0);

        if size > 1000 {
            eprintln!("WARNING: Queue backlog is high: {} jobs", size);
            // Alert operations team
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
});
```

### 6. Graceful Shutdown

```rust
#[tokio::main]
async fn main() -> Result<(), QueueError> {
    let mut worker = Worker::new(queue);

    // Register handlers...

    worker.start().await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    println!("Shutting down worker...");
    worker.stop().await?;

    // Wait for in-flight jobs to complete
    tokio::time::sleep(Duration::from_secs(30)).await;

    println!("Shutdown complete");
    Ok(())
}
```

## Integration with Armature

### Using DI for Queue Service

```rust
use armature_framework::prelude::*;
use armature_queue::*;

#[injectable]
struct QueueService {
    queue: Arc<Queue>,
}

impl QueueService {
    pub async fn new() -> Result<Self, QueueError> {
        let queue = Queue::new("redis://localhost:6379", "default").await?;
        Ok(Self {
            queue: Arc::new(queue),
        })
    }

    pub async fn enqueue_email(&self, to: &str, subject: &str) -> Result<JobId, QueueError> {
        self.queue.enqueue(
            "send_email",
            serde_json::json!({
                "to": to,
                "subject": subject
            })
        ).await
    }
}

#[controller("/api/users")]
struct UserController {
    queue_service: QueueService,
}

#[routes]
impl UserController {
    #[post("/register")]
    async fn register(&self, #[body] data: UserDto) -> Result<Json<Response>, Error> {
        // Save user to database...

        // Enqueue welcome email
        self.queue_service
            .enqueue_email(&data.email, "Welcome!")
            .await
            .map_err(|e| Error::InternalServerError(e.to_string()))?;

        Ok(Json(Response { success: true }))
    }
}
```

### Background Worker Service

```rust
#[injectable]
struct WorkerService {
    worker: Arc<RwLock<Worker>>,
}

impl WorkerService {
    pub async fn new(queue: Queue) -> Self {
        let mut worker = Worker::new(queue);

        // Register all handlers
        worker.register_handler("send_email", |job| {
            Box::pin(async move {
                send_email_impl(job.data).await
            })
        });

        worker.register_handler("process_image", |job| {
            Box::pin(async move {
                process_image_impl(job.data).await
            })
        });

        Self {
            worker: Arc::new(RwLock::new(worker)),
        }
    }

    pub async fn start(&self) -> Result<(), QueueError> {
        let mut worker = self.worker.write().await;
        worker.start().await
    }

    pub async fn stop(&self) -> Result<(), QueueError> {
        let mut worker = self.worker.write().await;
        worker.stop().await
    }
}

#[module]
struct AppModule {
    providers: vec![QueueService::provider(), WorkerService::provider()],
}
```

### Queue Management Endpoints

```rust
#[controller("/api/queue")]
struct QueueController {
    queue: QueueService,
}

#[routes]
impl QueueController {
    #[get("/stats")]
    async fn get_stats(&self) -> Result<Json<QueueStats>, Error> {
        let size = self.queue.queue.size().await
            .map_err(|e| Error::InternalServerError(e.to_string()))?;

        Ok(Json(QueueStats {
            pending_jobs: size,
            queue_name: "default".to_string(),
        }))
    }

    #[get("/job/:id")]
    async fn get_job(&self, #[param] id: String) -> Result<Json<Job>, Error> {
        let job_id = id.parse()
            .map_err(|_| Error::BadRequest("Invalid job ID".to_string()))?;

        let job = self.queue.queue.get_job(job_id).await
            .map_err(|e| Error::InternalServerError(e.to_string()))?
            .ok_or_else(|| Error::NotFound("Job not found".to_string()))?;

        Ok(Json(job))
    }
}
```

## Common Patterns

### Email Queue

```rust
worker.register_handler("send_email", |job| {
    Box::pin(async move {
        let to = job.data["to"].as_str().ok_or_else(||
            QueueError::ExecutionFailed("Missing 'to' field".to_string())
        )?;

        let subject = job.data["subject"].as_str().ok_or_else(||
            QueueError::ExecutionFailed("Missing 'subject' field".to_string())
        )?;

        let body = job.data["body"].as_str().unwrap_or("");

        // Send email via SMTP or API
        send_email(to, subject, body).await
            .map_err(|e| QueueError::ExecutionFailed(e.to_string()))?;

        Ok(())
    })
});
```

### Image Processing Queue

```rust
worker.register_handler("process_image", |job| {
    Box::pin(async move {
        let url = job.data["url"].as_str().unwrap();

        // Download image
        let image_data = download_image(url).await?;

        // Process (resize, compress, etc.)
        let processed = process_image(image_data).await?;

        // Upload to storage
        let new_url = upload_image(processed).await?;

        println!("Image processed: {}", new_url);

        Ok(())
    })
});
```

### Scheduled Reports

```rust
// Enqueue daily report generation
let tomorrow_9am = Utc::now()
    .date()
    .and_hms(9, 0, 0) + Duration::days(1);

let job = Job::new(
    "reports",
    "generate_daily_report",
    serde_json::json!({
        "report_type": "sales",
        "date": Utc::now().format("%Y-%m-%d").to_string()
    })
).schedule_at(tomorrow_9am);

queue.enqueue_job(job).await?;
```

## Summary

The Armature queue system provides:

- ✅ **Redis-backed reliability** for persistent job storage
- ✅ **Automatic retries** with exponential backoff
- ✅ **Priority queues** for important jobs
- ✅ **Delayed execution** for scheduled tasks
- ✅ **Concurrent processing** with worker pools
- ✅ **Job tracking** with progress updates
- ✅ **Dead letter queue** for failed jobs
- ✅ **Type-safe** job data with JSON

Perfect for:
- Email sending
- Image/video processing
- Report generation
- Data synchronization
- Batch operations
- Webhook delivery
- Scheduled tasks
- Any asynchronous work

For more examples, see `examples/queue_jobs.rs`.

