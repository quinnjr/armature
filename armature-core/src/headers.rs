//! Interned, `Bytes`-backed HTTP header storage.
//!
//! Most requests have fewer than 12 headers, so they are stored inline on the
//! stack. Names are interned to a [`HeaderId`] — an enum variant for the ~33
//! well-known fields, a lowercased [`ByteStr`] for everything else — so a lookup
//! for a well-known name is an integer comparison rather than a
//! case-insensitive string compare. A custom name (`x-request-id` and friends)
//! still costs an ASCII-insensitive compare per stored header, but no
//! allocation: by-name lookups resolve the needle borrowed rather than
//! materializing a `HeaderId::Other` per call. Values are [`Bytes`], so once
//! Plan 4 wires the serve path through `armature-h1` they become slices of the
//! connection read buffer rather than copies.
//!
//! ## Case normalization
//!
//! Field names are case-insensitive (RFC 9110 §5.1), and interning settles the
//! question once: `iter()`, `keys()`, and `to_hash_map()` report lowercase names
//! regardless of how they were inserted. Lookups remain case-insensitive.
//!
//! ## Non-UTF-8 values
//!
//! A header value is bytes on the wire, not text. [`HeaderMap::get`] promises a
//! `&str` and therefore returns `None` for a value that is not valid UTF-8;
//! [`HeaderMap::get_bytes`] returns it. Handing back lossy text from `get` would
//! be worse than returning nothing, because the caller would go on to trust it.
//!
//! ## Performance
//!
//! | Operation | HashMap | HeaderMap |
//! |-----------|---------|-----------|
//! | Insert (first 12) | Heap alloc | Stack only |
//! | Lookup | O(1) hash of the name | O(n) compares, no alloc |
//! | Well-known name | `String` alloc | enum variant, no alloc |
//! | Value clone | `String` copy | refcount bump |

use armature_h1::{ByteStr, HeaderId, header as header_id};
use bytes::Bytes;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::fmt;

/// Number of headers to store inline (on stack).
///
/// Most HTTP requests have 5–10 headers.
pub const INLINE_HEADERS: usize = 12;

/// A by-name lookup needle, resolved without allocating.
///
/// `header_id::intern` returns `HeaderId::Other(ByteStr::from(lowercased))` for
/// any name outside the well-known table — a `String` plus a `Bytes` per call.
/// Interning the needle would therefore charge an allocation to every lookup of
/// exactly the custom names applications reach for most (`x-request-id`,
/// `x-tenant-id`). A borrowed needle compares against the stored name instead,
/// while a well-known name still collapses to a discriminant compare.
enum Needle<'a> {
    Known(HeaderId),
    Custom(&'a str),
}

impl<'a> Needle<'a> {
    #[inline]
    fn new(name: &'a str) -> Self {
        match HeaderId::from_bytes(name.as_bytes()) {
            Some(id) => Needle::Known(id),
            None => Needle::Custom(name),
        }
    }

    /// Whether a stored header's name is this needle.
    ///
    /// `HeaderId::as_str` is always the canonical lowercase form, so the
    /// custom arm only has to be insensitive on the needle's side.
    #[inline]
    fn matches(&self, id: &HeaderId) -> bool {
        match self {
            Needle::Known(known) => known == id,
            Needle::Custom(name) => id.as_str().eq_ignore_ascii_case(name),
        }
    }
}

/// A header field: an interned name and a `Bytes` value.
#[derive(Clone, PartialEq, Eq)]
pub struct Header {
    /// The interned field name.
    pub id: HeaderId,
    /// The field value, exactly as it arrived.
    pub value: Bytes,
}

impl Header {
    /// Create a header, interning the name.
    #[inline]
    pub fn new(name: impl AsRef<str>, value: impl HeaderValueInput) -> Self {
        Self {
            id: header_id::intern(name.as_ref()),
            value: value.into_value(),
        }
    }

    /// The field name, lowercased.
    #[inline]
    pub fn name(&self) -> &str {
        self.id.as_str()
    }

    /// The value as UTF-8, or `None` if it is not valid UTF-8.
    #[inline]
    pub fn value_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.value).ok()
    }
}

impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.value_str() {
            Some(v) => write!(f, "{}: {}", self.name(), v),
            None => write!(f, "{}: <{} non-utf8 bytes>", self.name(), self.value.len()),
        }
    }
}

/// Anything that can become a header value.
///
/// This exists so every existing `insert(name, value)` call site keeps compiling
/// across `&str`, `String`, and `Bytes` alike. A plain `impl Into<Bytes>` bound
/// would not: `Bytes: From<&'static str>` but not `From<&'a str>`, so every
/// borrowed-`&str` call site would break.
pub trait HeaderValueInput {
    /// Convert into the stored representation.
    fn into_value(self) -> Bytes;
}

impl HeaderValueInput for Bytes {
    #[inline]
    fn into_value(self) -> Bytes {
        self
    }
}

impl HeaderValueInput for &str {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::copy_from_slice(self.as_bytes())
    }
}

impl HeaderValueInput for &String {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::copy_from_slice(self.as_bytes())
    }
}

impl HeaderValueInput for String {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::from(self.into_bytes())
    }
}

impl HeaderValueInput for &[u8] {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::copy_from_slice(self)
    }
}

impl HeaderValueInput for Vec<u8> {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::from(self)
    }
}

impl HeaderValueInput for ByteStr {
    #[inline]
    fn into_value(self) -> Bytes {
        self.into_bytes()
    }
}

impl HeaderValueInput for std::borrow::Cow<'_, str> {
    #[inline]
    fn into_value(self) -> Bytes {
        match self {
            std::borrow::Cow::Borrowed(s) => Bytes::copy_from_slice(s.as_bytes()),
            std::borrow::Cow::Owned(s) => Bytes::from(s.into_bytes()),
        }
    }
}

/// A compact header map using `SmallVec` for inline storage.
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
/// // Lookup is case-insensitive; the value comes back as a `&str`.
/// assert_eq!(headers.get("content-type"), Some("application/json"));
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
    /// If capacity <= `INLINE_HEADERS`, no heap allocation occurs.
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

    /// The value of `name` as UTF-8, case-insensitively.
    ///
    /// Returns `None` for a value that is not valid UTF-8; use
    /// [`HeaderMap::get_bytes`] for those.
    #[inline]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.get_bytes(name)
            .and_then(|v| std::str::from_utf8(v).ok())
    }

    /// The raw value of `name`, case-insensitively.
    #[inline]
    pub fn get_bytes(&self, name: &str) -> Option<&Bytes> {
        let needle = Needle::new(name);
        self.inner
            .iter()
            .find(|h| needle.matches(&h.id))
            .map(|h| &h.value)
    }

    /// The raw value for an already-interned name.
    ///
    /// The hot-path accessor: no interning, and for a well-known name the
    /// comparison is on the enum discriminant.
    #[inline]
    pub fn get_id(&self, id: &HeaderId) -> Option<&Bytes> {
        self.inner.iter().find(|h| &h.id == id).map(|h| &h.value)
    }

    /// The value of `name` as UTF-8, case-insensitively.
    ///
    /// Identical to [`HeaderMap::get`]; kept because call sites use both names.
    #[inline]
    pub fn get_ignore_case(&self, name: &str) -> Option<&str> {
        self.get(name)
    }

    /// Check if header exists (case-insensitive).
    #[inline]
    pub fn contains(&self, name: &str) -> bool {
        self.get_bytes(name).is_some()
    }

    /// Check if header exists (case-insensitive).
    ///
    /// HashMap-compatible alias for [`contains`](Self::contains).
    #[inline]
    pub fn contains_key(&self, name: &str) -> bool {
        self.contains(name)
    }

    /// Insert a header, replacing any existing header with the same name.
    ///
    /// Returns the old value if one was replaced.
    #[inline]
    pub fn insert(&mut self, name: impl AsRef<str>, value: impl HeaderValueInput) -> Option<Bytes> {
        let id = header_id::intern(name.as_ref());
        let value = value.into_value();
        if let Some(existing) = self.inner.iter_mut().find(|h| h.id == id) {
            return Some(std::mem::replace(&mut existing.value, value));
        }
        self.inner.push(Header { id, value });
        None
    }

    /// Append a header, allowing duplicates.
    ///
    /// Unlike `insert`, this does not replace. Use it for fields that may repeat,
    /// such as `Set-Cookie`.
    #[inline]
    pub fn append(&mut self, name: impl AsRef<str>, value: impl HeaderValueInput) {
        self.inner.push(Header {
            id: header_id::intern(name.as_ref()),
            value: value.into_value(),
        });
    }

    /// Remove a header by name (case-insensitive), returning its value.
    #[inline]
    pub fn remove(&mut self, name: &str) -> Option<Bytes> {
        let needle = Needle::new(name);
        let pos = self.inner.iter().position(|h| needle.matches(&h.id))?;
        Some(self.inner.remove(pos).value)
    }

    /// Remove every header with the given name, returning how many were removed.
    #[inline]
    pub fn remove_all(&mut self, name: &str) -> usize {
        let needle = Needle::new(name);
        let before = self.inner.len();
        self.inner.retain(|h| !needle.matches(&h.id));
        before - self.inner.len()
    }

    /// Iterate over headers as `(name, value)`, skipping non-UTF-8 values.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .iter()
            .filter_map(|h| h.value_str().map(|v| (h.name(), v)))
    }

    /// Iterate over every header, including those with non-UTF-8 values.
    #[inline]
    pub fn iter_raw(&self) -> impl Iterator<Item = (&HeaderId, &Bytes)> {
        self.inner.iter().map(|h| (&h.id, &h.value))
    }

    /// Iterate over header names, lowercased.
    #[inline]
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.iter().map(|h| h.name())
    }

    /// Iterate over header names.
    ///
    /// HashMap-compatible alias for [`names`](Self::names).
    #[inline]
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.names()
    }

    /// Iterate over header values, skipping non-UTF-8 ones.
    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.inner.iter().filter_map(|h| h.value_str())
    }

    /// Every value for a header name, for multi-value fields.
    #[inline]
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        let needle = Needle::new(name);
        self.inner
            .iter()
            .filter(|h| needle.matches(&h.id))
            .filter_map(|h| h.value_str())
            .collect()
    }

    /// Clear all headers.
    #[inline]
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Extend with headers from an iterator.
    #[inline]
    pub fn extend<I, K, V>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: HeaderValueInput,
    {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }

    /// Convert to a `HashMap`, for compatibility.
    ///
    /// Names come out lowercased and non-UTF-8 values are dropped. This
    /// allocates a `String` per name and value — reach for it only when a
    /// `HashMap` is genuinely required.
    #[inline]
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        self.iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    /// Create from a `HashMap`.
    #[inline]
    pub fn from_hash_map(map: HashMap<String, String>) -> Self {
        let mut headers = Self::with_capacity(map.len());
        for (k, v) in map {
            headers.insert(k, v);
        }
        headers
    }

    // ========================================================================
    // Common Header Accessors
    // ========================================================================

    /// Get the `Content-Type` header.
    #[inline]
    pub fn content_type(&self) -> Option<&str> {
        self.str_of(&HeaderId::ContentType)
    }

    /// Get the `Content-Length` header as a `usize`.
    #[inline]
    pub fn content_length(&self) -> Option<usize> {
        self.str_of(&HeaderId::ContentLength)?.parse().ok()
    }

    /// Get the `Accept` header.
    #[inline]
    pub fn accept(&self) -> Option<&str> {
        self.str_of(&HeaderId::Accept)
    }

    /// Get the `Authorization` header.
    #[inline]
    pub fn authorization(&self) -> Option<&str> {
        self.str_of(&HeaderId::Authorization)
    }

    /// Get the `User-Agent` header.
    #[inline]
    pub fn user_agent(&self) -> Option<&str> {
        self.str_of(&HeaderId::UserAgent)
    }

    /// Get the `Host` header.
    #[inline]
    pub fn host(&self) -> Option<&str> {
        self.str_of(&HeaderId::Host)
    }

    /// Get the `Cookie` header.
    #[inline]
    pub fn cookie(&self) -> Option<&str> {
        self.str_of(&HeaderId::Cookie)
    }

    /// Check for a keep-alive connection.
    #[inline]
    pub fn is_keep_alive(&self) -> bool {
        self.str_of(&HeaderId::Connection)
            .map(|v| v.eq_ignore_ascii_case("keep-alive"))
            .unwrap_or(true) // HTTP/1.1 default is keep-alive
    }

    /// Check for chunked transfer encoding.
    #[inline]
    pub fn is_chunked(&self) -> bool {
        self.str_of(&HeaderId::TransferEncoding)
            .map(|v| v.contains("chunked"))
            .unwrap_or(false)
    }

    /// Set the `Content-Type` header.
    #[inline]
    pub fn set_content_type(&mut self, value: impl HeaderValueInput) {
        self.insert("content-type", value);
    }

    /// Set the `Content-Length` header.
    #[inline]
    pub fn set_content_length(&mut self, len: usize) {
        self.insert("content-length", len.to_string());
    }

    /// The UTF-8 value for an already-interned name.
    #[inline]
    fn str_of(&self, id: &HeaderId) -> Option<&str> {
        self.get_id(id).and_then(|v| std::str::from_utf8(v).ok())
    }
}

impl fmt::Debug for HeaderMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(
                self.inner
                    .iter()
                    .map(|h| (h.name(), h.value_str().unwrap_or("<non-utf8>"))),
            )
            .finish()
    }
}

impl<K, V> FromIterator<(K, V)> for HeaderMap
where
    K: AsRef<str>,
    V: HeaderValueInput,
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

/// Project a header to its `(name, value)` pair, dropping non-UTF-8 values.
///
/// A free function rather than a closure so [`IntoIterator`] can name its
/// iterator type.
fn utf8_pair(h: &Header) -> Option<(&str, &str)> {
    h.value_str().map(|v| (h.name(), v))
}

/// Project an owned header to an owned pair, dropping non-UTF-8 values.
fn owned_utf8_pair(h: Header) -> Option<(String, String)> {
    let name = h.name().to_owned();
    String::from_utf8(h.value.to_vec())
        .ok()
        .map(|value| (name, value))
}

impl<'a> IntoIterator for &'a HeaderMap {
    type Item = (&'a str, &'a str);
    type IntoIter = std::iter::FilterMap<
        std::slice::Iter<'a, Header>,
        fn(&'a Header) -> Option<(&'a str, &'a str)>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter().filter_map(utf8_pair as _)
    }
}

impl IntoIterator for HeaderMap {
    type Item = (String, String);
    type IntoIter = std::iter::FilterMap<
        smallvec::IntoIter<[Header; INLINE_HEADERS]>,
        fn(Header) -> Option<(String, String)>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter().filter_map(owned_utf8_pair as _)
    }
}

// Allow HashMap-like indexing.
impl std::ops::Index<&str> for HeaderMap {
    type Output = str;

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
    fn get_returns_str_and_well_known_names_are_interned() {
        let mut h = HeaderMap::new();
        h.insert("Content-Type", "application/json");
        h.insert("X-Tenant-Id", "acme".to_string());

        // Case-insensitive lookup survives the move to HeaderId.
        assert_eq!(h.get("content-type"), Some("application/json"));
        assert_eq!(h.get("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(h.get("x-tenant-id"), Some("acme"));
        assert_eq!(h.get("absent"), None);

        // Well-known names cost no allocation and compare as a discriminant.
        assert_eq!(
            h.get_id(&HeaderId::ContentType).map(|b| &b[..]),
            Some(&b"application/json"[..])
        );
    }

    #[test]
    fn custom_names_stay_case_insensitive_through_the_borrowed_needle() {
        // Names outside the well-known table take the non-allocating compare
        // path, which still has to honour RFC 9110 case-insensitivity in both
        // directions — however the header was inserted, however it is asked for.
        let mut h = HeaderMap::new();
        h.insert("X-Request-ID", "abc123");
        h.append("x-request-id", "def456");

        assert_eq!(h.get("x-request-id"), Some("abc123"));
        assert_eq!(h.get("X-REQUEST-ID"), Some("abc123"));
        assert!(h.contains("X-Request-Id"));
        assert_eq!(h.get_all("X-Request-Id"), vec!["abc123", "def456"]);

        // A custom needle must not match a well-known stored name, or vice versa.
        h.insert("Content-Type", "text/plain");
        assert_eq!(h.get("x-content-type"), None);

        assert_eq!(h.remove_all("X-Request-ID"), 2);
        assert_eq!(h.get("x-request-id"), None);
    }

    #[test]
    fn non_utf8_value_is_invisible_to_get_but_reachable_as_bytes() {
        let mut h = HeaderMap::new();
        h.insert("x-raw", Bytes::from_static(&[0xff, 0x00]));
        // `get` promises a `&str`, and there isn't one. Returning None beats
        // returning lossy text that a caller would go on to trust.
        assert_eq!(h.get("x-raw"), None);
        assert_eq!(h.get_bytes("x-raw").map(|b| b.len()), Some(2));
        // ...and it is still there, so a caller that iterates raw sees it.
        assert_eq!(h.len(), 1);
        assert_eq!(h.iter().count(), 0);
        assert_eq!(h.iter_raw().count(), 1);
    }

    #[test]
    fn test_insert_and_get() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        assert_eq!(headers.len(), 2);
        assert_eq!(headers.get("Content-Type"), Some("application/json"));
        assert_eq!(headers.get("content-type"), Some("application/json"));
    }

    #[test]
    fn test_insert_replaces() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "text/plain");
        let old = headers.insert("Content-Type", "application/json");

        assert_eq!(old.as_deref(), Some(&b"text/plain"[..]));
        assert_eq!(headers.len(), 1);
        assert_eq!(headers.get("Content-Type"), Some("application/json"));
    }

    #[test]
    fn test_append_duplicates() {
        let mut headers = HeaderMap::new();
        headers.append("Set-Cookie", "session=abc");
        headers.append("Set-Cookie", "user=123");

        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers.get_all("set-cookie"),
            vec!["session=abc", "user=123"]
        );
    }

    #[test]
    fn test_remove() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        let removed = headers.remove("Content-Type");
        assert_eq!(removed.as_deref(), Some(&b"application/json"[..]));
        assert_eq!(headers.len(), 1);
        assert!(!headers.contains("Content-Type"));
    }

    #[test]
    fn test_remove_all() {
        let mut headers = HeaderMap::new();
        headers.append("Set-Cookie", "a=1");
        headers.append("set-cookie", "b=2");
        headers.insert("Accept", "*/*");

        assert_eq!(headers.remove_all("Set-Cookie"), 2);
        assert_eq!(headers.len(), 1);
    }

    #[test]
    fn test_inline_capacity() {
        let mut headers = HeaderMap::new();

        for i in 0..INLINE_HEADERS {
            headers.insert(format!("Header-{i}"), format!("Value-{i}"));
        }
        assert!(headers.is_inline());

        headers.insert("Extra-Header", "Extra-Value");
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
    fn iter_yields_lowercased_names_for_custom_headers() {
        let mut h = HeaderMap::new();
        h.insert("X-A", "1");
        // Interning lowercases custom names once, at insert. A caller comparing
        // `name == "x-a"` must not have to guess which case survived.
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![("x-a", "1")]);
    }

    #[test]
    fn test_common_accessors() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Content-Length", "100");
        headers.insert("Connection", "keep-alive");
        headers.insert("Transfer-Encoding", "chunked");

        assert_eq!(headers.content_type(), Some("application/json"));
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
    fn test_to_hash_map_normalizes_names_to_lowercase() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");

        let map = headers.to_hash_map();
        // Names are interned, so the case they were inserted with is gone. This
        // is the documented behavior change in 0.6: lookups stay
        // case-insensitive, but a `HashMap` snapshot reports canonical names.
        assert_eq!(
            map.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert_eq!(map.get("Content-Type"), None);
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

        assert!(headers.contains_key("Content-Type"));
        assert!(headers.contains_key("content-type"));
        assert!(!headers.contains_key("Accept"));
    }

    #[test]
    fn test_keys() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        let keys: Vec<_> = headers.keys().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"content-type"));
        assert!(keys.contains(&"accept"));
    }

    #[test]
    fn test_values() {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", "application/json");
        headers.insert("Accept", "text/html");

        let values: Vec<_> = headers.values().collect();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"application/json"));
        assert!(values.contains(&"text/html"));
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

        let extra: Vec<(String, String)> = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "text/html".to_string()),
        ];
        Extend::extend(&mut headers, extra);

        assert_eq!(headers.len(), 3);
        assert_eq!(headers.get("Content-Type"), Some("application/json"));
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

        let collected: Vec<(&str, &str)> = (&headers).into_iter().collect();
        assert_eq!(collected, vec![("a", "1")]);
    }

    #[test]
    fn test_hashmap_roundtrip() {
        let mut map = HashMap::new();
        map.insert("Content-Type".to_string(), "application/json".to_string());

        let headers: HeaderMap = map.clone().into();
        assert!(headers.contains_key("content-type"));
        let back: HashMap<String, String> = headers.into();
        // Canonical (lowercase) name on the way out; see
        // `test_to_hash_map_normalizes_names_to_lowercase`.
        assert_eq!(back.get("content-type"), map.get("Content-Type"));
    }

    #[test]
    fn cloning_a_value_does_not_copy_it() {
        let mut headers = HeaderMap::new();
        let big = Bytes::from(vec![b'x'; 4096]);
        headers.insert("x-big", big.clone());
        let copy = headers.clone();
        assert_eq!(
            copy.get_bytes("x-big").map(|b| b.as_ptr()),
            Some(big.as_ptr())
        );
    }
}
