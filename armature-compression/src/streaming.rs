//! Streaming Compression - Compress chunks as they're generated
//!
//! This module provides streaming compression that compresses data incrementally
//! as it's produced, rather than buffering the entire response.
//!
//! # Features
//!
//! - Incremental compression for streaming responses
//! - Multiple algorithm support (gzip, brotli, zstd)
//! - Configurable flush intervals
//! - Integration with SSE and chunked responses
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_compression::streaming::{StreamingCompressor, StreamingConfig};
//!
//! let config = StreamingConfig::new()
//!     .algorithm(CompressionAlgorithm::Gzip)
//!     .flush_interval(1024);
//!
//! let mut compressor = StreamingCompressor::new(config)?;
//!
//! // Compress chunks as they arrive
//! let compressed = compressor.compress_chunk(data)?;
//!
//! // Finish compression
//! let final_chunk = compressor.finish()?;
//! ```

use crate::{CompressionAlgorithm, CompressionError, Result};
use bytes::{Bytes, BytesMut};

#[cfg(feature = "gzip")]
use flate2::Compression as GzipCompression;
#[cfg(feature = "gzip")]
use flate2::write::GzEncoder;

#[cfg(feature = "brotli")]
use brotli::CompressorWriter as BrotliEncoder;

#[cfg(feature = "zstd")]
use zstd::stream::write::Encoder as ZstdEncoder;

#[cfg(any(feature = "gzip", feature = "brotli", feature = "zstd"))]
use std::io::Write;

/// Configuration for streaming compression.
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Compression algorithm to use
    pub algorithm: CompressionAlgorithm,
    /// Compression level (algorithm-specific)
    pub level: u32,
    /// Flush after this many bytes (0 = auto)
    pub flush_interval: usize,
    /// Minimum chunk size before compressing
    pub min_chunk_size: usize,
    /// Buffer size for compression output
    pub buffer_size: usize,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Auto,
            level: 6,
            flush_interval: 4096,
            min_chunk_size: 64,
            buffer_size: 8192,
        }
    }
}

impl StreamingConfig {
    /// Create new streaming config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set compression algorithm.
    pub fn algorithm(mut self, algorithm: CompressionAlgorithm) -> Self {
        self.algorithm = algorithm;
        self
    }

    /// Set compression level.
    pub fn level(mut self, level: u32) -> Self {
        self.level = level;
        self
    }

    /// Set flush interval (bytes).
    pub fn flush_interval(mut self, interval: usize) -> Self {
        self.flush_interval = interval;
        self
    }

    /// Set minimum chunk size.
    pub fn min_chunk_size(mut self, size: usize) -> Self {
        self.min_chunk_size = size;
        self
    }

    /// Set buffer size.
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Create config optimized for low latency streaming.
    pub fn low_latency() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Auto,
            level: 1,            // Fastest
            flush_interval: 256, // Frequent flushes
            min_chunk_size: 16,
            buffer_size: 1024,
        }
    }

    /// Create config optimized for high compression ratio.
    pub fn high_compression() -> Self {
        Self {
            algorithm: CompressionAlgorithm::Auto,
            level: 9,              // Best compression
            flush_interval: 16384, // Less frequent flushes
            min_chunk_size: 1024,
            buffer_size: 32768,
        }
    }
}

/// Internal encoder state.
#[allow(clippy::large_enum_variant)] // Boxing adds complexity, variant is stack-allocated once
enum EncoderState {
    None,
    #[cfg(feature = "gzip")]
    Gzip(GzEncoder<Vec<u8>>),
    #[cfg(feature = "brotli")]
    Brotli(BrotliEncoder<Vec<u8>>),
    #[cfg(feature = "zstd")]
    Zstd(ZstdEncoder<'static, Vec<u8>>),
}

/// Streaming compressor that compresses chunks incrementally.
pub struct StreamingCompressor {
    config: StreamingConfig,
    encoder: EncoderState,
    bytes_in: u64,
    bytes_out: u64,
    unflushed_bytes: usize,
    /// Input staged but not yet handed to the encoder because it hasn't
    /// reached `min_chunk_size`. Drained on the next threshold-crossing
    /// chunk, or on `flush`/`finish`.
    input_buffer: BytesMut,
    finished: bool,
}

impl StreamingCompressor {
    /// Create a new streaming compressor.
    pub fn new(config: StreamingConfig) -> Result<Self> {
        let encoder = Self::create_encoder(&config)?;

        Ok(Self {
            config,
            encoder,
            bytes_in: 0,
            bytes_out: 0,
            unflushed_bytes: 0,
            input_buffer: BytesMut::new(),
            finished: false,
        })
    }

    /// Create with specific algorithm selected from Accept-Encoding.
    pub fn from_accept_encoding(accept_encoding: &str, config: StreamingConfig) -> Result<Self> {
        let algorithm = CompressionAlgorithm::select_from_accept_encoding(accept_encoding);
        let config = StreamingConfig {
            algorithm,
            ..config
        };
        Self::new(config)
    }

    fn create_encoder(config: &StreamingConfig) -> Result<EncoderState> {
        let algorithm = match config.algorithm {
            CompressionAlgorithm::Auto => {
                // Default to gzip for streaming (best compatibility)
                #[cfg(feature = "gzip")]
                {
                    CompressionAlgorithm::Gzip
                }
                #[cfg(not(feature = "gzip"))]
                {
                    CompressionAlgorithm::None
                }
            }
            other => other,
        };

        match algorithm {
            CompressionAlgorithm::None | CompressionAlgorithm::Auto => Ok(EncoderState::None),

            #[cfg(feature = "gzip")]
            CompressionAlgorithm::Gzip => {
                let level = config.level.clamp(1, 9);
                let encoder = GzEncoder::new(
                    Vec::with_capacity(config.buffer_size),
                    GzipCompression::new(level),
                );
                Ok(EncoderState::Gzip(encoder))
            }

            #[cfg(feature = "brotli")]
            CompressionAlgorithm::Brotli => {
                let level = config.level.clamp(0, 11);
                let encoder = BrotliEncoder::new(
                    Vec::with_capacity(config.buffer_size),
                    config.buffer_size,
                    level,
                    22, // lgwin
                );
                Ok(EncoderState::Brotli(encoder))
            }

            #[cfg(feature = "zstd")]
            CompressionAlgorithm::Zstd => {
                let level = config.level.clamp(1, 22) as i32;
                let encoder = ZstdEncoder::new(Vec::with_capacity(config.buffer_size), level)
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                Ok(EncoderState::Zstd(encoder))
            }

            #[allow(unreachable_patterns)]
            _ => Err(CompressionError::UnsupportedAlgorithm(format!(
                "{:?} not available",
                algorithm
            ))),
        }
    }

    /// Get the Content-Encoding value for this compressor.
    pub fn encoding(&self) -> Option<&'static str> {
        match &self.encoder {
            EncoderState::None => None,
            #[cfg(feature = "gzip")]
            EncoderState::Gzip(_) => Some("gzip"),
            #[cfg(feature = "brotli")]
            EncoderState::Brotli(_) => Some("br"),
            #[cfg(feature = "zstd")]
            EncoderState::Zstd(_) => Some("zstd"),
        }
    }

    /// Compress a chunk of data.
    ///
    /// Returns compressed bytes. May return empty if data is being buffered.
    ///
    /// Sub-threshold input is staged in an internal buffer and only handed to
    /// the encoder once at least `min_chunk_size` bytes have accumulated, so
    /// tiny chunks aren't compressed prematurely. The pass-through (`None`)
    /// algorithm is exempt — with no compression to defer, its output is
    /// emitted immediately to preserve latency.
    pub fn compress_chunk(&mut self, data: &[u8]) -> Result<Bytes> {
        if self.finished {
            return Err(CompressionError::CompressionFailed(
                "Compressor already finished".to_string(),
            ));
        }

        if data.is_empty() {
            return Ok(Bytes::new());
        }

        self.bytes_in += data.len() as u64;

        // Pass-through: nothing to compress, so `min_chunk_size` (a knob that
        // gates *compression*) does not apply. Emit immediately.
        if matches!(self.encoder, EncoderState::None) {
            self.bytes_out += data.len() as u64;
            return Ok(Bytes::copy_from_slice(data));
        }

        // Stage input until we have at least `min_chunk_size` bytes to compress.
        self.input_buffer.extend_from_slice(data);
        if self.input_buffer.len() < self.config.min_chunk_size {
            return Ok(Bytes::new());
        }

        let buffered = std::mem::take(&mut self.input_buffer);
        self.feed_encoder(&buffered)?;

        // Flush if we've accumulated enough since the last flush.
        if self.unflushed_bytes >= self.config.flush_interval {
            self.flush_internal()
        } else {
            Ok(Bytes::new())
        }
    }

    /// Write raw bytes into the active encoder, tracking unflushed volume.
    ///
    /// Does not itself flush; callers decide when to drain via `flush_internal`.
    fn feed_encoder(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        match &mut self.encoder {
            // `None` never stages input, so it never reaches this path.
            EncoderState::None => {}

            #[cfg(feature = "gzip")]
            EncoderState::Gzip(encoder) => {
                encoder
                    .write_all(data)
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                self.unflushed_bytes += data.len();
            }

            #[cfg(feature = "brotli")]
            EncoderState::Brotli(encoder) => {
                encoder
                    .write_all(data)
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                self.unflushed_bytes += data.len();
            }

            #[cfg(feature = "zstd")]
            EncoderState::Zstd(encoder) => {
                encoder
                    .write_all(data)
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                self.unflushed_bytes += data.len();
            }
        }

        Ok(())
    }

    /// Flush compressed data without finishing.
    ///
    /// Any input still staged below the `min_chunk_size` threshold is drained
    /// into the encoder first so no buffered data is lost.
    pub fn flush(&mut self) -> Result<Bytes> {
        if !self.input_buffer.is_empty() {
            let buffered = std::mem::take(&mut self.input_buffer);
            self.feed_encoder(&buffered)?;
        }
        self.flush_internal()
    }

    fn flush_internal(&mut self) -> Result<Bytes> {
        self.unflushed_bytes = 0;

        match &mut self.encoder {
            EncoderState::None => Ok(Bytes::new()),

            #[cfg(feature = "gzip")]
            EncoderState::Gzip(encoder) => {
                encoder
                    .flush()
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                let inner = encoder.get_mut();
                if inner.is_empty() {
                    return Ok(Bytes::new());
                }
                let output = std::mem::take(inner);
                self.bytes_out += output.len() as u64;
                Ok(Bytes::from(output))
            }

            #[cfg(feature = "brotli")]
            EncoderState::Brotli(encoder) => {
                encoder
                    .flush()
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                let inner = encoder.get_mut();
                if inner.is_empty() {
                    return Ok(Bytes::new());
                }
                let output = std::mem::take(inner);
                self.bytes_out += output.len() as u64;
                Ok(Bytes::from(output))
            }

            #[cfg(feature = "zstd")]
            EncoderState::Zstd(encoder) => {
                encoder
                    .flush()
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                let inner = encoder.get_mut();
                if inner.is_empty() {
                    return Ok(Bytes::new());
                }
                let output = std::mem::take(inner);
                self.bytes_out += output.len() as u64;
                Ok(Bytes::from(output))
            }
        }
    }

    /// Finish compression and return final bytes.
    pub fn finish(mut self) -> Result<Bytes> {
        if self.finished {
            return Ok(Bytes::new());
        }
        self.finished = true;

        // Drain any input still staged below the `min_chunk_size` threshold so
        // the tail of the stream isn't silently dropped.
        if !self.input_buffer.is_empty() {
            let buffered = std::mem::take(&mut self.input_buffer);
            self.feed_encoder(&buffered)?;
        }

        match self.encoder {
            EncoderState::None => Ok(Bytes::new()),

            #[cfg(feature = "gzip")]
            EncoderState::Gzip(encoder) => {
                let output = encoder
                    .finish()
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                self.bytes_out += output.len() as u64;
                Ok(Bytes::from(output))
            }

            #[cfg(feature = "brotli")]
            EncoderState::Brotli(mut encoder) => {
                encoder
                    .flush()
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                let output = encoder.into_inner();
                self.bytes_out += output.len() as u64;
                Ok(Bytes::from(output))
            }

            #[cfg(feature = "zstd")]
            EncoderState::Zstd(encoder) => {
                let output = encoder
                    .finish()
                    .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
                self.bytes_out += output.len() as u64;
                Ok(Bytes::from(output))
            }
        }
    }

    /// Get compression statistics.
    pub fn stats(&self) -> CompressionStats {
        CompressionStats {
            bytes_in: self.bytes_in,
            bytes_out: self.bytes_out,
            ratio: if self.bytes_in > 0 {
                self.bytes_out as f64 / self.bytes_in as f64
            } else {
                1.0
            },
        }
    }
}

/// Compression statistics.
#[derive(Debug, Clone, Copy)]
pub struct CompressionStats {
    /// Total bytes before compression
    pub bytes_in: u64,
    /// Total bytes after compression
    pub bytes_out: u64,
    /// Compression ratio (out/in, lower is better)
    pub ratio: f64,
}

impl CompressionStats {
    /// Get space savings as percentage (0-100).
    pub fn savings_percent(&self) -> f64 {
        if self.bytes_in == 0 {
            return 0.0;
        }
        (1.0 - self.ratio) * 100.0
    }
}

/// Async streaming compressor wrapper.
///
/// Wraps a [`StreamingCompressor`] so it slots into `async` stream pipelines
/// (its `process`/`flush` methods can be `.await`ed at call sites).
///
/// Note: compression here is CPU-bound and fully synchronous — these methods
/// perform no I/O and never actually suspend. They are `async` purely for
/// ergonomic composition with async code, not because buffering happens off
/// the task. Input buffering (the `min_chunk_size` threshold) is handled by
/// the inner [`StreamingCompressor`], so this wrapper holds no state of its
/// own beyond the inner compressor.
pub struct AsyncStreamingCompressor {
    inner: StreamingCompressor,
}

impl AsyncStreamingCompressor {
    /// Create a new async streaming compressor.
    pub fn new(config: StreamingConfig) -> Result<Self> {
        Ok(Self {
            inner: StreamingCompressor::new(config)?,
        })
    }

    /// Get the Content-Encoding header value.
    pub fn encoding(&self) -> Option<&'static str> {
        self.inner.encoding()
    }

    /// Process a chunk and return compressed data.
    ///
    /// Synchronous wrapper (see the type-level note): delegates directly to
    /// [`StreamingCompressor::compress_chunk`].
    pub async fn process(&mut self, chunk: Bytes) -> Result<Bytes> {
        self.inner.compress_chunk(&chunk)
    }

    /// Flush any pending compressed data.
    ///
    /// Synchronous wrapper: delegates to [`StreamingCompressor::flush`].
    pub async fn flush(&mut self) -> Result<Bytes> {
        self.inner.flush()
    }

    /// Finish compression.
    pub fn finish(self) -> Result<Bytes> {
        self.inner.finish()
    }

    /// Get compression statistics.
    pub fn stats(&self) -> CompressionStats {
        self.inner.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_config() {
        let config = StreamingConfig::new()
            .algorithm(CompressionAlgorithm::None)
            .level(6)
            .flush_interval(1024);

        assert_eq!(config.level, 6);
        assert_eq!(config.flush_interval, 1024);
    }

    #[test]
    fn test_passthrough_compressor() {
        let config = StreamingConfig::new().algorithm(CompressionAlgorithm::None);

        let mut compressor = StreamingCompressor::new(config).unwrap();

        let data = b"Hello, World!";
        let compressed = compressor.compress_chunk(data).unwrap();

        assert_eq!(compressed.as_ref(), data);

        let final_chunk = compressor.finish().unwrap();
        assert!(final_chunk.is_empty());
    }

    #[test]
    #[cfg(feature = "gzip")]
    fn test_gzip_streaming() {
        let config = StreamingConfig::new()
            .algorithm(CompressionAlgorithm::Gzip)
            .flush_interval(10); // Flush frequently for test

        let mut compressor = StreamingCompressor::new(config).unwrap();

        let mut total_compressed = BytesMut::new();

        // Send multiple chunks
        for _ in 0..10 {
            let data = b"Hello, World! This is a test chunk.\n";
            let compressed = compressor.compress_chunk(data).unwrap();
            total_compressed.extend_from_slice(&compressed);
        }

        // Get stats before finishing (finish consumes self)
        let stats = compressor.stats();
        assert_eq!(stats.bytes_in, 10 * 36);

        // Finish
        let final_chunk = compressor.finish().unwrap();
        total_compressed.extend_from_slice(&final_chunk);

        // Should be smaller than original
        let original_size = 10 * 36; // 10 chunks * 36 bytes each
        assert!(total_compressed.len() < original_size);
    }

    #[test]
    fn test_compression_stats() {
        let stats = CompressionStats {
            bytes_in: 1000,
            bytes_out: 400,
            ratio: 0.4,
        };

        assert_eq!(stats.savings_percent(), 60.0);
    }

    #[test]
    fn test_low_latency_config() {
        let config = StreamingConfig::low_latency();
        assert_eq!(config.level, 1);
        assert_eq!(config.flush_interval, 256);
    }

    #[test]
    fn test_high_compression_config() {
        let config = StreamingConfig::high_compression();
        assert_eq!(config.level, 9);
        assert!(config.flush_interval > 1024);
    }

    /// Regression: `min_chunk_size` must gate when input reaches the encoder.
    /// A sub-threshold chunk is staged (not compressed) even when
    /// `flush_interval` is tiny. Against the pre-fix code this fails because
    /// the chunk was written to the encoder and flushed immediately.
    #[test]
    #[cfg(feature = "gzip")]
    fn test_min_chunk_size_buffers_small_chunks() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let config = StreamingConfig::new()
            .algorithm(CompressionAlgorithm::Gzip)
            .flush_interval(1) // would force an immediate flush without staging
            .min_chunk_size(1024);
        let mut compressor = StreamingCompressor::new(config).unwrap();

        // Sub-threshold chunk: must be staged, producing no output and
        // reaching the encoder not at all.
        let out = compressor.compress_chunk(b"small chunk").unwrap();
        assert!(
            out.is_empty(),
            "sub-threshold chunk should be buffered, not compressed"
        );
        assert_eq!(
            compressor.stats().bytes_out,
            0,
            "nothing should reach the encoder below min_chunk_size"
        );

        // Crossing the threshold drains the staged bytes and compresses them.
        let big = vec![b'x'; 2048];
        let out2 = compressor.compress_chunk(&big).unwrap();
        assert!(
            !out2.is_empty(),
            "crossing min_chunk_size should emit compressed data"
        );

        // Full round-trip: staged + threshold-crossing data must all survive.
        let mut all = BytesMut::new();
        all.extend_from_slice(&out2);
        all.extend_from_slice(&compressor.finish().unwrap());

        let mut decoder = GzDecoder::new(&all[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();

        let mut expected = b"small chunk".to_vec();
        expected.extend_from_slice(&big);
        assert_eq!(decompressed, expected);
    }

    /// A stream whose entire input stays below `min_chunk_size` must still be
    /// emitted in full by `finish()` (the staged tail is not dropped).
    #[test]
    #[cfg(feature = "gzip")]
    fn test_finish_drains_staged_input() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let config = StreamingConfig::new()
            .algorithm(CompressionAlgorithm::Gzip)
            .min_chunk_size(1_000_000); // nothing ever crosses the threshold
        let mut compressor = StreamingCompressor::new(config).unwrap();

        assert!(
            compressor
                .compress_chunk(b"tiny payload")
                .unwrap()
                .is_empty()
        );
        let final_bytes = compressor.finish().unwrap();
        assert!(!final_bytes.is_empty());

        let mut decoder = GzDecoder::new(&final_bytes[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        assert_eq!(decompressed, b"tiny payload");
    }

    /// The async wrapper is a thin (synchronous) shim over the inner
    /// compressor: process/finish must round-trip correctly.
    #[tokio::test]
    #[cfg(feature = "gzip")]
    async fn test_async_wrapper_round_trip() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let config = StreamingConfig::new()
            .algorithm(CompressionAlgorithm::Gzip)
            .min_chunk_size(4)
            .flush_interval(4);
        let mut compressor = AsyncStreamingCompressor::new(config).unwrap();

        let mut all = BytesMut::new();
        for _ in 0..8 {
            let out = compressor
                .process(Bytes::from_static(b"chunk-"))
                .await
                .unwrap();
            all.extend_from_slice(&out);
        }
        all.extend_from_slice(&compressor.finish().unwrap());

        let mut decoder = GzDecoder::new(&all[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        assert_eq!(decompressed, b"chunk-".repeat(8));
    }
}
