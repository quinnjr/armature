# Compression Module

HTTP response compression middleware for the Armature framework.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Compression Algorithms](#compression-algorithms)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The `armature-compression` module provides middleware that automatically compresses HTTP responses to reduce bandwidth and improve page load times. It supports multiple compression algorithms and intelligently selects the best one based on client capabilities.

## Features

- ✅ Multiple compression algorithms (gzip, brotli, zstd)
- ✅ Automatic algorithm selection based on `Accept-Encoding`
- ✅ Configurable compression levels
- ✅ Minimum size thresholds to avoid compressing small responses
- ✅ Content-type aware compression (text, JSON, etc.)
- ✅ Proper `Content-Encoding` and `Vary` header handling
- ✅ Feature flags for each algorithm to minimize binary size

## Installation

Add the compression feature to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["compression"] }
```

Or use specific compression algorithms:

```toml
[dependencies]
armature-compression = { version = "0.1", features = ["gzip", "brotli"] }
```

### Available Features

| Feature | Description | Default |
|---------|-------------|---------|
| `gzip` | Enable gzip compression | ✅ |
| `brotli` | Enable brotli compression | ✅ |
| `zstd` | Enable zstd compression | ❌ |
| `full` | Enable all algorithms | ❌ |

## Usage

### Basic Usage

```rust
use armature_framework::prelude::*;
use armature_compression::CompressionMiddleware;

fn main() {
    let mut chain = MiddlewareChain::new();

    // Add compression with default settings (auto-select algorithm)
    chain.use_middleware(CompressionMiddleware::new());
}
```

### With Custom Configuration

```rust
use armature_compression::{CompressionMiddleware, CompressionConfig, CompressionAlgorithm};

let config = CompressionConfig::builder()
    .algorithm(CompressionAlgorithm::Brotli)  // Force brotli
    .level(4)                                  // Compression level
    .min_size(1024)                           // Only compress > 1KB
    .build();

let middleware = CompressionMiddleware::with_config(config);
```

### Quick Algorithm Selection

```rust
use armature_compression::{CompressionMiddleware, CompressionConfig};

// Use gzip
let gzip = CompressionConfig::builder().gzip().build();

// Use brotli
let brotli = CompressionConfig::builder().brotli().build();

// Use zstd (requires "zstd" feature)
let zstd = CompressionConfig::builder().zstd().build();

// Disable compression
let none = CompressionConfig::builder().no_compression().build();
```

## Configuration

### CompressionConfig Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `algorithm` | `CompressionAlgorithm` | `Auto` | Which algorithm to use |
| `level` | `u32` | Algorithm default | Compression level |
| `min_size` | `usize` | `860` | Minimum response size to compress |
| `compressible_types` | `Vec<String>` | See below | Content types to compress |
| `compress_encoded` | `bool` | `false` | Compress already-encoded responses |

### Default Compressible Types

The middleware compresses these content types by default:

- `text/*` (HTML, CSS, JavaScript, plain text)
- `application/json`
- `application/javascript`
- `application/xml`
- `image/svg+xml`
- `application/wasm`
- Various font types

### Adding Custom Types

```rust
let config = CompressionConfig::builder()
    .add_compressible_type("application/custom+json")
    .add_compressible_type("text/vnd.custom")
    .build();
```

## Compression Algorithms

### Algorithm Comparison

| Algorithm | Ratio | Speed | Browser Support |
|-----------|-------|-------|-----------------|
| **Brotli** | Best | Slower | Chrome, Firefox, Edge, Safari |
| **Zstd** | Very Good | Fastest | Limited (Chrome 123+) |
| **Gzip** | Good | Fast | Universal |

### Auto-Selection Priority

When using `CompressionAlgorithm::Auto`, the middleware selects based on `Accept-Encoding`:

1. **Brotli** (`br`) - Preferred for best compression
2. **Zstd** (`zstd`) - If available and no brotli
3. **Gzip** (`gzip`) - Universal fallback

### Compression Levels

Each algorithm has its own level range:

| Algorithm | Min | Max | Default | Notes |
|-----------|-----|-----|---------|-------|
| Gzip | 1 | 9 | 6 | Higher = smaller but slower |
| Brotli | 0 | 11 | 4 | 4-6 recommended for web |
| Zstd | 1 | 22 | 3 | Fast default, 19+ for archives |

## Best Practices

### 1. Use Auto Selection

For most applications, auto-selection provides the best results:

```rust
// Let the middleware choose based on Accept-Encoding
let middleware = CompressionMiddleware::new();
```

### 2. Set Appropriate Minimum Size

Don't compress tiny responses (overhead may exceed savings):

```rust
let config = CompressionConfig::builder()
    .min_size(860)  // Typical MTU - good default
    .build();
```

### 3. Choose Level Based on Use Case

```rust
// API server: prioritize speed
let config = CompressionConfig::builder()
    .gzip()
    .level(1)  // Fast compression
    .build();

// Static assets: prioritize size
let config = CompressionConfig::builder()
    .brotli()
    .level(9)  // Better compression (slower)
    .build();
```

### 4. Don't Compress Already-Compressed Content

Images (JPEG, PNG), videos, and archives are already compressed. The default content type list excludes these.

## Common Pitfalls

- ❌ **Don't** compress binary content (images, videos, PDFs)
- ❌ **Don't** use maximum compression levels for dynamic content
- ❌ **Don't** compress responses smaller than ~1KB
- ✅ **Do** let the middleware handle `Content-Encoding` headers
- ✅ **Do** use auto-selection for broad browser compatibility

## API Reference

### CompressionMiddleware

```rust
impl CompressionMiddleware {
    /// Create with default settings
    pub fn new() -> Self;

    /// Create with custom configuration
    pub fn with_config(config: CompressionConfig) -> Self;

    /// Get configuration reference
    pub fn config(&self) -> &CompressionConfig;
}
```

### CompressionConfig

```rust
impl CompressionConfig {
    /// Create with defaults
    pub fn new() -> Self;

    /// Create a builder
    pub fn builder() -> CompressionConfigBuilder;

    /// Get effective compression level
    pub fn effective_level(&self) -> u32;

    /// Check if content type should be compressed
    pub fn should_compress_content_type(&self, ct: &str) -> bool;

    /// Check if size meets threshold
    pub fn should_compress_size(&self, size: usize) -> bool;
}
```

### CompressionAlgorithm

```rust
pub enum CompressionAlgorithm {
    Auto,    // Select based on Accept-Encoding
    Gzip,    // gzip compression
    Brotli,  // brotli compression
    Zstd,    // zstd compression
    None,    // No compression
}

impl CompressionAlgorithm {
    /// Get Content-Encoding header value
    pub fn encoding_name(&self) -> Option<&'static str>;

    /// Check if feature is enabled
    pub fn is_available(&self) -> bool;

    /// Select from Accept-Encoding header
    pub fn select_from_accept_encoding(header: &str) -> Self;

    /// Compress data
    pub fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>>;
}
```

## Summary

**Key Points:**

1. Use `CompressionMiddleware::new()` for auto-selection
2. Configure with `CompressionConfig::builder()` for custom settings
3. Default settings work well for most web applications
4. Brotli provides best compression, gzip has widest support
5. Set appropriate `min_size` to avoid compressing tiny responses

**Quick Start:**

```rust
use armature_compression::CompressionMiddleware;

let mut chain = MiddlewareChain::new();
chain.use_middleware(CompressionMiddleware::new());
```

