//! SmallVec-Based HTTP Header Storage
//!
//! This module provides a stack-allocated header map for typical HTTP requests.
//! Most requests have fewer than 8-12 headers, so storing them inline avoids
//! heap allocations entirely.
//!
//! ## Performance
//!
//! - **Inline storage**: Up to 12 headers stored on stack (no allocation)
//! - **O(n) lookup**: Linear scan, but N is typically small (<20)
//! - **Cache-friendly**: Contiguous memory layout
//! - **Zero heap alloc**: For typical requests
//!
//! ## Comparison
//!
//! | Operation | HashMap | HeaderMap (SmallVec) |
//! |-----------|---------|---------------------|
//! | Insert (first 12) | Heap alloc | Stack only |
//! | Lookup | O(1) hash | O(n) linear |
//! | Memory | ~48 bytes min + heap | ~384 bytes inline |
//! | Cache | Pointer chasing | Contiguous |
//!
//! For typical HTTP workloads with <12 headers, SmallVec is faster due to
//! avoiding allocator overhead and better cache locality.

use smallvec::SmallVec;
use std::collections::HashMap;
use std::fmt;

/// Number of headers to store inline (on stack).
/// 12 headers × 32 bytes = 384 bytes, reasonable stack footprint.
/// Most HTTP requests have 5-10 headers.
pub const INLINE_HEADERS: usize = 12;

/// A header name-value pair.
#[derive(Clone, PartialEq, Eq)]
pub struct Header {
    /// Header name (case-insensitive for lookup)
    pub name: String,
    /// Header value
    pub value: String,
}

impl Header {
    /// Create a new header
    #[inline]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Check if name matches (case-insensitive)
    #[inline]
    pub fn name_eq(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name)
    }
}

impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.value)
    }
}

/// A compact header map using SmallVec for inline storage.
///
/// Stores up to 12 headers inline (on the stack), only allocating
/// on the heap if more headers are added.
///
/// # Example
///
/// ```rust
/// use armature_core::headers::HeaderMap;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("Content-Type", "application/json");
/// headers.insert("Accept", "text/html");
///
/// assert_eq!(headers.get("content-type"), Some(&"application/json".to_string()));
/// assert!(headers.is_inline()); // Still on stack
/// ```
#[derive(Clone, Default)]
pub struct HeaderMap {
    inner: SmallVec<[Header; INLINE_HEADERS]>,
}

impl HeaderMap {
    /// Create a new empty header map.
    #[inline]
    pub const fn new() -> Self {
        Self {
            inner: SmallVec::new_const(),
        }
    }

    /// Create with pre-allocated capacity.
    ///
    /// If capacity <= INLINE_HEADERS, no heap allocation occurs.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: SmallVec::with_capacity(capacity),
        }
    }

    /// Check if storage is inline (no heap allocation).
    #[inline]
    pub fn is_inline(&self) -> bool {
        !self.inner.spilled()
    }

    /// Get the number of headers.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get header value by name (case-insensitive).
    #[inline]
    pub fn get(&self, name: &str) -> Option<&String> {
        self.inner
            .iter()
            .find(|h| h.name_eq(name))
            .map(|h| &h.value)
    }

    /// Get header value by name, with lowercase fallback.
    ///
    /// First tries exact match, then lowercase. This handles
    /// the common case where headers might be stored in different cases.
    #[inline]
    pub fn get_ignore_case(&self, name: &str) -> Option<&String> {
        self.get(name)
    }

    /// Check if header exists (case-insensitive).
    #[inline]
    pub fn contains(&self, name: &str) -> bool {
        self.inner.iter().any(|h| h.name_eq(name))
    }

    /// Check if header exists (case-insensitive).
    ///
    /// HashMap-compatible alias for [`contains`](Self::contains).
    #[inline]
    pub fn contains_key(&self, name: &str) -> bool {
        self.contains(name)
    }

    /// Insert a header, replacing any existing header with same name.
    ///
    /// Returns the old value if replaced.
    #[inline]
    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) -> Option<String> {
        let name = name.into();
        let value = value.into();

        // Check if header exists
        for h in &mut self.inner {
            if h.name_eq(&name) {
                let old = std::mem::replace(&mut h.value, value);
                return Some(old);
            }
        }

        // New header
        self.inner.push(Header { name, value });
        None
    }

    /// Append a header (allows duplicates).
    ///
    /// Unlike `insert`, this doesn't replace existing headers.
    /// Use for headers that can have multiple values (e.g., Set-Cookie).
    #[inline]
    pub fn append(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.inner.push(Header {
            name: name.into(),
            value: value.into(),
        });
    }

    /// Remove a header by name (case-insensitive).
    ///
    /// Returns the removed value if found.
    #[inline]
    pub fn remove(&mut self, name: &str) -> Option<String> {
        if let Some(pos) = self.inner.iter().position(|h| h.name_eq(name)) {
            Some(self.inner.remove(pos).value)
        } else {
            None
        }
    }

    /// Remove all headers with given name (case-insensitive).
    ///
    /// Returns number of headers removed.
    #[inline]
    pub fn remove_all(&mut self, name: &str) -> usize {
        let before = self.inner.len();
        self.inner.retain(|h| !h.name_eq(name));
        before - self.inner.len()
    }

    /// Iterate over all headers.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.inner.iter().map(|h| (&h.name, &h.value))
    }

    /// Iterate over header names.
    #[inline]
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.inner.iter().map(|h| &h.name)
    }

    /// Iterate over header names.
    ///
    /// HashMap-compatible alias for [`names`](Self::names).
    #[inline]
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.names()
    }

    /// Iterate over header values.
    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &String> {
        self.inner.iter().map(|h| &h.value)
    }

    /// Get all values for a header name (for multi-value headers).
    #[inline]
    pub fn get_all(&self, name: &str) -> Vec<&String> {
        self.inner
            .iter()
            .filter(|h| h.name_eq(name))
            .map(|h| &h.value)
            .collect()
    }

    /// Clear all headers.
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Extend with headers from iterator.
    #[inline]
    pub fn extend<I, K, V>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }

    /// Convert to HashMap (for compatibility).
    #[inline]
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        self.inner
            .iter()
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect()
    }

    /// Create from HashMap.
    #[inline]
    pub fn from_hash_map(map: HashMap<String, String>) -> Self {
        let mut headers = Self::with_capacity(map.len());
        for (k, v) in map {
            headers.inner.push(Header { name: k, value: v });
        }
        headers
    }

    // ========================================================================
    // Common Header Accessors
    // ========================================================================

    /// Get Content-Type header.
    #[inline]
    pub fn content_type(&self) -> Option<&String> {
        self.get("Content-Type")
    }

    /// Get Content-Length header as usize.
    #[inline]
    pub fn content_length(&self) -> Option<usize> {
        self.get("Content-Length")?.parse().ok()
    }

    /// Get Accept header.
    #[inline]
    pub fn accept(&self) -> Option<&String> {
        self.get("Accept")
    }

    /// Get Authorization header.
    #[inline]
    pub fn authorization(&self) -> Option<&String> {
        self.get("Authorization")
    }

    /// Get User-Agent header.
    #[inline]
    pub fn user_agent(&self) -> Option<&String> {
        self.get("User-Agent")
    }

    /// Get Host header.
    #[inline]
    pub fn host(&self) -> Option<&String> {
        self.get("Host")
    }

    /// Get Cookie header.
    #[inline]
    pub fn cookie(&self) -> Option<&String> {
        self.get("Cookie")
    }

    /// Check if Keep-Alive connection.
    #[inline]
    pub fn is_keep_alive(&self) -> bool {
        self.get("Connection")
            .map(|v| v.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(true) // HTTP/1.1 default is keep-alive
    }

    /// Check if chunked transfer encoding.
    #[inline]
    pub fn is_chunked(&self) -> bool {
        self.get("Transfer-Encoding")
            .map(|v| v.contains("chunked"))
            .unwrap_or(false)
    }

    /// Set Content-Type header.
    #[inline]
    pub fn set_content_type(&mut self, value: impl Into<String>) {
        self.insert("Content-Type", value);
    }

    /// Set Content-Length header.
    #[inline]
    pub fn set_content_length(&mut self, len: usize) {
        self.insert("Content-Length", len.to_string());
    }
}

impl fmt::Debug for HeaderMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.inner.iter().map(|h| (&h.name, &h.value)))
            .finish()
    }
}

impl<K, V> FromIterator<(K, V)> for HeaderMap
where
    K: Into<String>,
    V: Into<String>,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let (min, max) = iter.size_hint();
        let mut map = HeaderMap::with_capacity(max.unwrap_or(min));
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl Extend<(String, String)> for HeaderMap {
    fn extend<I: IntoIterator<Item = (String, String)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl<'a> IntoIterator for &'a HeaderMap {
    type Item = (&'a String, &'a String);
    type IntoIter =
        std::iter::Map<std::slice::Iter<'a, Header>, fn(&'a Header) -> (&'a String, &'a String)>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter().map(|h| (&h.name, &h.value))
    }
}

impl IntoIterator for HeaderMap {
    type Item = (String, String);
    type IntoIter = std::iter::Map<
        smallvec::IntoIter<[Header; INLINE_HEADERS]>,
        fn(Header) -> (String, String),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter().map(|h| (h.name, h.value))
    }
}

// Allow HashMap-like indexing
impl std::ops::Index<&str> for HeaderMap {
    type Output = String;

    fn index(&self, name: &str) -> &Self::Output {
        self.get(name).expect("header not found")
    }
}

// ============================================================================
// Conversion from/to HashMap for backwards compatibility
// ============================================================================

impl From<HashMap<String, String>> for HeaderMap {
    fn from(map: HashMap<String, String>) -> Self {
        Self::from_hash_map(map)
    }
}

impl From<HeaderMap> for HashMap<String, String> {
    fn from(map: HeaderMap) -> Self {
        map.to_hash_map()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_inline() {
        let headers = HeaderMap::new();
        assert!(headers.is_inline());
        assert!(headers.is_empty());
    }

    #[test]
    fn test_insert_and_get() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
        assert_eq!(
            headers.get("content-type"),
            Some(&"application/json".to_string())
        ); // case insensitive
    }

    #[test]
    fn test_insert_replaces() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "text/plain");
        let old = headers.insert("Content-Type", "application/json");

        assert_eq!(old, Some("text/plain".to_string()));
        assert_eq!(headers.len(), 1);
        assert_eq!(
            headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_append_duplicates() {
        let mut headers = HeaderMap::new();
        headers.append("Set-Cookie", "session=abc");
        headers.append("Set-Cookie", "user=123");

        assert_eq!(headers.len(), 2);
        let cookies = headers.get_all("Set-Cookie");
        assert_eq!(cookies.len(), 2);
    }

    #[test]
    fn test_remove() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        let removed = headers.remove("Content-Type");
        assert_eq!(removed, Some("application/json".to_string()));
        assert_eq!(headers.len(), 1);
        assert!(!headers.contains("Content-Type"));
    }

    #[test]
    fn test_inline_capacity() {
        let mut headers = HeaderMap::new();

        // Add INLINE_HEADERS headers
        for i in 0..INLINE_HEADERS {
            headers.insert(format!("Header-{}", i), format!("Value-{}", i));
        }

        assert!(headers.is_inline()); // Still inline

        // Add one more
        headers.insert("Extra-Header", "Extra-Value");

        // Now it should have spilled to heap
        assert!(!headers.is_inline());
    }

    #[test]
    fn test_iter() {
        let mut headers = HeaderMap::new();
        headers.insert("A", "1");
        headers.insert("B", "2");

        let pairs: Vec<_> = headers.iter().collect();
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn test_common_accessors() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Content-Length", "100");
        headers.insert("Connection", "keep-alive");
        headers.insert("Transfer-Encoding", "chunked");

        assert_eq!(
            headers.content_type(),
            Some(&"application/json".to_string())
        );
        assert_eq!(headers.content_length(), Some(100));
        assert!(headers.is_keep_alive());
        assert!(headers.is_chunked());
    }

    #[test]
    fn test_from_hash_map() {
        let mut map = HashMap::new();
        map.insert("Content-Type".to_string(), "application/json".to_string());
        map.insert("Accept".to_string(), "text/html".to_string());

        let headers = HeaderMap::from_hash_map(map);
        assert_eq!(headers.len(), 2);
        assert!(headers.contains("Content-Type"));
    }

    #[test]
    fn test_to_hash_map() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");

        let map = headers.to_hash_map();
        assert_eq!(
            map.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_from_iterator() {
        let headers: HeaderMap = [
            ("Content-Type", "application/json"),
            ("Accept", "text/html"),
        ]
        .into_iter()
        .collect();

        assert_eq!(headers.len(), 2);
    }

    #[test]
    fn test_indexing() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");

        assert_eq!(&headers["Content-Type"], "application/json");
    }

    #[test]
    fn test_contains_key() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");

        // Case-insensitive, HashMap-compatible alias for `contains`.
        assert!(headers.contains_key("Content-Type"));
        assert!(headers.contains_key("content-type"));
        assert!(!headers.contains_key("Accept"));
    }

    #[test]
    fn test_keys() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        let keys: Vec<_> = headers.keys().cloned().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"Content-Type".to_string()));
        assert!(keys.contains(&"Accept".to_string()));
    }

    #[test]
    fn test_values() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        let values: Vec<_> = headers.values().cloned().collect();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"application/json".to_string()));
        assert!(values.contains(&"text/html".to_string()));
    }

    #[test]
    fn test_is_empty() {
        let mut headers = HeaderMap::new();
        assert!(headers.is_empty());
        headers.insert("Content-Type", "application/json");
        assert!(!headers.is_empty());
    }

    #[test]
    fn test_default() {
        let headers = HeaderMap::default();
        assert!(headers.is_empty());
        assert!(headers.is_inline());
    }

    #[test]
    fn test_extend_trait() {
        let mut headers = HeaderMap::new();
        headers.insert("Existing", "1");

        // Exercise the `Extend<(String, String)>` trait impl explicitly.
        let extra: Vec<(String, String)> = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "text/html".to_string()),
        ];
        Extend::extend(&mut headers, extra);

        assert_eq!(headers.len(), 3);
        assert_eq!(
            headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_into_iterator_owned() {
        let mut headers = HeaderMap::new();
        headers.insert("A", "1");
        headers.insert("B", "2");

        let collected: Vec<(String, String)> = headers.into_iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_into_iterator_ref() {
        let mut headers = HeaderMap::new();
        headers.insert("A", "1");

        let collected: Vec<(&String, &String)> = (&headers).into_iter().collect();
        assert_eq!(collected.len(), 1);
    }

    #[test]
    fn test_hashmap_roundtrip() {
        let mut map = HashMap::new();
        map.insert("Content-Type".to_string(), "application/json".to_string());

        // HashMap -> HeaderMap -> HashMap round-trips via `From`.
        let headers: HeaderMap = map.clone().into();
        assert!(headers.contains_key("content-type")); // case-insensitive
        let back: HashMap<String, String> = headers.into();
        assert_eq!(back.get("Content-Type"), map.get("Content-Type"));
    }
}
