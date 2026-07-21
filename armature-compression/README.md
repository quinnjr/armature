# armature-compression

HTTP response compression middleware for the Armature framework.

## Features

- **Multiple Algorithms** - gzip, brotli, zstd (feature-gated)
- **Content Negotiation** - `Accept-Encoding` driven algorithm selection
- **Streaming** - Compress chunks as data is generated
- **Configurable** - Minimum size, compression level, content-type allow-list
- **Auto Detection** - Skip already-encoded and non-compressible content

Default features enable `gzip` and `brotli`; enable `zstd` (or `full`) as needed.

## Installation

```toml
[dependencies]
armature-compression = "0.1"
```

## Quick Start

```rust
use armature_compression::CompressionMiddleware;

// Auto-selects the best algorithm from the client's Accept-Encoding header.
let middleware = CompressionMiddleware::new();
```

## Configuration

Configuration is built with `CompressionConfig::builder()` and passed to
`CompressionMiddleware::with_config`:

```rust
use armature_compression::{CompressionAlgorithm, CompressionConfig, CompressionMiddleware};

let config = CompressionConfig::builder()
    .algorithm(CompressionAlgorithm::Brotli) // or .gzip() / .brotli() / .zstd()
    .level(6)                                 // 0 = algorithm default
    .min_size(1024)                           // don't compress bodies < 1 KiB
    .compressible_types(vec![                 // replace the default allow-list
        "text/*".to_string(),
        "application/json".to_string(),
    ])
    .compress_encoded(false)                  // skip already-encoded responses
    .build();

let middleware = CompressionMiddleware::with_config(config);
```

Algorithm selection with `CompressionAlgorithm::Auto` (the default) prefers
`br` > `zstd` > `gzip`, falling back to no compression when the client accepts
none of the enabled algorithms.

## Streaming Compression

```rust
use armature_compression::streaming::{StreamingCompressor, StreamingConfig};
use armature_compression::CompressionAlgorithm;

let config = StreamingConfig::new()
    .algorithm(CompressionAlgorithm::Gzip)
    .level(6)
    .flush_interval(4096)   // flush after this many buffered bytes
    .min_chunk_size(64);    // stage input until at least this many bytes accrue

let mut compressor = StreamingCompressor::new(config)?;

// Compress chunks as they are produced. Output may be empty while input is
// buffered below `min_chunk_size` or `flush_interval`.
for chunk in data_stream {
    let compressed = compressor.compress_chunk(&chunk)?;
    sink.write_all(&compressed)?;
}

// Emit any remaining buffered data and the compression trailer.
let tail = compressor.finish()?;
sink.write_all(&tail)?;
```

Select the algorithm directly from a request header with
`StreamingCompressor::from_accept_encoding(accept_encoding, config)`. For async
stream pipelines, `AsyncStreamingCompressor` offers `.process(chunk).await` and
`.flush().await` wrappers over the same compressor.

## License

MIT OR Apache-2.0
