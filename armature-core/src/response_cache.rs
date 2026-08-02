//! HTTP Response Caching
//!
//! This module provides comprehensive HTTP response caching support including:
//!
//! - `Cache-Control` header parsing and generation
//! - In-memory response caching with TTL
//! - Cache key generation from requests
//! - Vary header support
//! - Cache invalidation
//!
//! # Examples
//!
//! ## Cache-Control Headers
//!
//! ```
//! use armature_core::response_cache::{CacheControl, CacheDirective};
//! use std::time::Duration;
//!
//! // Create Cache-Control header
//! let cache_control = CacheControl::new()
//!     .public()
//!     .max_age(Duration::from_secs(3600))
//!     .must_revalidate();
//!
//! assert_eq!(cache_control.to_header_value(), "public, max-age=3600, must-revalidate");
//! ```
//!
//! ## Response Caching
//!
//! ```ignore
//! use armature_core::response_cache::{ResponseCache, CacheControl};
//!
//! let cache = ResponseCache::new();
//!
//! // Cache a response
//! cache.store(&request, &response).await;
//!
//! // Retrieve cached response
//! if let Some(cached) = cache.get(&request).await {
//!     return Ok(cached);
//! }
//! ```

use crate::{HttpRequest, HttpResponse};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::RwLock;

// ============================================================================
// Cache Directives
// ============================================================================

/// Individual cache directive from Cache-Control header.
#[derive(Debug, Clone, PartialEq)]
pub enum CacheDirective {
    /// Response may be cached by any cache
    Public,
    /// Response is for a single user and must not be stored by shared caches
    Private,
    /// Response must not be stored in any cache
    NoStore,
    /// Response can be stored but must be validated before use
    NoCache,
    /// Maximum time the response is fresh (in seconds)
    MaxAge(u64),
    /// Maximum time a shared cache may store the response (in seconds)
    SMaxAge(u64),
    /// Response must be revalidated after becoming stale
    MustRevalidate,
    /// Shared caches must revalidate after becoming stale
    ProxyRevalidate,
    /// Response must not be transformed (e.g., compressed)
    NoTransform,
    /// Response is immutable and won't change
    Immutable,
    /// Client will accept stale response up to N seconds
    MaxStale(Option<u64>),
    /// Client wants response fresh for at least N seconds
    MinFresh(u64),
    /// Client will only accept cached response
    OnlyIfCached,
    /// Custom/unknown directive
    Extension(String, Option<String>),
}

impl CacheDirective {
    /// Parse a single directive from a string.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_lowercase();

        // Check for directives with values
        if let Some((key, value)) = s.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');

            return match key {
                "max-age" => value.parse().ok().map(CacheDirective::MaxAge),
                "s-maxage" => value.parse().ok().map(CacheDirective::SMaxAge),
                "max-stale" => Some(CacheDirective::MaxStale(value.parse().ok())),
                "min-fresh" => value.parse().ok().map(CacheDirective::MinFresh),
                _ => Some(CacheDirective::Extension(
                    key.to_string(),
                    Some(value.to_string()),
                )),
            };
        }

        // Simple directives
        match s.as_str() {
            "public" => Some(CacheDirective::Public),
            "private" => Some(CacheDirective::Private),
            "no-store" => Some(CacheDirective::NoStore),
            "no-cache" => Some(CacheDirective::NoCache),
            "must-revalidate" => Some(CacheDirective::MustRevalidate),
            "proxy-revalidate" => Some(CacheDirective::ProxyRevalidate),
            "no-transform" => Some(CacheDirective::NoTransform),
            "immutable" => Some(CacheDirective::Immutable),
            "max-stale" => Some(CacheDirective::MaxStale(None)),
            "only-if-cached" => Some(CacheDirective::OnlyIfCached),
            _ => Some(CacheDirective::Extension(s, None)),
        }
    }

    /// Convert directive to header value string.
    pub fn to_header_value(&self) -> String {
        match self {
            CacheDirective::Public => "public".to_string(),
            CacheDirective::Private => "private".to_string(),
            CacheDirective::NoStore => "no-store".to_string(),
            CacheDirective::NoCache => "no-cache".to_string(),
            CacheDirective::MaxAge(secs) => format!("max-age={}", secs),
            CacheDirective::SMaxAge(secs) => format!("s-maxage={}", secs),
            CacheDirective::MustRevalidate => "must-revalidate".to_string(),
            CacheDirective::ProxyRevalidate => "proxy-revalidate".to_string(),
            CacheDirective::NoTransform => "no-transform".to_string(),
            CacheDirective::Immutable => "immutable".to_string(),
            CacheDirective::MaxStale(Some(secs)) => format!("max-stale={}", secs),
            CacheDirective::MaxStale(None) => "max-stale".to_string(),
            CacheDirective::MinFresh(secs) => format!("min-fresh={}", secs),
            CacheDirective::OnlyIfCached => "only-if-cached".to_string(),
            CacheDirective::Extension(key, Some(value)) => format!("{}={}", key, value),
            CacheDirective::Extension(key, None) => key.clone(),
        }
    }
}

// ============================================================================
// Cache-Control Header
// ============================================================================

/// Parsed or constructed Cache-Control header.
///
/// # Examples
///
/// ## Parsing
///
/// ```
/// use armature_core::response_cache::CacheControl;
///
/// let cc = CacheControl::parse("public, max-age=3600, must-revalidate");
/// assert!(cc.is_public());
/// assert_eq!(cc.get_max_age(), Some(3600));
/// ```
///
/// ## Building
///
/// ```
/// use armature_core::response_cache::CacheControl;
/// use std::time::Duration;
///
/// let cc = CacheControl::new()
///     .private()
///     .max_age(Duration::from_secs(300))
///     .no_transform();
///
/// assert!(cc.is_private());
/// ```
#[derive(Debug, Clone, Default)]
pub struct CacheControl {
    /// All directives in this Cache-Control header
    pub directives: Vec<CacheDirective>,
}

impl CacheControl {
    /// Create a new empty Cache-Control.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a Cache-Control header value.
    pub fn parse(header: &str) -> Self {
        let directives: Vec<CacheDirective> = header
            .split(',')
            .filter_map(|s| CacheDirective::parse(s.trim()))
            .collect();

        Self { directives }
    }

    /// Convert to header value string.
    pub fn to_header_value(&self) -> String {
        self.directives
            .iter()
            .map(|d| d.to_header_value())
            .collect::<Vec<_>>()
            .join(", ")
    }

    // ==================== Builder Methods ====================

    /// Add the `public` directive.
    pub fn public(mut self) -> Self {
        self.directives.push(CacheDirective::Public);
        self
    }

    /// Add the `private` directive.
    pub fn private(mut self) -> Self {
        self.directives.push(CacheDirective::Private);
        self
    }

    /// Add the `no-store` directive.
    pub fn no_store(mut self) -> Self {
        self.directives.push(CacheDirective::NoStore);
        self
    }

    /// Add the `no-cache` directive.
    pub fn no_cache(mut self) -> Self {
        self.directives.push(CacheDirective::NoCache);
        self
    }

    /// Add the `max-age` directive.
    pub fn max_age(mut self, duration: Duration) -> Self {
        self.directives
            .push(CacheDirective::MaxAge(duration.as_secs()));
        self
    }

    /// Add the `s-maxage` directive for shared caches.
    pub fn s_maxage(mut self, duration: Duration) -> Self {
        self.directives
            .push(CacheDirective::SMaxAge(duration.as_secs()));
        self
    }

    /// Add the `must-revalidate` directive.
    pub fn must_revalidate(mut self) -> Self {
        self.directives.push(CacheDirective::MustRevalidate);
        self
    }

    /// Add the `proxy-revalidate` directive.
    pub fn proxy_revalidate(mut self) -> Self {
        self.directives.push(CacheDirective::ProxyRevalidate);
        self
    }

    /// Add the `no-transform` directive.
    pub fn no_transform(mut self) -> Self {
        self.directives.push(CacheDirective::NoTransform);
        self
    }

    /// Add the `immutable` directive.
    pub fn immutable(mut self) -> Self {
        self.directives.push(CacheDirective::Immutable);
        self
    }

    /// Add a custom directive.
    pub fn directive(mut self, directive: CacheDirective) -> Self {
        self.directives.push(directive);
        self
    }

    // ==================== Query Methods ====================

    /// Check if `public` directive is present.
    pub fn is_public(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, CacheDirective::Public))
    }

    /// Check if `private` directive is present.
    pub fn is_private(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, CacheDirective::Private))
    }

    /// Check if `no-store` directive is present.
    pub fn is_no_store(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, CacheDirective::NoStore))
    }

    /// Check if `no-cache` directive is present.
    pub fn is_no_cache(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, CacheDirective::NoCache))
    }

    /// Check if `must-revalidate` directive is present.
    pub fn is_must_revalidate(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, CacheDirective::MustRevalidate))
    }

    /// Check if `immutable` directive is present.
    pub fn is_immutable(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, CacheDirective::Immutable))
    }

    /// Get the `max-age` value in seconds.
    pub fn get_max_age(&self) -> Option<u64> {
        self.directives.iter().find_map(|d| match d {
            CacheDirective::MaxAge(secs) => Some(*secs),
            _ => None,
        })
    }

    /// Get the `s-maxage` value in seconds.
    pub fn get_s_maxage(&self) -> Option<u64> {
        self.directives.iter().find_map(|d| match d {
            CacheDirective::SMaxAge(secs) => Some(*secs),
            _ => None,
        })
    }

    /// Check if the response is cacheable.
    pub fn is_cacheable(&self) -> bool {
        // Not cacheable if no-store is present
        if self.is_no_store() {
            return false;
        }

        // Cacheable if public, private, or has max-age/s-maxage
        self.is_public()
            || self.is_private()
            || self.get_max_age().is_some()
            || self.get_s_maxage().is_some()
    }

    /// Get the freshness lifetime in seconds.
    ///
    /// Returns s-maxage if present (for shared caches), otherwise max-age.
    pub fn freshness_lifetime(&self) -> Option<u64> {
        self.get_s_maxage().or_else(|| self.get_max_age())
    }

    // ==================== Preset Configurations ====================

    /// Create a "no-store" Cache-Control (never cache).
    pub fn never() -> Self {
        Self::new().no_store().no_cache()
    }

    /// Create a public cache with the given max-age.
    pub fn public_max_age(duration: Duration) -> Self {
        Self::new().public().max_age(duration)
    }

    /// Create a private cache with the given max-age.
    pub fn private_max_age(duration: Duration) -> Self {
        Self::new().private().max_age(duration)
    }

    /// Create an immutable public cache (for versioned assets).
    pub fn immutable_asset(duration: Duration) -> Self {
        Self::new().public().max_age(duration).immutable()
    }

    /// Create a must-revalidate cache.
    pub fn revalidate(duration: Duration) -> Self {
        Self::new().public().max_age(duration).must_revalidate()
    }
}

impl fmt::Display for CacheControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_header_value())
    }
}

// ============================================================================
// Cache Key
// ============================================================================

/// Cache key for HTTP responses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    /// HTTP method
    pub method: crate::Method,
    /// Request path
    pub path: crate::ByteStr,
    /// Query string (sorted for consistency)
    pub query: String,
    /// Vary header values that affect caching
    pub vary_values: Vec<(String, String)>,
    /// Hash of the request body, for methods where the body identifies the
    /// resource being queried (QUERY, draft-ietf-httpbis-safe-method-w-body).
    /// `None` for methods whose body does not participate in the cache key.
    pub body_hash: Option<u64>,
}

impl CacheKey {
    /// Generate a cache key from a request.
    pub fn from_request(request: &HttpRequest) -> Self {
        Self::from_request_with_vary(request, &[])
    }

    /// Generate a cache key from a request with Vary headers.
    pub fn from_request_with_vary(request: &HttpRequest, vary_headers: &[&str]) -> Self {
        // Sort query params for consistent keys
        let mut query_params: Vec<_> = request.query().iter().collect();
        query_params.sort_by(|a, b| a.0.cmp(b.0));
        let query = query_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        // Collect Vary header values
        let mut vary_values: Vec<(String, String)> = vary_headers
            .iter()
            .filter_map(|header| {
                // One lookup: header names intern case-insensitively, so the
                // lowercased retry was always redundant.
                request
                    .headers
                    .get(header)
                    .map(|v| (header.to_lowercase(), v.to_owned()))
            })
            .collect();
        vary_values.sort_by(|a, b| a.0.cmp(&b.0));

        let method = request.method.clone();

        // For QUERY the request body *is* the query, so two requests with
        // different bodies are different cache entries.
        let body_hash = if method == "QUERY" {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::hash::DefaultHasher::new();
            request.body_bytes().hash(&mut hasher);
            Some(hasher.finish())
        } else {
            None
        };

        Self {
            method,
            // The path alone: `query` is a separate, sorted field, so folding
            // the raw query in here would key the same request two ways.
            path: crate::ByteStr::from(request.path_only()),
            query,
            vary_values,
            body_hash,
        }
    }

    /// Convert to a string representation suitable for use as a cache key.
    pub fn to_string_key(&self) -> String {
        let vary_str = if self.vary_values.is_empty() {
            String::new()
        } else {
            format!(
                "|{}",
                self.vary_values
                    .iter()
                    .map(|(k, v)| format!("{}:{}", k, v))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };

        let body_str = self
            .body_hash
            .map(|h| format!("|body:{:016x}", h))
            .unwrap_or_default();

        if self.query.is_empty() {
            format!("{}:{}{}{}", self.method, self.path, body_str, vary_str)
        } else {
            format!(
                "{}:{}?{}{}{}",
                self.method, self.path, self.query, body_str, vary_str
            )
        }
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_key())
    }
}

// ============================================================================
// Cached Response
// ============================================================================

/// A cached HTTP response with metadata.
#[derive(Debug, Clone)]
pub struct CachedResponse {
    /// The cached response
    pub response: CachedResponseData,
    /// When the response was cached
    pub cached_at: Instant,
    /// When the response expires
    pub expires_at: Instant,
    /// ETag of the cached response
    pub etag: Option<String>,
    /// Last-Modified timestamp
    pub last_modified: Option<SystemTime>,
    /// Vary headers that affect this cache entry
    pub vary: Vec<String>,
    /// The base cache key (no Vary values) this entry was stored under.
    ///
    /// Recorded by [`ResponseCache::store_with_ttl`] so that eviction and TTL
    /// purging can keep the `vary_index` in lockstep with `entries`: when the
    /// last variant sharing a base key is removed, its `vary_index` entry can
    /// be removed too. `None` for entries created outside the cache (e.g. via
    /// [`CachedResponse::new`] directly), which are never tracked there.
    pub(crate) base_key: Option<String>,
    /// Monotonic insertion sequence used by the [`EvictionIndex`] to identify
    /// this entry's *current* position in `order`. Each store assigns a fresh
    /// value (see [`ResponseCache::store_with_ttl`]); a re-store (refresh)
    /// therefore bumps it, so eviction and compaction can tell a live entry's
    /// current `order` position from a stale duplicate left by an earlier
    /// insert. `0` for entries created outside the cache, which never enter the
    /// eviction index.
    pub(crate) eviction_seq: u64,
}

/// The actual cached response data.
#[derive(Debug, Clone)]
pub struct CachedResponseData {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: bytes::Bytes,
}

impl CachedResponse {
    /// Create a new cached response.
    pub fn new(response: &HttpResponse, ttl: Duration) -> Self {
        let now = Instant::now();

        let etag = response.headers.get("ETag").cloned();
        let last_modified = response
            .headers
            .get("Last-Modified")
            .and_then(|s| httpdate::parse_http_date(s).ok());
        let vary = response
            .headers
            .get("Vary")
            .map(|v| v.split(',').map(|s| s.trim().to_lowercase()).collect())
            .unwrap_or_default();

        Self {
            response: CachedResponseData {
                status: response.status,
                headers: response.headers.clone().into(),
                body: response.body.clone(),
            },
            cached_at: now,
            expires_at: now + ttl,
            etag,
            last_modified,
            vary,
            base_key: None,
            eviction_seq: 0,
        }
    }

    /// Check if the cached response is still fresh.
    pub fn is_fresh(&self) -> bool {
        Instant::now() < self.expires_at
    }

    /// Check if the cached response is stale.
    pub fn is_stale(&self) -> bool {
        !self.is_fresh()
    }

    /// Get the age of the cached response.
    pub fn age(&self) -> Duration {
        self.cached_at.elapsed()
    }

    /// Get the remaining TTL.
    pub fn remaining_ttl(&self) -> Option<Duration> {
        let now = Instant::now();
        if now < self.expires_at {
            Some(self.expires_at - now)
        } else {
            None
        }
    }

    /// Convert to an HttpResponse.
    pub fn to_response(&self) -> HttpResponse {
        // `with_bytes_body` rather than `from_parts`, so serving a cache hit is
        // a refcount bump on the stored body rather than a copy of it.
        let mut response = HttpResponse::from_parts(
            self.response.status,
            self.response.headers.clone(),
            Vec::new(),
        )
        .with_bytes_body(self.response.body.clone());

        // Add Age header
        response
            .headers
            .insert("Age".to_string(), self.age().as_secs().to_string());

        // Add X-Cache header
        response
            .headers
            .insert("X-Cache".to_string(), "HIT".to_string());

        response
    }
}

// ============================================================================
// In-Memory Response Cache
// ============================================================================

/// In-memory HTTP response cache.
///
/// # Examples
///
/// ```
/// use armature_core::response_cache::{ResponseCache, ResponseCacheConfig};
/// use std::time::Duration;
///
/// let cache = ResponseCache::new();
///
/// // Configure cache
/// let cache = ResponseCache::with_config(
///     ResponseCacheConfig::new()
///         .max_entries(1000)
///         .default_ttl(Duration::from_secs(300))
///         .max_body_size(1024 * 1024), // 1MB
/// );
/// ```
#[derive(Debug)]
pub struct ResponseCache {
    /// Cache configuration
    config: ResponseCacheConfig,
    /// Cached responses
    entries: Arc<RwLock<HashMap<String, CachedResponse>>>,
    /// Maps base cache keys (no Vary values) to the Vary header names the
    /// stored response was keyed with, so `get` can rebuild the full key
    /// from a request and `invalidate` can find all variants.
    vary_index: Arc<RwLock<HashMap<String, Vec<String>>>>,
    /// Insertion-order eviction bookkeeping, guarded independently but only ever
    /// mutated while holding the `entries` write lock (so the two stay
    /// consistent). See [`EvictionIndex`].
    eviction: Mutex<EvictionIndex>,
}

/// Insertion-order eviction bookkeeping kept in lockstep with `entries`.
///
/// `order` holds variant cache keys oldest-first. It is maintained *lazily*:
/// keys removed by invalidation or TTL purging are left in place (as
/// tombstones) and skipped when they surface during capacity-driven eviction,
/// so finding the oldest live entry is O(1) amortized instead of an O(n)
/// `min_by_key` scan of the whole map on every insert once the cache is full.
///
/// Two things leave tombstones in `order` without ever going through
/// `evict_oldest`, which is the only place that used to clean them up:
///
/// 1. Keys removed by `purge_stale`/`invalidate`/`invalidate_prefix`/expiry —
///    in the common regime where capacity is never hit, `evict_oldest` never
///    runs at all, so these tombstones would otherwise accumulate forever.
/// 2. Re-inserting an already-live key pushes a *second* `order` entry for the
///    same key (`record_insert`'s `replaced` branch only suppresses the
///    `base_key_counts` increment, not the push), so a single hot key
///    re-stored repeatedly (e.g. every TTL cycle) would also grow `order`
///    without bound.
///
/// Each `order` element is a `(key, seq)` pair where `seq` is a monotonic
/// counter assigned at insert time and mirrored onto the entry's
/// [`CachedResponse::eviction_seq`]. An `order` position is the key's *current*
/// one only when its `seq` matches the live entry's; an older `seq` marks a
/// stale duplicate left by a refresh. This lets a re-store reset eviction
/// recency: `evict_oldest` skips a popped position whose `seq` no longer
/// matches (rather than evicting the refreshed entry by its original insert
/// time), and `compact` keeps only the current-`seq` position of each key.
///
/// `dead` counts how many `order` entries no longer represent a live
/// position (tombstones from removal, or superseded duplicates from a
/// re-insert). Whenever `dead` exceeds the number of live entries,
/// `compact_if_needed` rebuilds `order` from scratch in a single pass over
/// the live entries, dropping the tombstones/duplicates and resetting `dead`
/// to 0. This keeps `order` bounded to roughly `O(live entries)` regardless
/// of how much churn (invalidation or re-insertion) has occurred, while still
/// keeping the common insert/evict path O(1) amortized.
///
/// `base_key_counts` tracks how many live variants share each base key. It lets
/// eviction decide in O(1) whether a base key's `vary_index` record is now dead
/// (its last variant was just evicted) instead of a second O(n) scan over every
/// entry's `base_key`.
#[derive(Debug, Default)]
struct EvictionIndex {
    order: VecDeque<(String, u64)>,
    base_key_counts: HashMap<String, usize>,
    /// Number of `order` entries that are tombstones (their key was removed
    /// from `entries` elsewhere) or stale duplicates (superseded by a later
    /// re-insert of the same key). See the struct docs.
    dead: usize,
    /// Monotonic source for per-insert sequence numbers. Never reset (not even
    /// by `clear`), so a sequence value is never reused for a different insert.
    next_seq: u64,
}

impl EvictionIndex {
    /// Allocate the next monotonic insertion sequence. The caller stamps it onto
    /// both the new `order` position (via [`record_insert`]) and the stored
    /// [`CachedResponse::eviction_seq`], so the two can later be cross-checked.
    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Record a freshly inserted variant key at sequence `seq`. `replaced` is
    /// true when an entry already existed under `key`, so its base key must not
    /// be double-counted — and the *previous* `order` entry for `key` becomes a
    /// dead duplicate, since the entry now carries the newer `seq`.
    fn record_insert(&mut self, key: &str, seq: u64, base_key: &str, replaced: bool) {
        self.order.push_back((key.to_string(), seq));
        if replaced {
            self.dead += 1;
        } else {
            *self
                .base_key_counts
                .entry(base_key.to_string())
                .or_insert(0) += 1;
        }
    }

    /// Record removal of one variant with the given base key. Returns true when
    /// that was the last live variant for the base key (its `vary_index` record
    /// is now dead and should be pruned).
    ///
    /// Callers that remove an entry from `entries` outside of `evict_oldest`
    /// (invalidation, prefix invalidation, TTL purging) must also bump `dead`
    /// themselves — the removed key's `order` entry is left behind as a
    /// tombstone, and this method only tracks `base_key_counts`.
    fn record_remove(&mut self, base_key: &str) -> bool {
        if let Some(count) = self.base_key_counts.get_mut(base_key) {
            *count -= 1;
            if *count == 0 {
                self.base_key_counts.remove(base_key);
                return true;
            }
        }
        false
    }

    /// Whether any live variant still references `base_key`.
    fn base_key_live(&self, base_key: &str) -> bool {
        self.base_key_counts.contains_key(base_key)
    }

    /// Rebuild `order` from `entries`, dropping tombstones (keys no longer
    /// present) and stale duplicates left by re-inserting an already-live key
    /// (see `record_insert`). A position is kept only when its `seq` matches the
    /// live entry's `eviction_seq` — i.e. it is the key's *current* position —
    /// so a refreshed entry keeps its newer (later) position and its eviction
    /// recency, rather than reverting to the original insert time. Single
    /// forward pass, preserving relative order; the `seen` guard is defensive
    /// (a matching `seq` is already unique per key).
    fn compact(&mut self, entries: &HashMap<String, CachedResponse>) {
        let mut seen = HashSet::with_capacity(entries.len());
        let mut compacted = VecDeque::with_capacity(entries.len());
        for (key, seq) in self.order.drain(..) {
            if entries.get(&key).is_some_and(|e| e.eviction_seq == seq) && seen.insert(key.clone())
            {
                compacted.push_back((key, seq));
            }
        }
        self.order = compacted;
        self.dead = 0;
    }

    /// Compact `order` when dead (tombstoned or superseded) entries outnumber
    /// live ones. Keeps `order` from growing without bound under
    /// invalidation/re-insertion churn even when capacity is never hit (so
    /// `evict_oldest`, the only other place that trims `order`, never runs).
    fn compact_if_needed(&mut self, entries: &HashMap<String, CachedResponse>) {
        if self.dead > entries.len() {
            self.compact(entries);
        }
    }

    fn clear(&mut self) {
        self.order.clear();
        self.base_key_counts.clear();
        self.dead = 0;
    }
}

/// Configuration for the response cache.
#[derive(Debug, Clone)]
pub struct ResponseCacheConfig {
    /// Maximum number of entries in the cache
    pub max_entries: usize,
    /// Default TTL for cached responses
    pub default_ttl: Duration,
    /// Maximum body size to cache (in bytes)
    pub max_body_size: usize,
    /// Only cache responses with these status codes
    pub cacheable_status_codes: Vec<u16>,
    /// Only cache these HTTP methods
    pub cacheable_methods: Vec<String>,
}

impl Default for ResponseCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            default_ttl: Duration::from_secs(300), // 5 minutes
            max_body_size: 1024 * 1024,            // 1MB
            cacheable_status_codes: vec![200, 203, 204, 206, 300, 301, 404, 405, 410, 414, 501],
            cacheable_methods: vec![
                "GET".to_string(),
                "HEAD".to_string(),
                // Safe query with a request body; the body participates in
                // the cache key (draft-ietf-httpbis-safe-method-w-body §4).
                "QUERY".to_string(),
            ],
        }
    }
}

impl ResponseCacheConfig {
    /// Create a new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum number of entries.
    pub fn max_entries(mut self, count: usize) -> Self {
        self.max_entries = count;
        self
    }

    /// Set the default TTL.
    pub fn default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl;
        self
    }

    /// Set the maximum body size to cache.
    pub fn max_body_size(mut self, size: usize) -> Self {
        self.max_body_size = size;
        self
    }
}

impl ResponseCache {
    /// Create a new response cache with default configuration.
    pub fn new() -> Self {
        Self::with_config(ResponseCacheConfig::default())
    }

    /// Create a new response cache with custom configuration.
    pub fn with_config(config: ResponseCacheConfig) -> Self {
        Self {
            config,
            entries: Arc::new(RwLock::new(HashMap::new())),
            vary_index: Arc::new(RwLock::new(HashMap::new())),
            eviction: Mutex::new(EvictionIndex::default()),
        }
    }

    /// Get a cached response for a request.
    ///
    /// Uses a two-phase lookup: the Vary header names recorded when the
    /// response was stored are looked up by base key first, then used to
    /// build the full (Vary-aware) cache key from the request's headers.
    pub async fn get(&self, request: &HttpRequest) -> Option<HttpResponse> {
        let base_key = CacheKey::from_request(request).to_string_key();
        let vary_headers = {
            let vary_index = self.vary_index.read().await;
            vary_index.get(&base_key).cloned().unwrap_or_default()
        };
        let vary_refs: Vec<&str> = vary_headers.iter().map(String::as_str).collect();
        self.get_with_vary(request, &vary_refs).await
    }

    /// Get a cached response with Vary header support.
    pub async fn get_with_vary(
        &self,
        request: &HttpRequest,
        vary_headers: &[&str],
    ) -> Option<HttpResponse> {
        let key = CacheKey::from_request_with_vary(request, vary_headers);
        let key_str = key.to_string_key();

        let entries = self.entries.read().await;
        if let Some(cached) = entries.get(&key_str)
            && cached.is_fresh()
        {
            return Some(cached.to_response());
        }
        None
    }

    /// Store a response in the cache.
    ///
    /// The TTL is derived from the response's `Cache-Control` header
    /// (`s-maxage` takes precedence over `max-age`) when present, falling
    /// back to the configured default TTL.
    pub async fn store(&self, request: &HttpRequest, response: &HttpResponse) {
        let ttl = response
            .headers
            .get("Cache-Control")
            .map(|h| CacheControl::parse(h))
            .and_then(|cc| cc.freshness_lifetime())
            .map(Duration::from_secs)
            .unwrap_or(self.config.default_ttl);

        self.store_with_ttl(request, response, ttl).await
    }

    /// Store a response with a specific TTL.
    pub async fn store_with_ttl(
        &self,
        request: &HttpRequest,
        response: &HttpResponse,
        ttl: Duration,
    ) {
        // Check if cacheable
        if !self.is_cacheable(request, response) {
            return;
        }

        // Get Vary headers from response
        let vary_headers: Vec<&str> = response
            .headers
            .get("Vary")
            .map(|v| v.split(',').map(|s| s.trim()).collect())
            .unwrap_or_default();

        let key = CacheKey::from_request_with_vary(request, &vary_headers);
        let key_str = key.to_string_key();
        let base_key = CacheKey::from_request(request).to_string_key();
        let mut cached = CachedResponse::new(response, ttl);
        cached.base_key = Some(base_key.clone());

        // Base key of any entry evicted to make room, but only when that was
        // the last variant sharing it (so its `vary_index` entry is now dead).
        let mut evicted_base_key = None;
        {
            let mut entries = self.entries.write().await;

            // Evict if at capacity
            if entries.len() >= self.config.max_entries {
                evicted_base_key = self.evict_oldest(&mut entries);
            }

            // Track insertion order and base-key multiplicity so eviction stays
            // O(1) amortized (see `EvictionIndex`), then compact away any
            // tombstones/duplicates that have accumulated in `order`. The
            // sequence is stamped onto the entry *before* it is inserted so the
            // eviction index can later match this `order` position to the live
            // entry (and tell it apart from a stale duplicate after a refresh).
            let mut index = self.eviction.lock().unwrap();
            let seq = index.next_seq();
            cached.eviction_seq = seq;
            let replaced = entries.insert(key_str.clone(), cached).is_some();
            index.record_insert(&key_str, seq, &base_key, replaced);
            index.compact_if_needed(&entries);
        }

        // Keep `vary_index` in lockstep with `entries`.
        let mut vary_index = self.vary_index.write().await;

        // Drop the evicted base key's Vary record, unless the entry we just
        // stored shares that base key (and therefore keeps it alive).
        if let Some(evicted) = evicted_base_key
            && evicted != base_key
        {
            vary_index.remove(&evicted);
        }

        // Record the Vary header names for this base key so `get` can rebuild
        // the full key from a future request's headers. Merge (union) rather
        // than overwrite so distinct Vary sets stored at the same base key
        // remain reachable instead of being clobbered last-writer-wins.
        let entry = vary_index.entry(base_key).or_default();
        for header in vary_headers.iter().map(|s| s.to_string()) {
            if !entry.contains(&header) {
                entry.push(header);
            }
        }
    }

    /// Check if a request/response pair is cacheable.
    fn is_cacheable(&self, request: &HttpRequest, response: &HttpResponse) -> bool {
        // Check method
        if !self
            .config
            .cacheable_methods
            .iter()
            .any(|m| m == request.method_str())
        {
            return false;
        }

        // Check status code
        if !self
            .config
            .cacheable_status_codes
            .contains(&response.status)
        {
            return false;
        }

        // Check body size
        if response.body.len() > self.config.max_body_size {
            return false;
        }

        // Check response Cache-Control (RFC 9111): this is a shared cache,
        // so no-store, private, and no-cache responses must not be stored.
        let cache_control = response
            .headers
            .get("Cache-Control")
            .map(|h| CacheControl::parse(h));
        if let Some(ref cc) = cache_control
            && (cc.is_no_store() || cc.is_private() || cc.is_no_cache())
        {
            return false;
        }

        // RFC 9111 §3.5: responses to requests with an Authorization header
        // must not be stored in a shared cache unless the response explicitly
        // allows it (public, s-maxage, or must-revalidate).
        let has_authorization = request
            .headers
            .get("Authorization")
            .or_else(|| request.headers.get("authorization"))
            .is_some();
        if has_authorization {
            let explicitly_allowed = cache_control.as_ref().is_some_and(|cc| {
                cc.is_public() || cc.get_s_maxage().is_some() || cc.is_must_revalidate()
            });
            if !explicitly_allowed {
                return false;
            }
        }

        true
    }

    /// Evict the oldest entry from the cache.
    ///
    /// Returns the evicted entry's base key when, after removal, no remaining
    /// entry shares that base key — signalling to the caller that the matching
    /// `vary_index` record is now dead and should be removed too. Returns
    /// `None` when the base key is still in use (another Vary variant remains)
    /// or the entry carried no tracked base key.
    fn evict_oldest(&self, entries: &mut HashMap<String, CachedResponse>) -> Option<String> {
        let mut index = self.eviction.lock().unwrap();

        // Pop insertion-ordered positions, skipping any that are no longer the
        // key's *current* position, until we reach the oldest live entry. This
        // replaces the previous O(n) `min_by_key` scan on the insert path.
        //
        // A popped `(key, seq)` is the genuine oldest live entry only when the
        // live entry's `eviction_seq` still equals `seq`. Otherwise it is either
        // a tombstone (key removed elsewhere) or a stale duplicate superseded by
        // a later re-store (refresh) — in which case the refreshed entry has a
        // newer position further back in `order`, so evicting here would wrongly
        // drop a recently-touched entry. Such positions are simply skipped.
        while let Some((oldest_key, seq)) = index.order.pop_front() {
            let is_current = entries
                .get(&oldest_key)
                .is_some_and(|e| e.eviction_seq == seq);
            if is_current {
                let removed = entries.remove(&oldest_key);
                // Report the base key as dead only once its last variant is gone,
                // decided in O(1) from the tracked counts rather than a second
                // O(n) scan of every entry.
                return match removed.and_then(|r| r.base_key) {
                    Some(base_key) => index.record_remove(&base_key).then_some(base_key),
                    None => None,
                };
            }
            // Tombstone or stale duplicate — it is leaving `order` right now, so
            // it stops counting as dead.
            index.dead = index.dead.saturating_sub(1);
        }
        None
    }

    /// Remove a specific entry from the cache, including all Vary variants.
    pub async fn invalidate(&self, request: &HttpRequest) {
        let base_key = CacheKey::from_request(request).to_string_key();
        // Variant keys are the base key followed by a `|`-separated list of
        // Vary header values (see `CacheKey::to_string_key`).
        let variant_prefix = format!("{}|", base_key);

        {
            let mut entries = self.entries.write().await;
            let mut removed_count = 0usize;
            entries.retain(|key, _| {
                if key == &base_key || key.starts_with(&variant_prefix) {
                    removed_count += 1;
                    false
                } else {
                    true
                }
            });
            let mut index = self.eviction.lock().unwrap();
            // Every removed variant shared this base key, so drop its count.
            index.base_key_counts.remove(&base_key);
            // Each removed variant leaves its `order` entry behind as a
            // tombstone (see `EvictionIndex` docs).
            index.dead += removed_count;
            index.compact_if_needed(&entries);
        }

        let mut vary_index = self.vary_index.write().await;
        vary_index.remove(&base_key);
    }

    /// Remove all entries matching a path prefix.
    pub async fn invalidate_prefix(&self, path_prefix: &str) {
        let needle = format!(":{}", path_prefix);
        let mut entries = self.entries.write().await;
        let mut index = self.eviction.lock().unwrap();
        entries.retain(|key, v| {
            if key.contains(&needle) {
                if let Some(bk) = &v.base_key {
                    index.record_remove(bk);
                }
                // The removed key's `order` entry is left behind as a
                // tombstone (see `EvictionIndex` docs).
                index.dead += 1;
                false
            } else {
                true
            }
        });
        index.compact_if_needed(&entries);
    }

    /// Clear all cached responses.
    pub async fn clear(&self) {
        {
            let mut entries = self.entries.write().await;
            entries.clear();
            self.eviction.lock().unwrap().clear();
        }
        let mut vary_index = self.vary_index.write().await;
        vary_index.clear();
    }

    /// Remove all stale entries.
    ///
    /// Keeps `vary_index` in lockstep: any base key whose last variant is
    /// purged here is also removed from `vary_index`, so TTL expiry cannot leak
    /// `vary_index` entries.
    pub async fn purge_stale(&self) {
        let dead_base_keys = {
            let mut entries = self.entries.write().await;
            let mut index = self.eviction.lock().unwrap();

            // Collect base keys of the stale entries being removed, decrementing
            // their live-variant counts as we go.
            let mut removed_base_keys: Vec<String> = Vec::new();
            entries.retain(|_, v| {
                if v.is_fresh() {
                    true
                } else {
                    if let Some(bk) = &v.base_key {
                        removed_base_keys.push(bk.clone());
                        index.record_remove(bk);
                    }
                    // The removed key's `order` entry is left behind as a
                    // tombstone (see `EvictionIndex` docs).
                    index.dead += 1;
                    false
                }
            });

            // Keep only base keys that no surviving entry still references —
            // decided in O(1) from the tracked counts instead of scanning every
            // remaining entry.
            removed_base_keys.retain(|bk| !index.base_key_live(bk));
            index.compact_if_needed(&entries);
            removed_base_keys
        };

        if !dead_base_keys.is_empty() {
            let mut vary_index = self.vary_index.write().await;
            for bk in dead_base_keys {
                vary_index.remove(&bk);
            }
        }
    }

    /// Get cache statistics.
    pub async fn stats(&self) -> CacheStats {
        let entries = self.entries.read().await;
        let fresh_count = entries.values().filter(|e| e.is_fresh()).count();
        let stale_count = entries.len() - fresh_count;
        let total_size: usize = entries.values().map(|e| e.response.body.len()).sum();

        CacheStats {
            total_entries: entries.len(),
            fresh_entries: fresh_count,
            stale_entries: stale_count,
            total_size_bytes: total_size,
            max_entries: self.config.max_entries,
        }
    }
}

impl Default for ResponseCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Total number of entries
    pub total_entries: usize,
    /// Number of fresh entries
    pub fresh_entries: usize,
    /// Number of stale entries
    pub stale_entries: usize,
    /// Total size of cached bodies in bytes
    pub total_size_bytes: usize,
    /// Maximum entries allowed
    pub max_entries: usize,
}

// ============================================================================
// Request/Response Extensions
// ============================================================================

/// Extension methods for HttpRequest related to caching.
impl HttpRequest {
    /// Get the Cache-Control header from the request.
    pub fn cache_control(&self) -> Option<CacheControl> {
        // One lookup: header names intern case-insensitively.
        self.headers.get("Cache-Control").map(CacheControl::parse)
    }

    /// Check if the request allows cached responses.
    pub fn allows_cached(&self) -> bool {
        if let Some(cc) = self.cache_control() {
            // Check for no-cache or no-store
            !cc.is_no_cache() && !cc.is_no_store()
        } else {
            true
        }
    }

    /// Get the max-stale tolerance from the request.
    pub fn max_stale(&self) -> Option<u64> {
        self.cache_control().and_then(|cc| {
            cc.directives.iter().find_map(|d| match d {
                CacheDirective::MaxStale(secs) => Some(secs.unwrap_or(u64::MAX)),
                _ => None,
            })
        })
    }

    /// Generate a cache key for this request.
    pub fn cache_key(&self) -> CacheKey {
        CacheKey::from_request(self)
    }

    /// Generate a cache key with Vary headers.
    pub fn cache_key_with_vary(&self, vary_headers: &[&str]) -> CacheKey {
        CacheKey::from_request_with_vary(self, vary_headers)
    }
}

/// Extension methods for HttpResponse related to caching.
impl HttpResponse {
    /// Set the Cache-Control header.
    pub fn with_cache_control(mut self, cache_control: CacheControl) -> Self {
        self.headers
            .insert("Cache-Control".to_string(), cache_control.to_header_value());
        self
    }

    /// Set a public cache with max-age.
    pub fn cache_public(self, max_age: Duration) -> Self {
        self.with_cache_control(CacheControl::public_max_age(max_age))
    }

    /// Set a private cache with max-age.
    pub fn cache_private(self, max_age: Duration) -> Self {
        self.with_cache_control(CacheControl::private_max_age(max_age))
    }

    /// Set cache for immutable assets.
    pub fn cache_immutable(self, max_age: Duration) -> Self {
        self.with_cache_control(CacheControl::immutable_asset(max_age))
    }

    /// Add Vary header.
    pub fn with_vary(mut self, headers: &[&str]) -> Self {
        let vary = headers.join(", ");
        self.headers.insert("Vary".to_string(), vary);
        self
    }

    /// Get the Cache-Control header from the response.
    pub fn get_cache_control(&self) -> Option<CacheControl> {
        self.headers
            .get("Cache-Control")
            .map(|h| CacheControl::parse(h))
    }

    /// Check if the response is cacheable based on Cache-Control.
    pub fn is_cacheable(&self) -> bool {
        if let Some(cc) = self.get_cache_control() {
            cc.is_cacheable()
        } else {
            // Default: only cache 200 OK without explicit Cache-Control
            self.status == 200
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_cache_directive_parse() {
        assert_eq!(
            CacheDirective::parse("public"),
            Some(CacheDirective::Public)
        );
        assert_eq!(
            CacheDirective::parse("private"),
            Some(CacheDirective::Private)
        );
        assert_eq!(
            CacheDirective::parse("no-store"),
            Some(CacheDirective::NoStore)
        );
        assert_eq!(
            CacheDirective::parse("max-age=3600"),
            Some(CacheDirective::MaxAge(3600))
        );
    }

    #[test]
    fn test_cache_control_parse() {
        let cc = CacheControl::parse("public, max-age=3600, must-revalidate");
        assert!(cc.is_public());
        assert_eq!(cc.get_max_age(), Some(3600));
        assert!(cc.is_must_revalidate());
    }

    #[test]
    fn test_cache_control_builder() {
        let cc = CacheControl::new()
            .public()
            .max_age(Duration::from_secs(3600))
            .must_revalidate();

        assert_eq!(
            cc.to_header_value(),
            "public, max-age=3600, must-revalidate"
        );
    }

    #[test]
    fn test_cache_control_presets() {
        let never = CacheControl::never();
        assert!(never.is_no_store());
        assert!(never.is_no_cache());

        let public = CacheControl::public_max_age(Duration::from_secs(3600));
        assert!(public.is_public());
        assert_eq!(public.get_max_age(), Some(3600));

        let immutable = CacheControl::immutable_asset(Duration::from_secs(31536000));
        assert!(immutable.is_immutable());
    }

    #[test]
    fn test_cache_control_is_cacheable() {
        assert!(CacheControl::public_max_age(Duration::from_secs(3600)).is_cacheable());
        assert!(CacheControl::private_max_age(Duration::from_secs(3600)).is_cacheable());
        assert!(!CacheControl::never().is_cacheable());
    }

    #[test]
    fn test_cache_key_from_request() {
        let request = HttpRequest::new("GET", "/api/users?page=1&limit=10");

        let key = CacheKey::from_request(&request);
        assert_eq!(key.method, "GET");
        assert_eq!(key.path, "/api/users");
        // Query params should be sorted
        assert!(key.query.contains("limit=10"));
        assert!(key.query.contains("page=1"));
    }

    #[test]
    fn test_cache_key_with_vary() {
        let mut request = HttpRequest::new("GET", "/api/users".to_string());
        request
            .headers
            .insert("Accept", "application/json".to_string());

        let key = CacheKey::from_request_with_vary(&request, &["Accept"]);
        assert_eq!(key.vary_values.len(), 1);
        assert_eq!(
            key.vary_values[0],
            ("accept".to_string(), "application/json".to_string())
        );
    }

    #[test]
    fn test_cached_response() {
        let mut response = HttpResponse::ok();
        response.body = Bytes::from_static(b"Hello, World!");
        response
            .headers
            .insert("ETag".to_string(), "\"abc123\"".to_string());

        let cached = CachedResponse::new(&response, Duration::from_secs(300));
        assert!(cached.is_fresh());
        assert_eq!(cached.etag, Some("\"abc123\"".to_string()));
    }

    #[tokio::test]
    async fn test_response_cache_store_and_get() {
        let cache = ResponseCache::new();
        let request = HttpRequest::new("GET", "/api/users".to_string());
        let mut response = HttpResponse::ok();
        response.body = Bytes::from_static(b"cached content");

        cache.store(&request, &response).await;

        let cached = cache.get(&request).await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().body, Bytes::from_static(b"cached content"));
    }

    #[tokio::test]
    async fn test_query_method_cached_with_body_in_key() {
        let cache = ResponseCache::new();

        let mut search_a = HttpRequest::new("QUERY", "/search".to_string());
        search_a.body = Bytes::from_static(b"name=alice");
        let mut response_a = HttpResponse::ok();
        response_a.body = Bytes::from_static(b"results for alice");

        cache.store(&search_a, &response_a).await;

        // Same path + same body: hit
        let cached = cache.get(&search_a).await;
        assert!(cached.is_some());
        assert_eq!(
            cached.unwrap().body,
            Bytes::from_static(b"results for alice")
        );

        // Same path, different body: distinct entry, must miss
        let mut search_b = HttpRequest::new("QUERY", "/search".to_string());
        search_b.body = Bytes::from_static(b"name=bob");
        assert!(cache.get(&search_b).await.is_none());

        // The two bodies produce different keys, so both can coexist
        let mut response_b = HttpResponse::ok();
        response_b.body = Bytes::from_static(b"results for bob");
        cache.store(&search_b, &response_b).await;
        assert_eq!(
            cache.get(&search_a).await.unwrap().body,
            Bytes::from_static(b"results for alice")
        );
        assert_eq!(
            cache.get(&search_b).await.unwrap().body,
            Bytes::from_static(b"results for bob")
        );
    }

    #[test]
    fn test_query_cache_key_includes_body_hash() {
        let mut req_a = HttpRequest::new("QUERY", "/search".to_string());
        req_a.body = Bytes::from_static(b"a");
        let mut req_b = HttpRequest::new("QUERY", "/search".to_string());
        req_b.body = Bytes::from_static(b"b");

        let key_a = CacheKey::from_request(&req_a);
        let key_b = CacheKey::from_request(&req_b);
        assert!(key_a.body_hash.is_some());
        assert_ne!(key_a, key_b);
        assert_ne!(key_a.to_string_key(), key_b.to_string_key());

        // GET keys are unaffected by the body
        let mut get_req = HttpRequest::new("GET", "/search".to_string());
        get_req.body = Bytes::from_static(b"ignored");
        assert!(CacheKey::from_request(&get_req).body_hash.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_invalidate() {
        let cache = ResponseCache::new();
        let request = HttpRequest::new("GET", "/api/users".to_string());
        let response = HttpResponse::ok();

        cache.store(&request, &response).await;
        assert!(cache.get(&request).await.is_some());

        cache.invalidate(&request).await;
        assert!(cache.get(&request).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_respects_no_store() {
        let cache = ResponseCache::new();
        let request = HttpRequest::new("GET", "/api/users".to_string());
        let response = HttpResponse::ok().no_cache();

        cache.store(&request, &response).await;

        // Should not be cached due to no-store
        assert!(cache.get(&request).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_respects_private() {
        let cache = ResponseCache::new();
        let request = HttpRequest::new("GET", "/api/users".to_string());
        let response = HttpResponse::ok().cache_private(Duration::from_secs(300));

        cache.store(&request, &response).await;

        // private responses must not be stored in a shared cache
        assert!(cache.get(&request).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_respects_no_cache_directive() {
        let cache = ResponseCache::new();
        let request = HttpRequest::new("GET", "/api/users".to_string());
        let response = HttpResponse::ok().with_cache_control(CacheControl::new().no_cache());

        cache.store(&request, &response).await;

        assert!(cache.get(&request).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_authorization_not_stored() {
        let cache = ResponseCache::new();
        let mut request = HttpRequest::new("GET", "/api/me".to_string());
        request
            .headers
            .insert("Authorization", "Bearer user-a".to_string());
        let response = HttpResponse::ok();

        cache.store(&request, &response).await;

        // Responses to authorized requests must not be replayed from a
        // shared cache without explicit permission.
        assert!(cache.get(&request).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_authorization_stored_when_public() {
        let cache = ResponseCache::new();
        let mut request = HttpRequest::new("GET", "/api/assets".to_string());
        request
            .headers
            .insert("Authorization", "Bearer user-a".to_string());
        let response = HttpResponse::ok().cache_public(Duration::from_secs(60));

        cache.store(&request, &response).await;

        assert!(cache.get(&request).await.is_some());
    }

    #[tokio::test]
    async fn test_response_cache_ttl_from_max_age() {
        let cache = ResponseCache::new();
        let request = HttpRequest::new("GET", "/api/users".to_string());
        // max-age=0 must override the 5-minute default TTL.
        let response = HttpResponse::ok().cache_public(Duration::from_secs(0));

        cache.store(&request, &response).await;

        assert!(cache.get(&request).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_vary_two_phase_lookup() {
        let cache = ResponseCache::new();
        let mut request = HttpRequest::new("GET", "/api/data".to_string());
        request
            .headers
            .insert("Accept", "application/json".to_string());

        let mut response = HttpResponse::ok().with_vary(&["Accept"]);
        response.body = Bytes::from_static(b"json");

        cache.store(&request, &response).await;

        // A plain get (no explicit vary list) must find the varied entry.
        let hit = cache.get(&request).await;
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().body, Bytes::from_static(b"json"));

        // A request with a different Accept value is a different variant.
        let mut other = HttpRequest::new("GET", "/api/data".to_string());
        other.headers.insert("Accept", "text/xml".to_string());
        assert!(cache.get(&other).await.is_none());
    }

    #[tokio::test]
    async fn test_response_cache_invalidate_removes_vary_variants() {
        let cache = ResponseCache::new();
        let mut request = HttpRequest::new("GET", "/api/data".to_string());
        request
            .headers
            .insert("Accept", "application/json".to_string());

        let response = HttpResponse::ok().with_vary(&["Accept"]);
        cache.store(&request, &response).await;
        assert!(cache.get(&request).await.is_some());

        // Invalidating with a plain request must remove all variants.
        let plain = HttpRequest::new("GET", "/api/data".to_string());
        cache.invalidate(&plain).await;
        assert!(cache.get(&request).await.is_none());
    }

    #[test]
    fn test_response_cache_control_methods() {
        let response = HttpResponse::ok().cache_public(Duration::from_secs(3600));

        let cc = response.get_cache_control().unwrap();
        assert!(cc.is_public());
        assert_eq!(cc.get_max_age(), Some(3600));
    }

    #[test]
    fn test_response_with_vary() {
        let response = HttpResponse::ok().with_vary(&["Accept", "Accept-Encoding"]);

        assert_eq!(
            response.headers.get("Vary"),
            Some(&"Accept, Accept-Encoding".to_string())
        );
    }

    #[test]
    fn test_request_allows_cached() {
        let request = HttpRequest::new("GET", "/api/users".to_string());
        assert!(request.allows_cached());

        let mut request_no_cache = HttpRequest::new("GET", "/api/users".to_string());
        request_no_cache
            .headers
            .insert("Cache-Control", "no-cache".to_string());
        assert!(!request_no_cache.allows_cached());
    }

    /// Regression: with QUERY body-hash keying, every distinct request body
    /// produces a distinct base key and therefore a `vary_index` entry. If
    /// eviction does not prune `vary_index`, it grows without bound while
    /// `entries` stays capped. After eviction, `vary_index` must never exceed
    /// the number of live entries.
    #[tokio::test]
    async fn test_vary_index_bounded_after_eviction() {
        let cache = ResponseCache::with_config(ResponseCacheConfig::new().max_entries(8));

        for i in 0..100 {
            let mut req = HttpRequest::new("QUERY", "/search".to_string());
            req.body = Bytes::from(format!("q={}", i).into_bytes());
            let mut resp = HttpResponse::ok();
            resp.body = Bytes::from(format!("result {}", i).into_bytes());
            cache.store(&req, &resp).await;
        }

        let entries_len = cache.entries.read().await.len();
        let vary_len = cache.vary_index.read().await.len();

        assert!(
            entries_len <= 8,
            "entries ({}) exceeded max_entries",
            entries_len
        );
        assert!(
            vary_len <= entries_len,
            "vary_index ({}) must not exceed entries ({}) after eviction",
            vary_len,
            entries_len,
        );
    }

    /// Eviction removes entries in insertion order: once at capacity, the
    /// oldest-inserted entry is the one dropped to make room, and newer entries
    /// survive.
    #[tokio::test]
    async fn test_evicts_in_insertion_order() {
        let cache = ResponseCache::with_config(ResponseCacheConfig::new().max_entries(3));

        for i in 0..3 {
            let req = HttpRequest::new("GET", format!("/p{}", i));
            cache.store(&req, &HttpResponse::ok()).await;
        }
        for i in 0..3 {
            let req = HttpRequest::new("GET", format!("/p{}", i));
            assert!(cache.get(&req).await.is_some(), "/p{} should be cached", i);
        }

        // A fourth insert must evict the oldest (/p0), not any newer entry.
        let req = HttpRequest::new("GET", "/p3".to_string());
        cache.store(&req, &HttpResponse::ok()).await;

        let p0 = HttpRequest::new("GET", "/p0".to_string());
        assert!(
            cache.get(&p0).await.is_none(),
            "oldest entry (/p0) must be evicted first"
        );
        for i in 1..4 {
            let req = HttpRequest::new("GET", format!("/p{}", i));
            assert!(cache.get(&req).await.is_some(), "/p{} must remain", i);
        }

        // vary_index must not outgrow the live entry set after eviction.
        let entries_len = cache.entries.read().await.len();
        let vary_len = cache.vary_index.read().await.len();
        assert!(entries_len <= 3);
        assert!(vary_len <= entries_len);
    }

    /// Regression: refreshing an entry while still under capacity must reset its
    /// eviction recency, so a later capacity-driven eviction drops the genuinely
    /// oldest *untouched* entry — not the refreshed one by its original insert
    /// time. Repro: max_entries(3), store A, store B, re-store A (refresh, under
    /// cap), store C, store D (at cap → eviction). B must be evicted and A must
    /// survive. The old index evicted A because a refresh left the stale
    /// front-most `order` position live.
    #[tokio::test]
    async fn test_eviction_refresh_resets_recency() {
        async fn store(cache: &ResponseCache, path: &str, body: &[u8]) {
            let req = HttpRequest::new("GET", path.to_string());
            let mut resp = HttpResponse::ok();
            resp.body = Bytes::copy_from_slice(body);
            cache.store(&req, &resp).await;
        }
        async fn present(cache: &ResponseCache, path: &str) -> bool {
            cache
                .get(&HttpRequest::new("GET", path.to_string()))
                .await
                .is_some()
        }

        let cache = ResponseCache::with_config(ResponseCacheConfig::new().max_entries(3));

        store(&cache, "/a", b"a").await;
        store(&cache, "/b", b"b").await;
        store(&cache, "/a", b"a2").await; // refresh while under capacity
        store(&cache, "/c", b"c").await;
        store(&cache, "/d", b"d").await; // at capacity → one eviction

        assert!(
            !present(&cache, "/b").await,
            "B (the oldest untouched entry) must be evicted"
        );
        assert!(
            present(&cache, "/a").await,
            "A was refreshed under capacity and must survive"
        );
        assert!(present(&cache, "/c").await, "/c must remain");
        assert!(present(&cache, "/d").await, "/d was just stored");
    }

    /// Regression: TTL purging must also prune `vary_index` so expired entries
    /// do not leak their base-key records.
    #[tokio::test]
    async fn test_purge_stale_shrinks_vary_index() {
        let cache = ResponseCache::new();

        let mut stale_req = HttpRequest::new("QUERY", "/search".to_string());
        stale_req.body = Bytes::from_static(b"q=stale");
        let mut fresh_req = HttpRequest::new("QUERY", "/search".to_string());
        fresh_req.body = Bytes::from_static(b"q=fresh");
        let resp = HttpResponse::ok();

        cache
            .store_with_ttl(&stale_req, &resp, Duration::from_secs(0))
            .await;
        cache
            .store_with_ttl(&fresh_req, &resp, Duration::from_secs(300))
            .await;

        assert_eq!(cache.vary_index.read().await.len(), 2);

        // Ensure the zero-TTL entry is observably stale.
        tokio::time::sleep(Duration::from_millis(5)).await;
        cache.purge_stale().await;

        assert_eq!(
            cache.entries.read().await.len(),
            1,
            "only the fresh entry should survive purge",
        );
        assert_eq!(
            cache.vary_index.read().await.len(),
            1,
            "vary_index must shrink in lockstep with purged entries",
        );
    }

    /// Regression: repeatedly storing then invalidating keys (capacity never
    /// hit, so `evict_oldest` never runs) must not let `order` grow without
    /// bound. Keys removed via `invalidate` were previously left in `order`
    /// as tombstones forever.
    #[tokio::test]
    async fn test_eviction_order_bounded_under_store_invalidate_churn() {
        let cache = ResponseCache::with_config(ResponseCacheConfig::new().max_entries(10_000));

        for i in 0..2000 {
            let req = HttpRequest::new("GET", format!("/churn/{}", i % 5));
            cache.store(&req, &HttpResponse::ok()).await;
            cache.invalidate(&req).await;
        }

        let live = cache.entries.read().await.len();
        let order_len = cache.eviction.lock().unwrap().order.len();
        assert!(
            order_len <= 2 * live + 16,
            "order ({}) grew unbounded relative to live entries ({}) under store/invalidate churn",
            order_len,
            live
        );
    }

    /// Regression: re-storing the *same* key repeatedly (e.g. a hot endpoint
    /// refreshed every TTL cycle) pushes a new `order` entry each time
    /// (`EvictionIndex::record_insert`'s `replaced` branch only suppresses the
    /// `base_key_counts` increment, not the push). Without compaction this
    /// grows without bound even though only one entry is ever live.
    #[tokio::test]
    async fn test_eviction_order_bounded_under_repeated_restore() {
        let cache = ResponseCache::with_config(ResponseCacheConfig::new().max_entries(10_000));
        let req = HttpRequest::new("GET", "/hot".to_string());

        for i in 0..2000 {
            let mut resp = HttpResponse::ok();
            resp.body = Bytes::from(format!("v{}", i).into_bytes());
            cache.store(&req, &resp).await;
        }

        let live = cache.entries.read().await.len();
        assert_eq!(live, 1, "only the latest write for the key should be live");

        let order_len = cache.eviction.lock().unwrap().order.len();
        assert!(
            order_len <= 2 * live + 16,
            "order ({}) grew unbounded across repeated re-stores of one key (live={})",
            order_len,
            live
        );

        // The live value must be the most recent store, not a stale one.
        let cached = cache.get(&req).await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().body, Bytes::from_static(b"v1999"));
    }

    /// Regression: two responses stored at the same path with *different* Vary
    /// header sets must both remain retrievable. Overwriting the `vary_index`
    /// record last-writer-wins would make the earlier variant unreachable;
    /// merging (union) keeps both reachable.
    #[tokio::test]
    async fn test_vary_index_merges_distinct_vary_sets() {
        let cache = ResponseCache::new();

        // Variant 1: keyed on Accept.
        let mut req_accept = HttpRequest::new("GET", "/api/data".to_string());
        req_accept
            .headers
            .insert("Accept", "application/json".to_string());
        let mut resp_accept = HttpResponse::ok().with_vary(&["Accept"]);
        resp_accept.body = Bytes::from_static(b"json-body");
        cache.store(&req_accept, &resp_accept).await;

        // Variant 2: same path, keyed on Accept-Encoding.
        let mut req_enc = HttpRequest::new("GET", "/api/data".to_string());
        req_enc
            .headers
            .insert("Accept-Encoding", "gzip".to_string());
        let mut resp_enc = HttpResponse::ok().with_vary(&["Accept-Encoding"]);
        resp_enc.body = Bytes::from_static(b"gzip-body");
        cache.store(&req_enc, &resp_enc).await;

        // Both variants must survive the second store's `vary_index` update.
        let hit_accept = cache.get(&req_accept).await;
        assert!(
            hit_accept.is_some(),
            "Accept variant lost after second store"
        );
        assert_eq!(hit_accept.unwrap().body, Bytes::from_static(b"json-body"));

        let hit_enc = cache.get(&req_enc).await;
        assert!(
            hit_enc.is_some(),
            "Accept-Encoding variant lost after second store",
        );
        assert_eq!(hit_enc.unwrap().body, Bytes::from_static(b"gzip-body"));
    }
}
