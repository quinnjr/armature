//! Zero-Allocation Route Parameter Extraction
//!
//! This module provides a standalone, optimized route parameter API:
//!
//! - **Zero-allocation matching**: Use borrowed strings and SmallVec
//! - **Wildcard support**: `*` and `**` patterns for catch-all routes
//! - **Type-safe extraction**: Parse params into typed values
//!
//! # Performance
//!
//! - No heap allocation for typical routes (≤8 params)
//! - Borrowed strings avoid copying during matching
//! - Optimized wildcard handling with single-pass parsing
//!
//! # Status: not (yet) wired into the live request path
//!
//! `CompiledPattern`, `Params`, and [`match_path_zero_alloc`] are a
//! correct, independently-tested API, but [`Router`](crate::routing::Router)
//! and [`OptimizedRouter`](crate::route_cache::OptimizedRouter) — the
//! routers that actually serve requests — do not call into this module.
//! They use their own matchers (`routing::match_path`,
//! `route_cache::CompiledRoute`/`CachedRoute`), which return an owned
//! `HashMap<String, String>` (the type `HttpRequest::path_params` requires)
//! and avoid the `Vec<&str>` segment-splitting allocation via `SmallVec`,
//! but still allocate one `String` per extracted parameter on every match.
//!
//! A full switch to this module's borrowed `Params<'a>` on the live path
//! would need `HttpRequest::path_params` (and everything that reads it) to
//! move off owned `HashMap<String, String>`, which is a public API change
//! out of scope for a drop-in fix. Also note a semantic difference that
//! blocks a direct swap even for the matching step alone: here a bare `*`
//! pattern segment matches exactly one path segment
//! ([`SegmentType::Wildcard`]), whereas `routing`/`route_cache` treat a
//! bare `*` as a multi-segment catch-all (consuming all remaining
//! segments), so substituting this module's matcher would silently change
//! matching behavior for that pattern.
//!
//! If you need genuinely zero-allocation extraction, use this module's
//! `CompiledPattern::match_path` / [`match_path_zero_alloc`] directly
//! rather than going through `Router`/`OptimizedRouter`.

use compact_str::CompactString;
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Constants
// ============================================================================

/// Maximum number of inline path parameters before heap allocation.
pub const INLINE_PARAM_COUNT: usize = 8;

/// Maximum path segments for inline storage.
pub const INLINE_SEGMENT_COUNT: usize = 16;

// ============================================================================
// Zero-Allocation Params
// ============================================================================

/// A borrowed path parameter (name, value) pair.
///
/// Both name and value are borrowed from the pattern and path strings,
/// avoiding allocation during matching.
#[derive(Debug, Clone, Copy)]
pub struct BorrowedParam<'a> {
    /// Parameter name (from pattern, e.g., "id" from ":id")
    pub name: &'a str,
    /// Parameter value (from path, e.g., "123")
    pub value: &'a str,
}

impl<'a> BorrowedParam<'a> {
    /// Create new borrowed param.
    #[inline]
    pub fn new(name: &'a str, value: &'a str) -> Self {
        Self { name, value }
    }

    /// Parse value as type T.
    #[inline]
    pub fn parse<T: std::str::FromStr>(&self) -> Result<T, T::Err> {
        self.value.parse()
    }

    /// Convert to owned strings.
    #[inline]
    pub fn to_owned(&self) -> (String, String) {
        (self.name.to_string(), self.value.to_string())
    }

    /// Convert to compact strings.
    #[inline]
    pub fn to_compact(&self) -> (CompactString, CompactString) {
        (
            CompactString::new(self.name),
            CompactString::new(self.value),
        )
    }
}

/// Zero-allocation parameter collection using borrowed strings.
///
/// Stores up to 8 parameters inline (on stack), falling back to
/// heap allocation only for routes with many parameters.
#[derive(Debug, Clone)]
pub struct Params<'a> {
    /// Parameters stored inline for typical routes.
    params: SmallVec<[BorrowedParam<'a>; INLINE_PARAM_COUNT]>,
    /// Wildcard capture (if any).
    ///
    /// A catch-all captures a *joined* value (e.g. `docs/readme.md`) that does
    /// not exist as a contiguous slice of the source path, so it is owned via
    /// `Cow`. Static/param values still borrow the path with zero allocation.
    wildcard: Option<Cow<'a, str>>,
    /// Name of the catch-all parameter (e.g. `path` for `*path`), if the
    /// wildcard value should also be retrievable by name via [`get`](Self::get).
    catch_all_name: Option<&'a str>,
}

impl<'a> Params<'a> {
    /// Create empty params.
    #[inline]
    pub fn new() -> Self {
        Self {
            params: SmallVec::new(),
            wildcard: None,
            catch_all_name: None,
        }
    }

    /// Create with capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            params: SmallVec::with_capacity(capacity),
            wildcard: None,
            catch_all_name: None,
        }
    }

    /// Add a parameter.
    #[inline]
    pub fn push(&mut self, name: &'a str, value: &'a str) {
        self.params.push(BorrowedParam::new(name, value));
        PARAMS_STATS.record_param_add(self.is_inline());
    }

    /// Set wildcard value.
    ///
    /// Accepts either a borrowed slice of the path or an owned/joined value
    /// (`String`), so catch-all captures no longer need to leak memory.
    #[inline]
    pub fn set_wildcard(&mut self, value: impl Into<Cow<'a, str>>) {
        self.wildcard = Some(value.into());
    }

    /// Set a named catch-all value.
    ///
    /// Stores the (owned) joined value as the wildcard and records `name` so it
    /// can also be retrieved via [`get`](Self::get).
    #[inline]
    pub fn set_catch_all(&mut self, name: &'a str, value: impl Into<Cow<'a, str>>) {
        self.wildcard = Some(value.into());
        self.catch_all_name = Some(name);
    }

    /// Get parameter by name.
    #[inline]
    pub fn get(&self, name: &str) -> Option<&str> {
        if let Some(p) = self.params.iter().find(|p| p.name == name) {
            return Some(p.value);
        }
        if self.catch_all_name == Some(name) {
            return self.wildcard.as_deref();
        }
        None
    }

    /// Get parameter and parse as type T.
    #[inline]
    pub fn get_parsed<T: std::str::FromStr>(&self, name: &str) -> Option<Result<T, T::Err>> {
        self.get(name).map(|v| v.parse())
    }

    /// Get wildcard capture.
    #[inline]
    pub fn wildcard(&self) -> Option<&str> {
        self.wildcard.as_deref()
    }

    /// Get number of parameters.
    #[inline]
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// Check if params are stored inline.
    #[inline]
    pub fn is_inline(&self) -> bool {
        !self.params.spilled()
    }

    /// Iterate over parameters.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &BorrowedParam<'a>> {
        self.params.iter()
    }

    /// Convert to HashMap (allocates).
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::with_capacity(self.params.len());
        for param in &self.params {
            map.insert(param.name.to_string(), param.value.to_string());
        }
        if let Some(ref wildcard) = self.wildcard {
            // Named catch-all is retrievable by its name as well as under "*".
            if let Some(name) = self.catch_all_name {
                map.insert(name.to_string(), wildcard.to_string());
            }
            map.insert("*".to_string(), wildcard.to_string());
        }
        map
    }

    /// Convert to compact map.
    pub fn to_compact_map(&self) -> SmallVec<[(CompactString, CompactString); INLINE_PARAM_COUNT]> {
        let mut map = SmallVec::with_capacity(self.params.len());
        for param in &self.params {
            map.push(param.to_compact());
        }
        map
    }
}

impl<'a> Default for Params<'a> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Pattern Matching
// ============================================================================

/// Result of pattern matching.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // Match is the common path, boxing would add indirection
pub enum MatchResult<'a> {
    /// Pattern matched, params extracted.
    Match(Params<'a>),
    /// Pattern did not match.
    NoMatch,
}

impl<'a> MatchResult<'a> {
    /// Check if matched.
    #[inline]
    pub fn is_match(&self) -> bool {
        matches!(self, MatchResult::Match(_))
    }

    /// Get params if matched.
    #[inline]
    pub fn params(self) -> Option<Params<'a>> {
        match self {
            MatchResult::Match(p) => Some(p),
            MatchResult::NoMatch => None,
        }
    }
}

/// Pattern segment type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentType {
    /// Static segment (e.g., "api", "users")
    Static,
    /// Named parameter (e.g., ":id", ":user_id")
    Param,
    /// Single-segment wildcard (e.g., "*")
    Wildcard,
    /// Multi-segment wildcard (e.g., "**", "*path")
    CatchAll,
}

/// A parsed pattern segment.
#[derive(Debug, Clone)]
pub struct PatternSegment<'a> {
    /// Segment type.
    pub segment_type: SegmentType,
    /// Static value or parameter name.
    pub value: &'a str,
}

impl<'a> PatternSegment<'a> {
    /// Parse a segment string.
    #[inline]
    pub fn parse(segment: &'a str) -> Self {
        if let Some(param_name) = segment.strip_prefix(':') {
            Self {
                segment_type: SegmentType::Param,
                value: param_name,
            }
        } else if segment == "*" {
            Self {
                segment_type: SegmentType::Wildcard,
                value: "*",
            }
        } else if segment == "**" || segment.starts_with('*') && segment.len() > 1 {
            Self {
                segment_type: SegmentType::CatchAll,
                value: if segment == "**" { "*" } else { &segment[1..] },
            }
        } else {
            Self {
                segment_type: SegmentType::Static,
                value: segment,
            }
        }
    }

    /// Check if this is a static segment.
    #[inline]
    pub fn is_static(&self) -> bool {
        self.segment_type == SegmentType::Static
    }

    /// Check if this is a parameter.
    #[inline]
    pub fn is_param(&self) -> bool {
        self.segment_type == SegmentType::Param
    }

    /// Check if this is a wildcard.
    #[inline]
    pub fn is_wildcard(&self) -> bool {
        matches!(
            self.segment_type,
            SegmentType::Wildcard | SegmentType::CatchAll
        )
    }
}

/// An owned, pre-parsed pattern segment.
///
/// Unlike [`PatternSegment`], this owns its value so it can live inside a
/// [`CompiledPattern`] without borrowing (or leaking) the pattern string.
#[derive(Debug, Clone)]
pub struct OwnedPatternSegment {
    /// Segment type.
    pub segment_type: SegmentType,
    /// Static value or parameter name (owned).
    pub value: CompactString,
}

/// Pre-compiled route pattern for fast matching.
#[derive(Debug, Clone)]
pub struct CompiledPattern {
    /// Original pattern string.
    pub pattern: String,
    /// Parsed segments (owned; no leaking or self-borrow of `pattern`).
    pub segments: SmallVec<[OwnedPatternSegment; INLINE_SEGMENT_COUNT]>,
    /// Index of catch-all segment (-1 if none).
    pub catch_all_index: Option<usize>,
    /// Number of required segments (before catch-all).
    pub required_segments: usize,
    /// Is this a static-only pattern?
    pub is_static: bool,
}

impl CompiledPattern {
    /// Compile a pattern string.
    pub fn new(pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();

        let mut segments = SmallVec::new();
        let mut catch_all_index = None;
        let mut is_static = true;

        // Parse borrowing the owned `pattern` locally, copying each segment's
        // value into owned storage. No `Box::leak` / `'static` needed.
        for (i, part) in pattern.split('/').filter(|s| !s.is_empty()).enumerate() {
            let segment = PatternSegment::parse(part);

            if segment.segment_type == SegmentType::CatchAll {
                catch_all_index = Some(i);
            }

            if !segment.is_static() {
                is_static = false;
            }

            segments.push(OwnedPatternSegment {
                segment_type: segment.segment_type,
                value: CompactString::new(segment.value),
            });
        }

        let required_segments = catch_all_index.unwrap_or(segments.len());

        Self {
            pattern,
            segments,
            catch_all_index,
            required_segments,
            is_static,
        }
    }

    /// Match against a path.
    ///
    /// Parameter *names* borrow from the compiled pattern (`&'a self`) and
    /// *values* borrow from the path; the joined catch-all value is owned, so
    /// no per-request memory is leaked.
    pub fn match_path<'a>(&'a self, path: &'a str) -> MatchResult<'a> {
        PARAMS_STATS.record_match_attempt();

        // Split path into segments
        let path_segments: SmallVec<[&str; INLINE_SEGMENT_COUNT]> =
            path.split('/').filter(|s| !s.is_empty()).collect();

        // Quick length check
        if self.catch_all_index.is_none() {
            if path_segments.len() != self.segments.len() {
                return MatchResult::NoMatch;
            }
        } else if path_segments.len() < self.required_segments {
            return MatchResult::NoMatch;
        }

        let mut params = Params::with_capacity(self.segments.len());

        for (i, pattern_seg) in self.segments.iter().enumerate() {
            match pattern_seg.segment_type {
                SegmentType::Static => {
                    if i >= path_segments.len() || path_segments[i] != pattern_seg.value.as_str() {
                        return MatchResult::NoMatch;
                    }
                }
                SegmentType::Param => {
                    if i >= path_segments.len() {
                        return MatchResult::NoMatch;
                    }
                    params.push(pattern_seg.value.as_str(), path_segments[i]);
                }
                SegmentType::Wildcard => {
                    if i >= path_segments.len() {
                        return MatchResult::NoMatch;
                    }
                    params.push(pattern_seg.value.as_str(), path_segments[i]);
                }
                SegmentType::CatchAll => {
                    // Capture remaining segments. The joined value is owned by
                    // the returned `Params` (via `Cow::Owned`) — no leak.
                    let remaining = &path_segments[i..];
                    if remaining.is_empty() {
                        params.set_wildcard("");
                    } else {
                        let wildcard_value = remaining.join("/");
                        if pattern_seg.value.as_str() != "*" {
                            params.set_catch_all(pattern_seg.value.as_str(), wildcard_value);
                        } else {
                            params.set_wildcard(wildcard_value);
                        }
                    }
                    break;
                }
            }
        }

        PARAMS_STATS.record_match_success();
        MatchResult::Match(params)
    }

    /// Check if pattern matches (without extracting params).
    #[inline]
    pub fn matches(&self, path: &str) -> bool {
        self.match_path(path).is_match()
    }
}

// ============================================================================
// Zero-Allocation Matcher
// ============================================================================

/// Zero-allocation route matcher.
///
/// Uses borrowed strings and avoids HashMap allocation during matching.
pub fn match_path_zero_alloc<'p, 'a>(pattern: &'p str, path: &'a str) -> Option<Params<'a>>
where
    'p: 'a,
{
    PARAMS_STATS.record_match_attempt();

    // Split into segments
    let pattern_parts: SmallVec<[&str; INLINE_SEGMENT_COUNT]> =
        pattern.split('/').filter(|s| !s.is_empty()).collect();

    let path_parts: SmallVec<[&str; INLINE_SEGMENT_COUNT]> =
        path.split('/').filter(|s| !s.is_empty()).collect();

    // Check for catch-all
    let has_catch_all = pattern_parts
        .last()
        .is_some_and(|s| s.starts_with('*') && *s != "*");

    // Length check
    if !has_catch_all && pattern_parts.len() != path_parts.len() {
        return None;
    }

    if has_catch_all && path_parts.len() < pattern_parts.len() - 1 {
        return None;
    }

    let mut params = Params::with_capacity(pattern_parts.len());

    for (i, pattern_part) in pattern_parts.iter().enumerate() {
        if let Some(name) = pattern_part.strip_prefix(':') {
            // Named parameter
            if i >= path_parts.len() {
                return None;
            }
            params.push(name, path_parts[i]);
        } else if *pattern_part == "*" {
            // Single wildcard
            if i >= path_parts.len() {
                return None;
            }
            params.push("*", path_parts[i]);
        } else if let Some(name) = pattern_part.strip_prefix('*') {
            // Catch-all (e.g., *path, *rest). The joined value is owned by the
            // returned `Params` (via `Cow::Owned`) — no leak.
            let remaining: String = path_parts[i..].join("/");
            params.set_catch_all(name, remaining);
            break;
        } else if i >= path_parts.len() || *pattern_part != path_parts[i] {
            // Static mismatch
            return None;
        }
    }

    PARAMS_STATS.record_match_success();
    Some(params)
}

// ============================================================================
// Type-Safe Extractors
// ============================================================================

/// Extract path parameters into typed tuple.
///
/// Zero-allocation for small numbers of parameters.
pub trait ExtractParams<'a> {
    /// Extract from params.
    fn extract(params: &Params<'a>) -> Option<Self>
    where
        Self: Sized;
}

impl<'a> ExtractParams<'a> for () {
    #[inline]
    fn extract(_params: &Params<'a>) -> Option<Self> {
        Some(())
    }
}

impl<'a> ExtractParams<'a> for &'a str {
    #[inline]
    fn extract(params: &Params<'a>) -> Option<Self> {
        params.params.first().map(|p| p.value)
    }
}

impl<'a, T: std::str::FromStr> ExtractParams<'a> for (T,) {
    fn extract(params: &Params<'a>) -> Option<Self> {
        let v = params.params.first()?.value.parse().ok()?;
        Some((v,))
    }
}

impl<'a, T1: std::str::FromStr, T2: std::str::FromStr> ExtractParams<'a> for (T1, T2) {
    fn extract(params: &Params<'a>) -> Option<Self> {
        if params.len() < 2 {
            return None;
        }
        let v1 = params.params[0].value.parse().ok()?;
        let v2 = params.params[1].value.parse().ok()?;
        Some((v1, v2))
    }
}

impl<'a, T1: std::str::FromStr, T2: std::str::FromStr, T3: std::str::FromStr> ExtractParams<'a>
    for (T1, T2, T3)
{
    fn extract(params: &Params<'a>) -> Option<Self> {
        if params.len() < 3 {
            return None;
        }
        let v1 = params.params[0].value.parse().ok()?;
        let v2 = params.params[1].value.parse().ok()?;
        let v3 = params.params[2].value.parse().ok()?;
        Some((v1, v2, v3))
    }
}

// ============================================================================
// Statistics
// ============================================================================

/// Parameter extraction statistics.
#[derive(Debug, Default)]
pub struct ParamsStats {
    /// Match attempts.
    match_attempts: AtomicU64,
    /// Match successes.
    match_successes: AtomicU64,
    /// Params added inline.
    params_inline: AtomicU64,
    /// Params added to heap.
    params_heap: AtomicU64,
}

impl ParamsStats {
    fn record_match_attempt(&self) {
        self.match_attempts.fetch_add(1, Ordering::Relaxed);
    }

    fn record_match_success(&self) {
        self.match_successes.fetch_add(1, Ordering::Relaxed);
    }

    fn record_param_add(&self, inline: bool) {
        if inline {
            self.params_inline.fetch_add(1, Ordering::Relaxed);
        } else {
            self.params_heap.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Get match attempts.
    pub fn match_attempts(&self) -> u64 {
        self.match_attempts.load(Ordering::Relaxed)
    }

    /// Get match successes.
    pub fn match_successes(&self) -> u64 {
        self.match_successes.load(Ordering::Relaxed)
    }

    /// Get match success ratio.
    pub fn match_ratio(&self) -> f64 {
        let attempts = self.match_attempts() as f64;
        let successes = self.match_successes() as f64;
        if attempts > 0.0 {
            successes / attempts
        } else {
            0.0
        }
    }

    /// Get inline param ratio.
    pub fn inline_ratio(&self) -> f64 {
        let inline = self.params_inline.load(Ordering::Relaxed) as f64;
        let heap = self.params_heap.load(Ordering::Relaxed) as f64;
        let total = inline + heap;
        if total > 0.0 { inline / total } else { 1.0 }
    }
}

/// Global statistics.
static PARAMS_STATS: ParamsStats = ParamsStats {
    match_attempts: AtomicU64::new(0),
    match_successes: AtomicU64::new(0),
    params_inline: AtomicU64::new(0),
    params_heap: AtomicU64::new(0),
};

/// Get global parameter stats.
pub fn params_stats() -> &'static ParamsStats {
    &PARAMS_STATS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_borrowed_param() {
        let param = BorrowedParam::new("id", "123");
        assert_eq!(param.name, "id");
        assert_eq!(param.value, "123");
        assert_eq!(param.parse::<i32>().unwrap(), 123);
    }

    #[test]
    fn test_params_inline() {
        let mut params = Params::new();
        params.push("id", "123");
        params.push("name", "test");

        assert!(params.is_inline());
        assert_eq!(params.len(), 2);
        assert_eq!(params.get("id"), Some("123"));
        assert_eq!(params.get("name"), Some("test"));
    }

    #[test]
    fn test_params_wildcard() {
        let mut params = Params::new();
        params.push("id", "123");
        params.set_wildcard("path/to/file");

        assert_eq!(params.wildcard(), Some("path/to/file"));
    }

    #[test]
    fn test_pattern_segment_static() {
        let seg = PatternSegment::parse("users");
        assert_eq!(seg.segment_type, SegmentType::Static);
        assert_eq!(seg.value, "users");
    }

    #[test]
    fn test_pattern_segment_param() {
        let seg = PatternSegment::parse(":id");
        assert_eq!(seg.segment_type, SegmentType::Param);
        assert_eq!(seg.value, "id");
    }

    #[test]
    fn test_pattern_segment_wildcard() {
        let seg = PatternSegment::parse("*");
        assert_eq!(seg.segment_type, SegmentType::Wildcard);
    }

    #[test]
    fn test_pattern_segment_catch_all() {
        let seg = PatternSegment::parse("*path");
        assert_eq!(seg.segment_type, SegmentType::CatchAll);
        assert_eq!(seg.value, "path");

        let seg = PatternSegment::parse("**");
        assert_eq!(seg.segment_type, SegmentType::CatchAll);
    }

    #[test]
    fn test_compiled_pattern_static() {
        let pattern = CompiledPattern::new("/api/users");
        assert!(pattern.is_static);
        assert_eq!(pattern.segments.len(), 2);
    }

    #[test]
    fn test_compiled_pattern_with_param() {
        let pattern = CompiledPattern::new("/users/:id");
        assert!(!pattern.is_static);
        assert_eq!(pattern.segments.len(), 2);
    }

    #[test]
    fn test_compiled_pattern_match() {
        let pattern = CompiledPattern::new("/users/:id");

        let result = pattern.match_path("/users/123");
        assert!(result.is_match());

        let params = result.params().unwrap();
        assert_eq!(params.get("id"), Some("123"));
    }

    #[test]
    fn test_compiled_pattern_catch_all() {
        let pattern = CompiledPattern::new("/files/*path");

        let result = pattern.match_path("/files/docs/readme.md");
        assert!(result.is_match());

        let params = result.params().unwrap();
        assert_eq!(params.get("path"), Some("docs/readme.md"));
    }

    #[test]
    fn test_zero_alloc_match() {
        let params = match_path_zero_alloc("/users/:id", "/users/123").unwrap();
        assert_eq!(params.get("id"), Some("123"));
    }

    #[test]
    fn test_zero_alloc_match_multiple() {
        let params =
            match_path_zero_alloc("/users/:user_id/posts/:post_id", "/users/123/posts/456")
                .unwrap();
        assert_eq!(params.get("user_id"), Some("123"));
        assert_eq!(params.get("post_id"), Some("456"));
    }

    #[test]
    fn test_zero_alloc_no_match() {
        let result = match_path_zero_alloc("/users/:id", "/posts/123");
        assert!(result.is_none());
    }

    #[test]
    fn test_zero_alloc_catch_all() {
        let params = match_path_zero_alloc("/files/*path", "/files/docs/readme.md").unwrap();
        assert_eq!(params.get("path"), Some("docs/readme.md"));
        assert_eq!(params.wildcard(), Some("docs/readme.md"));
    }

    #[test]
    fn test_extract_params_tuple() {
        let mut params = Params::new();
        params.push("id", "123");
        params.push("name", "test");

        let (id,): (i32,) = ExtractParams::extract(&params).unwrap();
        assert_eq!(id, 123);
    }

    #[test]
    fn test_extract_params_two() {
        let mut params = Params::new();
        params.push("user_id", "123");
        params.push("post_id", "456");

        let (user_id, post_id): (i32, i32) = ExtractParams::extract(&params).unwrap();
        assert_eq!(user_id, 123);
        assert_eq!(post_id, 456);
    }

    #[test]
    fn test_params_to_hash_map() {
        let mut params = Params::new();
        params.push("id", "123");
        params.push("name", "test");

        let map = params.to_hash_map();
        assert_eq!(map.get("id"), Some(&"123".to_string()));
        assert_eq!(map.get("name"), Some(&"test".to_string()));
    }

    #[test]
    fn test_catch_all_value_owned_no_leak() {
        // Compile once, match many times. Each catch-all match produces an
        // owned (non-leaked) joined value; the returned `Params` owns it.
        let pattern = CompiledPattern::new("/files/*path");

        for n in 0..1000 {
            let path = format!("/files/dir{n}/sub/readme.md");
            let params = pattern.match_path(&path).params().unwrap();
            // Named catch-all is retrievable by name...
            assert_eq!(
                params.get("path"),
                Some(format!("dir{n}/sub/readme.md").as_str())
            );
            // ...and via the wildcard accessor.
            assert_eq!(
                params.wildcard(),
                Some(format!("dir{n}/sub/readme.md").as_str())
            );
        }
    }

    #[test]
    fn test_catch_all_owns_beyond_path_lifetime() {
        // The joined catch-all value must remain valid after the source path
        // string is dropped — proving it is owned, not borrowed/leaked.
        let pattern = CompiledPattern::new("/files/*path");
        let owned_map = {
            let path = String::from("/files/docs/guide/intro.md");
            let params = pattern.match_path(&path).params().unwrap();
            params.to_hash_map()
        };
        assert_eq!(
            owned_map.get("path"),
            Some(&"docs/guide/intro.md".to_string())
        );
        assert_eq!(owned_map.get("*"), Some(&"docs/guide/intro.md".to_string()));
    }

    #[test]
    fn test_zero_alloc_catch_all_named_get() {
        let params = match_path_zero_alloc("/assets/*rest", "/assets/css/app.css").unwrap();
        assert_eq!(params.get("rest"), Some("css/app.css"));
        assert_eq!(params.wildcard(), Some("css/app.css"));
    }

    #[test]
    fn test_stats() {
        let stats = params_stats();
        let _ = stats.match_attempts();
        let _ = stats.match_successes();
        let _ = stats.match_ratio();
        let _ = stats.inline_ratio();
    }
}
