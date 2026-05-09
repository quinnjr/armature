# Graceful Shutdown Guide

Comprehensive guide to implementing graceful shutdown in Armature applications.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Connection Draining](#connection-draining)
- [Shutdown Hooks](#shutdown-hooks)
- [Health Status Integration](#health-status-integration)
- [Signal Handling](#signal-handling)
- [Custom Shutdown Phases](#custom-shutdown-phases)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Kubernetes Integration](#kubernetes-integration)
- [Summary](#summary)

---

## Overview

Graceful shutdown ensures your application shuts down cleanly without dropping in-flight requests or losing data. This is critical for production applications, especially in containerized environments like Kubernetes.

### Why Graceful Shutdown?

| Without Graceful Shutdown | With Graceful Shutdown |
|---------------------------|------------------------|
| ❌ Dropped requests | ✅ Completes in-flight requests |
| ❌ Data loss | ✅ Flushes caches and saves state |
| ❌ Corrupted state | ✅ Cleans up resources properly |
| ❌ Error logs | ✅ Clean shutdown logs |
| ❌ Bad user experience | ✅ Seamless updates |

---

## Features

- ✅ **Connection Draining** - Wait for in-flight requests to complete
- ✅ **Shutdown Hooks** - Custom cleanup functions
- ✅ **Health Status Integration** - Mark unhealthy during shutdown
- ✅ **Timeout Support** - Force shutdown after timeout
- ✅ **Signal Handling** - SIGTERM, SIGINT support
- ✅ **Connection Tracking** - Track active requests
- ✅ **Graceful Rejection** - Reject new connections during shutdown

---

## Quick Start

### 1. Create Shutdown Manager

```rust
use armature_core::*;
use std::sync::Arc;
use std::time::Duration;

let shutdown_manager = Arc::new(ShutdownManager::new());

// Configure timeout
shutdown_manager.set_timeout(Duration::from_secs(30)).await;
```

### 2. Register Shutdown Hooks

```rust
shutdown_manager.add_hook(Box::new(|| {
    Box::pin(async {
        println!("Cleaning up database connections...");
        // Your cleanup code here
        Ok(())
    })
})).await;
```

### 3. Track Connections

```rust
let tracker = shutdown_manager.tracker().clone();

// In your handler
let handler = Arc::new(move |req| {
    let tracker = tracker.clone();
    Box::pin(async move {
        // Track this connection
        let _guard = match tracker.increment() {
            Some(g) => g,
            None => {
                // Server is shutting down
                return Ok(HttpResponse::service_unavailable()
                    .with_json(&serde_json::json!({
                        "error": "Server is shutting down"
                    }))?);
            }
        };

        // Process request
        // ...

        Ok(HttpResponse::ok())
    })
});
```

### 4. Handle Signals

```rust
let shutdown_signal = shutdown_manager.clone();
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    shutdown_signal.initiate_shutdown().await;
    std::process::exit(0);
});
```

---

## Connection Draining

Connection draining ensures in-flight requests complete before shutdown.

### How It Works

1. **Stop Accepting** - New connections are rejected
2. **Wait for Completion** - Existing requests are allowed to finish
3. **Timeout** - Force shutdown if requests take too long

### Connection Tracker

```rust
use armature_core::*;

let tracker = ConnectionTracker::new();

// Track a connection
let guard = tracker.increment().unwrap();

// Check active connections
println!("Active: {}", tracker.active_count());

// Stop accepting new connections
tracker.stop_accepting();

// Wait for connections to drain
let drained = tracker.drain(Duration::from_secs(30)).await;
if drained {
    println!("All connections drained");
} else {
    println!("Timeout: force shutdown");
}
```

### Connection Guard

The `ConnectionGuard` automatically decrements the count when dropped:

```rust
{
    let _guard = tracker.increment().unwrap();
    // Connection is active

    // Do work...

} // Guard dropped here, connection count decremented
```

### Checking if Accepting

```rust
if tracker.is_accepting() {
    // Process new connection
} else {
    // Reject with 503 Service Unavailable
}
```

---

## Shutdown Hooks

Shutdown hooks allow you to perform cleanup when the application shuts down.

### Registering Hooks

```rust
use armature_core::*;

let manager = ShutdownManager::new();

// Hook 1: Close database
manager.add_hook(Box::new(|| {
    Box::pin(async {
        println!("Closing database...");
        // Close DB connections
        Ok(())
    })
})).await;

// Hook 2: Flush cache
manager.add_hook(Box::new(|| {
    Box::pin(async {
        println!("Flushing cache...");
        // Flush to disk
        Ok(())
    })
})).await;

// Hook 3: Send metrics
manager.add_hook(Box::new(|| {
    Box::pin(async {
        println!("Sending final metrics...");
        // Send to metrics server
        Ok(())
    })
})).await;
```

### Hook Execution

Hooks are executed:
- **In registration order**
- **With a 5-second timeout per hook**
- **Even if previous hooks fail**

### Hook Example: Database Cleanup

```rust
use std::sync::Arc;

let db_pool = Arc::new(DatabasePool::new());

manager.add_hook(Box::new(move || {
    let pool = db_pool.clone();
    Box::pin(async move {
        // Wait for queries to complete
        pool.wait_for_idle().await?;

        // Close connections
        pool.close().await?;

        println!("Database connections closed");
        Ok(())
    })
})).await;
```

### Hook Example: Cache Flush

```rust
let cache = Arc::new(Cache::new());

manager.add_hook(Box::new(move || {
    let cache = cache.clone();
    Box::pin(async move {
        // Flush to disk
        cache.flush_to_disk().await?;

        println!("Cache flushed");
        Ok(())
    })
})).await;
```

---

## Health Status Integration

Mark health checks as unhealthy during shutdown to prevent new traffic.

### Setup

```rust
use armature_core::*;
use std::sync::Arc;

// Create health registry
let health_registry = Arc::new(HealthCheckRegistry::new());

// Add health checks
health_registry.add_check("database", Arc::new(|| {
    Box::pin(async {
        Ok(HealthStatus::Healthy(Some("DB connected".to_string())))
    })
})).await;

// Link to shutdown manager
let shutdown_manager = Arc::new(ShutdownManager::new());
shutdown_manager.set_health_registry(health_registry.clone()).await;
```

### During Shutdown

When `initiate_shutdown()` is called:
1. Health checks are marked as unhealthy
2. Load balancers stop sending traffic
3. Existing requests complete
4. Shutdown proceeds

### Health Check Endpoint

```rust
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/health".to_string(),
    handler: Arc::new(move |_req| {
        let registry = health_registry.clone();
        Box::pin(async move {
            let result = registry.check_health().await;
            let status_code = if result.is_healthy() { 200 } else { 503 };
            Ok(HttpResponse::new(status_code).with_json(&result)?)
        })
    }),
    constraints: None,
});
```

### Readiness Check

```rust
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/ready".to_string(),
    handler: Arc::new(move |_req| {
        let shutdown = shutdown_manager.clone();
        Box::pin(async move {
            if shutdown.is_shutting_down() {
                Ok(HttpResponse::service_unavailable()
                    .with_json(&serde_json::json!({
                        "status": "shutting_down"
                    }))?)
            } else {
                Ok(HttpResponse::ok()
                    .with_json(&serde_json::json!({
                        "status": "ready"
                    }))?)
            }
        })
    }),
    constraints: None,
});
```

---

## Signal Handling

Handle SIGTERM and SIGINT for graceful shutdown.

### Basic Signal Handling

```rust
use armature_core::*;
use std::sync::Arc;

let shutdown_manager = Arc::new(ShutdownManager::new());

let shutdown_signal = shutdown_manager.clone();
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    shutdown_signal.initiate_shutdown().await;
    std::process::exit(0);
});
```

### SIGTERM and SIGINT

```rust
tokio::spawn(async move {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Received SIGINT (Ctrl+C)");
        }
        _ = async {
            #[cfg(unix)]
            {
                let mut sigterm = tokio::signal::unix::signal(
                    tokio::signal::unix::SignalKind::terminate()
                ).expect("Failed to setup SIGTERM handler");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        } => {
            println!("Received SIGTERM");
        }
    }

    shutdown_manager.initiate_shutdown().await;
    std::process::exit(0);
});
```

---

## Custom Shutdown Phases

For advanced use cases, you can control shutdown phases manually.

### Phases

```rust
use armature_core::*;

let manager = ShutdownManager::new();
let phases = manager.shutdown_with_phases().await;

// Phase 1: Mark unhealthy
phases.mark_unhealthy().await;

// Wait for load balancer to notice (e.g., 5 seconds)
tokio::time::sleep(Duration::from_secs(5)).await;

// Phase 2: Stop accepting connections
phases.stop_accepting().await;

// Phase 3: Drain connections
let drained = phases.drain_connections(Duration::from_secs(30)).await;
if !drained {
    println!("Force shutdown");
}

// Phase 4: Execute hooks
phases.execute_hooks().await;
```

### Use Case: Zero-Downtime Deployment

```rust
// 1. Mark unhealthy
phases.mark_unhealthy().await;

// 2. Wait for load balancer TTL
tokio::time::sleep(Duration::from_secs(10)).await;

// 3. Now safe to drain without dropping requests
phases.stop_accepting().await;
phases.drain_connections(Duration::from_secs(30)).await;

// 4. Cleanup
phases.execute_hooks().await;
```

---

## Best Practices

### 1. Set Appropriate Timeouts

```rust
// Development: shorter timeout
shutdown_manager.set_timeout(Duration::from_secs(10)).await;

// Production: longer timeout for long-running requests
shutdown_manager.set_timeout(Duration::from_secs(60)).await;
```

### 2. Prioritize Cleanup

Register hooks in order of importance:

```rust
// 1. Critical: Database
manager.add_hook(Box::new(|| Box::pin(async {
    close_database().await?;
    Ok(())
}))).await;

// 2. Important: Cache
manager.add_hook(Box::new(|| Box::pin(async {
    flush_cache().await?;
    Ok(())
}))).await;

// 3. Nice-to-have: Logs
manager.add_hook(Box::new(|| Box::pin(async {
    rotate_logs().await?;
    Ok(())
}))).await;
```

### 3. Handle Hook Failures

Hooks should not panic:

```rust
manager.add_hook(Box::new(|| {
    Box::pin(async {
        match cleanup().await {
            Ok(()) => {
                println!("Cleanup successful");
                Ok(())
            }
            Err(e) => {
                eprintln!("Cleanup failed: {}", e);
                // Return Ok to continue shutdown
                Ok(())
            }
        }
    })
})).await;
```

### 4. Track All Connections

Always use connection tracking in handlers:

```rust
// ✅ Good
let _guard = match tracker.increment() {
    Some(g) => g,
    None => return Ok(HttpResponse::service_unavailable()),
};

// ❌ Bad - connection not tracked
// No guard, request could be dropped during shutdown
```

### 5. Implement Readiness Check

```rust
// Kubernetes can use this to stop sending traffic
GET /ready -> 200 OK (ready)
GET /ready -> 503 Service Unavailable (shutting down)
```

---

## Examples

### Example 1: Basic Graceful Shutdown

```rust
use armature_core::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shutdown_manager = Arc::new(ShutdownManager::new());
    let tracker = shutdown_manager.tracker().clone();

    // Register hook
    shutdown_manager.add_hook(Box::new(|| {
        Box::pin(async {
            println!("Cleanup...");
            Ok(())
        })
    })).await;

    // Handle signal
    let shutdown_signal = shutdown_manager.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        shutdown_signal.initiate_shutdown().await;
        std::process::exit(0);
    });

    // Create app with connection tracking
    // ... (see full example in examples/graceful_shutdown.rs)

    Ok(())
}
```

### Example 2: Health Check Integration

```rust
let health_registry = Arc::new(HealthCheckRegistry::new());
let shutdown_manager = Arc::new(ShutdownManager::new());

shutdown_manager.set_health_registry(health_registry.clone()).await;

// During shutdown, health checks return 503
```

### Example 3: Custom Timeout per Hook

```rust
use tokio::time::timeout;
use std::time::Duration;

manager.add_hook(Box::new(|| {
    Box::pin(async {
        // Set custom timeout for this specific hook
        match timeout(Duration::from_secs(10), long_cleanup()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                eprintln!("Cleanup failed: {}", e);
                Ok(()) // Don't block other hooks
            }
            Err(_) => {
                eprintln!("Cleanup timed out");
                Ok(())
            }
        }
    })
})).await;
```

---

## Kubernetes Integration

### Pod Lifecycle

```yaml
apiVersion: v1
kind: Pod
spec:
  containers:
  - name: app
    image: myapp:latest
    ports:
    - containerPort: 3000
    livenessProbe:
      httpGet:
        path: /health
        port: 3000
      initialDelaySeconds: 10
      periodSeconds: 5
    readinessProbe:
      httpGet:
        path: /ready
        port: 3000
      initialDelaySeconds: 5
      periodSeconds: 3
    lifecycle:
      preStop:
        exec:
          command: ["/bin/sh", "-c", "sleep 5"]
  terminationGracePeriodSeconds: 60
```

### Shutdown Flow

1. **SIGTERM sent** - Kubernetes wants to terminate pod
2. **PreStop hook** - Sleep 5s (let load balancer notice)
3. **App marks unhealthy** - `/health` returns 503
4. **App stops accepting** - New connections rejected
5. **App drains** - Wait for in-flight requests (up to 60s)
6. **App executes hooks** - Cleanup
7. **SIGKILL** - If still running after 60s

### Configuration

```rust
// Set timeout less than terminationGracePeriodSeconds
shutdown_manager.set_timeout(Duration::from_secs(50)).await;

// Account for preStop sleep
// Total: 5s (preStop) + 50s (app shutdown) = 55s < 60s (terminationGracePeriodSeconds)
```

---

## Summary

**Key Points:**

1. **Connection Draining** - Track and wait for in-flight requests
2. **Shutdown Hooks** - Clean up resources properly
3. **Health Integration** - Mark unhealthy to stop new traffic
4. **Timeouts** - Force shutdown if needed
5. **Signal Handling** - Respond to SIGTERM/SIGINT

**Quick Reference:**

```rust
// Setup
let manager = Arc::new(ShutdownManager::new());
manager.set_timeout(Duration::from_secs(30)).await;

// Track connections
let tracker = manager.tracker().clone();
let _guard = tracker.increment();

// Register hooks
manager.add_hook(Box::new(|| {
    Box::pin(async {
        // Cleanup code
        Ok(())
    })
})).await;

// Handle signals
tokio::spawn(async move {
    tokio::signal::ctrl_c().await.ok();
    manager.initiate_shutdown().await;
    std::process::exit(0);
});
```

**Shutdown Phases:**

1. ✅ Mark health checks unhealthy
2. ✅ Stop accepting new connections
3. ✅ Wait for in-flight requests (with timeout)
4. ✅ Execute shutdown hooks
5. ✅ Exit cleanly

**Kubernetes Integration:**
- Set timeout < `terminationGracePeriodSeconds`
- Implement `/health` and `/ready` endpoints
- Account for `preStop` hook delay
- Test with rolling updates

**Resources:**
- [Example: Basic Shutdown](../../examples/graceful_shutdown.rs)
- [Example: Advanced Shutdown](../../examples/shutdown_advanced.rs)
- [Kubernetes Pod Lifecycle](https://kubernetes.io/docs/concepts/workloads/pods/pod-lifecycle/)

