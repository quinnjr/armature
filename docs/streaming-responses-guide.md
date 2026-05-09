# Streaming Responses Guide

Armature provides comprehensive support for streaming HTTP responses, enabling efficient delivery of large data sets, real-time data, and chunked transfers.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Basic Usage](#basic-usage)
- [Stream Types](#stream-types)
- [StreamingResponse](#streamingresponse)
- [Helper Functions](#helper-functions)
- [Progress Tracking](#progress-tracking)
- [Integration Examples](#integration-examples)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

Traditional HTTP responses buffer the entire response body in memory before sending. This works well for small responses but becomes problematic for:

- **Large files**: Buffering a 1GB file requires 1GB of memory
- **Real-time data**: Data must wait until generation completes
- **Long-running queries**: Users see nothing until everything is ready

Streaming responses solve these problems by sending data as it becomes available using HTTP chunked transfer encoding.

## Features

- ✅ Chunked transfer encoding
- ✅ Async stream-based response bodies
- ✅ Multiple stream types (bytes, JSON, text)
- ✅ NDJSON (Newline-Delimited JSON) support
- ✅ Progress tracking and callbacks
- ✅ File streaming utilities
- ✅ Iterator-to-stream helpers
- ✅ Backpressure handling

## Basic Usage

### Simple Byte Stream

```rust
use armature_core::streaming::{ByteStream, StreamingResponse};

async fn stream_data() -> StreamingResponse {
    let (stream, sender) = ByteStream::new();

    // Spawn a task to produce data
    tokio::spawn(async move {
        for i in 0..100 {
            sender.send(format!("Line {}\n", i).into_bytes()).await.ok();
        }
        sender.close().await;
    });

    StreamingResponse::new(stream)
        .content_type("text/plain")
        .no_cache()
}
```

### JSON Streaming (NDJSON)

```rust
use armature_core::streaming::{JsonStream, StreamingResponse};
use serde::Serialize;

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn stream_users() -> StreamingResponse {
    let (stream, sender) = JsonStream::new();

    tokio::spawn(async move {
        let users = load_users_from_database().await;
        for user in users {
            sender.send_json(&user).await.ok();
        }
        sender.close().await;
    });

    StreamingResponse::ndjson(stream)
}
```

## Stream Types

### ByteStream

The foundational stream type for raw binary data.

```rust
use armature_core::streaming::{ByteStream, ByteStreamSender};

// Create with default buffer size (64)
let (stream, sender) = ByteStream::new();

// Create with custom buffer size
let (stream, sender) = ByteStream::with_buffer_size(256);
```

#### ByteStreamSender Methods

| Method | Description |
|--------|-------------|
| `send(data)` | Send bytes (Vec<u8>, &[u8], etc.) |
| `send_bytes(bytes)` | Send a `Bytes` object |
| `send_str(s)` | Send a string slice |
| `send_error(msg)` | Signal an error |
| `close()` | Close the stream |
| `bytes_sent()` | Get total bytes sent |
| `is_closed()` | Check if receiver dropped |

### JsonStream

For streaming JSON objects as NDJSON (newline-delimited JSON).

```rust
use armature_core::streaming::{JsonStream, JsonStreamSender};

let (stream, sender) = JsonStream::new();

// Send a serializable value
sender.send_json(&my_struct).await?;

// Send raw JSON string
sender.send_raw(r#"{"key": "value"}"#).await?;

// Send an error as JSON
sender.send_error("Something went wrong").await?;
```

NDJSON format:
```
{"id":1,"name":"Alice"}
{"id":2,"name":"Bob"}
{"id":3,"name":"Charlie"}
```

### TextStream

For streaming text lines.

```rust
use armature_core::streaming::{TextStream, TextStreamSender};

let (stream, sender) = TextStream::new();

// Send a line (newline added automatically)
sender.send_line("First line").await?;

// Send raw text (no newline)
sender.send("Some text").await?;
```

## StreamingResponse

The main response type for streaming.

### Creating Responses

```rust
use armature_core::streaming::{StreamingResponse, ByteStream, JsonStream, TextStream};

// From ByteStream
let (byte_stream, _) = ByteStream::new();
let response = StreamingResponse::new(byte_stream);

// From JsonStream (NDJSON)
let (json_stream, _) = JsonStream::new();
let response = StreamingResponse::ndjson(json_stream);

// From TextStream
let (text_stream, _) = TextStream::new();
let response = StreamingResponse::text(text_stream);

// Empty response
let response = StreamingResponse::empty();
```

### Response Configuration

```rust
let response = StreamingResponse::new(stream)
    .status(200)                           // Set HTTP status
    .content_type("application/octet-stream") // Set Content-Type
    .header("X-Custom", "value")           // Add custom header
    .no_cache()                            // Disable caching
    .cors("*")                             // Enable CORS
    .nosniff();                            // X-Content-Type-Options: nosniff
```

### Converting to Buffered Response

If you need to convert a streaming response to a regular `HttpResponse` (not recommended for large streams):

```rust
let buffered = streaming_response.into_buffered().await?;
// buffered is now an HttpResponse with full body in memory
```

## Helper Functions

### stream_iter

Stream items from a synchronous iterator:

```rust
use armature_core::streaming::stream_iter;

let items = vec![1, 2, 3, 4, 5];
let (stream, handle) = stream_iter(
    items.into_iter(),
    |i| format!("Item: {}\n", i).into_bytes()
);
```

### stream_iter_with_delay

Stream items with a delay between each:

```rust
use armature_core::streaming::stream_iter_with_delay;
use std::time::Duration;

let items = vec!["a", "b", "c"];
let (stream, handle) = stream_iter_with_delay(
    items.into_iter(),
    |s| s.as_bytes().to_vec(),
    Duration::from_millis(100)
);
```

### stream_json_iter

Stream JSON items from an iterator:

```rust
use armature_core::streaming::stream_json_iter;

let users = vec![
    User { id: 1, name: "Alice".into() },
    User { id: 2, name: "Bob".into() },
];
let (stream, handle) = stream_json_iter(users.into_iter());
```

### stream_reader

Stream data from an async reader (files, network sockets):

```rust
use armature_core::streaming::stream_reader;
use tokio::fs::File;

let file = File::open("large_file.bin").await?;
let (stream, handle) = stream_reader(file, 8192);  // 8KB chunks

let response = StreamingResponse::new(stream)
    .content_type("application/octet-stream");
```

## Progress Tracking

Track streaming progress with `ProgressStream`:

```rust
use armature_core::streaming::{ByteStream, ProgressStream};

let (stream, sender) = ByteStream::new();

let progress_stream = ProgressStream::new(stream)
    .on_progress(|bytes_received| {
        println!("Received {} bytes", bytes_received);
    });

// Later, check total bytes
let total = progress_stream.bytes_received();
```

## Integration Examples

### File Download with Progress

```rust
use armature_core::streaming::{stream_reader, StreamingResponse};
use tokio::fs::File;

async fn download_file(filename: &str) -> Result<StreamingResponse, Error> {
    let file = File::open(filename).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();

    let (stream, _) = stream_reader(file, 64 * 1024);  // 64KB chunks

    Ok(StreamingResponse::new(stream)
        .content_type("application/octet-stream")
        .header("Content-Length", file_size.to_string())
        .header("Content-Disposition", format!("attachment; filename=\"{}\"", filename)))
}
```

### Database Cursor Streaming

```rust
use armature_core::streaming::{JsonStream, StreamingResponse};

async fn stream_query_results() -> StreamingResponse {
    let (stream, sender) = JsonStream::new();

    tokio::spawn(async move {
        let mut cursor = database.query("SELECT * FROM large_table").await.unwrap();

        while let Some(row) = cursor.next().await {
            match row {
                Ok(record) => {
                    if sender.send_json(&record).await.is_err() {
                        break;  // Client disconnected
                    }
                }
                Err(e) => {
                    sender.send_error(e.to_string()).await.ok();
                    break;
                }
            }
        }
        sender.close().await;
    });

    StreamingResponse::ndjson(stream)
}
```

### Real-Time Log Streaming

```rust
use armature_core::streaming::{TextStream, StreamingResponse};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

async fn stream_logs() -> StreamingResponse {
    let (stream, sender) = TextStream::new();

    tokio::spawn(async move {
        let mut child = Command::new("tail")
            .args(["-f", "/var/log/app.log"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout).lines();

        while let Ok(Some(line)) = reader.next_line().await {
            if sender.send_line(&line).await.is_err() {
                break;
            }
        }

        child.kill().await.ok();
    });

    StreamingResponse::text(stream)
        .no_cache()
}
```

### Chunked JSON Array

```rust
use armature_core::streaming::{ByteStream, StreamingResponse};

async fn stream_json_array() -> StreamingResponse {
    let (stream, sender) = ByteStream::new();

    tokio::spawn(async move {
        // Send opening bracket
        sender.send_str("[").await.ok();

        let items = fetch_items().await;
        let mut first = true;

        for item in items {
            if !first {
                sender.send_str(",").await.ok();
            }
            first = false;

            let json = serde_json::to_string(&item).unwrap();
            sender.send_str(&json).await.ok();
        }

        // Send closing bracket
        sender.send_str("]").await.ok();
        sender.close().await;
    });

    StreamingResponse::new(stream)
        .content_type("application/json")
}
```

## Best Practices

### 1. Handle Client Disconnection

Always check if the client is still connected:

```rust
if sender.send(data).await.is_err() {
    // Client disconnected, stop producing data
    break;
}
```

Or check explicitly:

```rust
if sender.is_closed() {
    break;
}
```

### 2. Use Appropriate Buffer Sizes

```rust
// Small, frequent updates (real-time)
let (stream, sender) = ByteStream::with_buffer_size(16);

// Large file transfers
let (stream, sender) = ByteStream::with_buffer_size(256);
```

### 3. Always Close Streams

Ensure streams are properly closed to signal completion:

```rust
tokio::spawn(async move {
    // ... produce data ...
    sender.close().await;  // Important!
});
```

### 4. Set Cache Headers

Streaming responses should typically not be cached:

```rust
StreamingResponse::new(stream)
    .no_cache()
```

### 5. Consider Content-Type

Set appropriate content types:

| Format | Content-Type |
|--------|-------------|
| NDJSON | `application/x-ndjson` |
| JSON Lines | `application/jsonl` |
| Plain text | `text/plain; charset=utf-8` |
| Binary | `application/octet-stream` |
| CSV | `text/csv` |

### 6. Error Handling

Propagate errors gracefully:

```rust
match process_item(&item) {
    Ok(data) => sender.send(data).await.ok(),
    Err(e) => {
        sender.send_error(e.to_string()).await.ok();
        break;
    }
}
```

## Common Pitfalls

### ❌ Forgetting to Close

```rust
// Bad: Stream never ends
tokio::spawn(async move {
    for item in items {
        sender.send(item).await.ok();
    }
    // Missing: sender.close().await;
});
```

### ❌ Ignoring Backpressure

```rust
// Bad: Ignores send errors
for item in items {
    let _ = sender.send(item).await;  // Client may have disconnected
}

// Good: Respect backpressure
for item in items {
    if sender.send(item).await.is_err() {
        break;
    }
}
```

### ❌ Blocking the Stream Producer

```rust
// Bad: Blocking I/O in async context
tokio::spawn(async move {
    let data = std::fs::read("file.txt").unwrap();  // Blocks!
    sender.send(data).await.ok();
});

// Good: Use async I/O
tokio::spawn(async move {
    let data = tokio::fs::read("file.txt").await.unwrap();
    sender.send(data).await.ok();
});
```

## API Reference

### Types

| Type | Description |
|------|-------------|
| `StreamingResponse` | Main streaming response type |
| `StreamBody` | Enum of stream body variants |
| `StreamChunk` | A chunk of streaming data |
| `ByteStream` | Stream of raw bytes |
| `ByteStreamSender` | Sender for byte streams |
| `JsonStream` | Stream of JSON objects (NDJSON) |
| `JsonStreamSender` | Sender for JSON streams |
| `TextStream` | Stream of text lines |
| `TextStreamSender` | Sender for text streams |
| `ProgressStream` | Progress-tracking wrapper |

### Functions

| Function | Description |
|----------|-------------|
| `stream_iter(iter, transform)` | Stream from iterator |
| `stream_iter_with_delay(iter, transform, delay)` | Stream with delays |
| `stream_json_iter(iter)` | Stream JSON from iterator |
| `stream_reader(reader, chunk_size)` | Stream from async reader |

## Summary

**Key Points:**

1. **Use streaming for large/real-time data** - Don't buffer when you can stream
2. **Choose the right stream type** - `ByteStream`, `JsonStream`, or `TextStream`
3. **Handle disconnection** - Check send results and `is_closed()`
4. **Close streams properly** - Call `close()` when done
5. **Set appropriate headers** - Content-Type and Cache-Control
6. **Consider backpressure** - Don't overwhelm the client

**When to Use Streaming:**

- File downloads/uploads
- Database query results
- Real-time logs/events
- Large JSON responses
- Any response > 1MB

**When NOT to Use Streaming:**

- Small, cacheable responses
- Simple API responses
- Static content (use `static_assets` module instead)


