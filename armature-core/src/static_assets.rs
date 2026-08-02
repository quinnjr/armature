//! Static asset serving with configurable caching and compression.
//!
//! This module provides high-performance static file serving with:
//! - Configurable cache strategies
//! - ETag support for conditional requests
//! - Compression support (gzip, brotli, zstd)
//! - Content-Type detection
//! - Security (path traversal prevention)
//! - File type-based cache policies

use crate::{Error, HttpRequest, HttpResponse};
use bytes::Bytes;
use lru::LruCache;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Default number of `(path, mtime, encoding)` entries retained in the
/// in-memory content cache.
pub const DEFAULT_CONTENT_CACHE_CAPACITY: usize = 128;

/// Default maximum file size (bytes) that will be read fully into memory and
/// cached. Larger files are streamed fresh on every request and never cached
/// to keep memory bounded.
pub const DEFAULT_MAX_SERVE_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

/// Cache key: file path, its modification time, and the negotiated encoding.
///
/// Keying on `mtime` means a modified file naturally misses the cache (the old
/// entry is never looked up again and is evicted by LRU pressure).
type ContentCacheKey = (PathBuf, SystemTime, Option<CompressionAlgorithm>);

/// A cached, ready-to-serve file body plus its ETag.
#[derive(Clone)]
struct CachedContent {
    /// Already-compressed (or raw) bytes, shared cheaply via `Bytes`.
    body: Bytes,
    /// ETag computed for this `(path, mtime, encoding)` triple.
    etag: Option<String>,
}

/// Cache strategy for static assets
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStrategy {
    /// No caching (Cache-Control: no-cache, no-store)
    NoCache,

    /// Public cache with max-age (Cache-Control: public, max-age=N)
    Public(Duration),

    /// Private cache with max-age (Cache-Control: private, max-age=N)
    Private(Duration),

    /// Immutable assets (Cache-Control: public, max-age=31536000, immutable)
    /// Perfect for hashed/versioned assets
    Immutable,

    /// Revalidate every time (Cache-Control: no-cache)
    MustRevalidate,
}

impl CacheStrategy {
    /// Convert strategy to Cache-Control header value
    pub fn to_header_value(&self) -> String {
        match self {
            CacheStrategy::NoCache => "no-cache, no-store, must-revalidate".to_string(),
            CacheStrategy::Public(duration) => {
                format!("public, max-age={}", duration.as_secs())
            }
            CacheStrategy::Private(duration) => {
                format!("private, max-age={}", duration.as_secs())
            }
            CacheStrategy::Immutable => "public, max-age=31536000, immutable".to_string(),
            CacheStrategy::MustRevalidate => "no-cache".to_string(),
        }
    }
}

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompressionAlgorithm {
    /// Gzip compression
    Gzip,

    /// Brotli compression (higher compression ratio than gzip)
    Brotli,

    /// Zstandard compression (fast, with a ratio competitive with Brotli)
    Zstd,
}

impl CompressionAlgorithm {
    /// Get Content-Encoding header value
    pub fn to_header_value(&self) -> &'static str {
        match self {
            CompressionAlgorithm::Gzip => "gzip",
            CompressionAlgorithm::Brotli => "br",
            CompressionAlgorithm::Zstd => "zstd",
        }
    }

    /// Get file extension for pre-compressed files
    pub fn file_extension(&self) -> &'static str {
        match self {
            CompressionAlgorithm::Gzip => ".gz",
            CompressionAlgorithm::Brotli => ".br",
            CompressionAlgorithm::Zstd => ".zst",
        }
    }
}

/// Compression level (0-11 for Brotli, 0-9 for Gzip)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    /// Fastest compression (lowest ratio)
    Fast,

    /// Balanced compression and speed
    Default,

    /// Best compression (slowest)
    Best,

    /// Custom level (0-11 for Brotli, 0-9 for Gzip)
    Custom(u32),
}

impl CompressionLevel {
    /// Get gzip compression level
    pub fn gzip_level(&self) -> flate2::Compression {
        match self {
            CompressionLevel::Fast => flate2::Compression::fast(),
            CompressionLevel::Default => flate2::Compression::default(),
            CompressionLevel::Best => flate2::Compression::best(),
            CompressionLevel::Custom(level) => flate2::Compression::new((*level).min(9)),
        }
    }

    /// Get brotli compression level
    pub fn brotli_level(&self) -> u32 {
        match self {
            CompressionLevel::Fast => 4,
            CompressionLevel::Default => 6,
            CompressionLevel::Best => 11,
            CompressionLevel::Custom(level) => (*level).min(11),
        }
    }

    /// Get zstd compression level (1-22; 20+ requires substantially more
    /// memory and time, so `Best` stops at 19 rather than the theoretical
    /// max of 22).
    pub fn zstd_level(&self) -> i32 {
        match self {
            CompressionLevel::Fast => 1,
            CompressionLevel::Default => 3,
            CompressionLevel::Best => 19,
            CompressionLevel::Custom(level) => (*level).min(22) as i32,
        }
    }
}

/// Compression configuration
#[derive(Debug, Clone)]
pub struct CompressionConfig {
    /// Enable compression
    pub enabled: bool,

    /// Compression level
    pub level: CompressionLevel,

    /// Preferred algorithm (tried first)
    pub prefer_brotli: bool,

    /// Enable serving pre-compressed files (.gz, .br)
    pub serve_precompressed: bool,

    /// Minimum file size to compress (bytes)
    pub min_size: usize,

    /// Maximum file size to compress (bytes)
    pub max_size: usize,

    /// File types to compress (if None, compress all compressible types)
    pub compress_types: Option<Vec<FileType>>,
}

impl CompressionConfig {
    /// Create a new compression configuration
    pub fn new() -> Self {
        Self {
            enabled: true,
            level: CompressionLevel::Default,
            prefer_brotli: true,
            serve_precompressed: true,
            min_size: 1024,       // 1 KB
            max_size: 10485760,   // 10 MB
            compress_types: None, // Compress all compressible types
        }
    }

    /// Disable compression
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::new()
        }
    }

    /// Set compression level
    pub fn with_level(mut self, level: CompressionLevel) -> Self {
        self.level = level;
        self
    }

    /// Prefer Brotli over Gzip
    pub fn prefer_brotli(mut self, prefer: bool) -> Self {
        self.prefer_brotli = prefer;
        self
    }

    /// Enable/disable serving pre-compressed files
    pub fn serve_precompressed(mut self, enable: bool) -> Self {
        self.serve_precompressed = enable;
        self
    }

    /// Set minimum file size to compress
    pub fn with_min_size(mut self, size: usize) -> Self {
        self.min_size = size;
        self
    }

    /// Set maximum file size to compress
    pub fn with_max_size(mut self, size: usize) -> Self {
        self.max_size = size;
        self
    }

    /// Set file types to compress
    pub fn with_compress_types(mut self, types: Vec<FileType>) -> Self {
        self.compress_types = Some(types);
        self
    }

    /// Check if a file should be compressed
    pub fn should_compress(&self, file_type: FileType, size: usize) -> bool {
        if !self.enabled {
            return false;
        }

        if size < self.min_size || size > self.max_size {
            return false;
        }

        // Check if file type is compressible
        let is_compressible = match file_type {
            FileType::JavaScript | FileType::Stylesheet | FileType::Html | FileType::Json => true,
            FileType::Image => false, // Images are usually already compressed
            FileType::Font => false,  // Fonts are usually already compressed
            FileType::Video | FileType::Audio => false, // Media is already compressed
            FileType::Other => false,
        };

        if !is_compressible {
            return false;
        }

        // Check allowed types if specified
        if let Some(ref types) = self.compress_types {
            types.contains(&file_type)
        } else {
            true
        }
    }
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// File type classification for cache policies
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    /// JavaScript files (.js, .mjs)
    JavaScript,

    /// CSS files (.css)
    Stylesheet,

    /// Image files (.png, .jpg, .jpeg, .gif, .svg, .webp, .avif)
    Image,

    /// Font files (.woff, .woff2, .ttf, .otf, .eot)
    Font,

    /// HTML files (.html, .htm)
    Html,

    /// JSON files (.json)
    Json,

    /// Video files (.mp4, .webm, .ogg)
    Video,

    /// Audio files (.mp3, .wav, .ogg, .m4a)
    Audio,

    /// Other/unknown files
    Other,
}

impl FileType {
    /// Detect file type from path extension
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("js") | Some("mjs") => FileType::JavaScript,
            Some("css") => FileType::Stylesheet,
            Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("svg") | Some("webp")
            | Some("avif") | Some("ico") => FileType::Image,
            Some("woff") | Some("woff2") | Some("ttf") | Some("otf") | Some("eot") => {
                FileType::Font
            }
            Some("html") | Some("htm") => FileType::Html,
            Some("json") => FileType::Json,
            Some("mp4") | Some("webm") | Some("ogv") => FileType::Video,
            Some("mp3") | Some("wav") | Some("ogg") | Some("m4a") => FileType::Audio,
            _ => FileType::Other,
        }
    }

    /// Get MIME type for file type
    pub fn mime_type(&self, path: &Path) -> String {
        match self {
            FileType::JavaScript => "application/javascript".to_string(),
            FileType::Stylesheet => "text/css".to_string(),
            FileType::Image => match path.extension().and_then(|ext| ext.to_str()) {
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("svg") => "image/svg+xml",
                Some("webp") => "image/webp",
                Some("avif") => "image/avif",
                Some("ico") => "image/x-icon",
                _ => "image/*",
            }
            .to_string(),
            FileType::Font => match path.extension().and_then(|ext| ext.to_str()) {
                Some("woff") => "font/woff",
                Some("woff2") => "font/woff2",
                Some("ttf") => "font/ttf",
                Some("otf") => "font/otf",
                Some("eot") => "application/vnd.ms-fontobject",
                _ => "font/*",
            }
            .to_string(),
            FileType::Html => "text/html".to_string(),
            FileType::Json => "application/json".to_string(),
            FileType::Video => "video/mp4".to_string(),
            FileType::Audio => "audio/mpeg".to_string(),
            FileType::Other => "application/octet-stream".to_string(),
        }
    }
}

/// Configuration for static asset serving
#[derive(Debug, Clone)]
pub struct StaticAssetsConfig {
    /// Root directory for static files
    pub root_dir: PathBuf,

    /// Default cache strategy
    pub default_strategy: CacheStrategy,

    /// File type-specific cache strategies
    pub type_strategies: HashMap<FileType, CacheStrategy>,

    /// Enable ETag generation and validation
    pub enable_etag: bool,

    /// Enable Last-Modified headers
    pub enable_last_modified: bool,

    /// Enable CORS for static assets
    pub enable_cors: bool,

    /// Custom CORS origin (if None, uses *)
    pub cors_origin: Option<String>,

    /// Fallback file for SPA (e.g., "index.html")
    pub fallback: Option<String>,

    /// List of index files to try (e.g., ["index.html", "index.htm"])
    pub index_files: Vec<String>,

    /// Compression configuration
    pub compression: CompressionConfig,

    /// Maximum number of `(path, mtime, encoding)` entries to keep in the
    /// in-memory content cache. `0` disables the cache.
    pub cache_capacity: usize,

    /// Files larger than this (bytes) are read fresh on every request and are
    /// never cached, keeping memory usage bounded for large assets.
    pub max_serve_size: usize,
}

impl StaticAssetsConfig {
    /// Create a new configuration with root directory
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        let mut type_strategies = HashMap::new();

        // Default strategies per file type
        type_strategies.insert(
            FileType::JavaScript,
            CacheStrategy::Public(Duration::from_secs(3600)),
        );
        type_strategies.insert(
            FileType::Stylesheet,
            CacheStrategy::Public(Duration::from_secs(3600)),
        );
        type_strategies.insert(
            FileType::Image,
            CacheStrategy::Public(Duration::from_secs(86400)),
        );
        type_strategies.insert(FileType::Font, CacheStrategy::Immutable);
        type_strategies.insert(FileType::Html, CacheStrategy::NoCache);
        type_strategies.insert(FileType::Json, CacheStrategy::NoCache);
        type_strategies.insert(
            FileType::Video,
            CacheStrategy::Public(Duration::from_secs(86400)),
        );
        type_strategies.insert(
            FileType::Audio,
            CacheStrategy::Public(Duration::from_secs(86400)),
        );

        Self {
            root_dir: root_dir.into(),
            default_strategy: CacheStrategy::Public(Duration::from_secs(3600)),
            type_strategies,
            enable_etag: true,
            enable_last_modified: true,
            enable_cors: true,
            cors_origin: None,
            fallback: None,
            index_files: vec!["index.html".to_string()],
            compression: CompressionConfig::new(),
            cache_capacity: DEFAULT_CONTENT_CACHE_CAPACITY,
            max_serve_size: DEFAULT_MAX_SERVE_SIZE,
        }
    }

    /// Set the in-memory content cache capacity (number of cached
    /// `(path, mtime, encoding)` entries). Set to `0` to disable caching.
    pub fn with_cache_capacity(mut self, capacity: usize) -> Self {
        self.cache_capacity = capacity;
        self
    }

    /// Set the maximum file size (bytes) that will be read fully into memory
    /// and cached. Larger files are streamed fresh on every request.
    pub fn with_max_serve_size(mut self, size: usize) -> Self {
        self.max_serve_size = size;
        self
    }

    /// Set default cache strategy
    pub fn with_default_strategy(mut self, strategy: CacheStrategy) -> Self {
        self.default_strategy = strategy;
        self
    }

    /// Set cache strategy for a file type
    pub fn with_type_strategy(mut self, file_type: FileType, strategy: CacheStrategy) -> Self {
        self.type_strategies.insert(file_type, strategy);
        self
    }

    /// Enable/disable ETag support
    pub fn with_etag(mut self, enable: bool) -> Self {
        self.enable_etag = enable;
        self
    }

    /// Enable/disable Last-Modified headers
    pub fn with_last_modified(mut self, enable: bool) -> Self {
        self.enable_last_modified = enable;
        self
    }

    /// Enable/disable CORS
    pub fn with_cors(mut self, enable: bool) -> Self {
        self.enable_cors = enable;
        self
    }

    /// Set CORS origin
    pub fn with_cors_origin(mut self, origin: impl Into<String>) -> Self {
        self.cors_origin = Some(origin.into());
        self
    }

    /// Set fallback file for SPA routing
    pub fn with_fallback(mut self, fallback: impl Into<String>) -> Self {
        self.fallback = Some(fallback.into());
        self
    }

    /// Set index files
    pub fn with_index_files(mut self, files: Vec<String>) -> Self {
        self.index_files = files;
        self
    }

    /// Set compression configuration
    pub fn with_compression(mut self, compression: CompressionConfig) -> Self {
        self.compression = compression;
        self
    }

    /// Enable/disable compression
    pub fn with_compression_enabled(mut self, enable: bool) -> Self {
        self.compression.enabled = enable;
        self
    }

    /// Set compression level
    pub fn with_compression_level(mut self, level: CompressionLevel) -> Self {
        self.compression.level = level;
        self
    }

    /// Configure for Single Page Application (SPA)
    pub fn spa_mode(self) -> Self {
        self.with_fallback("index.html")
            .with_type_strategy(FileType::Html, CacheStrategy::NoCache)
            .with_type_strategy(FileType::JavaScript, CacheStrategy::Immutable)
            .with_type_strategy(FileType::Stylesheet, CacheStrategy::Immutable)
            .with_compression_enabled(true)
    }

    /// Configure for maximum performance (aggressive caching and compression)
    pub fn max_performance(self) -> Self {
        self.with_type_strategy(FileType::JavaScript, CacheStrategy::Immutable)
            .with_type_strategy(FileType::Stylesheet, CacheStrategy::Immutable)
            .with_type_strategy(FileType::Image, CacheStrategy::Immutable)
            .with_type_strategy(FileType::Font, CacheStrategy::Immutable)
            .with_compression(
                CompressionConfig::new()
                    .with_level(CompressionLevel::Best)
                    .prefer_brotli(true)
                    .serve_precompressed(true),
            )
    }

    /// Configure for development (no caching, no compression)
    pub fn development(self) -> Self {
        self.with_default_strategy(CacheStrategy::NoCache)
            .with_etag(false)
            .with_last_modified(false)
            .with_compression(CompressionConfig::disabled())
    }
}

impl Default for StaticAssetsConfig {
    fn default() -> Self {
        Self::new("public")
    }
}

/// Static asset server
#[derive(Clone)]
pub struct StaticAssetServer {
    config: StaticAssetsConfig,
    /// Bounded in-memory cache of already-compressed (or raw) file bodies,
    /// shared across clones. `None` when caching is disabled
    /// (`cache_capacity == 0`).
    cache: Option<Arc<Mutex<LruCache<ContentCacheKey, CachedContent>>>>,
}

impl StaticAssetServer {
    /// Create a new static asset server
    pub fn new(config: StaticAssetsConfig) -> Result<Self, Error> {
        if !config.root_dir.exists() {
            return Err(Error::Internal(format!(
                "Static assets directory not found: {:?}",
                config.root_dir
            )));
        }

        let cache = NonZeroUsize::new(config.cache_capacity)
            .map(|cap| Arc::new(Mutex::new(LruCache::new(cap))));

        Ok(Self { config, cache })
    }

    /// Look up a cached body for this `(path, mtime, encoding)` key.
    fn cache_get(&self, key: &ContentCacheKey) -> Option<CachedContent> {
        let cache = self.cache.as_ref()?;
        cache.lock().get(key).cloned()
    }

    /// Store a body in the cache (no-op when caching is disabled).
    fn cache_put(&self, key: ContentCacheKey, value: CachedContent) {
        if let Some(cache) = self.cache.as_ref() {
            cache.lock().put(key, value);
        }
    }

    /// Read a file body, applying pre-compressed lookup / on-the-fly
    /// compression exactly as configured. Returns the ready-to-serve bytes.
    async fn load_body(
        &self,
        path: &Path,
        compression: Option<CompressionAlgorithm>,
    ) -> Result<Bytes, Error> {
        let bytes = if let Some(algo) = compression {
            if self.config.compression.serve_precompressed {
                if let Some(content) = self.try_serve_precompressed(path, algo).await? {
                    content
                } else {
                    let raw_content = tokio::fs::read(path)
                        .await
                        .map_err(|e| Error::Internal(format!("Failed to read file: {}", e)))?;
                    self.compress_content(&raw_content, algo)?
                }
            } else {
                let raw_content = tokio::fs::read(path)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to read file: {}", e)))?;
                self.compress_content(&raw_content, algo)?
            }
        } else {
            tokio::fs::read(path)
                .await
                .map_err(|e| Error::Internal(format!("Failed to read file: {}", e)))?
        };
        Ok(Bytes::from(bytes))
    }

    /// Serve a static file
    pub async fn serve(&self, req: &HttpRequest) -> Result<HttpResponse, Error> {
        let path = self.resolve_path(&req.path)?;

        // Check if path exists
        if !path.exists() {
            // Try fallback for SPA
            if let Some(ref fallback) = self.config.fallback {
                let fallback_path = self.config.root_dir.join(fallback);
                if fallback_path.exists() {
                    return self.serve_file(&fallback_path, req).await;
                }
            }
            return Err(Error::NotFound(format!("File not found: {}", req.path)));
        }

        // If directory, try index files
        if path.is_dir() {
            for index_file in &self.config.index_files {
                let index_path = path.join(index_file);
                if index_path.exists() && index_path.is_file() {
                    return self.serve_file(&index_path, req).await;
                }
            }
            return Err(Error::Forbidden("Directory listing disabled".to_string()));
        }

        self.serve_file(&path, req).await
    }

    /// Serve a specific file
    async fn serve_file(&self, path: &Path, req: &HttpRequest) -> Result<HttpResponse, Error> {
        // Get file metadata
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| Error::Internal(format!("Failed to read file metadata: {}", e)))?;

        let modified = metadata.modified().ok();
        let file_size = metadata.len() as usize;
        let file_type = FileType::from_path(path);

        // Determine compression
        let compression = self.select_compression(req, file_type, file_size);

        // Generate ETag (include compression in ETag)
        let etag = if self.config.enable_etag {
            Some(self.generate_etag_with_compression(path, &metadata, compression.as_ref()))
        } else {
            None
        };

        // Check conditional headers
        if let Some(ref etag_value) = etag
            && let Some(if_none_match) = req.headers.get("If-None-Match")
            && if_none_match == etag_value
        {
            return Ok(self.not_modified_response(etag_value));
        }

        if self.config.enable_last_modified
            && let Some(modified_time) = modified
            && let Some(if_modified_since) = req.headers.get("If-Modified-Since")
            && let Ok(since_time) = httpdate::parse_http_date(if_modified_since)
            && modified_time <= since_time
        {
            return Ok(self.not_modified_response(etag.as_deref().unwrap_or("")));
        }

        // Decide whether this response is cacheable. Files above
        // `max_serve_size` are read fresh every time and never cached so a few
        // huge assets cannot exhaust memory; caching also requires a known
        // mtime for invalidation.
        let used_compression = compression;
        let cacheable = file_size <= self.config.max_serve_size && modified.is_some();

        let content: Bytes = if cacheable {
            let key: ContentCacheKey = (path.to_path_buf(), modified.unwrap(), compression);
            if let Some(entry) = self.cache_get(&key) {
                // Cache hit: skip the read + compression entirely. The ETag is
                // derived from the same (path, mtime, encoding) key, so the
                // cached one must match the freshly computed one.
                debug_assert_eq!(entry.etag, etag);
                entry.body
            } else {
                let body = self.load_body(path, compression).await?;
                self.cache_put(
                    key,
                    CachedContent {
                        body: body.clone(),
                        etag: etag.clone(),
                    },
                );
                body
            }
        } else {
            self.load_body(path, compression).await?
        };

        // Build response
        let mut response = HttpResponse::ok().with_bytes_body(content);

        // Content-Type
        let content_type = file_type.mime_type(path);
        response
            .headers
            .insert("Content-Type".to_string(), content_type);

        // Content-Encoding
        if let Some(algo) = used_compression {
            response.headers.insert(
                "Content-Encoding".to_string(),
                algo.to_header_value().to_string(),
            );
            response
                .headers
                .insert("Vary".to_string(), "Accept-Encoding".to_string());
        }

        // Cache-Control
        let cache_strategy = self
            .config
            .type_strategies
            .get(&file_type)
            .copied()
            .unwrap_or(self.config.default_strategy);
        response.headers.insert(
            "Cache-Control".to_string(),
            cache_strategy.to_header_value(),
        );

        // ETag
        if let Some(etag_value) = etag {
            response.headers.insert("ETag".to_string(), etag_value);
        }

        // Last-Modified
        if self.config.enable_last_modified
            && let Some(modified_time) = modified
        {
            let formatted = httpdate::fmt_http_date(modified_time);
            response
                .headers
                .insert("Last-Modified".to_string(), formatted);
        }

        // CORS
        if self.config.enable_cors {
            let origin = self.config.cors_origin.as_deref().unwrap_or("*");
            response.headers.insert(
                "Access-Control-Allow-Origin".to_string(),
                origin.to_string(),
            );
            response.headers.insert(
                "Access-Control-Allow-Methods".to_string(),
                "GET, HEAD, OPTIONS".to_string(),
            );
        }

        Ok(response)
    }

    /// Select compression algorithm based on Accept-Encoding header and configuration
    fn select_compression(
        &self,
        req: &HttpRequest,
        file_type: FileType,
        file_size: usize,
    ) -> Option<CompressionAlgorithm> {
        if !self
            .config
            .compression
            .should_compress(file_type, file_size)
        {
            return None;
        }

        // Parse Accept-Encoding header
        let accept_encoding = req
            .headers
            .get("Accept-Encoding")
            .or_else(|| req.headers.get("accept-encoding"))?;

        let encodings: Vec<&str> = accept_encoding
            .split(',')
            .map(|s| s.trim().split(';').next().unwrap_or(""))
            .collect();

        // Check if client supports our compression algorithms
        let supports_brotli = encodings.contains(&"br");
        let supports_gzip = encodings.contains(&"gzip");
        let supports_zstd = encodings.contains(&"zstd");

        // Select based on preference and support. When Brotli is preferred
        // and supported it wins outright; otherwise the priority is
        // gzip > zstd > brotli, with zstd filling the gap for clients that
        // advertise it without also advertising gzip or brotli.
        if self.config.compression.prefer_brotli && supports_brotli {
            Some(CompressionAlgorithm::Brotli)
        } else if supports_gzip {
            Some(CompressionAlgorithm::Gzip)
        } else if supports_brotli {
            Some(CompressionAlgorithm::Brotli)
        } else if supports_zstd {
            Some(CompressionAlgorithm::Zstd)
        } else {
            None
        }
    }

    /// Try to serve a pre-compressed file
    async fn try_serve_precompressed(
        &self,
        path: &Path,
        algo: CompressionAlgorithm,
    ) -> Result<Option<Vec<u8>>, Error> {
        let compressed_path = path.with_extension(format!(
            "{}{}",
            path.extension().and_then(|e| e.to_str()).unwrap_or(""),
            algo.file_extension()
        ));

        if compressed_path.exists() {
            let content = tokio::fs::read(&compressed_path).await.map_err(|e| {
                Error::Internal(format!("Failed to read pre-compressed file: {}", e))
            })?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    /// Compress content on-the-fly
    fn compress_content(
        &self,
        content: &[u8],
        algo: CompressionAlgorithm,
    ) -> Result<Vec<u8>, Error> {
        match algo {
            CompressionAlgorithm::Gzip => {
                use flate2::write::GzEncoder;

                let mut encoder =
                    GzEncoder::new(Vec::new(), self.config.compression.level.gzip_level());
                encoder
                    .write_all(content)
                    .map_err(|e| Error::Internal(format!("Gzip compression failed: {}", e)))?;
                encoder
                    .finish()
                    .map_err(|e| Error::Internal(format!("Gzip compression failed: {}", e)))
            }
            CompressionAlgorithm::Brotli => {
                let mut output = Vec::new();
                let params = brotli::enc::BrotliEncoderParams {
                    quality: self.config.compression.level.brotli_level() as i32,
                    ..Default::default()
                };

                brotli::BrotliCompress(&mut std::io::Cursor::new(content), &mut output, &params)
                    .map_err(|e| Error::Internal(format!("Brotli compression failed: {}", e)))?;

                Ok(output)
            }
            CompressionAlgorithm::Zstd => {
                let level = self.config.compression.level.zstd_level();
                zstd::encode_all(std::io::Cursor::new(content), level)
                    .map_err(|e| Error::Internal(format!("Zstd compression failed: {}", e)))
            }
        }
    }

    /// Resolve request path to file system path
    fn resolve_path(&self, request_path: &str) -> Result<PathBuf, Error> {
        // Remove leading slash and query string
        let clean_path = request_path
            .trim_start_matches('/')
            .split('?')
            .next()
            .unwrap_or("");

        // Build full path
        let full_path = self.config.root_dir.join(clean_path);

        // Security: prevent directory traversal
        let canonical_root =
            self.config.root_dir.canonicalize().map_err(|_| {
                Error::Internal("Failed to canonicalize root directory".to_string())
            })?;

        let canonical_path = match full_path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // File doesn't exist, but path might be valid for fallback
                return Ok(full_path);
            }
        };

        if !canonical_path.starts_with(&canonical_root) {
            return Err(Error::Forbidden(
                "Access denied: path traversal attempt".to_string(),
            ));
        }

        Ok(canonical_path)
    }

    /// Generate ETag for a file with compression info
    fn generate_etag_with_compression(
        &self,
        path: &Path,
        metadata: &std::fs::Metadata,
        compression: Option<&CompressionAlgorithm>,
    ) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Hash file path
        path.to_string_lossy().hash(&mut hasher);

        // Hash file size
        metadata.len().hash(&mut hasher);

        // Hash modification time
        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH)
        {
            duration.as_secs().hash(&mut hasher);
        }

        // Hash compression algorithm
        if let Some(algo) = compression {
            algo.to_header_value().hash(&mut hasher);
        }

        format!("\"{}\"", hasher.finish())
    }

    /// Create a 304 Not Modified response
    fn not_modified_response(&self, etag: &str) -> HttpResponse {
        let mut response = HttpResponse::new(304);

        if !etag.is_empty() {
            response
                .headers
                .insert("ETag".to_string(), etag.to_string());
        }

        if self.config.enable_cors {
            let origin = self.config.cors_origin.as_deref().unwrap_or("*");
            response.headers.insert(
                "Access-Control-Allow-Origin".to_string(),
                origin.to_string(),
            );
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_strategy_headers() {
        assert_eq!(
            CacheStrategy::NoCache.to_header_value(),
            "no-cache, no-store, must-revalidate"
        );

        assert_eq!(
            CacheStrategy::Public(Duration::from_secs(3600)).to_header_value(),
            "public, max-age=3600"
        );

        assert_eq!(
            CacheStrategy::Immutable.to_header_value(),
            "public, max-age=31536000, immutable"
        );
    }

    #[test]
    fn test_file_type_detection() {
        assert_eq!(
            FileType::from_path(Path::new("script.js")),
            FileType::JavaScript
        );

        assert_eq!(
            FileType::from_path(Path::new("style.css")),
            FileType::Stylesheet
        );

        assert_eq!(FileType::from_path(Path::new("image.png")), FileType::Image);

        assert_eq!(FileType::from_path(Path::new("font.woff2")), FileType::Font);
    }

    #[test]
    fn test_config_builder() {
        let config = StaticAssetsConfig::new("public")
            .with_default_strategy(CacheStrategy::NoCache)
            .with_etag(true)
            .with_cors_origin("https://example.com")
            .with_compression_enabled(true);

        assert_eq!(config.default_strategy, CacheStrategy::NoCache);
        assert!(config.enable_etag);
        assert_eq!(config.cors_origin, Some("https://example.com".to_string()));
        assert!(config.compression.enabled);
    }

    #[test]
    fn test_spa_mode() {
        let config = StaticAssetsConfig::new("public").spa_mode();

        assert_eq!(config.fallback, Some("index.html".to_string()));
        assert_eq!(
            config.type_strategies.get(&FileType::Html),
            Some(&CacheStrategy::NoCache)
        );
        assert_eq!(
            config.type_strategies.get(&FileType::JavaScript),
            Some(&CacheStrategy::Immutable)
        );
        assert!(config.compression.enabled);
    }

    #[test]
    fn test_compression_algorithm() {
        assert_eq!(CompressionAlgorithm::Gzip.to_header_value(), "gzip");
        assert_eq!(CompressionAlgorithm::Brotli.to_header_value(), "br");
        assert_eq!(CompressionAlgorithm::Zstd.to_header_value(), "zstd");
        assert_eq!(CompressionAlgorithm::Gzip.file_extension(), ".gz");
        assert_eq!(CompressionAlgorithm::Brotli.file_extension(), ".br");
        assert_eq!(CompressionAlgorithm::Zstd.file_extension(), ".zst");
    }

    #[test]
    fn test_compression_level() {
        assert_eq!(CompressionLevel::Fast.brotli_level(), 4);
        assert_eq!(CompressionLevel::Default.brotli_level(), 6);
        assert_eq!(CompressionLevel::Best.brotli_level(), 11);
        assert_eq!(CompressionLevel::Custom(8).brotli_level(), 8);
        assert_eq!(CompressionLevel::Custom(20).brotli_level(), 11); // Capped at 11

        assert_eq!(CompressionLevel::Fast.zstd_level(), 1);
        assert_eq!(CompressionLevel::Default.zstd_level(), 3);
        assert_eq!(CompressionLevel::Best.zstd_level(), 19);
        assert_eq!(CompressionLevel::Custom(8).zstd_level(), 8);
        assert_eq!(CompressionLevel::Custom(50).zstd_level(), 22); // Capped at 22
    }

    #[test]
    fn test_compression_config_should_compress() {
        let config = CompressionConfig::new();

        // Should compress JS/CSS/HTML (compressible types)
        assert!(config.should_compress(FileType::JavaScript, 5000));
        assert!(config.should_compress(FileType::Stylesheet, 5000));
        assert!(config.should_compress(FileType::Html, 5000));

        // Should not compress already compressed types
        assert!(!config.should_compress(FileType::Image, 5000));
        assert!(!config.should_compress(FileType::Font, 5000));
        assert!(!config.should_compress(FileType::Video, 5000));

        // Should not compress files below min size
        assert!(!config.should_compress(FileType::JavaScript, 500)); // < 1KB

        // Should not compress files above max size
        assert!(!config.should_compress(FileType::JavaScript, 20_000_000)); // > 10MB
    }

    #[test]
    fn test_compression_disabled() {
        let config = CompressionConfig::disabled();

        assert!(!config.enabled);
        assert!(!config.should_compress(FileType::JavaScript, 5000));
    }

    #[test]
    fn test_development_mode_disables_compression() {
        let config = StaticAssetsConfig::new("public").development();

        assert!(!config.compression.enabled);
        assert_eq!(config.default_strategy, CacheStrategy::NoCache);
    }

    #[test]
    fn test_max_performance_enables_best_compression() {
        let config = StaticAssetsConfig::new("public").max_performance();

        assert!(config.compression.enabled);
        assert_eq!(config.compression.level, CompressionLevel::Best);
        assert!(config.compression.prefer_brotli);
    }

    // --- Content cache / large-file guard tests --------------------------

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("armature_sa_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, name: &str, contents: &[u8]) {
            std::fs::write(self.path.join(name), contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn get_req(path: &str) -> HttpRequest {
        HttpRequest::new("GET", path.to_string())
    }

    #[tokio::test]
    async fn test_second_request_served_from_cache_with_stable_etag() {
        let dir = TempDir::new();
        dir.write("data.js", &vec![b'a'; 2048]);

        let config = StaticAssetsConfig::new(dir.path.clone());
        let server = StaticAssetServer::new(config).unwrap();

        // First request populates the cache.
        let resp1 = server.serve(&get_req("/data.js")).await.unwrap();
        assert_eq!(resp1.status, 200);
        assert_eq!(resp1.body_ref().len(), 2048);
        let etag1 = resp1.headers.get("ETag").cloned().expect("etag present");

        // The (path, mtime, encoding=None) entry is now cached.
        let cache = server.cache.as_ref().unwrap();
        assert_eq!(cache.lock().len(), 1);

        // Second request: identical body and a *stable* ETag, served from cache.
        let resp2 = server.serve(&get_req("/data.js")).await.unwrap();
        assert_eq!(resp2.status, 200);
        assert_eq!(resp2.body_ref(), resp1.body_ref());
        let etag2 = resp2.headers.get("ETag").cloned().expect("etag present");
        assert_eq!(etag1, etag2, "ETag must be stable across requests");

        // Cache size unchanged (no duplicate entry).
        assert_eq!(cache.lock().len(), 1);

        // Conditional request with matching ETag yields 304 Not Modified.
        let mut cond = get_req("/data.js");
        cond.headers.insert("If-None-Match", etag1.clone());
        let resp304 = server.serve(&cond).await.unwrap();
        assert_eq!(resp304.status, 304);
        assert_eq!(resp304.headers.get("ETag"), Some(&etag1));
    }

    #[tokio::test]
    async fn test_compressed_response_cached_by_encoding() {
        let dir = TempDir::new();
        dir.write("app.js", &vec![b'x'; 4096]);

        let config = StaticAssetsConfig::new(dir.path.clone());
        let server = StaticAssetServer::new(config).unwrap();

        let mut req = get_req("/app.js");
        req.headers.insert("Accept-Encoding", "gzip".to_string());

        let resp1 = server.serve(&req).await.unwrap();
        assert_eq!(resp1.status, 200);
        assert_eq!(
            resp1.headers.get("Content-Encoding"),
            Some(&"gzip".to_string())
        );
        let etag1 = resp1.headers.get("ETag").cloned().unwrap();

        // Cached under the gzip encoding key.
        let cache = server.cache.as_ref().unwrap();
        assert_eq!(cache.lock().len(), 1);

        let resp2 = server.serve(&req).await.unwrap();
        assert_eq!(resp2.body_ref(), resp1.body_ref());
        assert_eq!(resp2.headers.get("ETag"), Some(&etag1));
        assert_eq!(cache.lock().len(), 1);
    }

    #[tokio::test]
    async fn test_large_file_not_cached() {
        let dir = TempDir::new();
        dir.write("big.bin", &vec![0u8; 1024]);

        // max_serve_size below the file size => never cached.
        let config = StaticAssetsConfig::new(dir.path.clone()).with_max_serve_size(10);
        let server = StaticAssetServer::new(config).unwrap();

        let resp = server.serve(&get_req("/big.bin")).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body_ref().len(), 1024);

        // Nothing cached for oversized files.
        let cache = server.cache.as_ref().unwrap();
        assert_eq!(cache.lock().len(), 0);
    }

    #[test]
    fn test_compress_content_round_trip_all_algorithms() {
        let dir = TempDir::new();
        let config = StaticAssetsConfig::new(dir.path.clone());
        let server = StaticAssetServer::new(config).unwrap();

        let data = b"Hello, World! This is a test string for compression.".repeat(20);

        // Gzip round-trip.
        let gz = server
            .compress_content(&data, CompressionAlgorithm::Gzip)
            .unwrap();
        assert_ne!(gz, data);
        let mut gz_decoder = flate2::read::GzDecoder::new(&gz[..]);
        let mut gz_decompressed = Vec::new();
        std::io::Read::read_to_end(&mut gz_decoder, &mut gz_decompressed).unwrap();
        assert_eq!(gz_decompressed, data);

        // Brotli round-trip.
        let br = server
            .compress_content(&data, CompressionAlgorithm::Brotli)
            .unwrap();
        assert_ne!(br, data);
        let mut br_decompressed = Vec::new();
        brotli::BrotliDecompress(&mut std::io::Cursor::new(&br), &mut br_decompressed).unwrap();
        assert_eq!(br_decompressed, data);

        // Zstd round-trip.
        let zst = server
            .compress_content(&data, CompressionAlgorithm::Zstd)
            .unwrap();
        assert_ne!(zst, data);
        let zst_decompressed = zstd::decode_all(std::io::Cursor::new(&zst)).unwrap();
        assert_eq!(zst_decompressed, data);
    }

    #[tokio::test]
    async fn test_zstd_compressed_response_cached_by_encoding() {
        let dir = TempDir::new();
        dir.write("app.js", &vec![b'x'; 4096]);

        let config = StaticAssetsConfig::new(dir.path.clone());
        let server = StaticAssetServer::new(config).unwrap();

        let mut req = get_req("/app.js");
        req.headers.insert("Accept-Encoding", "zstd".to_string());

        let resp1 = server.serve(&req).await.unwrap();
        assert_eq!(resp1.status, 200);
        assert_eq!(
            resp1.headers.get("Content-Encoding"),
            Some(&"zstd".to_string())
        );

        // The body actually decompresses back to the original content, not
        // just a correctly-labeled but bogus encoding.
        let decompressed = zstd::decode_all(std::io::Cursor::new(resp1.body_ref())).unwrap();
        assert_eq!(decompressed, vec![b'x'; 4096]);

        let etag1 = resp1.headers.get("ETag").cloned().unwrap();

        // Cached under the zstd encoding key.
        let cache = server.cache.as_ref().unwrap();
        assert_eq!(cache.lock().len(), 1);

        let resp2 = server.serve(&req).await.unwrap();
        assert_eq!(resp2.body_ref(), resp1.body_ref());
        assert_eq!(resp2.headers.get("ETag"), Some(&etag1));
        assert_eq!(cache.lock().len(), 1);
    }

    #[tokio::test]
    async fn test_cache_capacity_zero_disables_cache() {
        let dir = TempDir::new();
        dir.write("data.txt", b"hello world");

        let config = StaticAssetsConfig::new(dir.path.clone()).with_cache_capacity(0);
        let server = StaticAssetServer::new(config).unwrap();

        let resp = server.serve(&get_req("/data.txt")).await.unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body_ref(), b"hello world");
        assert!(server.cache.is_none());
    }
}
