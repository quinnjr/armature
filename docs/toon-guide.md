# TOON Guide

TOON (Token-Oriented Object Notation) support for Armature - optimized serialization for LLM applications.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Format Comparison](#format-comparison)
- [HTTP Integration](#http-integration)
- [Token Counting](#token-counting)
- [Batch Conversion](#batch-conversion)
- [Best Practices](#best-practices)

---

## Overview

TOON is a serialization format designed to reduce token count by 30-60% compared to JSON. This makes it ideal for:

- **LLM Applications**: Reduce API costs by minimizing tokens
- **AI Agents**: More efficient context management
- **Prompt Engineering**: Fit more data in context windows
- **Streaming**: Lower latency with smaller payloads

---

## Features

- ✅ **30-60% Token Reduction**: Optimized format for LLMs
- ✅ **Serde Compatible**: Works with existing Rust types
- ✅ **HTTP Integration**: Response helpers for TOON content
- ✅ **Format Conversion**: JSON ↔ TOON utilities
- ✅ **Token Counting**: Estimate LLM token usage
- ✅ **Content Negotiation**: Accept header handling

---

## Quick Start

### Installation

```toml
[dependencies]
armature-toon = "0.1"
```

With HTTP integration:

```toml
[dependencies]
armature-toon = { version = "0.1", features = ["http"] }
```

### Basic Usage

```rust
use armature_toon::{to_string, from_str};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct User {
    id: u32,
    name: String,
    email: String,
    active: bool,
}

fn main() {
    let user = User {
        id: 123,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        active: true,
    };

    // Serialize to TOON
    let toon = to_string(&user).unwrap();
    println!("TOON: {}", toon);

    // Deserialize from TOON
    let parsed: User = from_str(&toon).unwrap();
    assert_eq!(user, parsed);
}
```

---

## Format Comparison

### Compare Token Efficiency

```rust
use armature_toon::compare_formats;
use serde::Serialize;

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    data: Vec<Item>,
    pagination: Pagination,
}

#[derive(Serialize)]
struct Item {
    id: u64,
    name: String,
    description: String,
}

#[derive(Serialize)]
struct Pagination {
    page: u32,
    per_page: u32,
    total: u64,
}

let response = ApiResponse {
    success: true,
    data: vec![
        Item { id: 1, name: "Widget".into(), description: "A useful widget".into() },
        Item { id: 2, name: "Gadget".into(), description: "A handy gadget".into() },
    ],
    pagination: Pagination { page: 1, per_page: 10, total: 100 },
};

let comparison = compare_formats(&response).unwrap();

println!("JSON characters: {}", comparison.json_chars);
println!("TOON characters: {}", comparison.toon_chars);
println!("Token reduction: {:.1}%", comparison.reduction_percent);
println!("Est. JSON tokens: {}", comparison.json_tokens_estimate);
println!("Est. TOON tokens: {}", comparison.toon_tokens_estimate);
```

**Example Output:**
```
JSON characters: 245
TOON characters: 142
Token reduction: 42.0%
Est. JSON tokens: 62
Est. TOON tokens: 36
```

---

## HTTP Integration

### TOON Responses

```rust
use armature_toon::{Toon, ToonResponseExt};
use armature_core::http::HttpResponse;
use serde::Serialize;

#[derive(Serialize)]
struct ApiData {
    result: String,
    count: u32,
}

// Method 1: Using Toon wrapper
async fn handler1() -> HttpResponse {
    let data = ApiData { result: "success".into(), count: 42 };
    Toon::new(data).into_response().unwrap()
}

// Method 2: Using extension trait
async fn handler2() -> HttpResponse {
    let data = ApiData { result: "success".into(), count: 42 };
    HttpResponse::toon(data).unwrap()
}

// Method 3: With custom status
async fn handler3() -> HttpResponse {
    let data = ApiData { result: "created".into(), count: 1 };
    HttpResponse::toon_with_status(201, data).unwrap()
}
```

### Content Negotiation

```rust
use armature_toon::ToonContentNegotiator;
use armature_core::http::{HttpRequest, HttpResponse};
use serde::Serialize;

#[derive(Serialize)]
struct Data { value: i32 }

async fn handler(req: HttpRequest) -> HttpResponse {
    let data = Data { value: 42 };
    let accept = req.headers.get("Accept").map(|s| s.as_str());

    if ToonContentNegotiator::prefers_toon(accept) {
        HttpResponse::toon(data).unwrap()
    } else {
        HttpResponse::json(data).unwrap()
    }
}
```

---

## Token Counting

### Track Token Usage

```rust
use armature_toon::TokenCounter;
use serde::Serialize;

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

let mut counter = TokenCounter::new();

// Add messages to context
let messages = vec![
    Message { role: "system".into(), content: "You are a helpful assistant.".into() },
    Message { role: "user".into(), content: "Hello!".into() },
    Message { role: "assistant".into(), content: "Hi! How can I help you today?".into() },
];

for msg in &messages {
    counter.add(msg).unwrap();
}

println!("Total characters: {}", counter.total_chars());
println!("Estimated tokens: {}", counter.total_tokens_estimate());

// Check against context limit (e.g., 4096 tokens)
if counter.total_tokens_estimate() > 4000 {
    println!("Warning: Approaching context limit!");
}
```

---

## Batch Conversion

### JSON to TOON

```rust
use armature_toon::BatchConverter;

// Convert existing JSON to TOON
let json = r#"{"users":[{"id":1,"name":"Alice"},{"id":2,"name":"Bob"}]}"#;
let toon = BatchConverter::json_to_toon(json).unwrap();
println!("TOON: {}", toon);
```

### TOON to JSON

```rust
use armature_toon::BatchConverter;

// Convert TOON back to JSON for debugging
let toon = "..."; // TOON string
let json = BatchConverter::toon_to_json(toon).unwrap();
let json_pretty = BatchConverter::toon_to_json_pretty(toon).unwrap();
```

---

## Best Practices

### 1. Use for LLM Contexts

TOON is most beneficial when sending data to LLMs:

```rust
use armature_toon::to_string;

// Serialize context data in TOON format
let context = to_string(&user_data).unwrap();

// Send to LLM API
let prompt = format!("Given this user data: {}\n\nAnswer: ...", context);
```

### 2. Measure Token Savings

Always measure actual savings for your data:

```rust
use armature_toon::compare_formats;

let comparison = compare_formats(&your_data).unwrap();
if comparison.reduction_percent < 20.0 {
    // For small reductions, JSON might be preferable for compatibility
    println!("Consider using JSON for this data type");
}
```

### 3. Content Negotiation

Support both formats for maximum compatibility:

```rust
// Client can request preferred format
// Accept: application/toon, application/json;q=0.9

if ToonContentNegotiator::prefers_toon(accept) {
    HttpResponse::toon(data)
} else {
    HttpResponse::json(data)
}
```

### 4. Monitor Token Budgets

Track token usage across requests:

```rust
let mut counter = TokenCounter::new();

// Add all context items
for item in context_items {
    counter.add(&item)?;
}

// Reserve tokens for response
let available = 4096 - counter.total_tokens_estimate() - 500; // 500 for response
```

---

## API Reference

### Core Functions

```rust
// Serialization
to_string(&value) -> Result<String>
to_vec(&value) -> Result<Vec<u8>>

// Deserialization
from_str(s) -> Result<T>
from_slice(bytes) -> Result<T>

// Comparison
compare_formats(&value) -> Result<FormatComparison>
```

### Types

```rust
// HTTP responses (requires "http" feature)
Toon<T>                    // Response wrapper
ToonResponseExt            // Extension trait for HttpResponse
ToonContentNegotiator      // Accept header handling

// Utilities
ToonSerializer             // Configurable serializer
ToonDeserializer           // Configurable deserializer
TokenCounter               // Token usage tracking
BatchConverter             // JSON ↔ TOON conversion
FormatComparison           // Comparison results
```

### Content Type

```rust
const TOON_CONTENT_TYPE: &str = "application/toon";
```

---

## Summary

TOON support in Armature provides:

- **30-60% token reduction** vs JSON
- **Serde compatible** serialization
- **HTTP response helpers** for APIs
- **Token counting** for LLM context management
- **Format comparison** tools

Use TOON when:
- Sending structured data to LLMs
- Optimizing API costs
- Managing large context windows
- Building AI agents

---

**Optimize your LLM token usage!** 🎯

