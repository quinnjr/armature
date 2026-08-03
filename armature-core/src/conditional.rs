//! ETag and conditional request handling.
//!
//! This module provides support for HTTP conditional requests, enabling
//! efficient caching and optimistic concurrency control.
//!
//! # Supported Headers
//!
//! - `ETag` - Entity tag for resource versioning
//! - `If-None-Match` - Conditional GET (return 304 if ETag matches)
//! - `If-Match` - Conditional PUT/DELETE (fail if ETag doesn't match)
//! - `If-Modified-Since` - Conditional GET based on modification time
//! - `If-Unmodified-Since` - Conditional PUT/DELETE based on modification time
//!
//! # Examples
//!
//! ## Conditional GET with ETag
//!
//! ```
//! use armature_core::conditional::{ETag, ConditionalRequest};
//! use armature_core::HttpRequest;
//!
//! fn handle_get(request: &HttpRequest) -> Result<(), ()> {
//!     let etag = ETag::strong("abc123");
//!
//!     // Check if client has current version
//!     if request.if_none_match_matches(&etag) {
//!         // Return 304 Not Modified
//!         return Ok(());
//!     }
//!
//!     // Return full response with ETag
//!     Ok(())
//! }
//! ```
//!
//! ## Optimistic Concurrency with If-Match
//!
//! ```
//! use armature_core::conditional::{ETag, ConditionalRequest};
//! use armature_core::HttpRequest;
//!
//! fn handle_update(request: &HttpRequest, current_etag: &ETag) -> Result<(), ()> {
//!     // Verify client has current version before updating
//!     if !request.if_match_matches(current_etag) {
//!         // Return 412 Precondition Failed
//!         return Err(());
//!     }
//!
//!     // Proceed with update
//!     Ok(())
//! }
//! ```

use crate::{Error, HttpRequest, HttpResponse};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::time::SystemTime;

// ============================================================================
// ETag
// ============================================================================

/// Represents an HTTP ETag (Entity Tag).
///
/// ETags come in two varieties:
/// - **Strong ETags**: Byte-for-byte identical (`"abc123"`)
/// - **Weak ETags**: Semantically equivalent (`W/"abc123"`)
///
/// # Examples
///
/// ```
/// use armature_core::conditional::ETag;
///
/// // Create a strong ETag
/// let strong = ETag::strong("abc123");
/// assert_eq!(strong.to_header_value(), "\"abc123\"");
///
/// // Create a weak ETag
/// let weak = ETag::weak("abc123");
/// assert_eq!(weak.to_header_value(), "W/\"abc123\"");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ETag {
    /// The tag value (without quotes)
    pub value: String,
    /// Whether this is a weak ETag
    pub weak: bool,
}

impl ETag {
    /// Create a strong ETag.
    ///
    /// Strong ETags indicate byte-for-byte identity. Use for content
    /// that must be exactly identical.
    pub fn strong(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            weak: false,
        }
    }

    /// Create a weak ETag.
    ///
    /// Weak ETags indicate semantic equivalence. Use when minor
    /// variations (like whitespace) are acceptable.
    pub fn weak(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            weak: true,
        }
    }

    /// Parse an ETag from a header value.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_core::conditional::ETag;
    ///
    /// let strong = ETag::parse("\"abc123\"").unwrap();
    /// assert!(!strong.weak);
    /// assert_eq!(strong.value, "abc123");
    ///
    /// let weak = ETag::parse("W/\"abc123\"").unwrap();
    /// assert!(weak.weak);
    /// assert_eq!(weak.value, "abc123");
    /// ```
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();

        let (weak, value_part) = if s.starts_with("W/") || s.starts_with("w/") {
            (true, &s[2..])
        } else {
            (false, s)
        };

        // Extract value from quotes
        let value = value_part.strip_prefix('"')?.strip_suffix('"')?.to_string();

        Some(Self { value, weak })
    }

    /// Generate an ETag from bytes using a hash.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_core::conditional::ETag;
    ///
    /// let data = b"Hello, World!";
    /// let etag = ETag::from_bytes(data);
    /// assert!(!etag.weak);
    /// ```
    pub fn from_bytes(data: &[u8]) -> Self {
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        let hash = hasher.finish();

        Self::strong(format!("{:x}", hash))
    }

    /// Generate a weak ETag from bytes.
    pub fn weak_from_bytes(data: &[u8]) -> Self {
        let mut etag = Self::from_bytes(data);
        etag.weak = true;
        etag
    }

    /// Generate an ETag from a string using a hash.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self::from_bytes(s.as_bytes())
    }

    /// Generate an ETag from file metadata.
    ///
    /// Creates an ETag based on file size and modification time.
    pub fn from_file_metadata(size: u64, modified: SystemTime) -> Self {
        let modified_unix = modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self::strong(format!("{:x}-{:x}", size, modified_unix))
    }

    /// Generate an ETag from a version number or revision.
    pub fn from_version(version: u64) -> Self {
        Self::strong(format!("v{}", version))
    }

    /// Get the header value representation.
    pub fn to_header_value(&self) -> String {
        if self.weak {
            format!("W/\"{}\"", self.value)
        } else {
            format!("\"{}\"", self.value)
        }
    }

    /// Check if this ETag matches another using strong comparison.
    ///
    /// Strong comparison: Both ETags must be strong and have identical values.
    pub fn strong_match(&self, other: &ETag) -> bool {
        !self.weak && !other.weak && self.value == other.value
    }

    /// Check if this ETag matches another using weak comparison.
    ///
    /// Weak comparison: Values must match (weak flag is ignored).
    pub fn weak_match(&self, other: &ETag) -> bool {
        self.value == other.value
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_header_value())
    }
}

// ============================================================================
// ETag List (for If-None-Match, If-Match)
// ============================================================================

/// Represents a list of ETags from If-None-Match or If-Match headers.
#[derive(Debug, Clone, Default)]
pub struct ETagList {
    /// The list of ETags
    pub etags: Vec<ETag>,
    /// Whether the header contains a wildcard "*"
    pub any: bool,
}

impl ETagList {
    /// Create an empty ETag list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an ETag list that matches any ETag.
    pub fn any() -> Self {
        Self {
            etags: Vec::new(),
            any: true,
        }
    }

    /// Parse an ETag list from a header value.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_core::conditional::ETagList;
    ///
    /// let list = ETagList::parse("\"abc\", \"def\", W/\"ghi\"");
    /// assert_eq!(list.etags.len(), 3);
    ///
    /// let any = ETagList::parse("*");
    /// assert!(any.any);
    /// ```
    pub fn parse(header: &str) -> Self {
        let header = header.trim();

        // Check for wildcard
        if header == "*" {
            return Self::any();
        }

        let etags: Vec<ETag> = header
            .split(',')
            .filter_map(|s| ETag::parse(s.trim()))
            .collect();

        Self { etags, any: false }
    }

    /// Check if any ETag in the list matches (weak comparison).
    pub fn contains_weak(&self, etag: &ETag) -> bool {
        if self.any {
            return true;
        }
        self.etags.iter().any(|e| e.weak_match(etag))
    }

    /// Check if any ETag in the list matches (strong comparison).
    pub fn contains_strong(&self, etag: &ETag) -> bool {
        if self.any {
            // RFC 7232 §3.1: "*" matches whenever the resource has any
            // current representation, regardless of ETag weakness.
            return true;
        }
        self.etags.iter().any(|e| e.strong_match(etag))
    }

    /// Check if the list is empty (no ETags and not a wildcard).
    pub fn is_empty(&self) -> bool {
        !self.any && self.etags.is_empty()
    }
}

// ============================================================================
// Conditional Request Headers
// ============================================================================

/// Parsed conditional request headers.
#[derive(Debug, Clone, Default)]
pub struct ConditionalHeaders {
    /// If-None-Match header (for conditional GET)
    pub if_none_match: Option<ETagList>,
    /// If-Match header (for conditional PUT/DELETE)
    pub if_match: Option<ETagList>,
    /// If-Modified-Since header
    pub if_modified_since: Option<SystemTime>,
    /// If-Unmodified-Since header
    pub if_unmodified_since: Option<SystemTime>,
}

impl ConditionalHeaders {
    /// Parse conditional headers from an HTTP request.
    ///
    /// Lookups use the canonical casing only: `HeaderMap::get` compares names
    /// with `eq_ignore_ascii_case`, so a client sending `if-none-match` is
    /// matched by the same call that matches `If-None-Match`.
    pub fn from_request(request: &HttpRequest) -> Self {
        let if_none_match = request
            .headers
            .get("If-None-Match")
            .map(|h| ETagList::parse(h));

        let if_match = request.headers.get("If-Match").map(|h| ETagList::parse(h));

        let if_modified_since = request
            .headers
            .get("If-Modified-Since")
            .and_then(|h| httpdate::parse_http_date(h).ok());

        let if_unmodified_since = request
            .headers
            .get("If-Unmodified-Since")
            .and_then(|h| httpdate::parse_http_date(h).ok());

        Self {
            if_none_match,
            if_match,
            if_modified_since,
            if_unmodified_since,
        }
    }

    /// Check if the resource should return 304 Not Modified.
    ///
    /// Returns true if:
    /// - If-None-Match contains a matching ETag (weak comparison), or
    /// - If-Modified-Since is after the resource's last modification
    pub fn is_not_modified(&self, etag: Option<&ETag>, last_modified: Option<SystemTime>) -> bool {
        // Check If-None-Match first (takes precedence)
        if let Some(ref if_none_match) = self.if_none_match
            && let Some(etag) = etag
        {
            return if_none_match.contains_weak(etag);
        }

        // Check If-Modified-Since
        if let (Some(if_modified_since), Some(last_modified)) =
            (self.if_modified_since, last_modified)
        {
            return last_modified <= if_modified_since;
        }

        false
    }

    /// Check if the precondition fails (should return 412).
    ///
    /// Returns true if:
    /// - If-Match is present and no ETag matches (strong comparison), or
    /// - If-Unmodified-Since is before the resource's last modification
    pub fn precondition_failed(
        &self,
        etag: Option<&ETag>,
        last_modified: Option<SystemTime>,
    ) -> bool {
        // Check If-Match first (takes precedence)
        if let Some(ref if_match) = self.if_match {
            if let Some(etag) = etag {
                return !if_match.contains_strong(etag);
            } else {
                // If-Match present but no ETag to compare - fail
                return !if_match.any;
            }
        }

        // Check If-Unmodified-Since
        if let (Some(if_unmodified_since), Some(last_modified)) =
            (self.if_unmodified_since, last_modified)
        {
            return last_modified > if_unmodified_since;
        }

        false
    }
}

// ============================================================================
// Request Extensions
// ============================================================================

/// Extension trait for conditional request handling on HttpRequest.
pub trait ConditionalRequest {
    /// Get parsed conditional headers from the request.
    fn conditional_headers(&self) -> ConditionalHeaders;

    /// Get the If-None-Match header as an ETag list.
    fn if_none_match(&self) -> Option<ETagList>;

    /// Get the If-Match header as an ETag list.
    fn if_match(&self) -> Option<ETagList>;

    /// Get the If-Modified-Since header as a SystemTime.
    fn if_modified_since(&self) -> Option<SystemTime>;

    /// Get the If-Unmodified-Since header as a SystemTime.
    fn if_unmodified_since(&self) -> Option<SystemTime>;

    /// Check if If-None-Match contains a matching ETag (weak comparison).
    ///
    /// Returns true if the request should get a 304 Not Modified response.
    fn if_none_match_matches(&self, etag: &ETag) -> bool;

    /// Check if If-Match contains a matching ETag (strong comparison).
    ///
    /// Returns false if the precondition fails (should return 412).
    fn if_match_matches(&self, etag: &ETag) -> bool;

    /// Check if If-Modified-Since indicates the resource hasn't changed.
    fn not_modified_since(&self, last_modified: SystemTime) -> bool;

    /// Check if If-Unmodified-Since precondition fails.
    fn modified_since_precondition(&self, last_modified: SystemTime) -> bool;

    /// Evaluate all conditional headers and return the appropriate response.
    ///
    /// Returns:
    /// - `Some(304)` if resource is not modified
    /// - `Some(412)` if precondition failed
    /// - `None` if request should proceed normally
    fn evaluate_conditionals(
        &self,
        etag: Option<&ETag>,
        last_modified: Option<SystemTime>,
    ) -> Option<u16>;
}

impl ConditionalRequest for HttpRequest {
    fn conditional_headers(&self) -> ConditionalHeaders {
        ConditionalHeaders::from_request(self)
    }

    fn if_none_match(&self) -> Option<ETagList> {
        self.headers
            .get("If-None-Match")
            .map(|h| ETagList::parse(h))
    }

    fn if_match(&self) -> Option<ETagList> {
        self.headers.get("If-Match").map(|h| ETagList::parse(h))
    }

    fn if_modified_since(&self) -> Option<SystemTime> {
        self.headers
            .get("If-Modified-Since")
            .and_then(|h| httpdate::parse_http_date(h).ok())
    }

    fn if_unmodified_since(&self) -> Option<SystemTime> {
        self.headers
            .get("If-Unmodified-Since")
            .and_then(|h| httpdate::parse_http_date(h).ok())
    }

    fn if_none_match_matches(&self, etag: &ETag) -> bool {
        self.if_none_match()
            .map(|list| list.contains_weak(etag))
            .unwrap_or(false)
    }

    fn if_match_matches(&self, etag: &ETag) -> bool {
        match self.if_match() {
            Some(list) => list.contains_strong(etag),
            None => true, // No If-Match header means proceed
        }
    }

    fn not_modified_since(&self, last_modified: SystemTime) -> bool {
        self.if_modified_since()
            .map(|since| last_modified <= since)
            .unwrap_or(false)
    }

    fn modified_since_precondition(&self, last_modified: SystemTime) -> bool {
        self.if_unmodified_since()
            .map(|since| last_modified > since)
            .unwrap_or(false)
    }

    fn evaluate_conditionals(
        &self,
        etag: Option<&ETag>,
        last_modified: Option<SystemTime>,
    ) -> Option<u16> {
        let headers = self.conditional_headers();

        // Check preconditions first (412)
        if headers.precondition_failed(etag, last_modified) {
            return Some(412);
        }

        let method = self.method.to_uppercase();
        let is_safe = method == "GET" || method == "HEAD";

        if is_safe {
            // Check not modified (304) - only for safe methods
            if headers.is_not_modified(etag, last_modified) {
                return Some(304);
            }
        } else if let (Some(if_none_match), Some(etag)) = (&headers.if_none_match, etag) {
            // RFC 7232 §3.2: a matching If-None-Match on an unsafe method
            // (PUT/POST/DELETE/...) must fail with 412 Precondition Failed.
            if if_none_match.contains_weak(etag) {
                return Some(412);
            }
        }

        None
    }
}

// ============================================================================
// Response Extensions
// ============================================================================

/// Extension trait for conditional response handling on HttpResponse.
pub trait ConditionalResponse {
    /// Set the ETag header on the response.
    fn with_etag(self, etag: &ETag) -> Self;

    /// Set the Last-Modified header on the response.
    fn with_last_modified(self, time: SystemTime) -> Self;

    /// Create a 304 Not Modified response.
    fn not_modified() -> Self;

    /// Create a 304 Not Modified response with an ETag.
    fn not_modified_with_etag(etag: &ETag) -> Self;

    /// Create a 412 Precondition Failed response.
    fn precondition_failed() -> Self;

    /// Create a 412 Precondition Failed response with a message.
    fn precondition_failed_with_message(message: &str) -> Self;
}

impl ConditionalResponse for HttpResponse {
    fn with_etag(mut self, etag: &ETag) -> Self {
        self.headers
            .insert("ETag".to_string(), etag.to_header_value());
        self
    }

    fn with_last_modified(mut self, time: SystemTime) -> Self {
        let formatted = httpdate::fmt_http_date(time);
        self.headers.insert("Last-Modified".to_string(), formatted);
        self
    }

    fn not_modified() -> Self {
        Self::new(304)
    }

    fn not_modified_with_etag(etag: &ETag) -> Self {
        let mut response = Self::new(304);
        response
            .headers
            .insert("ETag".to_string(), etag.to_header_value());
        response
    }

    fn precondition_failed() -> Self {
        Self::new(412)
    }

    fn precondition_failed_with_message(message: &str) -> Self {
        let body = serde_json::json!({
            "error": "Precondition Failed",
            "message": message,
            "status": 412
        });

        let mut response = Self::new(412);
        if let Ok(body_bytes) = serde_json::to_vec(&body) {
            response.body = body_bytes;
            response
                .headers
                .insert("Content-Type".to_string(), "application/json".to_string());
        }
        response
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check conditional headers and return appropriate response or proceed.
///
/// This is a convenience function that handles the common pattern of:
/// 1. Check preconditions (return 412 if failed)
/// 2. Check not-modified (return 304 if not modified)
/// 3. Continue with normal processing
///
/// # Example
///
/// ```ignore
/// use armature_core::conditional::{check_conditionals, ETag};
///
/// #[get("/resource/:id")]
/// async fn get_resource(request: HttpRequest) -> Result<HttpResponse, Error> {
///     let resource = load_resource();
///     let etag = ETag::from_version(resource.version);
///     let last_modified = resource.updated_at;
///
///     // Check conditionals - returns early if 304 or 412
///     if let Some(response) = check_conditionals(&request, Some(&etag), Some(last_modified)) {
///         return Ok(response);
///     }
///
///     // Normal response
///     HttpResponse::ok()
///         .with_etag(&etag)
///         .with_last_modified(last_modified)
///         .with_json(&resource)
/// }
/// ```
pub fn check_conditionals(
    request: &HttpRequest,
    etag: Option<&ETag>,
    last_modified: Option<SystemTime>,
) -> Option<HttpResponse> {
    match request.evaluate_conditionals(etag, last_modified) {
        Some(304) => {
            let mut response = HttpResponse::not_modified();
            if let Some(etag) = etag {
                response = response.with_etag(etag);
            }
            if let Some(lm) = last_modified {
                response = response.with_last_modified(lm);
            }
            Some(response)
        }
        Some(412) => Some(HttpResponse::precondition_failed_with_message(
            "Resource has been modified",
        )),
        _ => None,
    }
}

/// Generate a cache-friendly response with ETag and Last-Modified headers.
///
/// # Example
///
/// ```ignore
/// use armature_core::conditional::{cacheable_response, ETag};
///
/// let data = get_data();
/// let etag = ETag::from_bytes(&serde_json::to_vec(&data)?);
/// let response = cacheable_response(data, &etag, Some(last_modified))?;
/// ```
pub fn cacheable_response<T: serde::Serialize>(
    data: &T,
    etag: &ETag,
    last_modified: Option<SystemTime>,
) -> Result<HttpResponse, Error> {
    let mut response = HttpResponse::ok().with_json(data)?.with_etag(etag);

    if let Some(lm) = last_modified {
        response = response.with_last_modified(lm);
    }

    // Add cache headers
    response.headers.insert(
        "Cache-Control".to_string(),
        "private, must-revalidate".to_string(),
    );
    response
        .headers
        .insert("Vary".to_string(), "Accept, Accept-Encoding".to_string());

    Ok(response)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_etag_strong() {
        let etag = ETag::strong("abc123");
        assert!(!etag.weak);
        assert_eq!(etag.value, "abc123");
        assert_eq!(etag.to_header_value(), "\"abc123\"");
    }

    #[test]
    fn test_etag_weak() {
        let etag = ETag::weak("abc123");
        assert!(etag.weak);
        assert_eq!(etag.value, "abc123");
        assert_eq!(etag.to_header_value(), "W/\"abc123\"");
    }

    #[test]
    fn test_etag_parse_strong() {
        let etag = ETag::parse("\"abc123\"").unwrap();
        assert!(!etag.weak);
        assert_eq!(etag.value, "abc123");
    }

    #[test]
    fn test_etag_parse_weak() {
        let etag = ETag::parse("W/\"abc123\"").unwrap();
        assert!(etag.weak);
        assert_eq!(etag.value, "abc123");
    }

    #[test]
    fn test_etag_parse_weak_lowercase() {
        let etag = ETag::parse("w/\"abc123\"").unwrap();
        assert!(etag.weak);
        assert_eq!(etag.value, "abc123");
    }

    #[test]
    fn test_etag_from_bytes() {
        let data = b"Hello, World!";
        let etag1 = ETag::from_bytes(data);
        let etag2 = ETag::from_bytes(data);
        assert_eq!(etag1.value, etag2.value);
        assert!(!etag1.weak);
    }

    #[test]
    fn test_etag_from_version() {
        let etag = ETag::from_version(42);
        assert_eq!(etag.value, "v42");
        assert!(!etag.weak);
    }

    #[test]
    fn test_etag_strong_match() {
        let e1 = ETag::strong("abc");
        let e2 = ETag::strong("abc");
        let e3 = ETag::weak("abc");

        assert!(e1.strong_match(&e2));
        assert!(!e1.strong_match(&e3)); // Weak doesn't strong-match
    }

    #[test]
    fn test_etag_weak_match() {
        let e1 = ETag::strong("abc");
        let e2 = ETag::weak("abc");

        assert!(e1.weak_match(&e2)); // Values match
    }

    #[test]
    fn test_etag_list_parse() {
        let list = ETagList::parse("\"abc\", \"def\", W/\"ghi\"");
        assert_eq!(list.etags.len(), 3);
        assert!(!list.any);
    }

    #[test]
    fn test_etag_list_parse_wildcard() {
        let list = ETagList::parse("*");
        assert!(list.any);
        assert!(list.etags.is_empty());
    }

    #[test]
    fn test_etag_list_contains_weak() {
        let list = ETagList::parse("\"abc\", W/\"def\"");
        let strong_abc = ETag::strong("abc");
        let weak_abc = ETag::weak("abc");
        let strong_xyz = ETag::strong("xyz");

        assert!(list.contains_weak(&strong_abc));
        assert!(list.contains_weak(&weak_abc)); // Weak comparison
        assert!(!list.contains_weak(&strong_xyz));
    }

    #[test]
    fn test_etag_list_contains_strong() {
        let list = ETagList::parse("\"abc\", W/\"def\"");
        let strong_abc = ETag::strong("abc");
        let weak_abc = ETag::weak("abc");
        let strong_def = ETag::strong("def");

        assert!(list.contains_strong(&strong_abc));
        assert!(!list.contains_strong(&weak_abc)); // Weak ETag doesn't strong-match
        assert!(!list.contains_strong(&strong_def)); // W/"def" doesn't strong-match "def"
    }

    #[test]
    fn test_etag_list_wildcard_contains() {
        let list = ETagList::any();
        let etag = ETag::strong("anything");

        assert!(list.contains_weak(&etag));
        assert!(list.contains_strong(&etag));
    }

    #[test]
    fn test_etag_list_wildcard_matches_weak_etag() {
        // RFC 7232 §3.1: "*" succeeds whenever the resource exists,
        // even if its current ETag is weak.
        let list = ETagList::any();
        let weak = ETag::weak("abc123");

        assert!(list.contains_strong(&weak));
        assert!(list.contains_weak(&weak));
    }

    #[test]
    fn test_if_match_wildcard_with_weak_etag_succeeds() {
        let mut request = HttpRequest::new("PUT".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-Match".to_string(), "*".to_string());

        let weak = ETag::weak("abc123");
        assert_eq!(request.evaluate_conditionals(Some(&weak), None), None);
    }

    #[test]
    fn test_if_none_match_unsafe_method_412() {
        // RFC 7232 §3.2: matching If-None-Match on an unsafe method → 412
        for method in ["PUT", "POST", "DELETE", "PATCH"] {
            let mut request = HttpRequest::new(method.to_string(), "/resource".to_string());
            request
                .headers
                .insert("If-None-Match".to_string(), "\"abc123\"".to_string());

            let etag = ETag::strong("abc123");
            assert_eq!(
                request.evaluate_conditionals(Some(&etag), None),
                Some(412),
                "expected 412 for {}",
                method
            );
        }
    }

    #[test]
    fn test_if_none_match_unsafe_method_no_match_proceeds() {
        let mut request = HttpRequest::new("PUT".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-None-Match".to_string(), "\"abc123\"".to_string());

        let etag = ETag::strong("different");
        assert_eq!(request.evaluate_conditionals(Some(&etag), None), None);
    }

    #[test]
    fn test_conditional_headers_if_none_match() {
        let mut request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-None-Match".to_string(), "\"abc123\"".to_string());

        let headers = ConditionalHeaders::from_request(&request);
        assert!(headers.if_none_match.is_some());

        let etag = ETag::strong("abc123");
        assert!(headers.is_not_modified(Some(&etag), None));
    }

    /// Clients are free to send header names in any casing, and HTTP/2 and
    /// HTTP/3 send them lowercased on the wire, so parsing must not depend on
    /// the canonical spelling used in the lookups above.
    #[test]
    fn test_conditional_headers_lowercase_header_names() {
        let mut request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        request
            .headers
            .insert("if-none-match".to_string(), "\"abc123\"".to_string());
        request
            .headers
            .insert("if-match".to_string(), "\"abc123\"".to_string());
        request.headers.insert(
            "if-modified-since".to_string(),
            "Sun, 06 Nov 1994 08:49:37 GMT".to_string(),
        );
        request.headers.insert(
            "if-unmodified-since".to_string(),
            "Sun, 06 Nov 1994 08:49:37 GMT".to_string(),
        );

        let headers = ConditionalHeaders::from_request(&request);
        assert!(headers.if_none_match.is_some());
        assert!(headers.if_match.is_some());
        assert!(headers.if_modified_since.is_some());
        assert!(headers.if_unmodified_since.is_some());

        let etag = ETag::strong("abc123");
        assert!(headers.is_not_modified(Some(&etag), None));

        // The ConditionalRequest accessors read the same headers directly.
        assert!(request.if_none_match().is_some());
        assert!(request.if_match().is_some());
        assert!(request.if_modified_since().is_some());
        assert!(request.if_unmodified_since().is_some());
    }

    #[test]
    fn test_conditional_headers_if_match() {
        let mut request = HttpRequest::new("PUT".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-Match".to_string(), "\"abc123\"".to_string());

        let headers = ConditionalHeaders::from_request(&request);
        assert!(headers.if_match.is_some());

        let matching = ETag::strong("abc123");
        let non_matching = ETag::strong("xyz789");

        assert!(!headers.precondition_failed(Some(&matching), None));
        assert!(headers.precondition_failed(Some(&non_matching), None));
    }

    #[test]
    fn test_request_if_none_match_matches() {
        let mut request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-None-Match".to_string(), "\"abc123\"".to_string());

        let matching = ETag::strong("abc123");
        let non_matching = ETag::strong("xyz789");

        assert!(request.if_none_match_matches(&matching));
        assert!(!request.if_none_match_matches(&non_matching));
    }

    #[test]
    fn test_request_if_match_matches() {
        let mut request = HttpRequest::new("PUT".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-Match".to_string(), "\"abc123\"".to_string());

        let matching = ETag::strong("abc123");
        let non_matching = ETag::strong("xyz789");

        assert!(request.if_match_matches(&matching));
        assert!(!request.if_match_matches(&non_matching));
    }

    #[test]
    fn test_request_evaluate_conditionals_304() {
        let mut request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-None-Match".to_string(), "\"abc123\"".to_string());

        let etag = ETag::strong("abc123");
        assert_eq!(request.evaluate_conditionals(Some(&etag), None), Some(304));
    }

    #[test]
    fn test_request_evaluate_conditionals_412() {
        let mut request = HttpRequest::new("PUT".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-Match".to_string(), "\"abc123\"".to_string());

        let etag = ETag::strong("xyz789");
        assert_eq!(request.evaluate_conditionals(Some(&etag), None), Some(412));
    }

    #[test]
    fn test_request_evaluate_conditionals_proceed() {
        let request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        let etag = ETag::strong("abc123");

        // No conditional headers - should proceed
        assert_eq!(request.evaluate_conditionals(Some(&etag), None), None);
    }

    #[test]
    fn test_response_with_etag() {
        let etag = ETag::strong("abc123");
        let response = HttpResponse::ok().with_etag(&etag);

        assert_eq!(
            response.headers.get("ETag"),
            Some(&"\"abc123\"".to_string())
        );
    }

    #[test]
    fn test_response_not_modified() {
        let response = HttpResponse::not_modified();
        assert_eq!(response.status, 304);
    }

    #[test]
    fn test_response_precondition_failed() {
        let response = HttpResponse::precondition_failed();
        assert_eq!(response.status, 412);
    }

    #[test]
    fn test_check_conditionals_returns_304() {
        let mut request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-None-Match".to_string(), "\"abc123\"".to_string());

        let etag = ETag::strong("abc123");
        let response = check_conditionals(&request, Some(&etag), None);

        assert!(response.is_some());
        assert_eq!(response.unwrap().status, 304);
    }

    #[test]
    fn test_check_conditionals_returns_412() {
        let mut request = HttpRequest::new("PUT".to_string(), "/resource".to_string());
        request
            .headers
            .insert("If-Match".to_string(), "\"abc123\"".to_string());

        let etag = ETag::strong("different");
        let response = check_conditionals(&request, Some(&etag), None);

        assert!(response.is_some());
        assert_eq!(response.unwrap().status, 412);
    }

    #[test]
    fn test_check_conditionals_returns_none() {
        let request = HttpRequest::new("GET".to_string(), "/resource".to_string());
        let etag = ETag::strong("abc123");

        let response = check_conditionals(&request, Some(&etag), None);
        assert!(response.is_none());
    }
}
