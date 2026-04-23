//! Memory profiling server for leak detection
//!
//! This example creates a server instrumented for memory profiling.
//! It can be used with DHAT, Valgrind, or other memory profilers.
//!
//! # Usage
//!
//! With DHAT (recommended for Rust):
//! ```bash
//! cargo build --example memory_profile_server --release --features memory-profiling
//! ./target/release/examples/memory_profile_server
//! # Generate load, then Ctrl+C to stop and generate report
//! ```
//!
//! With Valgrind:
//! ```bash
//! cargo build --example memory_profile_server --release
//! valgrind --leak-check=full ./target/release/examples/memory_profile_server
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;

// Conditionally use DHAT allocator
#[cfg(feature = "memory-profiling")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Request counter for tracking
static REQUEST_COUNT: AtomicU64 = AtomicU64::new(0);

/// Memory statistics for tracking allocations
#[derive(Debug, Default)]
struct MemoryTracker {
    /// Number of objects created
    objects_created: AtomicU64,
    /// Number of objects dropped
    objects_dropped: AtomicU64,
    /// Bytes allocated (approximate)
    bytes_allocated: AtomicU64,
    /// Bytes freed (approximate)
    bytes_freed: AtomicU64,
}

impl MemoryTracker {
    fn track_alloc(&self, size: usize) {
        self.objects_created.fetch_add(1, Ordering::Relaxed);
        self.bytes_allocated
            .fetch_add(size as u64, Ordering::Relaxed);
    }

    fn track_free(&self, size: usize) {
        self.objects_dropped.fetch_add(1, Ordering::Relaxed);
        self.bytes_freed.fetch_add(size as u64, Ordering::Relaxed);
    }

    fn stats(&self) -> MemoryStats {
        MemoryStats {
            objects_created: self.objects_created.load(Ordering::Relaxed),
            objects_dropped: self.objects_dropped.load(Ordering::Relaxed),
            bytes_allocated: self.bytes_allocated.load(Ordering::Relaxed),
            bytes_freed: self.bytes_freed.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Serialize)]
struct MemoryStats {
    objects_created: u64,
    objects_dropped: u64,
    bytes_allocated: u64,
    bytes_freed: u64,
}

static TRACKER: MemoryTracker = MemoryTracker {
    objects_created: AtomicU64::new(0),
    objects_dropped: AtomicU64::new(0),
    bytes_allocated: AtomicU64::new(0),
    bytes_freed: AtomicU64::new(0),
};

// ============================================================================
// Domain Models - Tracked for memory profiling
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<HashMap<String, String>>,
}

impl User {
    fn new(id: u64, name: String, email: String) -> Self {
        let size = std::mem::size_of::<Self>() + name.capacity() + email.capacity();
        TRACKER.track_alloc(size);

        Self {
            id,
            name,
            email,
            metadata: None,
        }
    }

    #[allow(dead_code)]
    fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        let size: usize = metadata
            .iter()
            .map(|(k, v)| k.capacity() + v.capacity())
            .sum();
        TRACKER.track_alloc(size);
        self.metadata = Some(metadata);
        self
    }
}

impl Drop for User {
    fn drop(&mut self) {
        let size = std::mem::size_of::<Self>() + self.name.capacity() + self.email.capacity();
        TRACKER.track_free(size);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    id: u64,
    user_id: u64,
    title: String,
    body: String,
    tags: Vec<String>,
}

impl Post {
    #[allow(dead_code)]
    fn new(id: u64, user_id: u64, title: String, body: String, tags: Vec<String>) -> Self {
        let size = std::mem::size_of::<Self>()
            + title.capacity()
            + body.capacity()
            + tags.iter().map(|t| t.capacity()).sum::<usize>();
        TRACKER.track_alloc(size);

        Self {
            id,
            user_id,
            title,
            body,
            tags,
        }
    }
}

impl Drop for Post {
    fn drop(&mut self) {
        let size = std::mem::size_of::<Self>()
            + self.title.capacity()
            + self.body.capacity()
            + self.tags.iter().map(|t| t.capacity()).sum::<usize>();
        TRACKER.track_free(size);
    }
}

// ============================================================================
// In-Memory Store (Potential leak source if not managed)
// ============================================================================

#[derive(Default)]
struct DataStore {
    users: RwLock<HashMap<u64, User>>,
    posts: RwLock<HashMap<u64, Post>>,
    // Simulated cache that could leak if not bounded
    cache: RwLock<HashMap<String, Vec<u8>>>,
}

impl DataStore {
    fn new() -> Self {
        Self::default()
    }

    async fn get_user(&self, id: u64) -> Option<User> {
        self.users.read().await.get(&id).cloned()
    }

    async fn create_user(&self, user: User) -> User {
        let mut users = self.users.write().await;
        users.insert(user.id, user.clone());
        user
    }

    /// Simulates a bounded cache that could leak if not properly managed
    #[allow(dead_code)]
    async fn cache_set(&self, key: String, value: Vec<u8>) {
        let mut cache = self.cache.write().await;

        // Bounded cache - evict old entries if too large
        const MAX_CACHE_ENTRIES: usize = 1000;
        if cache.len() >= MAX_CACHE_ENTRIES {
            // Simple eviction: remove first entry
            if let Some(first_key) = cache.keys().next().cloned() {
                cache.remove(&first_key);
            }
        }

        TRACKER.track_alloc(key.capacity() + value.capacity());
        cache.insert(key, value);
    }

    async fn stats(&self) -> StoreStats {
        StoreStats {
            user_count: self.users.read().await.len(),
            post_count: self.posts.read().await.len(),
            cache_entries: self.cache.read().await.len(),
            memory: TRACKER.stats(),
        }
    }
}

#[derive(Serialize)]
struct StoreStats {
    user_count: usize,
    post_count: usize,
    cache_entries: usize,
    memory: MemoryStats,
}

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Serialize)]
struct JsonResponse<T> {
    data: T,
    request_id: u64,
}

impl<T: Serialize> JsonResponse<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            request_id: REQUEST_COUNT.fetch_add(1, Ordering::Relaxed),
        }
    }
}

// ============================================================================
// Simple HTTP Handler
// ============================================================================

async fn handle_request(
    store: &Arc<DataStore>,
    method: &str,
    path: &str,
    _body: &str,
) -> (u16, String, Vec<u8>) {
    REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);

    match (method, path) {
        ("GET", "/health") => (200, "text/plain".to_string(), b"OK".to_vec()),

        ("GET", "/json") => {
            let response = serde_json::json!({
                "message": "Hello, World!",
                "timestamp": chrono_lite_timestamp(),
            });
            (
                200,
                "application/json".to_string(),
                serde_json::to_vec(&response).unwrap_or_default(),
            )
        }

        ("GET", "/stats") => {
            let stats = store.stats().await;
            let response = JsonResponse::new(stats);
            (
                200,
                "application/json".to_string(),
                serde_json::to_vec(&response).unwrap_or_default(),
            )
        }

        ("GET", p) if p.starts_with("/users/") => {
            let id_str = p.strip_prefix("/users/").unwrap_or("0");
            let id: u64 = id_str.parse().unwrap_or(0);

            match store.get_user(id).await {
                Some(user) => {
                    let response = JsonResponse::new(user);
                    (
                        200,
                        "application/json".to_string(),
                        serde_json::to_vec(&response).unwrap_or_default(),
                    )
                }
                None => (
                    404,
                    "application/json".to_string(),
                    br#"{"error":"User not found"}"#.to_vec(),
                ),
            }
        }

        ("GET", p) if p.starts_with("/heavy/") => {
            let size_str = p.strip_prefix("/heavy/").unwrap_or("1000");
            let size: usize = size_str.parse().unwrap_or(1000).min(10_000_000);

            // Allocate a vector of specified size
            let data: Vec<u8> = vec![0u8; size];
            TRACKER.track_alloc(data.len());

            // Simulate some processing
            let sum: u64 = data.iter().map(|&b| b as u64).sum();

            TRACKER.track_free(data.len());

            let response = serde_json::json!({
                "allocated_bytes": size,
                "checksum": sum
            });
            (
                200,
                "application/json".to_string(),
                serde_json::to_vec(&response).unwrap_or_default(),
            )
        }

        _ => (404, "text/plain".to_string(), b"Not Found".to_vec()),
    }
}

/// Simple timestamp without heavy dependencies
fn chrono_lite_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() {
    // Initialize DHAT profiler if enabled
    #[cfg(feature = "memory-profiling")]
    let _profiler = dhat::Profiler::new_heap();

    println!("🧠 Armature Memory Profile Server");
    println!("==================================");
    println!();

    #[cfg(feature = "memory-profiling")]
    println!("📊 DHAT profiler enabled - memory stats will be written on exit");

    #[cfg(not(feature = "memory-profiling"))]
    println!("ℹ️  Build with --features memory-profiling for DHAT support");

    let store = Arc::new(DataStore::new());

    // Pre-populate some data
    for i in 0..100 {
        let user = User::new(i, format!("User {}", i), format!("user{}@example.com", i));
        store.create_user(user).await;
    }
    println!("✅ Pre-populated 100 users");

    println!();
    println!("Endpoints:");
    println!("  GET  /health       - Health check");
    println!("  GET  /json         - Simple JSON response");
    println!("  GET  /stats        - Memory statistics");
    println!("  GET  /users/:id    - Get user by ID");
    println!("  GET  /heavy/:size  - Allocate memory");
    println!();
    println!("🚀 Starting server on http://localhost:3000");
    println!("   Press Ctrl+C to stop and generate memory report");
    println!();

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    // Handle graceful shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        println!();
        println!("🛑 Shutdown signal received...");
        let _ = shutdown_tx.send(());
    });

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let store = store.clone();
                        tokio::spawn(async move {
                            let _ = handle_connection(stream, store).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("Accept error: {}", e);
                    }
                }
            }
            _ = &mut shutdown_rx => {
                println!("📊 Final memory statistics:");
                let stats = TRACKER.stats();
                println!("   Objects created: {}", stats.objects_created);
                println!("   Objects dropped: {}", stats.objects_dropped);
                println!("   Bytes allocated: {}", stats.bytes_allocated);
                println!("   Bytes freed: {}", stats.bytes_freed);
                println!("   Potential leak: {} bytes",
                    stats.bytes_allocated.saturating_sub(stats.bytes_freed));
                break;
            }
        }
    }

    println!();
    println!("🎉 Server stopped. Check memory report.");
}

/// Simplified connection handler
async fn handle_connection(
    stream: tokio::net::TcpStream,
    store: Arc<DataStore>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut request_line = String::new();

    reader.read_line(&mut request_line).await?;

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }

    let method = parts[0];
    let path = parts[1];

    // Read headers and body (simplified)
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
        if line.to_lowercase().starts_with("content-length:") {
            content_length = line
                .split(':')
                .nth(1)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
        }
    }

    // Read body if present
    let body = if content_length > 0 {
        let mut buf = vec![0u8; content_length];
        tokio::io::AsyncReadExt::read_exact(&mut reader, &mut buf).await?;
        String::from_utf8_lossy(&buf).to_string()
    } else {
        String::new()
    };

    // Handle request
    let (status, content_type, body_bytes) = handle_request(&store, method, path, &body).await;

    // Write response
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        status_text(status),
        content_type,
        body_bytes.len()
    );

    writer.write_all(response.as_bytes()).await?;
    writer.write_all(&body_bytes).await?;
    writer.flush().await?;

    Ok(())
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}
