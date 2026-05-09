# Lifecycle Hooks

Armature provides a comprehensive lifecycle hook system that allows you to execute code at specific points during the application lifecycle. This is inspired by NestJS and provides similar functionality for Rust web applications.

## Table of Contents

- [Overview](#overview)
- [Available Hooks](#available-hooks)
- [Hook Execution Order](#hook-execution-order)
- [Usage](#usage)
- [Best Practices](#best-practices)
- [Examples](#examples)

---

## Overview

Lifecycle hooks enable you to perform operations like:

- **Initialization**: Connect to databases, start background tasks
- **Cleanup**: Close connections, flush caches, stop workers
- **Monitoring**: Log application state changes
- **Resource Management**: Acquire and release resources safely

All lifecycle hooks are **async** and return a `LifecycleResult`:

```rust
pub type LifecycleResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
```

---

## Available Hooks

### OnModuleInit

Called after a module's dependencies are resolved but before the application is fully bootstrapped.

```rust
#[async_trait]
pub trait OnModuleInit: Send + Sync {
    async fn on_module_init(&self) -> LifecycleResult;
}
```

**Use cases:**
- Initialize database connections
- Load configuration
- Set up caches
- Start background tasks specific to the module

### OnModuleDestroy

Called before a module is destroyed during application shutdown.

```rust
#[async_trait]
pub trait OnModuleDestroy: Send + Sync {
    async fn on_module_destroy(&self) -> LifecycleResult;
}
```

**Use cases:**
- Close database connections
- Flush caches
- Stop background tasks
- Clean up temporary resources

### OnApplicationBootstrap

Called after all modules have been initialized and the application is fully ready.

```rust
#[async_trait]
pub trait OnApplicationBootstrap: Send + Sync {
    async fn on_application_bootstrap(&self) -> LifecycleResult;
}
```

**Use cases:**
- Perform post-initialization setup
- Start global services
- Log application readiness
- Trigger initial data synchronization

### OnApplicationShutdown

Called during graceful application shutdown.

```rust
#[async_trait]
pub trait OnApplicationShutdown: Send + Sync {
    async fn on_application_shutdown(&self, signal: Option<String>) -> LifecycleResult;
}
```

**Use cases:**
- Gracefully terminate long-running operations
- Send final metrics/logs
- Notify external systems of shutdown
- Save application state

### BeforeApplicationShutdown

Called before the main shutdown hooks, allowing for pre-shutdown operations.

```rust
#[async_trait]
pub trait BeforeApplicationShutdown: Send + Sync {
    async fn before_application_shutdown(&self, signal: Option<String>) -> LifecycleResult;
}
```

**Use cases:**
- Stop accepting new requests
- Drain request queues
- Notify load balancers
- Prepare for shutdown

---

## Hook Execution Order

### Startup Sequence

```
1. Module Registration
   └─> Providers and controllers registered in DI container

2. OnModuleInit
   └─> Called for each service/controller (FIFO order)

3. OnApplicationBootstrap
   └─> Called after all modules initialized (FIFO order)

4. Application Ready
   └─> Server starts accepting requests
```

### Shutdown Sequence

```
1. Shutdown Signal Received
   └─> SIGTERM, SIGINT, or manual shutdown

2. BeforeApplicationShutdown
   └─> Called for pre-shutdown operations (FIFO order)

3. OnApplicationShutdown
   └─> Called for graceful shutdown (LIFO/reverse order)

4. OnModuleDestroy
   └─> Called for cleanup (LIFO/reverse order)

5. Application Terminated
```

**Important**: Destroy and shutdown hooks are called in **reverse order** (LIFO) to ensure proper cleanup of dependencies.

---

## Usage

### Basic Implementation

Implement lifecycle hooks on your services or controllers:

```rust
use armature_core::{Provider, lifecycle::{OnModuleInit, OnModuleDestroy}};
use async_trait::async_trait;

struct DatabaseService {
    connection: Option<Connection>,
}

impl Provider for DatabaseService {}

#[async_trait]
impl OnModuleInit for DatabaseService {
    async fn on_module_init(&self) -> LifecycleResult {
        println!("Connecting to database...");
        // Initialize database connection
        Ok(())
    }
}

#[async_trait]
impl OnModuleDestroy for DatabaseService {
    async fn on_module_destroy(&self) -> LifecycleResult {
        println!("Closing database connection...");
        // Close database connection
        Ok(())
    }
}
```

### Registration with Lifecycle Manager

```rust
use armature_core::LifecycleManager;
use std::sync::Arc;

let lifecycle = LifecycleManager::new();
let db_service = Arc::new(DatabaseService { connection: None });

// Register hooks
lifecycle.register_on_init("DatabaseService".to_string(), db_service.clone()).await;
lifecycle.register_on_destroy("DatabaseService".to_string(), db_service).await;
```

### Integration with Application

The lifecycle manager is integrated into `Application`:

```rust
use armature_core::Application;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create application (hooks are called automatically)
    let app = Application::create::<AppModule>().await;

    // Application runs...

    // Graceful shutdown
    app.shutdown(Some("SIGTERM".to_string())).await?;

    Ok(())
}
```

### Error Handling

Lifecycle hooks can return errors, which are collected and reported:

```rust
#[async_trait]
impl OnModuleInit for MyService {
    async fn on_module_init(&self) -> LifecycleResult {
        if let Err(e) = self.connect().await {
            return Err(format!("Failed to connect: {}", e).into());
        }
        Ok(())
    }
}
```

Errors don't stop the application lifecycle - all hooks are called, and errors are logged:

```
🔄 Calling module initialization hooks...
  ✗ MyService: onModuleInit() failed: Failed to connect: Connection refused
  ✓ OtherService: onModuleInit() completed
```

---

## Best Practices

### ✅ Do's

1. **Keep hooks fast**: Lifecycle hooks should complete quickly
2. **Handle errors gracefully**: Return meaningful errors
3. **Use appropriate hooks**: Choose the right hook for your use case
4. **Clean up resources**: Always implement both init and destroy if needed
5. **Log operations**: Provide visibility into what's happening
6. **Make hooks idempotent**: Hooks should be safe to call multiple times

### ❌ Don'ts

1. **Don't perform long-running operations**: Startup should be fast
2. **Don't ignore errors**: Always handle and return errors properly
3. **Don't assume order**: Don't rely on specific hook execution order
4. **Don't block**: Use async operations, not blocking I/O
5. **Don't panic**: Return errors instead of panicking

### Idempotency Example

```rust
struct CacheService {
    initialized: Arc<RwLock<bool>>,
}

#[async_trait]
impl OnModuleInit for CacheService {
    async fn on_module_init(&self) -> LifecycleResult {
        let mut init = self.initialized.write().await;

        // Guard against multiple initializations
        if *init {
            println!("Cache already initialized, skipping");
            return Ok(());
        }

        // Initialize cache
        println!("Initializing cache...");
        *init = true;
        Ok(())
    }
}
```

---

## Examples

### Example 1: Database Connection Service

```rust
use armature_core::{Provider, lifecycle::{OnModuleInit, OnModuleDestroy}};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

struct DatabaseService {
    connection_string: String,
    pool: Arc<RwLock<Option<ConnectionPool>>>,
}

impl Provider for DatabaseService {}

#[async_trait]
impl OnModuleInit for DatabaseService {
    async fn on_module_init(&self) -> LifecycleResult {
        println!("📊 Connecting to database: {}", self.connection_string);

        // Create connection pool
        let pool = create_pool(&self.connection_string).await?;
        *self.pool.write().await = Some(pool);

        println!("✅ Database connection established");
        Ok(())
    }
}

#[async_trait]
impl OnModuleDestroy for DatabaseService {
    async fn on_module_destroy(&self) -> LifecycleResult {
        println!("📊 Closing database connections...");

        // Close pool
        if let Some(pool) = self.pool.write().await.take() {
            pool.close().await?;
        }

        println!("✅ Database connections closed");
        Ok(())
    }
}
```

### Example 2: Background Worker

```rust
use armature_core::{Provider, lifecycle::{OnApplicationBootstrap, OnApplicationShutdown}};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

struct WorkerService {
    running: Arc<RwLock<bool>>,
    handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl Provider for WorkerService {}

#[async_trait]
impl OnApplicationBootstrap for WorkerService {
    async fn on_application_bootstrap(&self) -> LifecycleResult {
        println!("🔄 Starting background worker...");

        *self.running.write().await = true;
        let running = self.running.clone();

        let handle = tokio::spawn(async move {
            while *running.read().await {
                // Do work
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        });

        *self.handle.write().await = Some(handle);
        println!("✅ Background worker started");
        Ok(())
    }
}

#[async_trait]
impl OnApplicationShutdown for WorkerService {
    async fn on_application_shutdown(&self, signal: Option<String>) -> LifecycleResult {
        if let Some(sig) = signal {
            println!("🛑 Stopping worker (signal: {})...", sig);
        } else {
            println!("🛑 Stopping worker...");
        }

        // Signal worker to stop
        *self.running.write().await = false;

        // Wait for worker to finish
        if let Some(handle) = self.handle.write().await.take() {
            handle.await?;
        }

        println!("✅ Worker stopped gracefully");
        Ok(())
    }
}
```

### Example 3: Health Check Service

```rust
use armature_core::{Provider, lifecycle::{OnApplicationBootstrap, BeforeApplicationShutdown}};
use async_trait::async_trait;

struct HealthCheckService {
    endpoint: String,
}

impl Provider for HealthCheckService {}

#[async_trait]
impl OnApplicationBootstrap for HealthCheckService {
    async fn on_application_bootstrap(&self) -> LifecycleResult {
        println!("✅ Application ready - health checks enabled");

        // Notify load balancer that we're ready
        self.notify_ready().await?;

        Ok(())
    }
}

#[async_trait]
impl BeforeApplicationShutdown for HealthCheckService {
    async fn before_application_shutdown(&self, _signal: Option<String>) -> LifecycleResult {
        println!("⚠️  Marking application as unhealthy...");

        // Notify load balancer to stop sending traffic
        self.notify_shutting_down().await?;

        // Wait for existing connections to drain
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        println!("✅ Application marked unhealthy, connections drained");
        Ok(())
    }
}
```

### Example 4: Multiple Hooks on One Service

A service can implement multiple lifecycle hooks:

```rust
struct ComprehensiveService {
    name: String,
    initialized: Arc<RwLock<bool>>,
}

impl Provider for ComprehensiveService {}

#[async_trait]
impl OnModuleInit for ComprehensiveService {
    async fn on_module_init(&self) -> LifecycleResult {
        println!("{}: Module initialization", self.name);
        *self.initialized.write().await = true;
        Ok(())
    }
}

#[async_trait]
impl OnApplicationBootstrap for ComprehensiveService {
    async fn on_application_bootstrap(&self) -> LifecycleResult {
        println!("{}: Application bootstrap complete", self.name);
        Ok(())
    }
}

#[async_trait]
impl BeforeApplicationShutdown for ComprehensiveService {
    async fn before_application_shutdown(&self, signal: Option<String>) -> LifecycleResult {
        println!("{}: Preparing for shutdown: {:?}", self.name, signal);
        Ok(())
    }
}

#[async_trait]
impl OnApplicationShutdown for ComprehensiveService {
    async fn on_application_shutdown(&self, _signal: Option<String>) -> LifecycleResult {
        println!("{}: Shutting down", self.name);
        Ok(())
    }
}

#[async_trait]
impl OnModuleDestroy for ComprehensiveService {
    async fn on_module_destroy(&self) -> LifecycleResult {
        println!("{}: Module cleanup", self.name);
        *self.initialized.write().await = false;
        Ok(())
    }
}
```

---

## Signal Handling

The lifecycle system supports passing shutdown signals to hooks:

```rust
use tokio::signal;

async fn run_with_signal_handling(app: Application) -> Result<(), Box<dyn std::error::Error>> {
    // Wait for shutdown signal
    tokio::select! {
        _ = signal::ctrl_c() => {
            println!("Received Ctrl+C");
            app.shutdown(Some("SIGINT".to_string())).await?;
        }
        _ = wait_for_sigterm() => {
            println!("Received SIGTERM");
            app.shutdown(Some("SIGTERM".to_string())).await?;
        }
    }

    Ok(())
}
```

---

## Testing Lifecycle Hooks

You can test lifecycle hooks directly:

```rust
#[tokio::test]
async fn test_service_lifecycle() {
    let service = Arc::new(MyService::new());

    // Test initialization
    assert!(service.on_module_init().await.is_ok());
    assert!(service.is_initialized().await);

    // Test cleanup
    assert!(service.on_module_destroy().await.is_ok());
    assert!(!service.is_initialized().await);
}
```

---

## Advanced Usage

### Conditional Hooks

You can implement conditional logic in hooks:

```rust
#[async_trait]
impl OnModuleInit for MyService {
    async fn on_module_init(&self) -> LifecycleResult {
        if std::env::var("SKIP_INIT").is_ok() {
            println!("Skipping initialization (SKIP_INIT set)");
            return Ok(());
        }

        // Normal initialization
        self.initialize().await?;
        Ok(())
    }
}
```

### Timeout Protection

Add timeouts to prevent hooks from hanging:

```rust
use tokio::time::{timeout, Duration};

#[async_trait]
impl OnModuleInit for MyService {
    async fn on_module_init(&self) -> LifecycleResult {
        match timeout(Duration::from_secs(30), self.initialize()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Initialization timeout".into()),
        }
    }
}
```

---

## Summary

Lifecycle hooks in Armature provide a powerful way to manage application state and resources:

- ✅ **5 hook types** for different lifecycle phases
- ✅ **Async by default** for modern Rust applications
- ✅ **Error handling** with Result types
- ✅ **Automatic execution** by the Application
- ✅ **FIFO/LIFO ordering** for proper initialization and cleanup
- ✅ **Signal support** for graceful shutdown
- ✅ **Testable** lifecycle logic

Use lifecycle hooks to build robust, maintainable Rust web applications with proper resource management! 🚀

