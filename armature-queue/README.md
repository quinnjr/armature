# armature-queue

Background job queue for the Armature framework.

## Features

- **Redis-Backed** - Reliable job storage with Redis
- **Priorities** - Multiple priority levels
- **Retries** - Automatic retry with backoff
- **Scheduling** - Delayed job execution
- **Concurrency** - Multi-worker processing
- **Dead Letter Queue** - Failed job handling

## Installation

```toml
[dependencies]
armature-queue = "0.1"
```

## Quick Start

```rust
use armature_queue::{Queue, Worker, WorkerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a queue.
    let queue = Queue::new("redis://localhost:6379", "default").await?;

    // Enqueue a job.
    queue.enqueue(
        "send_email",
        serde_json::json!({
            "to": "user@example.com",
            "subject": "Welcome!"
        })
    ).await?;

    // Build a worker with a custom concurrency via `WorkerConfig`
    // (`Worker::new(queue)` uses the defaults).
    let config = WorkerConfig { concurrency: 4, ..Default::default() };
    let mut worker = Worker::with_config(queue, config);

    // Registration is async and completes before it returns, so the handler
    // is guaranteed to be present once `start()` begins dequeuing.
    worker
        .register_handler("send_email", |job| async move {
            println!("Sending email: {:?}", job.data);
            Ok(())
        })
        .await;

    // `start()` spawns the worker tasks and returns immediately.
    worker.start().await?;
    Ok(())
}
```

## Scheduling

```rust
use chrono::{Duration, Utc};

// Delayed execution: run 5 minutes from now.
queue.enqueue_in(
    Duration::minutes(5),
    "reminder",
    serde_json::json!({"message": "Don't forget!"})
).await?;

// Scheduled time: run at a specific instant.
queue.enqueue_at(
    Utc::now() + Duration::hours(1),
    "report",
    serde_json::json!({})
).await?;
```

## License

MIT OR Apache-2.0

