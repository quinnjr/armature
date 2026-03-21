//! Streaming HTTP Responses
//!
//! This module provides support for streaming HTTP responses, enabling efficient
//! delivery of large data sets, real-time data, and chunked transfers.
//!
//! # Features
//!
//! - Chunked transfer encoding
//! - Async stream-based response bodies
//! - JSON array streaming (NDJSON)
//! - Text/line streaming
//! - Binary data streaming
//! - Progress callbacks
//!
//! # Examples
//!
//! ## Basic Streaming
//!
//! ```ignore
//! use armature_core::streaming::{StreamingResponse, ByteStream};
//!
//! async fn stream_data() -> StreamingResponse {
//!     let (stream, sender) = ByteStream::new();
//!
//!     tokio::spawn(async move {
//!         for i in 0..100 {
//!             sender.send(format!("chunk {}\n", i).into_bytes()).await;
//!         }
//!     });
//!
//!     StreamingResponse::new(stream)
//!         .content_type("text/plain")
//! }
//! ```
//!
//! ## JSON Streaming (NDJSON)
//!
//! ```ignore
//! use armature_core::streaming::{StreamingResponse, JsonStream};
//!
//! async fn stream_json() -> StreamingResponse {
//!     let (stream, sender) = JsonStream::new();
//!
//!     tokio::spawn(async move {
//!         for user in load_users() {
//!             sender.send_json(&user).await;
//!         }
//!     });
//!
//!     StreamingResponse::ndjson(stream)
//! }
//! ```

use crate::{Error, HttpResponse};
use bytes::Bytes;
use futures_util::Stream;
use serde::Serialize;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::mpsc;

// ============================================================================
// Streaming Body Types
// ============================================================================

/// A chunk of streaming data.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Raw bytes
    Bytes(Bytes),
    /// End of stream
    End,
    /// Error occurred
    Error(String),
}

impl From<Vec<u8>> for StreamChunk {
    fn from(v: Vec<u8>) -> Self {
        StreamChunk::Bytes(Bytes::from(v))
    }
}

impl From<Bytes> for StreamChunk {
    fn from(b: Bytes) -> Self {
        StreamChunk::Bytes(b)
    }
}

impl From<String> for StreamChunk {
    fn from(s: String) -> Self {
        StreamChunk::Bytes(Bytes::from(s))
    }
}

impl From<&str> for StreamChunk {
    fn from(s: &str) -> Self {
        StreamChunk::Bytes(Bytes::from(s.to_owned()))
    }
}

// ============================================================================
// Byte Stream
// ============================================================================

/// A stream of raw bytes for streaming responses.
///
/// # Example
///
/// ```
/// use armature_core::streaming::ByteStream;
///
/// # tokio_test::block_on(async {
/// let (stream, sender) = ByteStream::new();
///
/// // Send data in background
/// tokio::spawn(async move {
///     sender.send(b"Hello, ".to_vec()).await.ok();
///     sender.send(b"World!".to_vec()).await.ok();
///     sender.close().await;
/// });
/// # });
/// ```
pub struct ByteStream {
    receiver: mpsc::Receiver<StreamChunk>,
}

/// Sender half of a byte stream.
pub struct ByteStreamSender {
    sender: mpsc::Sender<StreamChunk>,
    bytes_sent: Arc<AtomicU64>,
}

impl ByteStream {
    /// Create a new byte stream with default buffer size (64).
    pub fn new() -> (Self, ByteStreamSender) {
        Self::with_buffer_size(64)
    }

    /// Create a new byte stream with custom buffer size.
    pub fn with_buffer_size(size: usize) -> (Self, ByteStreamSender) {
        let (sender, receiver) = mpsc::channel(size);
        let bytes_sent = Arc::new(AtomicU64::new(0));
        (Self { receiver }, ByteStreamSender { sender, bytes_sent })
    }
}

impl Default for ByteStream {
    fn default() -> Self {
        let (stream, _) = Self::new();
        stream
    }
}

impl Stream for ByteStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.receiver).poll_recv(cx) {
            Poll::Ready(Some(chunk)) => match chunk {
                StreamChunk::Bytes(bytes) => Poll::Ready(Some(Ok(bytes))),
                StreamChunk::End => Poll::Ready(None),
                StreamChunk::Error(e) => Poll::Ready(Some(Err(Error::Internal(e)))),
            },
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl ByteStreamSender {
    /// Send bytes to the stream.
    pub async fn send(&self, data: impl Into<Vec<u8>>) -> Result<(), Error> {
        let bytes = data.into();
        let len = bytes.len() as u64;
        self.sender
            .send(StreamChunk::Bytes(Bytes::from(bytes)))
            .await
            .map_err(|e| Error::Internal(format!("Failed to send to stream: {}", e)))?;
        self.bytes_sent.fetch_add(len, Ordering::Relaxed);
        Ok(())
    }

    /// Send bytes from a Bytes object.
    pub async fn send_bytes(&self, bytes: Bytes) -> Result<(), Error> {
        let len = bytes.len() as u64;
        self.sender
            .send(StreamChunk::Bytes(bytes))
            .await
            .map_err(|e| Error::Internal(format!("Failed to send to stream: {}", e)))?;
        self.bytes_sent.fetch_add(len, Ordering::Relaxed);
        Ok(())
    }

    /// Send a string to the stream.
    pub async fn send_str(&self, s: &str) -> Result<(), Error> {
        self.send(s.as_bytes().to_vec()).await
    }

    /// Signal an error to the stream.
    pub async fn send_error(&self, error: impl Into<String>) -> Result<(), Error> {
        self.sender
            .send(StreamChunk::Error(error.into()))
            .await
            .map_err(|e| Error::Internal(format!("Failed to send error: {}", e)))
    }

    /// Close the stream.
    pub async fn close(&self) {
        let _ = self.sender.send(StreamChunk::End).await;
    }

    /// Get the total bytes sent so far.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Check if the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

// ============================================================================
// JSON Stream (NDJSON)
// ============================================================================

/// A stream for sending JSON objects as newline-delimited JSON (NDJSON).
///
/// Each JSON object is serialized and followed by a newline character.
/// This format is compatible with tools like `jq` and is easy to parse.
///
/// # Example
///
/// ```
/// use armature_core::streaming::JsonStream;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct User { id: u64, name: String }
///
/// # tokio_test::block_on(async {
/// let (stream, sender) = JsonStream::new();
///
/// tokio::spawn(async move {
///     sender.send_json(&User { id: 1, name: "Alice".into() }).await.ok();
///     sender.send_json(&User { id: 2, name: "Bob".into() }).await.ok();
///     sender.close().await;
/// });
/// # });
/// ```
pub struct JsonStream {
    inner: ByteStream,
}

/// Sender half of a JSON stream.
pub struct JsonStreamSender {
    inner: ByteStreamSender,
    items_sent: Arc<AtomicU64>,
}

impl JsonStream {
    /// Create a new JSON stream.
    pub fn new() -> (Self, JsonStreamSender) {
        Self::with_buffer_size(64)
    }

    /// Create a new JSON stream with custom buffer size.
    pub fn with_buffer_size(size: usize) -> (Self, JsonStreamSender) {
        let (stream, sender) = ByteStream::with_buffer_size(size);
        let items_sent = Arc::new(AtomicU64::new(0));
        (
            Self { inner: stream },
            JsonStreamSender {
                inner: sender,
                items_sent,
            },
        )
    }

    /// Get the inner byte stream.
    pub fn into_inner(self) -> ByteStream {
        self.inner
    }
}

impl Default for JsonStream {
    fn default() -> Self {
        let (stream, _) = Self::new();
        stream
    }
}

impl Stream for JsonStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl JsonStreamSender {
    /// Send a JSON-serializable value.
    pub async fn send_json<T: Serialize>(&self, value: &T) -> Result<(), Error> {
        let json = serde_json::to_string(value).map_err(|e| Error::Serialization(e.to_string()))?;
        self.inner.send(format!("{}\n", json)).await?;
        self.items_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Send a raw JSON string (must be valid JSON).
    pub async fn send_raw(&self, json: &str) -> Result<(), Error> {
        self.inner.send(format!("{}\n", json.trim())).await?;
        self.items_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Signal an error as a JSON object.
    pub async fn send_error(&self, error: impl Into<String>) -> Result<(), Error> {
        let error_json = serde_json::json!({
            "error": error.into()
        });
        self.send_json(&error_json).await
    }

    /// Close the stream.
    pub async fn close(&self) {
        self.inner.close().await;
    }

    /// Get the total items sent so far.
    pub fn items_sent(&self) -> u64 {
        self.items_sent.load(Ordering::Relaxed)
    }

    /// Check if the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

// ============================================================================
// Text/Line Stream
// ============================================================================

/// A stream for sending text lines.
///
/// Each message is followed by a newline character.
pub struct TextStream {
    inner: ByteStream,
}

/// Sender half of a text stream.
pub struct TextStreamSender {
    inner: ByteStreamSender,
    lines_sent: Arc<AtomicU64>,
}

impl TextStream {
    /// Create a new text stream.
    pub fn new() -> (Self, TextStreamSender) {
        Self::with_buffer_size(64)
    }

    /// Create a new text stream with custom buffer size.
    pub fn with_buffer_size(size: usize) -> (Self, TextStreamSender) {
        let (stream, sender) = ByteStream::with_buffer_size(size);
        let lines_sent = Arc::new(AtomicU64::new(0));
        (
            Self { inner: stream },
            TextStreamSender {
                inner: sender,
                lines_sent,
            },
        )
    }

    /// Get the inner byte stream.
    pub fn into_inner(self) -> ByteStream {
        self.inner
    }
}

impl Default for TextStream {
    fn default() -> Self {
        let (stream, _) = Self::new();
        stream
    }
}

impl Stream for TextStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl TextStreamSender {
    /// Send a line of text (newline is added automatically).
    pub async fn send_line(&self, line: &str) -> Result<(), Error> {
        self.inner.send(format!("{}\n", line)).await?;
        self.lines_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Send raw text (no newline added).
    pub async fn send(&self, text: &str) -> Result<(), Error> {
        self.inner.send(text.as_bytes().to_vec()).await
    }

    /// Close the stream.
    pub async fn close(&self) {
        self.inner.close().await;
    }

    /// Get the total lines sent so far.
    pub fn lines_sent(&self) -> u64 {
        self.lines_sent.load(Ordering::Relaxed)
    }

    /// Check if the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

// ============================================================================
// Streaming Response
// ============================================================================

/// A streaming HTTP response.
///
/// Unlike `HttpResponse` which buffers the entire body, `StreamingResponse`
/// sends data as it becomes available using chunked transfer encoding.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```ignore
/// use armature_core::streaming::{StreamingResponse, ByteStream};
///
/// let (stream, sender) = ByteStream::new();
///
/// // Spawn task to produce data
/// tokio::spawn(async move {
///     for i in 0..10 {
///         sender.send(format!("Line {}\n", i)).await.ok();
///         tokio::time::sleep(Duration::from_millis(100)).await;
///     }
///     sender.close().await;
/// });
///
/// StreamingResponse::new(stream)
///     .status(200)
///     .content_type("text/plain")
/// ```
pub struct StreamingResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// The stream body
    body: StreamBody,
}

/// The body of a streaming response.
pub enum StreamBody {
    /// A byte stream
    Bytes(ByteStream),
    /// A JSON stream
    Json(JsonStream),
    /// A text stream
    Text(TextStream),
    /// An empty body
    Empty,
}

impl StreamingResponse {
    /// Create a new streaming response from a byte stream.
    pub fn new(stream: ByteStream) -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            body: StreamBody::Bytes(stream),
        }
    }

    /// Create a new NDJSON streaming response.
    pub fn ndjson(stream: JsonStream) -> Self {
        let mut response = Self {
            status: 200,
            headers: HashMap::new(),
            body: StreamBody::Json(stream),
        };
        response.headers.insert(
            "Content-Type".to_string(),
            "application/x-ndjson".to_string(),
        );
        response
    }

    /// Create a new text streaming response.
    pub fn text(stream: TextStream) -> Self {
        let mut response = Self {
            status: 200,
            headers: HashMap::new(),
            body: StreamBody::Text(stream),
        };
        response.headers.insert(
            "Content-Type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        );
        response
    }

    /// Create an empty streaming response.
    pub fn empty() -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            body: StreamBody::Empty,
        }
    }

    /// Set the HTTP status code.
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Set the Content-Type header.
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.headers
            .insert("Content-Type".to_string(), content_type.into());
        self
    }

    /// Add a header.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set Cache-Control to no-cache (recommended for streams).
    pub fn no_cache(mut self) -> Self {
        self.headers.insert(
            "Cache-Control".to_string(),
            "no-cache, no-store, must-revalidate".to_string(),
        );
        self
    }

    /// Enable CORS for the response.
    pub fn cors(mut self, origin: impl Into<String>) -> Self {
        self.headers
            .insert("Access-Control-Allow-Origin".to_string(), origin.into());
        self
    }

    /// Set X-Content-Type-Options to nosniff.
    pub fn nosniff(mut self) -> Self {
        self.headers
            .insert("X-Content-Type-Options".to_string(), "nosniff".to_string());
        self
    }

    /// Get the stream body, consuming the response.
    pub fn into_body(self) -> StreamBody {
        self.body
    }

    /// Check if this is an empty response.
    pub fn is_empty(&self) -> bool {
        matches!(self.body, StreamBody::Empty)
    }
}

impl Default for StreamingResponse {
    fn default() -> Self {
        Self::empty()
    }
}

// ============================================================================
// Stream Iterators
// ============================================================================

/// Stream items from an async iterator.
///
/// # Example
///
/// ```ignore
/// use armature_core::streaming::stream_iter;
///
/// let items = vec![1, 2, 3, 4, 5];
/// let (stream, _) = stream_iter(items.into_iter(), |i| format!("{}\n", i));
/// ```
pub fn stream_iter<I, T, F>(iter: I, transform: F) -> (ByteStream, tokio::task::JoinHandle<()>)
where
    I: Iterator<Item = T> + Send + 'static,
    T: Send + 'static,
    F: Fn(T) -> Vec<u8> + Send + 'static,
{
    let (stream, sender) = ByteStream::new();
    let items: Vec<T> = iter.collect(); // Collect to avoid iterator lifetime issues
    let handle = tokio::spawn(async move {
        for item in items {
            if sender.send(transform(item)).await.is_err() {
                break;
            }
        }
        sender.close().await;
    });
    (stream, handle)
}

/// Stream items from an async iterator with delays.
pub fn stream_iter_with_delay<I, T, F>(
    iter: I,
    transform: F,
    delay: Duration,
) -> (ByteStream, tokio::task::JoinHandle<()>)
where
    I: Iterator<Item = T> + Send + 'static,
    T: Send + 'static,
    F: Fn(T) -> Vec<u8> + Send + 'static,
{
    let (stream, sender) = ByteStream::new();
    let items: Vec<T> = iter.collect(); // Collect to avoid iterator lifetime issues
    let handle = tokio::spawn(async move {
        for item in items {
            if sender.send(transform(item)).await.is_err() {
                break;
            }
            tokio::time::sleep(delay).await;
        }
        sender.close().await;
    });
    (stream, handle)
}

/// Stream JSON items from an iterator.
pub fn stream_json_iter<I, T>(iter: I) -> (JsonStream, tokio::task::JoinHandle<()>)
where
    I: Iterator<Item = T> + Send + 'static,
    T: Serialize + Send + Sync + 'static,
{
    let (stream, sender) = JsonStream::new();
    let items: Vec<T> = iter.collect(); // Collect to avoid iterator lifetime issues
    let handle = tokio::spawn(async move {
        for item in items {
            if sender.send_json(&item).await.is_err() {
                break;
            }
        }
        sender.close().await;
    });
    (stream, handle)
}

// ============================================================================
// Stream from Reader
// ============================================================================

/// Stream data from an async reader (e.g., file, network).
///
/// # Example
///
/// ```ignore
/// use tokio::fs::File;
/// use armature_core::streaming::stream_reader;
///
/// let file = File::open("large_file.bin").await?;
/// let (stream, _) = stream_reader(file, 8192);  // 8KB chunks
/// ```
pub fn stream_reader<R>(reader: R, chunk_size: usize) -> (ByteStream, tokio::task::JoinHandle<()>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;

    let (stream, sender) = ByteStream::new();
    let handle = tokio::spawn(async move {
        let mut reader = reader;
        let mut buffer = vec![0u8; chunk_size];

        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if sender.send(buffer[..n].to_vec()).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = sender.send_error(e.to_string()).await;
                    break;
                }
            }
        }
        sender.close().await;
    });
    (stream, handle)
}

// ============================================================================
// Progress Tracking
// ============================================================================

/// A wrapper that tracks progress of a stream.
pub struct ProgressStream {
    inner: ByteStream,
    bytes_received: Arc<AtomicU64>,
    callback: Option<Box<dyn Fn(u64) + Send + Sync>>,
}

impl ProgressStream {
    /// Create a new progress tracking stream.
    pub fn new(inner: ByteStream) -> Self {
        Self {
            inner,
            bytes_received: Arc::new(AtomicU64::new(0)),
            callback: None,
        }
    }

    /// Set a callback to be called on each chunk received.
    pub fn on_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(u64) + Send + Sync + 'static,
    {
        self.callback = Some(Box::new(callback));
        self
    }

    /// Get the total bytes received so far.
    pub fn bytes_received(&self) -> u64 {
        self.bytes_received.load(Ordering::Relaxed)
    }
}

impl Stream for ProgressStream {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                let len = bytes.len() as u64;
                let total = self.bytes_received.fetch_add(len, Ordering::Relaxed) + len;
                if let Some(ref callback) = self.callback {
                    callback(total);
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            other => other,
        }
    }
}

// ============================================================================
// Conversion to HttpResponse
// ============================================================================

impl StreamingResponse {
    /// Collect the entire stream into an HttpResponse.
    ///
    /// This buffers the entire response body, defeating the purpose of streaming.
    /// Only use when you need to convert to a buffered response.
    pub async fn into_buffered(mut self) -> Result<HttpResponse, Error> {
        use futures_util::StreamExt;

        let mut body = Vec::new();

        match &mut self.body {
            StreamBody::Bytes(stream) => {
                while let Some(chunk) = stream.next().await {
                    body.extend_from_slice(&chunk?);
                }
            }
            StreamBody::Json(stream) => {
                while let Some(chunk) = stream.next().await {
                    body.extend_from_slice(&chunk?);
                }
            }
            StreamBody::Text(stream) => {
                while let Some(chunk) = stream.next().await {
                    body.extend_from_slice(&chunk?);
                }
            }
            StreamBody::Empty => {}
        }

        let mut response = HttpResponse::new(self.status);
        response.headers = self.headers.into();
        response.body = body;
        Ok(response)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn test_byte_stream() {
        let (mut stream, sender) = ByteStream::new();

        tokio::spawn(async move {
            sender.send(b"hello".to_vec()).await.unwrap();
            sender.send(b" world".to_vec()).await.unwrap();
            sender.close().await;
        });

        let mut result = Vec::new();
        while let Some(chunk) = stream.next().await {
            result.extend_from_slice(&chunk.unwrap());
        }

        assert_eq!(result, b"hello world");
    }

    #[tokio::test]
    async fn test_json_stream() {
        let (mut stream, sender) = JsonStream::new();

        #[derive(Serialize)]
        struct Item {
            id: u64,
        }

        tokio::spawn(async move {
            sender.send_json(&Item { id: 1 }).await.unwrap();
            sender.send_json(&Item { id: 2 }).await.unwrap();
            sender.close().await;
        });

        let mut result = Vec::new();
        while let Some(chunk) = stream.next().await {
            result.extend_from_slice(&chunk.unwrap());
        }

        let result_str = String::from_utf8(result).unwrap();
        assert!(result_str.contains("{\"id\":1}"));
        assert!(result_str.contains("{\"id\":2}"));
    }

    #[tokio::test]
    async fn test_text_stream() {
        let (mut stream, sender) = TextStream::new();

        tokio::spawn(async move {
            sender.send_line("line 1").await.unwrap();
            sender.send_line("line 2").await.unwrap();
            sender.close().await;
        });

        let mut result = Vec::new();
        while let Some(chunk) = stream.next().await {
            result.extend_from_slice(&chunk.unwrap());
        }

        let result_str = String::from_utf8(result).unwrap();
        assert_eq!(result_str, "line 1\nline 2\n");
    }

    #[tokio::test]
    async fn test_streaming_response() {
        let (stream, sender) = ByteStream::new();

        tokio::spawn(async move {
            sender.send(b"test data".to_vec()).await.unwrap();
            sender.close().await;
        });

        let response = StreamingResponse::new(stream)
            .status(200)
            .content_type("text/plain")
            .no_cache();

        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&"text/plain".to_string())
        );
    }

    #[tokio::test]
    async fn test_stream_iter() {
        let items = vec![1, 2, 3];
        let (mut stream, _) = stream_iter(items.into_iter(), |i| format!("{}", i).into_bytes());

        let mut result = Vec::new();
        while let Some(chunk) = stream.next().await {
            result.extend_from_slice(&chunk.unwrap());
        }

        assert_eq!(String::from_utf8(result).unwrap(), "123");
    }

    #[tokio::test]
    async fn test_bytes_sent_tracking() {
        let (stream, sender) = ByteStream::new();

        sender.send(b"hello".to_vec()).await.unwrap();
        assert_eq!(sender.bytes_sent(), 5);

        sender.send(b" world".to_vec()).await.unwrap();
        assert_eq!(sender.bytes_sent(), 11);

        // Keep stream alive until we're done
        drop(stream);
    }

    #[tokio::test]
    async fn test_json_items_sent_tracking() {
        let (stream, sender) = JsonStream::new();

        #[derive(Serialize)]
        struct Item {
            id: u64,
        }

        sender.send_json(&Item { id: 1 }).await.unwrap();
        assert_eq!(sender.items_sent(), 1);

        sender.send_json(&Item { id: 2 }).await.unwrap();
        assert_eq!(sender.items_sent(), 2);

        // Keep stream alive until we're done
        drop(stream);
    }

    #[tokio::test]
    async fn test_streaming_response_into_buffered() {
        let (stream, sender) = ByteStream::new();

        tokio::spawn(async move {
            sender.send(b"buffered".to_vec()).await.unwrap();
            sender.close().await;
        });

        let response = StreamingResponse::new(stream)
            .status(200)
            .content_type("text/plain");

        let buffered = response.into_buffered().await.unwrap();
        assert_eq!(buffered.status, 200);
        assert_eq!(buffered.body, b"buffered");
    }

    #[test]
    fn test_stream_chunk_from() {
        let from_vec: StreamChunk = vec![1, 2, 3].into();
        assert!(matches!(from_vec, StreamChunk::Bytes(_)));

        let from_string: StreamChunk = "hello".to_string().into();
        assert!(matches!(from_string, StreamChunk::Bytes(_)));

        let from_str: StreamChunk = "world".into();
        assert!(matches!(from_str, StreamChunk::Bytes(_)));
    }

    // Advanced streaming tests

    #[test]
    fn test_backpressure_config() {
        let config = BackpressureConfig::new()
            .high_watermark(100)
            .low_watermark(20)
            .strategy(BackpressureStrategy::PauseResume);

        assert_eq!(config.high_watermark, 100);
        assert_eq!(config.low_watermark, 20);
    }

    #[test]
    fn test_chunk_optimizer_default() {
        let optimizer = ChunkOptimizer::default();
        assert_eq!(optimizer.min_chunk, DEFAULT_MIN_CHUNK);
        assert_eq!(optimizer.max_chunk, DEFAULT_MAX_CHUNK);
    }

    #[test]
    fn test_chunk_optimizer_sizing() {
        let optimizer = ChunkOptimizer::new(512, 8192);

        assert_eq!(optimizer.optimal_chunk_size(100), 512); // Below min
        assert_eq!(optimizer.optimal_chunk_size(1000), 1000); // In range
        assert_eq!(optimizer.optimal_chunk_size(10000), 8192); // Above max
    }

    #[test]
    fn test_streaming_stats() {
        let stats = streaming_stats();
        let _ = stats.streams_created();
        let _ = stats.chunks_sent();
        let _ = stats.bytes_sent();
    }

    #[tokio::test]
    async fn test_streaming_body_builder() {
        let (body, handle) = StreamingBodyBuilder::new()
            .chunk_size(1024)
            .build_with_sender();

        tokio::spawn(async move {
            handle.send(b"test data".to_vec()).await.ok();
            handle.close().await;
        });

        let mut total = 0;
        let mut body = body;
        while let Some(chunk) = body.next().await {
            total += chunk.unwrap().len();
        }
        assert_eq!(total, 9);
    }

    #[test]
    fn test_rate_limiter() {
        let limiter = StreamRateLimiter::new(1024); // 1KB/s
        assert_eq!(limiter.bytes_per_sec, 1024);
    }

    // Advanced chunk optimization tests

    #[test]
    fn test_chunk_content_type_detection() {
        assert_eq!(
            ChunkContentType::from_mime("application/json"),
            ChunkContentType::Json
        );
        assert_eq!(
            ChunkContentType::from_mime("text/html"),
            ChunkContentType::Html
        );
        assert_eq!(
            ChunkContentType::from_mime("text/event-stream"),
            ChunkContentType::RealTime
        );
        assert_eq!(
            ChunkContentType::from_mime("video/mp4"),
            ChunkContentType::Media
        );
        assert_eq!(
            ChunkContentType::from_mime("application/octet-stream"),
            ChunkContentType::Binary
        );
    }

    #[test]
    fn test_chunk_content_type_recommendations() {
        let realtime = ChunkContentType::RealTime;
        assert!(realtime.recommended_chunk_size() < CHUNK_SMALL);

        let media = ChunkContentType::Media;
        assert!(media.recommended_chunk_size() >= CHUNK_TCP_OPTIMAL);

        let binary = ChunkContentType::Binary;
        assert!(binary.recommended_chunk_size() >= CHUNK_LARGE);
    }

    #[test]
    fn test_network_condition_from_rtt() {
        assert_eq!(
            NetworkCondition::from_rtt_ms(5),
            NetworkCondition::Excellent
        );
        assert_eq!(NetworkCondition::from_rtt_ms(30), NetworkCondition::Good);
        assert_eq!(NetworkCondition::from_rtt_ms(80), NetworkCondition::Fair);
        assert_eq!(NetworkCondition::from_rtt_ms(300), NetworkCondition::Poor);
        assert_eq!(
            NetworkCondition::from_rtt_ms(1000),
            NetworkCondition::Terrible
        );
    }

    #[test]
    fn test_network_condition_multipliers() {
        assert!(NetworkCondition::Excellent.chunk_multiplier() > 1.0);
        assert!((NetworkCondition::Good.chunk_multiplier() - 1.0).abs() < 0.01);
        assert!(NetworkCondition::Poor.chunk_multiplier() < 1.0);
    }

    #[test]
    fn test_adaptive_chunk_optimizer() {
        let optimizer = AdaptiveChunkOptimizer::new(ChunkContentType::Json);

        // Default conditions
        let size = optimizer.optimal_size();
        assert!(size >= optimizer.min_chunk);
        assert!(size <= optimizer.max_chunk);
    }

    #[test]
    fn test_adaptive_optimizer_rtt_adaptation() {
        let optimizer = AdaptiveChunkOptimizer::new(ChunkContentType::Binary);

        // Record poor network conditions
        for _ in 0..5 {
            optimizer.record_rtt(300);
        }

        let poor_size = optimizer.optimal_size();

        // Record excellent conditions
        for _ in 0..10 {
            optimizer.record_rtt(5);
        }

        let good_size = optimizer.optimal_size();

        // Good conditions should allow larger chunks
        assert!(good_size >= poor_size);
    }

    #[test]
    fn test_chunked_encoding_optimizer() {
        let optimizer = ChunkedEncodingOptimizer::new();

        // Small data - single chunk
        let plan = optimizer.optimal_for_data(500);
        assert_eq!(plan.num_chunks, 1);
        assert_eq!(plan.chunk_size, 500);

        // Data larger than max_chunk - multiple chunks
        let large_data = optimizer.max_chunk * 3;
        let plan = optimizer.optimal_for_data(large_data);
        assert!(plan.num_chunks > 1);
        assert!(plan.efficiency > 0.99);
    }

    #[test]
    fn test_chunked_encoding_efficiency() {
        // Small chunks have lower efficiency
        let small_eff = ChunkedEncodingOptimizer::chunk_efficiency(100);
        let large_eff = ChunkedEncodingOptimizer::chunk_efficiency(16384);

        assert!(large_eff > small_eff);
        assert!(large_eff > 0.99); // Large chunks very efficient
    }

    #[test]
    fn test_chunked_encoding_create_chunks() {
        let optimizer = ChunkedEncodingOptimizer::new().target_chunk(100);

        let data = vec![0u8; 350];
        let chunks = optimizer.create_chunks(&data);

        // Should create multiple chunks
        assert!(chunks.len() >= 3);

        // Total size should match
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 350);
    }

    #[test]
    fn test_chunk_stats() {
        let stats = chunk_stats();
        let _ = stats.chunks_created();
        let _ = stats.bytes_chunked();
        let _ = stats.average_chunk_size();
        let _ = stats.average_rtt();
    }
}

// ============================================================================
// Advanced Streaming Features
// ============================================================================

// Default chunk sizes
/// Minimum chunk size (4KB)
pub const DEFAULT_MIN_CHUNK: usize = 4 * 1024;
/// Default chunk size (16KB)
pub const DEFAULT_CHUNK_SIZE: usize = 16 * 1024;
/// Maximum chunk size (64KB)
pub const DEFAULT_MAX_CHUNK: usize = 64 * 1024;

// ============================================================================
// Backpressure Handling
// ============================================================================

/// Strategy for handling backpressure when consumer is slow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressureStrategy {
    /// Pause production when buffer is full (default)
    #[default]
    PauseResume,
    /// Drop oldest chunks when buffer is full
    DropOldest,
    /// Drop newest chunks when buffer is full
    DropNewest,
    /// Block producer until space is available
    Block,
    /// Error when buffer is full
    Error,
}

/// Configuration for backpressure handling.
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    /// High watermark - pause when buffer exceeds this
    pub high_watermark: usize,
    /// Low watermark - resume when buffer drops below this
    pub low_watermark: usize,
    /// Backpressure strategy
    pub strategy: BackpressureStrategy,
    /// Maximum buffer size (for DropOldest/DropNewest)
    pub max_buffer: usize,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            high_watermark: 64,
            low_watermark: 16,
            strategy: BackpressureStrategy::PauseResume,
            max_buffer: 256,
        }
    }
}

impl BackpressureConfig {
    /// Create new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set high watermark.
    pub fn high_watermark(mut self, watermark: usize) -> Self {
        self.high_watermark = watermark;
        self
    }

    /// Set low watermark.
    pub fn low_watermark(mut self, watermark: usize) -> Self {
        self.low_watermark = watermark;
        self
    }

    /// Set backpressure strategy.
    pub fn strategy(mut self, strategy: BackpressureStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Set maximum buffer size.
    pub fn max_buffer(mut self, size: usize) -> Self {
        self.max_buffer = size;
        self
    }
}

/// Backpressure controller for flow control with slow clients.
///
/// Manages the flow of data to slow consumers by tracking buffer levels
/// and pausing/resuming production based on watermarks.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::streaming::{BackpressureController, BackpressureConfig};
///
/// let config = BackpressureConfig::new()
///     .high_watermark(100)
///     .low_watermark(20);
///
/// let mut controller = BackpressureController::new(config);
///
/// // Producer loop
/// loop {
///     // Wait if backpressure is applied
///     controller.wait_if_paused().await;
///
///     // Check if we can send
///     if controller.can_send() {
///         // Send data...
///         controller.record_send(chunk_size);
///     }
/// }
///
/// // Consumer acknowledged data
/// controller.record_ack(bytes_consumed);
/// ```
#[derive(Debug)]
pub struct BackpressureController {
    config: BackpressureConfig,
    /// Current buffer level (bytes pending)
    buffer_level: AtomicUsize,
    /// Whether production is paused
    is_paused: AtomicBool,
    /// Notification for resume
    resume_notify: Arc<tokio::sync::Notify>,
    /// Statistics
    stats: BackpressureStats,
}

/// Statistics for backpressure monitoring.
#[derive(Debug, Default)]
pub struct BackpressureStats {
    /// Total bytes sent
    pub bytes_sent: AtomicU64,
    /// Total bytes acknowledged
    pub bytes_acked: AtomicU64,
    /// Number of times paused
    pub pause_count: AtomicU64,
    /// Number of times resumed
    pub resume_count: AtomicU64,
    /// Number of dropped chunks (if using drop strategy)
    pub dropped_chunks: AtomicU64,
    /// Number of dropped bytes
    pub dropped_bytes: AtomicU64,
}

impl BackpressureController {
    /// Create a new backpressure controller.
    pub fn new(config: BackpressureConfig) -> Self {
        Self {
            config,
            buffer_level: AtomicUsize::new(0),
            is_paused: AtomicBool::new(false),
            resume_notify: Arc::new(tokio::sync::Notify::new()),
            stats: BackpressureStats::default(),
        }
    }

    /// Create with default configuration.
    pub fn default_controller() -> Self {
        Self::new(BackpressureConfig::default())
    }

    /// Check if data can be sent without blocking.
    #[inline]
    pub fn can_send(&self) -> bool {
        !self.is_paused.load(Ordering::Acquire)
    }

    /// Check if currently paused.
    #[inline]
    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::Acquire)
    }

    /// Get current buffer level.
    #[inline]
    pub fn buffer_level(&self) -> usize {
        self.buffer_level.load(Ordering::Acquire)
    }

    /// Get buffer utilization (0.0 - 1.0+).
    pub fn buffer_utilization(&self) -> f64 {
        let level = self.buffer_level() as f64;
        let max = self.config.max_buffer as f64;
        level / max
    }

    /// Record data being sent (increases buffer level).
    pub fn record_send(&self, bytes: usize) {
        let new_level = self.buffer_level.fetch_add(bytes, Ordering::AcqRel) + bytes;
        self.stats
            .bytes_sent
            .fetch_add(bytes as u64, Ordering::Relaxed);

        // Check if we should pause
        if new_level >= self.config.high_watermark && !self.is_paused.swap(true, Ordering::AcqRel) {
            self.stats.pause_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record data being acknowledged by consumer (decreases buffer level).
    pub fn record_ack(&self, bytes: usize) {
        let old_level = self
            .buffer_level
            .fetch_sub(bytes.min(self.buffer_level()), Ordering::AcqRel);
        let new_level = old_level.saturating_sub(bytes);
        self.stats
            .bytes_acked
            .fetch_add(bytes as u64, Ordering::Relaxed);

        // Check if we should resume
        if new_level <= self.config.low_watermark && self.is_paused.swap(false, Ordering::AcqRel) {
            self.stats.resume_count.fetch_add(1, Ordering::Relaxed);
            self.resume_notify.notify_waiters();
        }
    }

    /// Wait until not paused (for async producers).
    pub async fn wait_if_paused(&self) {
        while self.is_paused() {
            self.resume_notify.notified().await;
        }
    }

    /// Try to send data, handling backpressure according to strategy.
    ///
    /// Returns:
    /// - `Ok(true)` if data was accepted
    /// - `Ok(false)` if data was dropped (drop strategies)
    /// - `Err` if strategy is Error and buffer is full
    pub fn try_send(&self, bytes: usize) -> Result<bool, BackpressureError> {
        let current = self.buffer_level();

        match self.config.strategy {
            BackpressureStrategy::PauseResume => {
                if current < self.config.max_buffer {
                    self.record_send(bytes);
                    Ok(true)
                } else {
                    // Will be unblocked when consumer catches up
                    Ok(false)
                }
            }
            BackpressureStrategy::Block => {
                // Always accept, let wait_if_paused handle blocking
                self.record_send(bytes);
                Ok(true)
            }
            BackpressureStrategy::DropOldest | BackpressureStrategy::DropNewest => {
                if current + bytes > self.config.max_buffer {
                    self.stats.dropped_chunks.fetch_add(1, Ordering::Relaxed);
                    self.stats
                        .dropped_bytes
                        .fetch_add(bytes as u64, Ordering::Relaxed);
                    Ok(false)
                } else {
                    self.record_send(bytes);
                    Ok(true)
                }
            }
            BackpressureStrategy::Error => {
                if current + bytes > self.config.max_buffer {
                    Err(BackpressureError::BufferFull {
                        current,
                        max: self.config.max_buffer,
                    })
                } else {
                    self.record_send(bytes);
                    Ok(true)
                }
            }
        }
    }

    /// Reset the controller state.
    pub fn reset(&self) {
        self.buffer_level.store(0, Ordering::Release);
        self.is_paused.store(false, Ordering::Release);
        self.resume_notify.notify_waiters();
    }

    /// Get statistics.
    pub fn stats(&self) -> &BackpressureStats {
        &self.stats
    }

    /// Get a snapshot of current state.
    pub fn snapshot(&self) -> BackpressureSnapshot {
        BackpressureSnapshot {
            buffer_level: self.buffer_level(),
            is_paused: self.is_paused(),
            utilization: self.buffer_utilization(),
            bytes_sent: self.stats.bytes_sent.load(Ordering::Relaxed),
            bytes_acked: self.stats.bytes_acked.load(Ordering::Relaxed),
            pause_count: self.stats.pause_count.load(Ordering::Relaxed),
            dropped_chunks: self.stats.dropped_chunks.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of backpressure state.
#[derive(Debug, Clone)]
pub struct BackpressureSnapshot {
    pub buffer_level: usize,
    pub is_paused: bool,
    pub utilization: f64,
    pub bytes_sent: u64,
    pub bytes_acked: u64,
    pub pause_count: u64,
    pub dropped_chunks: u64,
}

/// Error when backpressure buffer is full.
#[derive(Debug, Clone)]
pub enum BackpressureError {
    BufferFull { current: usize, max: usize },
}

impl std::fmt::Display for BackpressureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferFull { current, max } => {
                write!(f, "Backpressure buffer full: {} / {} bytes", current, max)
            }
        }
    }
}

impl std::error::Error for BackpressureError {}

// ============================================================================
// Chunk Optimization
// ============================================================================

/// Optimizes chunk sizes for efficient streaming.
#[derive(Debug, Clone)]
pub struct ChunkOptimizer {
    /// Minimum chunk size
    pub min_chunk: usize,
    /// Maximum chunk size
    pub max_chunk: usize,
    /// Target latency in milliseconds
    pub target_latency_ms: u64,
    /// Observed throughput (bytes/sec)
    throughput: Arc<AtomicU64>,
    /// Chunk count
    chunk_count: Arc<AtomicU64>,
}

impl ChunkOptimizer {
    /// Create a new chunk optimizer.
    pub fn new(min_chunk: usize, max_chunk: usize) -> Self {
        Self {
            min_chunk,
            max_chunk,
            target_latency_ms: 50, // 50ms default
            throughput: Arc::new(AtomicU64::new(0)),
            chunk_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create with target latency.
    pub fn with_target_latency(mut self, ms: u64) -> Self {
        self.target_latency_ms = ms;
        self
    }

    /// Calculate optimal chunk size based on available data.
    #[inline]
    pub fn optimal_chunk_size(&self, available: usize) -> usize {
        available.clamp(self.min_chunk, self.max_chunk)
    }

    /// Record a chunk being sent for throughput tracking.
    pub fn record_chunk(&self, size: usize) {
        self.throughput.fetch_add(size as u64, Ordering::Relaxed);
        self.chunk_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total bytes sent.
    pub fn total_bytes(&self) -> u64 {
        self.throughput.load(Ordering::Relaxed)
    }

    /// Get total chunks sent.
    pub fn total_chunks(&self) -> u64 {
        self.chunk_count.load(Ordering::Relaxed)
    }

    /// Get average chunk size.
    pub fn average_chunk_size(&self) -> usize {
        self.total_bytes()
            .checked_div(self.total_chunks())
            .map(|v| v as usize)
            .unwrap_or(self.min_chunk)
    }
}

impl Default for ChunkOptimizer {
    fn default() -> Self {
        Self::new(DEFAULT_MIN_CHUNK, DEFAULT_MAX_CHUNK)
    }
}

// ============================================================================
// Advanced Chunk Size Optimization
// ============================================================================

/// Chunk size presets for different content types.
pub const CHUNK_TINY: usize = 512;
/// Small chunk for real-time data (1KB)
pub const CHUNK_SMALL: usize = 1024;
/// Medium chunk for mixed content (8KB)
pub const CHUNK_MEDIUM: usize = 8 * 1024;
/// Large chunk for bulk transfers (32KB)
pub const CHUNK_LARGE: usize = 32 * 1024;
/// Extra large chunk for static files (128KB)
pub const CHUNK_XLARGE: usize = 128 * 1024;
/// Optimal for TCP window (64KB - typical MSS multiple)
pub const CHUNK_TCP_OPTIMAL: usize = 64 * 1024;

/// Content type categories for chunk optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkContentType {
    /// Real-time event streams (SSE, WebSocket-like)
    RealTime,
    /// JSON data
    Json,
    /// HTML content
    Html,
    /// Plain text
    Text,
    /// Binary data (images, files)
    Binary,
    /// Streaming media (video, audio)
    Media,
    /// Unknown/generic
    Unknown,
}

impl ChunkContentType {
    /// Detect content type from MIME type string.
    pub fn from_mime(mime: &str) -> Self {
        let mime_lower = mime.to_lowercase();
        if mime_lower.contains("text/event-stream") || mime_lower.contains("x-ndjson") {
            Self::RealTime
        } else if mime_lower.contains("json") {
            Self::Json
        } else if mime_lower.contains("html") {
            Self::Html
        } else if mime_lower.contains("text/") {
            Self::Text
        } else if mime_lower.contains("application/octet-stream")
            || mime_lower.contains("image/")
            || mime_lower.contains("font/")
        {
            Self::Binary
        } else if mime_lower.contains("video/") || mime_lower.contains("audio/") {
            Self::Media
        } else {
            Self::Unknown
        }
    }

    /// Get recommended chunk size for this content type.
    pub fn recommended_chunk_size(&self) -> usize {
        match self {
            Self::RealTime => CHUNK_TINY,        // 512B - minimize latency
            Self::Json => CHUNK_MEDIUM,          // 8KB - balance latency/throughput
            Self::Html => CHUNK_MEDIUM,          // 8KB - good for progressive rendering
            Self::Text => CHUNK_SMALL,           // 1KB - line-oriented
            Self::Binary => CHUNK_LARGE,         // 32KB - maximize throughput
            Self::Media => CHUNK_TCP_OPTIMAL,    // 64KB - optimal for streaming
            Self::Unknown => DEFAULT_CHUNK_SIZE, // 16KB - safe default
        }
    }

    /// Get minimum chunk size for this content type.
    pub fn min_chunk_size(&self) -> usize {
        match self {
            Self::RealTime => 64,         // Can send very small updates
            Self::Json => CHUNK_SMALL,    // At least one object
            Self::Html => CHUNK_SMALL,    // At least one tag
            Self::Text => 128,            // At least one line
            Self::Binary => CHUNK_MEDIUM, // Worth the overhead
            Self::Media => CHUNK_MEDIUM,  // Minimize fragmentation
            Self::Unknown => CHUNK_SMALL, // Conservative
        }
    }

    /// Get maximum chunk size for this content type.
    pub fn max_chunk_size(&self) -> usize {
        match self {
            Self::RealTime => CHUNK_SMALL, // Keep latency low
            Self::Json => CHUNK_LARGE,     // Single objects can be large
            Self::Html => CHUNK_LARGE,     // Full pages
            Self::Text => CHUNK_MEDIUM,    // Not too large
            Self::Binary => CHUNK_XLARGE,  // Large files
            Self::Media => CHUNK_XLARGE,   // Video frames
            Self::Unknown => CHUNK_LARGE,  // Safe default
        }
    }
}

/// Network condition estimate for adaptive chunking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkCondition {
    /// Excellent (< 10ms RTT, > 100 Mbps)
    Excellent,
    /// Good (< 50ms RTT, > 10 Mbps)
    Good,
    /// Fair (< 100ms RTT, > 1 Mbps)
    Fair,
    /// Poor (< 500ms RTT, > 100 Kbps)
    Poor,
    /// Terrible (> 500ms RTT)
    Terrible,
    /// Unknown conditions
    Unknown,
}

impl NetworkCondition {
    /// Estimate condition from RTT in milliseconds.
    pub fn from_rtt_ms(rtt_ms: u64) -> Self {
        match rtt_ms {
            0..=10 => Self::Excellent,
            11..=50 => Self::Good,
            51..=100 => Self::Fair,
            101..=500 => Self::Poor,
            _ => Self::Terrible,
        }
    }

    /// Estimate condition from throughput in bytes/sec.
    pub fn from_throughput(bytes_per_sec: u64) -> Self {
        match bytes_per_sec {
            x if x > 12_500_000 => Self::Excellent, // > 100 Mbps
            x if x > 1_250_000 => Self::Good,       // > 10 Mbps
            x if x > 125_000 => Self::Fair,         // > 1 Mbps
            x if x > 12_500 => Self::Poor,          // > 100 Kbps
            _ => Self::Terrible,
        }
    }

    /// Get recommended chunk size multiplier.
    pub fn chunk_multiplier(&self) -> f32 {
        match self {
            Self::Excellent => 2.0, // Larger chunks, fewer round trips
            Self::Good => 1.0,      // Default sizes
            Self::Fair => 0.75,     // Slightly smaller
            Self::Poor => 0.5,      // Smaller chunks, faster feedback
            Self::Terrible => 0.25, // Very small, prevent timeouts
            Self::Unknown => 1.0,   // Default
        }
    }
}

/// Advanced chunk size optimizer with adaptive sizing.
#[derive(Debug)]
pub struct AdaptiveChunkOptimizer {
    /// Content type for optimization
    content_type: ChunkContentType,
    /// Current network condition estimate
    network_condition: std::sync::atomic::AtomicU8,
    /// Base chunk size
    base_chunk: usize,
    /// Minimum allowed chunk
    min_chunk: usize,
    /// Maximum allowed chunk
    max_chunk: usize,
    /// RTT samples (circular buffer of last 16)
    rtt_samples: std::sync::Mutex<RttTracker>,
    /// Throughput tracker
    throughput_tracker: ThroughputTracker,
    /// Bytes sent
    bytes_sent: AtomicU64,
    /// Chunks sent
    chunks_sent: AtomicU64,
}

impl AdaptiveChunkOptimizer {
    /// Create a new adaptive optimizer.
    pub fn new(content_type: ChunkContentType) -> Self {
        Self {
            min_chunk: content_type.min_chunk_size(),
            max_chunk: content_type.max_chunk_size(),
            base_chunk: content_type.recommended_chunk_size(),
            content_type,
            network_condition: std::sync::atomic::AtomicU8::new(NetworkCondition::Unknown as u8),
            rtt_samples: std::sync::Mutex::new(RttTracker::new()),
            throughput_tracker: ThroughputTracker::new(),
            bytes_sent: AtomicU64::new(0),
            chunks_sent: AtomicU64::new(0),
        }
    }

    /// Create from MIME type.
    pub fn from_mime(mime: &str) -> Self {
        Self::new(ChunkContentType::from_mime(mime))
    }

    /// Create with custom bounds.
    pub fn with_bounds(mut self, min: usize, max: usize) -> Self {
        self.min_chunk = min;
        self.max_chunk = max;
        self
    }

    /// Set base chunk size.
    pub fn with_base_chunk(mut self, base: usize) -> Self {
        self.base_chunk = base;
        self
    }

    /// Calculate optimal chunk size.
    #[inline]
    pub fn optimal_size(&self) -> usize {
        let condition = self.current_condition();
        let multiplier = condition.chunk_multiplier();
        let optimal = (self.base_chunk as f32 * multiplier) as usize;
        optimal.clamp(self.min_chunk, self.max_chunk)
    }

    /// Calculate optimal chunk size for given data.
    #[inline]
    pub fn optimal_for_data(&self, data_len: usize) -> usize {
        let optimal = self.optimal_size();
        // Don't create tiny final chunks
        if data_len <= optimal * 3 / 2 {
            data_len // Send all at once
        } else {
            optimal
        }
    }

    /// Record RTT sample for adaptive sizing.
    pub fn record_rtt(&self, rtt_ms: u64) {
        let mut tracker = self.rtt_samples.lock().unwrap();
        tracker.add_sample(rtt_ms);
        let avg_rtt = tracker.average();
        let condition = NetworkCondition::from_rtt_ms(avg_rtt);
        self.network_condition
            .store(condition as u8, Ordering::Relaxed);
        CHUNK_STATS.record_rtt_sample(rtt_ms);
    }

    /// Record throughput sample.
    pub fn record_throughput(&self, bytes: usize, duration_ms: u64) {
        let Some(bytes_per_sec) = (bytes as u64 * 1000).checked_div(duration_ms) else {
            return;
        };
        self.throughput_tracker.record(bytes_per_sec);
        // Update condition based on throughput too
        let throughput_condition = NetworkCondition::from_throughput(bytes_per_sec);
        let rtt_condition = self.current_condition();
        // Use worse of the two estimates
        let combined = if (throughput_condition as u8) > (rtt_condition as u8) {
            throughput_condition
        } else {
            rtt_condition
        };
        self.network_condition
            .store(combined as u8, Ordering::Relaxed);
    }

    /// Record a chunk being sent.
    pub fn record_chunk(&self, size: usize) {
        self.bytes_sent.fetch_add(size as u64, Ordering::Relaxed);
        self.chunks_sent.fetch_add(1, Ordering::Relaxed);
        CHUNK_STATS.record_chunk(size);
    }

    /// Get current network condition estimate.
    pub fn current_condition(&self) -> NetworkCondition {
        let val = self.network_condition.load(Ordering::Relaxed);
        match val {
            0 => NetworkCondition::Excellent,
            1 => NetworkCondition::Good,
            2 => NetworkCondition::Fair,
            3 => NetworkCondition::Poor,
            4 => NetworkCondition::Terrible,
            _ => NetworkCondition::Unknown,
        }
    }

    /// Get average RTT.
    pub fn average_rtt(&self) -> u64 {
        self.rtt_samples.lock().unwrap().average()
    }

    /// Get estimated throughput (bytes/sec).
    pub fn estimated_throughput(&self) -> u64 {
        self.throughput_tracker.average()
    }

    /// Get total bytes sent.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get total chunks sent.
    pub fn chunks_sent(&self) -> u64 {
        self.chunks_sent.load(Ordering::Relaxed)
    }

    /// Get average chunk size.
    pub fn average_chunk_size(&self) -> usize {
        self.bytes_sent()
            .checked_div(self.chunks_sent())
            .map(|v| v as usize)
            .unwrap_or(self.base_chunk)
    }

    /// Get content type.
    pub fn content_type(&self) -> ChunkContentType {
        self.content_type
    }
}

/// Circular buffer for RTT tracking.
#[derive(Debug)]
struct RttTracker {
    samples: [u64; 16],
    index: usize,
    count: usize,
}

impl RttTracker {
    fn new() -> Self {
        Self {
            samples: [0; 16],
            index: 0,
            count: 0,
        }
    }

    fn add_sample(&mut self, rtt_ms: u64) {
        self.samples[self.index] = rtt_ms;
        self.index = (self.index + 1) % 16;
        if self.count < 16 {
            self.count += 1;
        }
    }

    fn average(&self) -> u64 {
        if self.count == 0 {
            return 50; // Default assumption
        }
        let sum: u64 = self.samples[..self.count].iter().sum();
        sum / self.count as u64
    }
}

/// Throughput tracking.
#[derive(Debug)]
struct ThroughputTracker {
    samples: std::sync::Mutex<Vec<u64>>,
    max_samples: usize,
}

impl ThroughputTracker {
    fn new() -> Self {
        Self {
            samples: std::sync::Mutex::new(Vec::with_capacity(16)),
            max_samples: 16,
        }
    }

    fn record(&self, bytes_per_sec: u64) {
        let mut samples = self.samples.lock().unwrap();
        if samples.len() >= self.max_samples {
            samples.remove(0);
        }
        samples.push(bytes_per_sec);
    }

    fn average(&self) -> u64 {
        let samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            return 0;
        }
        let sum: u64 = samples.iter().sum();
        sum / samples.len() as u64
    }
}

// ============================================================================
// HTTP Chunked Encoding Optimizer
// ============================================================================

/// Optimizes chunks specifically for HTTP chunked transfer encoding.
///
/// HTTP chunked encoding has overhead per chunk:
/// - Chunk size in hex + CRLF (variable, typically 1-8 bytes)
/// - Chunk data
/// - CRLF (2 bytes)
///
/// Total overhead per chunk: ~4-10 bytes
#[derive(Debug, Clone)]
pub struct ChunkedEncodingOptimizer {
    /// Minimum chunk to justify encoding overhead
    pub min_chunk: usize,
    /// Target chunk size
    pub target_chunk: usize,
    /// Maximum chunk size
    pub max_chunk: usize,
    /// Overhead threshold (minimum efficiency %)
    pub min_efficiency: f32,
}

impl ChunkedEncodingOptimizer {
    /// Create a new optimizer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set target chunk size.
    pub fn target_chunk(mut self, size: usize) -> Self {
        self.target_chunk = size;
        self
    }

    /// Set minimum efficiency (0.0-1.0).
    pub fn min_efficiency(mut self, efficiency: f32) -> Self {
        self.min_efficiency = efficiency.clamp(0.5, 1.0);
        self
    }

    /// Calculate overhead for a chunk size.
    #[inline]
    pub fn chunk_overhead(chunk_size: usize) -> usize {
        // hex size + CRLF + data + CRLF
        let hex_digits = if chunk_size == 0 {
            1
        } else {
            (chunk_size as f64).log(16.0).floor() as usize + 1
        };
        hex_digits + 4 // hex + CRLF + CRLF
    }

    /// Calculate efficiency for a chunk size.
    #[inline]
    pub fn chunk_efficiency(chunk_size: usize) -> f32 {
        if chunk_size == 0 {
            return 0.0;
        }
        let overhead = Self::chunk_overhead(chunk_size);
        chunk_size as f32 / (chunk_size + overhead) as f32
    }

    /// Calculate optimal chunk size for given data.
    pub fn optimal_for_data(&self, data_len: usize) -> ChunkingPlan {
        if data_len == 0 {
            return ChunkingPlan {
                chunk_size: 0,
                num_chunks: 0,
                final_chunk: 0,
                efficiency: 1.0,
            };
        }

        // If data fits in one chunk efficiently, send it all
        if data_len <= self.max_chunk {
            let eff = Self::chunk_efficiency(data_len);
            if eff >= self.min_efficiency {
                return ChunkingPlan {
                    chunk_size: data_len,
                    num_chunks: 1,
                    final_chunk: 0,
                    efficiency: eff,
                };
            }
        }

        // Calculate number of chunks at target size
        let target = self.target_chunk.min(data_len);
        let num_chunks = data_len / target;
        let remainder = data_len % target;

        // Avoid tiny final chunk
        let (chunk_size, final_chunk) = if remainder > 0 && remainder < self.min_chunk {
            // Redistribute to avoid tiny chunk
            let adjusted_chunks = num_chunks;
            let adjusted_size = data_len / adjusted_chunks;
            let adjusted_remainder = data_len % adjusted_chunks;
            (adjusted_size, adjusted_remainder)
        } else {
            (target, remainder)
        };

        let total_chunks = if final_chunk > 0 {
            num_chunks + 1
        } else {
            num_chunks
        };
        let total_overhead = total_chunks * Self::chunk_overhead(chunk_size);
        let efficiency = data_len as f32 / (data_len + total_overhead) as f32;

        ChunkingPlan {
            chunk_size,
            num_chunks: total_chunks,
            final_chunk,
            efficiency,
        }
    }

    /// Create chunks from data following the optimal plan.
    pub fn create_chunks(&self, data: &[u8]) -> Vec<Bytes> {
        let plan = self.optimal_for_data(data.len());
        if plan.num_chunks == 0 {
            return vec![];
        }

        let mut chunks = Vec::with_capacity(plan.num_chunks);
        let mut offset = 0;

        for i in 0..plan.num_chunks {
            let size = if i == plan.num_chunks - 1 && plan.final_chunk > 0 {
                plan.final_chunk
            } else {
                plan.chunk_size
            };
            chunks.push(Bytes::copy_from_slice(&data[offset..offset + size]));
            offset += size;
        }

        chunks
    }
}

impl Default for ChunkedEncodingOptimizer {
    fn default() -> Self {
        Self {
            min_chunk: CHUNK_SMALL,           // 1KB minimum
            target_chunk: DEFAULT_CHUNK_SIZE, // 16KB target
            max_chunk: CHUNK_XLARGE,          // 128KB max
            min_efficiency: 0.99,             // 99% efficiency
        }
    }
}

/// Plan for chunking data.
#[derive(Debug, Clone, Copy)]
pub struct ChunkingPlan {
    /// Size of each chunk (except possibly final)
    pub chunk_size: usize,
    /// Total number of chunks
    pub num_chunks: usize,
    /// Size of final chunk (0 if evenly divisible)
    pub final_chunk: usize,
    /// Overall efficiency (data / total bytes)
    pub efficiency: f32,
}

impl ChunkingPlan {
    /// Get total overhead in bytes.
    pub fn total_overhead(&self) -> usize {
        self.num_chunks * ChunkedEncodingOptimizer::chunk_overhead(self.chunk_size)
    }
}

// ============================================================================
// Global Chunk Statistics
// ============================================================================

/// Global statistics for chunk optimization.
#[derive(Debug, Default)]
pub struct ChunkStats {
    /// Total chunks created
    chunks_created: AtomicU64,
    /// Total bytes chunked
    bytes_chunked: AtomicU64,
    /// RTT samples recorded
    rtt_samples: AtomicU64,
    /// Total RTT sum (for averaging)
    rtt_sum: AtomicU64,
}

impl ChunkStats {
    fn record_chunk(&self, size: usize) {
        self.chunks_created.fetch_add(1, Ordering::Relaxed);
        self.bytes_chunked.fetch_add(size as u64, Ordering::Relaxed);
    }

    fn record_rtt_sample(&self, rtt_ms: u64) {
        self.rtt_samples.fetch_add(1, Ordering::Relaxed);
        self.rtt_sum.fetch_add(rtt_ms, Ordering::Relaxed);
    }

    /// Get total chunks created.
    pub fn chunks_created(&self) -> u64 {
        self.chunks_created.load(Ordering::Relaxed)
    }

    /// Get total bytes chunked.
    pub fn bytes_chunked(&self) -> u64 {
        self.bytes_chunked.load(Ordering::Relaxed)
    }

    /// Get average chunk size.
    pub fn average_chunk_size(&self) -> usize {
        self.bytes_chunked()
            .checked_div(self.chunks_created())
            .map(|v| v as usize)
            .unwrap_or(0)
    }

    /// Get average RTT.
    pub fn average_rtt(&self) -> u64 {
        self.rtt_sum
            .load(Ordering::Relaxed)
            .checked_div(self.rtt_samples.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}

/// Global chunk statistics.
static CHUNK_STATS: ChunkStats = ChunkStats {
    chunks_created: AtomicU64::new(0),
    bytes_chunked: AtomicU64::new(0),
    rtt_samples: AtomicU64::new(0),
    rtt_sum: AtomicU64::new(0),
};

/// Get global chunk statistics.
pub fn chunk_stats() -> &'static ChunkStats {
    &CHUNK_STATS
}

// ============================================================================
// Streaming Body Builder
// ============================================================================

/// Fluent builder for streaming response bodies.
///
/// # Example
///
/// ```rust,ignore
/// let (body, handle) = StreamingBodyBuilder::new()
///     .chunk_size(8192)
///     .backpressure(BackpressureConfig::new().high_watermark(100))
///     .build_with_sender();
///
/// // Send data
/// handle.send(data).await?;
/// handle.close().await;
/// ```
pub struct StreamingBodyBuilder {
    chunk_size: usize,
    buffer_size: usize,
    backpressure: BackpressureConfig,
    content_type: Option<String>,
    rate_limit: Option<u64>,
}

impl StreamingBodyBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            buffer_size: 64,
            backpressure: BackpressureConfig::default(),
            content_type: None,
            rate_limit: None,
        }
    }

    /// Set chunk size.
    pub fn chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Set buffer size (number of chunks).
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.buffer_size = size;
        self
    }

    /// Set backpressure configuration.
    pub fn backpressure(mut self, config: BackpressureConfig) -> Self {
        self.backpressure = config;
        self
    }

    /// Set content type.
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Set rate limit in bytes per second.
    pub fn rate_limit(mut self, bytes_per_sec: u64) -> Self {
        self.rate_limit = Some(bytes_per_sec);
        self
    }

    /// Build a byte stream with sender handle.
    pub fn build_with_sender(self) -> (ByteStream, StreamingHandle) {
        let (stream, sender) = ByteStream::with_buffer_size(self.buffer_size);
        let handle = StreamingHandle {
            sender,
            chunk_size: self.chunk_size,
            rate_limiter: self.rate_limit.map(StreamRateLimiter::new),
            stats: Arc::new(StreamingHandleStats::default()),
        };
        STREAMING_STATS.record_stream_created();
        (stream, handle)
    }

    /// Build as a streaming response.
    pub fn build_response(self) -> (StreamingResponse, StreamingHandle) {
        let content_type = self.content_type.clone();
        let (stream, handle) = self.build_with_sender();
        let mut response = StreamingResponse::new(stream);
        if let Some(ct) = content_type {
            response = response.content_type(ct);
        }
        (response, handle)
    }
}

impl Default for StreamingBodyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle for sending data to a streaming body.
pub struct StreamingHandle {
    sender: ByteStreamSender,
    chunk_size: usize,
    rate_limiter: Option<StreamRateLimiter>,
    stats: Arc<StreamingHandleStats>,
}

impl StreamingHandle {
    /// Send data to the stream.
    pub async fn send(&self, data: impl Into<Vec<u8>>) -> Result<(), Error> {
        let data = data.into();
        let len = data.len();

        // Apply rate limiting if configured
        if let Some(ref limiter) = self.rate_limiter {
            limiter.wait_for_capacity(len).await;
        }

        self.sender.send(data).await?;
        self.stats.record_send(len);
        STREAMING_STATS.record_chunk_sent(len);
        Ok(())
    }

    /// Send bytes.
    pub async fn send_bytes(&self, bytes: Bytes) -> Result<(), Error> {
        let len = bytes.len();

        if let Some(ref limiter) = self.rate_limiter {
            limiter.wait_for_capacity(len).await;
        }

        self.sender.send_bytes(bytes).await?;
        self.stats.record_send(len);
        STREAMING_STATS.record_chunk_sent(len);
        Ok(())
    }

    /// Send a chunk of data, splitting if necessary.
    pub async fn send_chunked(&self, data: &[u8]) -> Result<(), Error> {
        for chunk in data.chunks(self.chunk_size) {
            self.send(chunk.to_vec()).await?;
        }
        Ok(())
    }

    /// Send an error.
    pub async fn send_error(&self, error: impl Into<String>) -> Result<(), Error> {
        self.sender.send_error(error).await
    }

    /// Close the stream.
    pub async fn close(&self) {
        self.sender.close().await;
    }

    /// Check if the receiver has been dropped.
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    /// Get bytes sent.
    pub fn bytes_sent(&self) -> u64 {
        self.stats.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get chunks sent.
    pub fn chunks_sent(&self) -> u64 {
        self.stats.chunks_sent.load(Ordering::Relaxed)
    }
}

/// Statistics for a streaming handle.
#[derive(Debug, Default)]
struct StreamingHandleStats {
    bytes_sent: AtomicU64,
    chunks_sent: AtomicU64,
}

impl StreamingHandleStats {
    fn record_send(&self, len: usize) {
        self.bytes_sent.fetch_add(len as u64, Ordering::Relaxed);
        self.chunks_sent.fetch_add(1, Ordering::Relaxed);
    }
}

// ============================================================================
// Rate Limiting
// ============================================================================

/// Rate limiter for streaming data.
pub struct StreamRateLimiter {
    /// Bytes per second limit
    pub bytes_per_sec: u64,
    /// Bytes sent in current window
    bytes_in_window: AtomicU64,
    /// Window start time
    window_start: std::sync::Mutex<std::time::Instant>,
}

impl StreamRateLimiter {
    /// Create a new rate limiter.
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            bytes_per_sec,
            bytes_in_window: AtomicU64::new(0),
            window_start: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    /// Wait until capacity is available for sending.
    pub async fn wait_for_capacity(&self, bytes: usize) {
        loop {
            // Check current window
            let now = std::time::Instant::now();
            let elapsed = {
                let start = self.window_start.lock().unwrap();
                now.duration_since(*start)
            };

            // Reset window if more than 1 second has passed
            if elapsed.as_secs() >= 1 {
                self.bytes_in_window.store(0, Ordering::Relaxed);
                *self.window_start.lock().unwrap() = now;
            }

            let current = self.bytes_in_window.load(Ordering::Relaxed);
            if current + bytes as u64 <= self.bytes_per_sec {
                self.bytes_in_window
                    .fetch_add(bytes as u64, Ordering::Relaxed);
                return;
            }

            // Wait until next window
            let remaining = Duration::from_secs(1).saturating_sub(elapsed);
            if !remaining.is_zero() {
                tokio::time::sleep(remaining.min(Duration::from_millis(10))).await;
            }
        }
    }
}

// ============================================================================
// Global Streaming Statistics
// ============================================================================

/// Global statistics for streaming operations.
#[derive(Debug, Default)]
pub struct StreamingStats {
    /// Streams created
    streams_created: AtomicU64,
    /// Total chunks sent
    chunks_sent: AtomicU64,
    /// Total bytes sent
    bytes_sent: AtomicU64,
}

impl StreamingStats {
    /// Create new stats.
    pub fn new() -> Self {
        Self::default()
    }

    fn record_stream_created(&self) {
        self.streams_created.fetch_add(1, Ordering::Relaxed);
    }

    fn record_chunk_sent(&self, len: usize) {
        self.chunks_sent.fetch_add(1, Ordering::Relaxed);
        self.bytes_sent.fetch_add(len as u64, Ordering::Relaxed);
    }

    /// Get streams created.
    pub fn streams_created(&self) -> u64 {
        self.streams_created.load(Ordering::Relaxed)
    }

    /// Get chunks sent.
    pub fn chunks_sent(&self) -> u64 {
        self.chunks_sent.load(Ordering::Relaxed)
    }

    /// Get bytes sent.
    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    /// Get average chunk size.
    pub fn average_chunk_size(&self) -> usize {
        self.bytes_sent()
            .checked_div(self.chunks_sent())
            .map(|v| v as usize)
            .unwrap_or(0)
    }
}

/// Global streaming statistics.
static STREAMING_STATS: StreamingStats = StreamingStats {
    streams_created: AtomicU64::new(0),
    chunks_sent: AtomicU64::new(0),
    bytes_sent: AtomicU64::new(0),
};

/// Get global streaming statistics.
pub fn streaming_stats() -> &'static StreamingStats {
    &STREAMING_STATS
}
