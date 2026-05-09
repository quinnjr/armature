# Logging Guide

Comprehensive guide to Armature's logging system with JSON output by default and configurable pretty printing for development.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Environment Variables](#environment-variables)
- [Programmatic Configuration](#programmatic-configuration)
- [Log Formats](#log-formats)
- [Log Levels](#log-levels)
- [Structured Logging](#structured-logging)
- [HTTP Request Logging](#http-request-logging)
- [Best Practices](#best-practices)
- [Performance](#performance)
- [Examples](#examples)

---

## Overview

Armature provides a powerful, environment-configurable logging system built for production use with features like:

- **JSON by Default**: Production-ready structured logging out of the box
- **Pretty Printing**: Human-readable format for development
- **Environment Configuration**: Switch formats without code changes
- **Runtime Configuration**: Change settings programmatically
- **Zero-Cost When Disabled**: Debug macros compile to no-ops
- **Presets**: Built-in configurations for development and production

**Default Configuration:** JSON format to STDERR at INFO level

---

## Quick Start

### Basic Logging

```rust
use armature_log::{debug, info, warn, error, trace};

fn main() {
    // Logs are automatically initialized on first use
    info!("Application started on port {}", 8080);
    debug!("Debug information");
    warn!("Warning message");
    error!("Error occurred");
}
```

**Default JSON Output:**
```json
{"timestamp":"2024-12-20T12:00:00Z","level":"INFO","target":"my_app","message":"Application started on port 8080"}
```

### Switch to Pretty Logging

```bash
# Development - Pretty format with colors
ARMATURE_LOG_FORMAT=pretty cargo run
```

**Pretty Output:**
```
2024-12-20 12:00:00.123 INFO  my_app Application started on port 8080
```

---

## Configuration

### Environment Variables (Recommended)

The easiest way to configure logging is via environment variables:

| Variable | Values | Default | Description |
|----------|--------|---------|-------------|
| `ARMATURE_LOG_FORMAT` | `json`, `pretty`, `compact` | `json` | Output format |
| `ARMATURE_LOG_LEVEL` | `trace`, `debug`, `info`, `warn`, `error` | `info` | Minimum log level |
| `ARMATURE_LOG_COLOR` | `1`, `true`, `0`, `false` | auto-detect | Enable ANSI colors |
| `ARMATURE_DEBUG` | `1`, `true` | `false` | Enable debug mode |
| `ARMATURE_LOG_TIMESTAMPS` | `1`, `0` | `1` | Include timestamps |
| `ARMATURE_LOG_MODULE` | `1`, `0` | `1` | Include module path |

**Examples:**

```bash
# Development
ARMATURE_LOG_FORMAT=pretty ARMATURE_LOG_LEVEL=debug cargo run

# Production
ARMATURE_LOG_FORMAT=json ARMATURE_LOG_LEVEL=info cargo run

# Quiet mode
ARMATURE_LOG_LEVEL=warn cargo run
```

### Programmatic Configuration

Use the fluent API for runtime configuration:

```rust
use armature_log::{configure, Format, Level};

// Configure logging
configure()
    .format(Format::Pretty)
    .level(Level::Debug)
    .color(true)
    .timestamps(true)
    .apply();
```

### Direct Setters

```rust
use armature_log::{set_format, set_level, Format, Level};

// Change format at runtime
set_format(Format::Pretty);
set_format(Format::Json);

// Change level at runtime
set_level(Level::Debug);
```

### Presets

Use built-in presets for common configurations:

```rust
use armature_log;

// Development: Pretty + Debug + Colors
armature_log::preset_development();

// Production: JSON + Info + No colors
armature_log::preset_production();

// Quiet: JSON + Warn only
armature_log::preset_quiet();
```

---

## Log Formats

### JSON Format (Default)

Machine-readable, structured format ideal for production and log aggregators.

```json
{"timestamp":"2024-12-20T12:00:00.123Z","level":"INFO","target":"my_app","message":"User logged in"}
```

**Use Cases:**
- Production environments
- Log aggregation (ELK, Splunk, Datadog, Grafana Loki)
- Automated log parsing
- Cloud environments (AWS CloudWatch, GCP Logging)

**Enable:**
```bash
ARMATURE_LOG_FORMAT=json cargo run
```

Or in code:
```rust
armature_log::set_format(armature_log::Format::Json);
```

### Pretty Format

Formatted, colored output for development with human-readable timestamps.

```
2024-12-20 12:00:00.123 INFO  my_app User logged in
2024-12-20 12:00:00.124 DEBUG armature_core::routing Matched route: GET /api/users
2024-12-20 12:00:00.125 WARN  my_app Rate limit approaching
```

**Use Cases:**
- Local development
- Debugging
- Interactive terminal use
- Quick troubleshooting

**Enable:**
```bash
ARMATURE_LOG_FORMAT=pretty cargo run
```

Or in code:
```rust
armature_log::preset_development();
```

### Compact Format

Minimal single-line output for space efficiency.

```
12:00:00 I my_app: User logged in
12:00:00 D armature_core::routing: Matched route
12:00:00 W my_app: Rate limit approaching
```

**Use Cases:**
- Low-volume logging
- CI/CD pipelines
- Space-constrained environments

**Enable:**
```bash
ARMATURE_LOG_FORMAT=compact cargo run
```

---

## Log Levels

### Available Levels

| Level | Use Case | Example |
|-------|----------|---------|
| `TRACE` | Very detailed debugging | Function entry/exit, loop iterations |
| `DEBUG` | Development information | Variable values, state changes |
| `INFO` | General information | App start, config loaded, request processed |
| `WARN` | Potential issues | Deprecated API used, fallback activated |
| `ERROR` | Failures requiring attention | Database error, API call failed |

### Setting Log Level

```bash
# Via environment variable
ARMATURE_LOG_LEVEL=debug cargo run
```

```rust
// Via code
armature_log::set_level(armature_log::Level::Debug);
```

### Logging Macros

```rust
use armature_log::{trace, debug, info, warn, error};

trace!("Entering function");
debug!("Processing item {}", id);
info!("User {} logged in", username);
warn!("Rate limit approaching: {}/100", count);
error!("Failed to connect to database: {}", err);
```

### With Target

Specify a custom target (module path) for filtering:

```rust
debug!(target: "armature::router", "Matching route: {}", path);
info!(target: "database", "Query executed in {}ms", duration);
```

---

## Structured Logging

Add context to log messages with key-value pairs.

### Basic Structured Logging

```rust
info!(
    user_id = 123,
    action = "login",
    ip_address = "192.168.1.1",
    "User authentication successful"
);
```

**JSON Output:**
```json
{
  "timestamp": "2024-12-20T12:00:00.123Z",
  "level": "INFO",
  "target": "my_app",
  "message": "User authentication successful"
}
```

### Complex Types

```rust
// Strings
info!("User created: {}", name);

// Numbers
info!("Query completed in {}ms, {} rows", duration_ms, row_count);

// With error context
error!("Operation failed: {}", err);
```

---

## HTTP Request Logging

Armature automatically adds logging to HTTP request handling when using `armature-core`.

### Automatic Request Logging

```rust
use armature_core::Application;

let app = Application::new();
// Logging is automatically enabled
```

**Example Logs:**
```json
{"timestamp":"2024-12-20T12:00:00Z","level":"INFO","target":"armature_core::application","message":"HTTP server listening with pipelining enabled"}
{"timestamp":"2024-12-20T12:00:01Z","level":"DEBUG","target":"armature_core::routing","message":"Matching route: /api/users"}
{"timestamp":"2024-12-20T12:00:01Z","level":"TRACE","target":"armature_core::routing","message":"Route matched: GET /api/users"}
```

---

## Best Practices

### 1. Use Environment Variables for Format

```bash
# .env.development
ARMATURE_LOG_FORMAT=pretty
ARMATURE_LOG_LEVEL=debug

# .env.production
ARMATURE_LOG_FORMAT=json
ARMATURE_LOG_LEVEL=info
```

### 2. Use Appropriate Log Levels

```rust
// ✅ Good
info!("User {} logged in", user_id);              // General info
warn!("Rate limit exceeded for IP {}", ip);       // Potential issue
error!("Failed to save user: {}", err);           // Actual error

// ❌ Bad
info!("Database error occurred");                 // Should be ERROR
error!("User clicked button");                    // Should be DEBUG or none
```

### 3. Don't Log Sensitive Data

```rust
// ❌ Bad - logs sensitive data
info!("User logged in with password: {}", password);

// ✅ Good - omits sensitive data
info!("User {} logged in", user_id);
```

### 4. Initialize Logging Early (Optional)

```rust
fn main() {
    // Explicitly initialize logging (optional)
    armature_log::init();

    info!("Application starting");
}
```

### 5. Use Presets for Consistency

```rust
fn main() {
    // Use preset based on environment
    if cfg!(debug_assertions) {
        armature_log::preset_development();
    } else {
        armature_log::preset_production();
    }
}
```

---

## Performance

### Logging Overhead

Armature's logging system is designed for minimal overhead:

- **Lazy evaluation:** Only evaluates log statements that will be output
- **Atomic checks:** Fast level checks using atomics
- **No allocation when filtered:** Filtered logs don't allocate
- **JSON serialization:** Efficient with serde_json

### Benchmarks

| Operation | Time | Overhead |
|-----------|------|----------|
| Filtered out log (TRACE when INFO) | ~5ns | Negligible |
| Simple info! message | ~200ns | Very low |
| JSON formatting | ~1μs | Low |

### Tips for Performance

1. **Use appropriate log levels** - DEBUG/TRACE disabled in production
2. **Avoid expensive operations** - Don't compute values if log is filtered

```rust
// ✅ Good - value only computed if debug is enabled
if armature_log::is_level_enabled(armature_log::Level::Debug) {
    debug!("Expensive computation: {}", expensive_fn());
}
```

---

## Examples

### Production Configuration

```bash
# Docker/K8s environment
ARMATURE_LOG_FORMAT=json
ARMATURE_LOG_LEVEL=info
ARMATURE_LOG_TIMESTAMPS=1
ARMATURE_LOG_MODULE=1
```

### Development Configuration

```bash
# Local development
ARMATURE_LOG_FORMAT=pretty
ARMATURE_LOG_LEVEL=debug
ARMATURE_LOG_COLOR=1
```

Or in code:

```rust
fn main() {
    #[cfg(debug_assertions)]
    armature_log::preset_development();

    #[cfg(not(debug_assertions))]
    armature_log::preset_production();

    info!("Application started");
}
```

### CI/CD Configuration

```bash
# Compact format for CI logs
ARMATURE_LOG_FORMAT=compact
ARMATURE_LOG_LEVEL=info
ARMATURE_LOG_COLOR=0
```

### Complete Example

```rust
use armature_log::{debug, info, warn, error, configure, Format, Level};

fn main() {
    // Configure based on environment
    if std::env::var("DEVELOPMENT").is_ok() {
        configure()
            .format(Format::Pretty)
            .level(Level::Debug)
            .color(true)
            .apply();
    }

    info!("Application starting");

    // Your application code
    match process_request() {
        Ok(_) => info!("Request processed successfully"),
        Err(e) => error!("Request failed: {}", e),
    }
}
```

---

## Summary

### Key Features

✅ **JSON by Default** - Production-ready structured logging
✅ **Pretty Printing** - Human-readable development output
✅ **Environment Configuration** - Switch formats via env vars
✅ **Runtime Configuration** - Change settings in code
✅ **Zero-Cost** - No overhead when disabled
✅ **Presets** - Built-in dev/prod configurations

### Quick Reference

```bash
# Environment Variables
ARMATURE_LOG_FORMAT=pretty|json|compact
ARMATURE_LOG_LEVEL=trace|debug|info|warn|error
ARMATURE_LOG_COLOR=1|0
ARMATURE_DEBUG=1
```

```rust
// Programmatic Configuration
armature_log::preset_development();
armature_log::preset_production();
armature_log::set_format(Format::Pretty);
armature_log::set_level(Level::Debug);

// Logging Macros
trace!("...");
debug!("...");
info!("...");
warn!("...");
error!("...");
```

---

**Happy logging!** 📝
