# Request Timeouts and Limits

This guide covers the request deadlines, body size limits and connection caps
Armature applies, and how to configure them.

## Table of Contents

- [Overview](#overview)
- [Defaults](#defaults)
- [Basic Usage](#basic-usage)
- [Configuration Options](#configuration-options)
- [Preset Configurations](#preset-configurations)
- [Timeout Behavior](#timeout-behavior)
- [Error Responses](#error-responses)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

## Overview

Armature protects every listener against:

- **Slow clients**: peers that trickle the request head or body to hold a
  connection open (slowloris, slow POST)
- **Large payloads**: request bodies larger than the application accepts
- **Connection floods**: more open sockets than the process can serve
- **Slow handlers** (opt-in): handlers that run longer than you allow

The slow-client defences are **on by default** on every transport (armature-h1,
hyper HTTP/1.1 and HTTP/2, HTTP/3, and the HTTP→HTTPS redirect server). You do
not need to configure anything to get them.

## Defaults

| Protection | Default | Where it is configured |
|------------|---------|------------------------|
| Request head (headers) deadline | 10 s (`pipeline::HEADER_READ_TIMEOUT`) | Fixed |
| TLS handshake deadline | 10 s | Fixed |
| Body inactivity deadline | 30 s without a body byte → `408` | `BodyReadPolicy::idle_timeout` |
| Body minimum transfer rate | 1 KiB/s after the grace period → `408` | `BodyReadPolicy::min_bytes_per_sec` |
| Maximum body size | 10 MB (`DEFAULT_MAX_BODY_SIZE`) → `413` | `Application::with_max_body_size` |
| Maximum request head size | 16 KiB | `PipelineConfig::max_header_size` |
| Concurrent connections per accept loop | 10 000 (`pipeline::DEFAULT_MAX_CONNECTIONS`) | `PipelineConfig::max_connections` |
| Idle keep-alive connection | 60 s | `PipelineConfig::keep_alive_timeout` |
| Single response write | 300 s | `PipelineConfig::write_timeout` |
| Handler deadline | none | `PipelineConfig::request_timeout` |

A request for a route that does not exist is answered `404` **before** its body
is read when no global guard or middleware is configured, so unknown URLs
cannot be used to make the server buffer bodies.

## Basic Usage

### Default Configuration

```rust,ignore
use armature::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // Defaults: 10 s header deadline, 30 s body inactivity / 1 KiB/s minimum
    // body rate, 10 MB max body, 10 000 connections per accept loop.
    let app = Application::create::<AppModule>().await;
    app.listen(3000).await
}
```

### Custom Configuration

```rust,ignore
use armature_core::{BodyReadPolicy, pipeline::PipelineConfig};
use armature::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let pipeline = PipelineConfig::builder()
        .keep_alive_timeout(Duration::from_secs(30))
        .max_header_size(16 * 1024)   // 16 KiB request head
        .max_connections(5_000)       // per accept loop
        .build();

    let app = Application::create::<AppModule>()
        .await
        .with_pipeline_config(pipeline)
        .with_max_body_size(1024 * 1024) // 1 MB
        .with_body_read_policy(BodyReadPolicy {
            idle_timeout: Duration::from_secs(15),
            min_bytes_per_sec: 4 * 1024,
        });

    app.listen(3000).await
}
```

## Configuration Options

### `BodyReadPolicy`

How long a client may take to deliver a request body. It is enforced while the
body is read, on every transport, and independently of any handler deadline, so
long-polls and slow handlers are unaffected.

| Field | Default | Description |
|-------|---------|-------------|
| `idle_timeout` | 30 s | Longest silence allowed before the first body chunk and between two chunks |
| `min_bytes_per_sec` | 1024 | Minimum average rate. The read may take at most `idle_timeout` plus one second per `min_bytes_per_sec` bytes received. `0` disables the rate rule |

Set it with `Application::with_body_read_policy`.

### `PipelineConfig`

`PipelineConfig` (in `armature_core::pipeline`) is passed with
`Application::with_pipeline_config`. The security-relevant fields:

| Field | Default | Description |
|-------|---------|-------------|
| `max_connections` | 10 000 | Open connections one accept loop serves. At the cap the loop stops accepting until a connection closes, leaving new ones in the kernel backlog. On the default armature-h1 backend it is split across workers as `armature_h1::Limits::max_connections` |
| `max_header_size` | 16 KiB | Byte cap on the request line plus headers (armature-h1 backend) |
| `keep_alive_timeout` | 60 s | Deadline for the next request on an idle keep-alive connection (armature-h1 backend) |
| `write_timeout` | `Some(300 s)` | How long one response write may take before the connection is torn down. Defends against peers that stop reading |
| `request_timeout` | `None` | Handler deadline. When it expires the connection answers `503` and closes. Off by default so long-polls and SSE streams keep working |

The builder (`PipelineConfig::builder()`) exposes `keep_alive_timeout`,
`max_header_size`, `max_connections` and the other tuning knobs. The
`Option` deadlines have no builder method; set them with struct-update syntax:

```rust,ignore
use armature_core::pipeline::PipelineConfig;
use std::time::Duration;

let pipeline = PipelineConfig {
    request_timeout: Some(Duration::from_secs(30)),
    write_timeout: Some(Duration::from_secs(60)),
    ..PipelineConfig::default()
};
```

`PipelineConfig` has public fields, so a struct literal must end with
`..PipelineConfig::default()` (or a preset). New fields such as
`max_connections` are then filled in for you.

### Fixed deadlines

- **Request head**: every hyper HTTP/1.1 builder applies a 10 s header-read
  timeout (`armature_core::pipeline::HEADER_READ_TIMEOUT`), and armature-h1
  applies its own 10 s head deadline.
- **TLS handshake**: every TLS accept must complete within 10 s.
- **HTTP/2**: hyper's HTTP/2 builder is given a timer, so keep-alive pings and
  timeouts work. Earlier versions panicked on every HTTP/2 connection.

## Preset Configurations

`PipelineConfig` ships presets. All of them keep `max_connections` at
`DEFAULT_MAX_CONNECTIONS` and `request_timeout` at `None`.

```rust,ignore
use armature_core::pipeline::PipelineConfig;

let config = PipelineConfig::default();            // 60 s keep-alive, 16 KiB head
let config = PipelineConfig::high_performance();   // 120 s keep-alive, 32 KiB head
let config = PipelineConfig::low_latency();        // 30 s keep-alive, 60 s write timeout
let config = PipelineConfig::memory_efficient();   // 30 s keep-alive, 8 KiB head
```

## Timeout Behavior

### Body read

```
┌───────────── BodyReadPolicy ─────────────┐
│ idle ≤ 30 s between chunks                │
│ total ≤ 30 s + received / 1 KiB/s         │
├───────────────────────────────────────────┤
│                Read Body                  │
└───────────────────────────────────────────┘
```

Either rule ends the read with `408 Request Timeout`. The rate rule stops a
client that sends one byte just inside every idle window from holding the
connection, and its buffered body, indefinitely.

### Handler deadline

`PipelineConfig::request_timeout` races the whole handler future from the
moment it is called (on the armature-h1 backend, as
`armature_h1::Limits::handler_timeout`), and answers `503 Service Unavailable`
when it expires. Slow request bodies are the `BodyReadPolicy`'s job, not this
deadline's. Leave it at `None` unless you know your handlers' upper bound.

## Error Responses

Errors use the canonical `{"error": ..., "status": ...}` body.

### 408 Request Timeout

```json
{
  "error": "Request Timeout: request body was not received in time",
  "status": 408
}
```

### 413 Payload Too Large

A body that grows past the cap while streaming gets the JSON envelope. On the
default armature-h1 backend, a **declared** `Content-Length` over the cap is
refused by armature-h1 itself with a bare `413` and a close, before the handler
runs.

### 400 Bad Request (non-canonical path)

Every transport canonicalizes the request path before middleware, guards and
routing run. `//` collapses and percent-encoded unreserved characters decode;
`.`/`..` segments, `%2F`, `%5C`, `%00`, `\` and malformed `%` escapes are
refused with `400`.

## Best Practices

### 1. Size the body cap to the endpoint

```rust,ignore
// JSON API - small payloads
let app = app.with_max_body_size(100 * 1024); // 100 KB
```

Run upload endpoints in a separate service (or behind a proxy rule) when they
need a much larger cap than the rest of the API.

### 2. Relax the body policy for slow uploads only when you must

```rust,ignore
use armature_core::BodyReadPolicy;
use std::time::Duration;

// Mobile clients on poor links
let app = app.with_body_read_policy(BodyReadPolicy {
    idle_timeout: Duration::from_secs(60),
    min_bytes_per_sec: 256,
});
```

Setting `min_bytes_per_sec: 0` disables the rate rule and leaves only the idle
deadline. Do not do that on public listeners.

### 3. Cap connections below your file-descriptor limit

`max_connections` should be comfortably below `ulimit -n` divided by the number
of accept loops (workers), so the process never runs out of descriptors.

### 4. Defense in depth

Combine server limits with rate limiting (`armature-ratelimit`) and a reverse
proxy that has its own timeouts. Behind a proxy, read the client address with
`HttpRequest::client_address(trusted_proxy_depth)` set to the number of proxies
you run. Never trust the leftmost `X-Forwarded-For` entry.

## Examples

### Complete API Server

```rust,ignore
use armature_core::{BodyReadPolicy, pipeline::PipelineConfig};
use armature::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _guard = Application::init_logging();

    let pipeline = PipelineConfig {
        request_timeout: Some(Duration::from_secs(30)),
        ..PipelineConfig::builder()
            .keep_alive_timeout(Duration::from_secs(10))
            .max_connections(2_000)
            .build()
    };

    let app = Application::create::<AppModule>()
        .await
        .with_pipeline_config(pipeline)
        .with_max_body_size(1024 * 1024)
        .with_body_read_policy(BodyReadPolicy::default());

    app.listen(3000).await
}
```

### Handling a bootstrap failure

`Application::create` panics if a module guard cannot be instantiated, so a
module's routes are never served unguarded. Use `Application::try_create` to
handle the error yourself:

```rust,ignore
let app = match Application::try_create::<AppModule>().await {
    Ok(app) => app,
    Err(e) => {
        eprintln!("bootstrap failed: {e}");
        std::process::exit(1);
    }
};
```

## Common Pitfalls

- ❌ Setting `min_bytes_per_sec: 0` on a public listener
- ❌ Raising `max_body_size` globally for one upload route
- ❌ Constructing `PipelineConfig` with a struct literal without
  `..PipelineConfig::default()`
- ❌ Setting a finite `request_timeout` when you serve SSE or long-polls

- ✅ Keep the default body policy unless you have measured slow clients
- ✅ Monitor 408 and 413 responses to tune limits
- ✅ Keep `max_connections` below your descriptor limit

## Summary

| Setting | API |
|---------|-----|
| Body inactivity / minimum rate | `Application::with_body_read_policy(BodyReadPolicy { .. })` |
| Body size | `Application::with_max_body_size(bytes)` |
| Connection cap, head size, keep-alive | `Application::with_pipeline_config(PipelineConfig::builder()...build())` |
| Handler / write deadline | `PipelineConfig { request_timeout, write_timeout, ..Default::default() }` |
| Header and TLS handshake deadlines | Fixed at 10 s |
