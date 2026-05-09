# Debug Logging Guide

Comprehensive guide to debug logging throughout the Armature framework.

## Table of Contents

- [Overview](#overview)
- [Enabling Debug Logging](#enabling-debug-logging)
- [Logged Components](#logged-components)
- [Log Levels](#log-levels)
- [Examples](#examples)
- [Troubleshooting](#troubleshooting)

---

## Overview

Armature includes comprehensive debug logging throughout the framework to help with:
- **Development** - Understanding application flow
- **Debugging** - Tracking down issues
- **Monitoring** - Observing production behavior
- **Performance** - Identifying bottlenecks

All logging uses the `tracing` ecosystem for structured, high-performance logging.

---

## Enabling Debug Logging

### Development Mode

Enable debug logging for development:

```rust
use armature_core::*;

#[tokio::main]
async fn main() {
    // Pretty format with debug level
    let config = LogConfig::new()
        .level(LogLevel::Debug)
        .format(LogFormat::Pretty)
        .with_colors(true)
        .with_targets(true)
        .with_file_line(true);

    let _guard = Application::init_logging_with_config(config);

    // Your application code
}
```

### Via Environment Variable

Set log level via `RUST_LOG`:

```bash
# Debug for all
RUST_LOG=debug cargo run

# Debug for Armature only
RUST_LOG=armature=debug cargo run

# Trace for specific module
RUST_LOG=armature_core::application=trace cargo run

# Mixed levels
RUST_LOG=armature=debug,hyper=info cargo run
```

---

## Logged Components

### 1. Application Bootstrap

**What's Logged:**
- Module registration
- Provider registration
- Controller registration
- Lifecycle hook execution
- Application startup

**Log Levels:**
- `INFO` - High-level bootstrap steps
- `DEBUG` - Module/provider/controller details
- `TRACE` - Internal operations

**Example Output:**

```json
{
  "timestamp": "2024-12-06T12:00:00.123Z",
  "level": "INFO",
  "message": "Bootstrapping Armature application"
}
{
  "level": "DEBUG",
  "module_type": "app::AppModule",
  "message": "Creating application from root module"
}
{
  "level": "DEBUG",
  "message": "DI container initialized"
}
{
  "level": "DEBUG",
  "module_type": "app::AppModule",
  "import_count": 2,
  "message": "Registering imported modules"
}
{
  "level": "DEBUG",
  "module_type": "app::AppModule",
  "provider_count": 5,
  "message": "Registering providers"
}
{
  "level": "DEBUG",
  "module_type": "app::AppModule",
  "provider": "UserService",
  "message": "Provider registered"
}
{
  "level": "DEBUG",
  "module_type": "app::AppModule",
  "controller_count": 3,
  "message": "Registering controllers"
}
{
  "level": "DEBUG",
  "controller": "UserController",
  "base_path": "/users",
  "message": "Controller registered"
}
{
  "level": "INFO",
  "message": "Application bootstrap complete"
}
```

### 2. Dependency Injection Container

**What's Logged:**
- Provider registration
- Provider resolution
- Factory invocations
- Provider lookups

**Log Levels:**
- `DEBUG` - Registration and resolution
- `TRACE` - Lock acquisition, existence checks

**Example Output:**

```json
{
  "level": "DEBUG",
  "message": "Creating new DI container"
}
{
  "level": "DEBUG",
  "provider": "UserService",
  "message": "Provider registered in DI container"
}
{
  "level": "TRACE",
  "provider": "DatabaseService",
  "message": "Attempting to resolve provider"
}
{
  "level": "DEBUG",
  "provider": "DatabaseService",
  "message": "Provider resolved successfully"
}
{
  "level": "DEBUG",
  "provider": "NonExistentService",
  "message": "Provider not found in container"
}
```

### 3. HTTP Server

**What's Logged:**
- Server binding and startup
- Connection acceptance
- TLS handshakes
- Connection errors

**Log Levels:**
- `INFO` - Server started
- `DEBUG` - Binding details, TLS success
- `TRACE` - Connection acceptance
- `ERROR` - Connection/TLS failures

**Example Output:**

```json
{
  "level": "DEBUG",
  "address": "0.0.0.0:3000",
  "message": "Binding to address"
}
{
  "level": "INFO",
  "address": "0.0.0.0:3000",
  "message": "HTTP server listening"
}
{
  "level": "TRACE",
  "client_address": "192.168.1.100:54321",
  "message": "Connection accepted"
}
{
  "level": "ERROR",
  "error": "connection reset by peer",
  "client": "192.168.1.100:54321",
  "message": "Error serving connection"
}
```

### 4. HTTP Request Handling

**What's Logged:**
- Request receipt
- Header parsing
- Body parsing
- Routing
- Response generation
- Request duration

**Log Levels:**
- `DEBUG` - Request routing and completion
- `TRACE` - Request details, headers, body
- `WARN` - Request handling failures

**Example Output:**

```json
{
  "level": "TRACE",
  "method": "POST",
  "path": "/api/users",
  "message": "Incoming request"
}
{
  "level": "TRACE",
  "header_count": 8,
  "message": "Headers parsed"
}
{
  "level": "TRACE",
  "body_size": 256,
  "message": "Request body received"
}
{
  "level": "DEBUG",
  "method": "POST",
  "path": "/api/users",
  "message": "Routing request"
}
{
  "level": "DEBUG",
  "method": "POST",
  "path": "/api/users",
  "status": 201,
  "message": "Request handled successfully"
}
{
  "level": "DEBUG",
  "method": "POST",
  "path": "/api/users",
  "status": 201,
  "duration_ms": 45,
  "message": "Request completed"
}
```

### 5. Lifecycle Hooks

**What's Logged:**
- Hook execution start/completion
- Hook failures
- Shutdown sequence

**Log Levels:**
- `INFO` - Lifecycle phase changes
- `DEBUG` - Individual hook execution
- `WARN` - Hook failures
- `ERROR` - Hook errors with details

**Example Output:**

```json
{
  "level": "INFO",
  "message": "Executing lifecycle hooks"
}
{
  "level": "DEBUG",
  "message": "Calling OnModuleInit hooks"
}
{
  "level": "DEBUG",
  "message": "All OnModuleInit hooks completed successfully"
}
{
  "level": "DEBUG",
  "message": "Calling OnApplicationBootstrap hooks"
}
{
  "level": "ERROR",
  "hook_name": "DatabaseService::on_bootstrap",
  "error": "Connection refused",
  "message": "Bootstrap hook failed"
}
{
  "level": "INFO",
  "signal": "SIGTERM",
  "message": "Gracefully shutting down application"
}
{
  "level": "DEBUG",
  "message": "Calling OnApplicationShutdown hooks"
}
{
  "level": "INFO",
  "message": "Application shutdown complete"
}
```

---

## Log Levels

### TRACE (Most Verbose)

**When to use:** Deep debugging, tracking exact execution flow

**What's logged:**
- Connection acceptance details
- Request/response headers and bodies
- Lock acquisition
- Provider existence checks
- Internal state transitions

**Example:**
```json
{"level": "TRACE", "client_address": "192.168.1.100:54321", "message": "Connection accepted"}
```

### DEBUG

**When to use:** Development, troubleshooting, understanding behavior

**What's logged:**
- Module/provider/controller registration
- Provider resolution
- Request routing
- Hook execution
- TLS handshakes
- Request completion with metrics

**Example:**
```json
{"level": "DEBUG", "provider": "UserService", "message": "Provider registered in DI container"}
```

### INFO

**When to use:** Production monitoring, high-level tracking

**What's logged:**
- Application bootstrap
- Server startup
- Lifecycle phase changes
- Shutdown sequence

**Example:**
```json
{"level": "INFO", "address": "0.0.0.0:3000", "message": "HTTP server listening"}
```

### WARN

**When to use:** Potential issues, degraded performance

**What's logged:**
- Hook failures (non-fatal)
- Request handling issues
- Retry attempts

**Example:**
```json
{"level": "WARN", "error_count": 2, "message": "Some module init hooks failed"}
```

### ERROR

**When to use:** Failures, exceptions, errors

**What's logged:**
- Connection errors
- TLS handshake failures
- Hook failures with details
- Provider instantiation failures
- Request routing errors

**Example:**
```json
{"level": "ERROR", "error": "connection reset", "client": "192.168.1.100:54321", "message": "Error serving connection"}
```

---

## Examples

### Example 1: Debug Application Bootstrap

```rust
use armature_core::*;

#[tokio::main]
async fn main() {
    // Enable debug logging
    let config = LogConfig::new()
        .level(LogLevel::Debug)
        .format(LogFormat::Pretty)
        .with_colors(true);

    let _guard = Application::init_logging_with_config(config);

    // Bootstrap application - watch logs
    let app = Application::create::<AppModule>().await;

    // Start server
    app.listen(3000).await.unwrap();
}
```

**Output:**
```
DEBUG Creating new DI container
DEBUG module_type="app::AppModule" Creating application from root module
DEBUG DI container initialized
DEBUG Router initialized
...
```

### Example 2: Track Request Handling

```rust
let config = LogConfig::new()
    .level(LogLevel::Trace)  // TRACE for full request details
    .format(LogFormat::Json);

let _guard = Application::init_logging_with_config(config);
```

**Output:**
```json
{"level":"TRACE","method":"POST","path":"/api/users","message":"Incoming request"}
{"level":"TRACE","header_count":8,"message":"Headers parsed"}
{"level":"TRACE","body_size":256,"message":"Request body received"}
{"level":"DEBUG","method":"POST","path":"/api/users","message":"Routing request"}
{"level":"DEBUG","method":"POST","path":"/api/users","status":201,"duration_ms":45,"message":"Request completed"}
```

### Example 3: Monitor Dependency Injection

```rust
let config = LogConfig::new()
    .level(LogLevel::Debug)
    .format(LogFormat::Pretty)
    .with_env_filter("armature_core::container=debug");

let _guard = Application::init_logging_with_config(config);
```

**Output:**
```
DEBUG armature_core::container: provider="UserService" Provider registered in DI container
DEBUG armature_core::container: provider="DatabaseService" Attempting to resolve provider
DEBUG armature_core::container: provider="DatabaseService" Provider resolved successfully
```

### Example 4: Debug Lifecycle Issues

```rust
let config = LogConfig::new()
    .level(LogLevel::Debug)
    .with_env_filter("armature=debug");

let _guard = Application::init_logging_with_config(config);

let app = Application::create::<AppModule>().await;

// Later, trigger shutdown
app.shutdown(Some("SIGTERM".to_string())).await.unwrap();
```

**Output:**
```json
{"level":"DEBUG","message":"Calling OnModuleInit hooks"}
{"level":"DEBUG","message":"All OnModuleInit hooks completed successfully"}
...
{"level":"INFO","signal":"SIGTERM","message":"Gracefully shutting down application"}
{"level":"DEBUG","message":"Calling BeforeApplicationShutdown hooks"}
{"level":"DEBUG","message":"All BeforeApplicationShutdown hooks completed successfully"}
```

---

## Troubleshooting

### No Logs Appearing

**Problem:** Logs not showing up

**Solutions:**
1. Ensure logging is initialized:
   ```rust
   let _guard = Application::init_logging();  // Don't drop!
   ```

2. Check log level:
   ```rust
   LogConfig::new().level(LogLevel::Debug)  // Not Info!
   ```

3. Check environment variable:
   ```bash
   RUST_LOG=debug cargo run
   ```

### Too Many Logs

**Problem:** Overwhelming log output

**Solutions:**
1. Filter by module:
   ```rust
   .with_env_filter("armature_core::application=debug")
   ```

2. Increase log level:
   ```rust
   .level(LogLevel::Info)  // Less verbose
   ```

3. Filter out noisy crates:
   ```rust
   .with_env_filter("armature=debug,hyper=warn,tokio=warn")
   ```

### JSON Not Formatting

**Problem:** JSON logs not pretty-printing

**Solution:** JSON is meant for machines, use Pretty for humans:
```rust
LogConfig::new().format(LogFormat::Pretty)
```

### Missing Structured Fields

**Problem:** Not seeing context fields in logs

**Solutions:**
1. Enable target to see module:
   ```rust
   .with_targets(true)
   ```

2. Use JSON format to see all fields:
   ```rust
   .format(LogFormat::Json)
   ```

---

## Summary

### Key Takeaways

✅ **Comprehensive Coverage** - All major components log their operations
✅ **Structured Logging** - Rich context with key-value pairs
✅ **Multiple Levels** - TRACE to ERROR for different needs
✅ **Production Ready** - Low overhead, configurable
✅ **Easy Debugging** - Clear, informative messages

### Quick Reference

| Component | Log Level | What's Logged |
|-----------|-----------|---------------|
| **Bootstrap** | INFO | High-level steps |
| **Bootstrap** | DEBUG | Module/provider/controller details |
| **DI Container** | DEBUG | Registration and resolution |
| **DI Container** | TRACE | Internal operations |
| **HTTP Server** | INFO | Server startup |
| **HTTP Server** | DEBUG | Binding, TLS details |
| **HTTP Server** | TRACE | Connection acceptance |
| **HTTP Requests** | DEBUG | Routing, completion, metrics |
| **HTTP Requests** | TRACE | Headers, body, full details |
| **Lifecycle** | INFO | Phase changes |
| **Lifecycle** | DEBUG | Hook execution |
| **Lifecycle** | ERROR | Hook failures |

### Recommended Configurations

**Development:**
```rust
LogConfig::new()
    .level(LogLevel::Debug)
    .format(LogFormat::Pretty)
    .with_colors(true)
    .with_file_line(true)
```

**Production:**
```rust
LogConfig::new()
    .level(LogLevel::Info)
    .format(LogFormat::Json)
    .output(LogOutput::RollingFile { ... })
```

**Troubleshooting:**
```rust
LogConfig::new()
    .level(LogLevel::Trace)
    .format(LogFormat::Pretty)
    .with_env_filter("armature=trace,problematic_module=trace")
```

---

**Happy debugging!** 🔍✨

