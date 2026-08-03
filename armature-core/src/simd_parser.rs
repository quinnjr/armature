//! SIMD-Optimized HTTP Parsing
//!
//! This module provides high-performance HTTP parsing utilities using SIMD
//! instructions where available. It complements Hyper's built-in parsing
//! with additional optimizations for:
//!
//! - Header name interning (avoid repeated allocations)
//! - Fast query string parsing
//! - URL path parsing with SIMD byte search
//! - Request line parsing
//!
//! ## Performance
//!
//! On x86/x86_64 with AVX2, these parsers can process ~2GB/s of HTTP headers.
//! Even without SIMD, the optimized algorithms provide significant speedups
//! over naive implementations.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use armature_core::simd_parser::{parse_query_string_fast, intern_header_name};
//!
//! // Fast query string parsing
//! let params = parse_query_string_fast("name=john&age=30");
//!
//! // Header name interning
//! let name = intern_header_name("Content-Type");
//! ```

use crate::Error;
use memchr::memchr;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;

// Common header names for interning - using static strings avoids allocation
static COMMON_HEADERS: &[(&str, &str)] = &[
    ("accept", "Accept"),
    ("accept-charset", "Accept-Charset"),
    ("accept-encoding", "Accept-Encoding"),
    ("accept-language", "Accept-Language"),
    ("authorization", "Authorization"),
    ("cache-control", "Cache-Control"),
    ("connection", "Connection"),
    ("content-encoding", "Content-Encoding"),
    ("content-length", "Content-Length"),
    ("content-type", "Content-Type"),
    ("cookie", "Cookie"),
    ("date", "Date"),
    ("host", "Host"),
    ("if-match", "If-Match"),
    ("if-modified-since", "If-Modified-Since"),
    ("if-none-match", "If-None-Match"),
    ("if-unmodified-since", "If-Unmodified-Since"),
    ("origin", "Origin"),
    ("pragma", "Pragma"),
    ("range", "Range"),
    ("referer", "Referer"),
    ("sec-fetch-dest", "Sec-Fetch-Dest"),
    ("sec-fetch-mode", "Sec-Fetch-Mode"),
    ("sec-fetch-site", "Sec-Fetch-Site"),
    ("te", "TE"),
    ("transfer-encoding", "Transfer-Encoding"),
    ("upgrade", "Upgrade"),
    ("user-agent", "User-Agent"),
    ("x-forwarded-for", "X-Forwarded-For"),
    ("x-forwarded-host", "X-Forwarded-Host"),
    ("x-forwarded-proto", "X-Forwarded-Proto"),
    ("x-real-ip", "X-Real-IP"),
    ("x-request-id", "X-Request-Id"),
];

/// Intern a header name to avoid allocation for common headers.
///
/// Returns a static string for known headers, or the original for unknown ones.
///
/// # Performance
///
/// This uses a case-insensitive binary search, which is O(log n) for the
/// common headers list. For unknown headers, it returns the original string.
///
/// # Example
///
/// ```rust,ignore
/// let name = intern_header_name("content-type");
/// assert_eq!(name, "Content-Type"); // Returns canonical form
/// ```
#[inline]
pub fn intern_header_name(name: &str) -> Cow<'static, str> {
    // Binary search through the (lowercase, sorted) common-header table without
    // allocating a lowercased copy of `name`. Each key is already lowercase, so
    // we compare it against `name` case-insensitively byte-by-byte.
    let needle = name.as_bytes();
    let found = COMMON_HEADERS.binary_search_by(|(key, _)| {
        let key = key.as_bytes();
        let common = key.len().min(needle.len());
        for i in 0..common {
            // Key bytes are already lowercase; lowercase the needle byte to
            // compare case-insensitively.
            match key[i].cmp(&needle[i].to_ascii_lowercase()) {
                Ordering::Equal => {}
                non_eq => return non_eq,
            }
        }
        key.len().cmp(&needle.len())
    });

    if let Ok(idx) = found {
        Cow::Borrowed(COMMON_HEADERS[idx].1)
    } else {
        Cow::Owned(name.to_string())
    }
}

/// Parse a query string using SIMD-optimized byte searching.
///
/// This is significantly faster than the naive approach for typical
/// query strings due to:
/// - SIMD-accelerated delimiter search (memchr)
/// - Minimal string allocations
/// - Single-pass parsing
///
/// # Performance
///
/// - ~3x faster than naive split-based parsing
/// - ~10x faster for long query strings with many parameters
///
/// # Example
///
/// ```rust,ignore
/// let params = parse_query_string_fast("name=john&age=30&city=NYC");
/// assert_eq!(params.get("name").map(String::as_str), Some("john"));
/// ```
#[inline]
pub fn parse_query_string_fast(query: &str) -> HashMap<String, String> {
    let bytes = query.as_bytes();
    let mut params = HashMap::with_capacity(8); // Pre-allocate for typical case
    let mut pos = 0;

    while pos < bytes.len() {
        // Find the next '&' first to delimit this key-value pair
        let amp_pos = match memchr(b'&', &bytes[pos..]) {
            Some(p) => pos + p,
            None => bytes.len(),
        };

        // Now look for '=' within this segment
        let segment = &bytes[pos..amp_pos];
        let segment_str = &query[pos..amp_pos];

        if let Some(eq_offset) = memchr(b'=', segment) {
            // Found '=', split into key and value
            let key = &segment_str[..eq_offset];
            let value = &segment_str[eq_offset + 1..];
            if !key.is_empty() {
                params.insert(key.to_string(), value.to_string());
            }
        } else {
            // No '=', treat entire segment as key with empty value
            if !segment_str.is_empty() {
                params.insert(segment_str.to_string(), String::new());
            }
        }

        pos = amp_pos + 1;
    }

    params
}

/// Parse a query string with URL decoding using SIMD-optimized byte searching.
///
/// This handles percent-encoded characters like %20 for space.
///
/// Not on the serve path: it allocates a `HashMap` and an owned `String` per
/// key and value, whether or not a handler reads any of them.
/// [`crate::HttpRequest::query`] is the request-path accessor — it parses on
/// first access and memoizes. This remains for callers with a query string in
/// hand and a genuine need for an owned map.
///
/// # Example
///
/// ```rust,ignore
/// let params = parse_query_string_decoded("name=john%20doe&age=30");
/// assert_eq!(params.get("name").map(String::as_str), Some("john doe"));
/// ```
#[inline]
pub fn parse_query_string_decoded(query: &str) -> HashMap<String, String> {
    let bytes = query.as_bytes();
    let mut params = HashMap::with_capacity(8);
    let mut pos = 0;

    while pos < bytes.len() {
        // Find the next '&' first to delimit this key-value pair
        let amp_pos = match memchr(b'&', &bytes[pos..]) {
            Some(p) => pos + p,
            None => bytes.len(),
        };

        // Now look for '=' within this segment
        let segment = &bytes[pos..amp_pos];
        let segment_str = &query[pos..amp_pos];

        if let Some(eq_offset) = memchr(b'=', segment) {
            // Found '=', split into key and value
            let key = url_decode(&segment_str[..eq_offset]);
            let value = url_decode(&segment_str[eq_offset + 1..]);
            if !key.is_empty() {
                params.insert(key, value);
            }
        } else {
            // No '=', treat entire segment as key with empty value
            let key = url_decode(segment_str);
            if !key.is_empty() {
                params.insert(key, String::new());
            }
        }

        pos = amp_pos + 1;
    }

    params
}

/// URL decode a string, handling percent-encoded characters.
///
/// Uses SIMD to quickly scan for '%' characters.
#[inline]
pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();

    // Fast path: no percent signs, return as-is
    if memchr(b'%', bytes).is_none() && memchr(b'+', bytes).is_none() {
        return input.to_string();
    }

    // Decode into raw bytes first: percent-escapes are bytes of a UTF-8
    // sequence, so pushing them as individual chars would mangle multibyte
    // characters (e.g. "%E2%82%AC" must decode to "€", not mojibake).
    let mut result = Vec::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                // Try to decode hex
                if let (Some(h1), Some(h2)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                    result.push(h1 << 4 | h2);
                    i += 3;
                } else {
                    result.push(b'%');
                    i += 1;
                }
            }
            b'+' => {
                result.push(b' ');
                i += 1;
            }
            c => {
                result.push(c);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&result).into_owned()
}

/// Convert a hex digit character to its value.
#[inline(always)]
fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Split a path into segments.
///
/// This is a plain scalar `str::split('/')` (no SIMD); it is listed here
/// alongside the SIMD helpers only because it is part of the fast path API.
///
/// Returns an iterator over path segments, skipping empty segments.
///
/// # Example
///
/// ```rust,ignore
/// let segments: Vec<_> = split_path("/api/v1/users/123").collect();
/// assert_eq!(segments, vec!["api", "v1", "users", "123"]);
/// ```
#[inline]
pub fn split_path(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

/// Extract path and query from a URI using SIMD search.
///
/// # Example
///
/// ```rust,ignore
/// let (path, query) = split_uri("/api/users?page=1&limit=10");
/// assert_eq!(path, "/api/users");
/// assert_eq!(query, Some("page=1&limit=10"));
/// ```
#[inline]
pub fn split_uri(uri: &str) -> (&str, Option<&str>) {
    let bytes = uri.as_bytes();

    if let Some(pos) = memchr(b'?', bytes) {
        (&uri[..pos], Some(&uri[pos + 1..]))
    } else {
        (uri, None)
    }
}

/// Parse HTTP headers from raw bytes using httparse.
///
/// This uses SIMD-optimized parsing internally via httparse.
///
/// The input must be a complete header block terminated by `\r\n\r\n`. A
/// truncated buffer is an error, not a short result: handing back the headers
/// that happened to fit would let a caller act on a request whose remaining
/// headers — `Authorization`, `Content-Length`, `Host` — had not arrived yet.
///
/// # Returns
///
/// A vector of (name, value) pairs for the headers.
#[inline]
pub fn parse_headers(buf: &[u8]) -> Result<Vec<(&str, &str)>, Error> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    match req
        .parse(buf)
        .map_err(|e| Error::BadRequest(format!("Malformed request headers: {e}")))?
    {
        httparse::Status::Complete(_) => {
            let result = req
                .headers
                .iter()
                .filter(|h| !h.name.is_empty())
                .map(|h| (h.name, std::str::from_utf8(h.value).unwrap_or("")))
                .collect();
            Ok(result)
        }
        httparse::Status::Partial => Err(Error::BadRequest(
            "Incomplete request headers: buffer ends before the header block does".to_string(),
        )),
    }
}

/// Parse an HTTP request line (method, path, version).
///
/// # Example
///
/// ```rust,ignore
/// let (method, path, version) = parse_request_line(b"GET /api/users HTTP/1.1\r\n")?;
/// assert_eq!(method, "GET");
/// assert_eq!(path, "/api/users");
/// ```
///
/// A truncated request line is an error rather than a partial result:
/// defaulting the missing fields would report `GET /` for a buffer that never
/// said either, which a caller has no way to distinguish from a real request.
#[inline]
pub fn parse_request_line(buf: &[u8]) -> Result<(&str, &str, u8), Error> {
    let mut headers = [httparse::EMPTY_HEADER; 0];
    let mut req = httparse::Request::new(&mut headers);

    match req
        .parse(buf)
        .map_err(|e| Error::BadRequest(format!("Malformed request line: {e}")))?
    {
        httparse::Status::Complete(_) => match (req.method, req.path, req.version) {
            (Some(method), Some(path), Some(version)) => Ok((method, path, version)),
            _ => Err(Error::BadRequest(
                "Incomplete request line: method, target, or version missing".to_string(),
            )),
        },
        httparse::Status::Partial => Err(Error::BadRequest(
            "Incomplete request line: buffer ends before CRLF".to_string(),
        )),
    }
}

/// Check if a byte sequence contains only valid header name characters.
///
/// A header field name is a `token` (RFC 9110 §5.6.2), so the legal set is
/// alphanumerics plus ``! # $ % & ' * + - . ^ _ ` | ~``. An empty name is not a
/// token and is rejected.
///
/// Scalar validation (`iter().all(..)`); it does not use SIMD.
#[inline]
pub fn is_valid_header_name(name: &[u8]) -> bool {
    !name.is_empty() && name.iter().all(|&c| is_tchar(c))
}

/// Whether `c` is an RFC 9110 `tchar`.
#[inline]
const fn is_tchar(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Fast path parameter extraction.
///
/// Given a route pattern and actual path, extract parameters using scalar
/// string operations (segment split and comparison; no SIMD).
///
/// # Example
///
/// ```rust,ignore
/// let params = extract_path_params("/users/:id/posts/:post_id", "/users/123/posts/456");
/// assert_eq!(params.get("id").map(String::as_str), Some("123"));
/// assert_eq!(params.get("post_id").map(String::as_str), Some("456"));
/// ```
#[inline]
pub fn extract_path_params(pattern: &str, path: &str) -> HashMap<String, String> {
    let mut params = HashMap::with_capacity(4);

    let pattern_segments: Vec<_> = split_path(pattern).collect();
    let path_segments: Vec<_> = split_path(path).collect();

    if pattern_segments.len() != path_segments.len() {
        return params;
    }

    for (pat, seg) in pattern_segments.iter().zip(path_segments.iter()) {
        if let Some(param_name) = pat.strip_prefix(':') {
            params.insert(param_name.to_string(), (*seg).to_string());
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intern_header_name() {
        // Known headers
        assert_eq!(intern_header_name("content-type"), "Content-Type");
        assert_eq!(intern_header_name("Content-Type"), "Content-Type");
        assert_eq!(intern_header_name("CONTENT-TYPE"), "Content-Type");
        assert_eq!(intern_header_name("authorization"), "Authorization");

        // Mixed-case matches for every entry regardless of input casing
        assert_eq!(intern_header_name("AcCePt"), "Accept");
        assert_eq!(intern_header_name("X-FORWARDED-FOR"), "X-Forwarded-For");
        assert_eq!(intern_header_name("te"), "TE");
        assert_eq!(intern_header_name("TE"), "TE");

        // Unknown headers are returned unchanged (original casing preserved)
        let custom = intern_header_name("X-Custom-Header");
        assert_eq!(custom.as_ref(), "X-Custom-Header");
        assert!(matches!(custom, Cow::Owned(_)));

        // A name that shares a prefix with a known header but is longer/shorter
        assert_eq!(intern_header_name("content-typ").as_ref(), "content-typ");
        assert_eq!(
            intern_header_name("content-type-extra").as_ref(),
            "content-type-extra"
        );

        // Known matches return a borrowed static string (no allocation)
        assert!(matches!(intern_header_name("host"), Cow::Borrowed(_)));
    }

    #[test]
    fn test_parse_query_string_fast() {
        let params = parse_query_string_fast("name=john&age=30&city=NYC");
        assert_eq!(params.get("name").map(String::as_str), Some("john"));
        assert_eq!(params.get("age").map(String::as_str), Some("30"));
        assert_eq!(params.get("city").map(String::as_str), Some("NYC"));

        // Empty query
        let params = parse_query_string_fast("");
        assert!(params.is_empty());

        // Single param
        let params = parse_query_string_fast("key=value");
        assert_eq!(params.get("key").map(String::as_str), Some("value"));

        // Key without value
        let params = parse_query_string_fast("flag&debug=true");
        assert!(params.contains_key("flag"));
        assert_eq!(params.get("debug").map(String::as_str), Some("true"));
    }

    #[test]
    fn test_parse_query_string_decoded() {
        let params = parse_query_string_decoded("name=john%20doe&age=30");
        assert_eq!(params.get("name").map(String::as_str), Some("john doe"));
        assert_eq!(params.get("age").map(String::as_str), Some("30"));

        // Plus as space
        let params = parse_query_string_decoded("name=john+doe");
        assert_eq!(params.get("name").map(String::as_str), Some("john doe"));

        // Special characters
        let params = parse_query_string_decoded("email=test%40example.com");
        assert_eq!(
            params.get("email").map(String::as_str),
            Some("test@example.com")
        );
    }

    #[test]
    fn test_url_decode() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("hello+world"), "hello world");
        assert_eq!(url_decode("test%40example.com"), "test@example.com");
        assert_eq!(url_decode("normal"), "normal");
        assert_eq!(url_decode("%2F"), "/");
        assert_eq!(url_decode("%3A%2F%2F"), "://");
    }

    #[test]
    fn test_url_decode_multibyte_utf8() {
        // Percent-escapes are bytes of a UTF-8 sequence and must be decoded
        // as a whole, not byte-by-byte as Latin-1 chars.
        assert_eq!(url_decode("%E2%82%AC"), "\u{20AC}"); // €
        assert_eq!(url_decode("caf%C3%A9"), "caf\u{e9}"); // café
        assert_eq!(url_decode("%F0%9F%A6%80"), "\u{1F980}"); // 🦀

        let params = parse_query_string_decoded("price=%E2%82%AC10&name=caf%C3%A9");
        assert_eq!(params.get("price").map(String::as_str), Some("\u{20AC}10"));
        assert_eq!(params.get("name").map(String::as_str), Some("caf\u{e9}"));
    }

    #[test]
    fn test_split_uri() {
        let (path, query) = split_uri("/api/users?page=1&limit=10");
        assert_eq!(path, "/api/users");
        assert_eq!(query, Some("page=1&limit=10"));

        let (path, query) = split_uri("/api/users");
        assert_eq!(path, "/api/users");
        assert_eq!(query, None);
    }

    #[test]
    fn test_split_path() {
        let segments: Vec<_> = split_path("/api/v1/users/123").collect();
        assert_eq!(segments, vec!["api", "v1", "users", "123"]);

        let segments: Vec<_> = split_path("/").collect();
        assert!(segments.is_empty());

        let segments: Vec<_> = split_path("/api//users/").collect();
        assert_eq!(segments, vec!["api", "users"]);
    }

    #[test]
    fn test_extract_path_params() {
        let params = extract_path_params("/users/:id/posts/:post_id", "/users/123/posts/456");
        assert_eq!(params.get("id").map(String::as_str), Some("123"));
        assert_eq!(params.get("post_id").map(String::as_str), Some("456"));

        // No params
        let params = extract_path_params("/users/list", "/users/list");
        assert!(params.is_empty());
    }

    #[test]
    fn test_is_valid_header_name() {
        assert!(is_valid_header_name(b"Content-Type"));
        assert!(is_valid_header_name(b"X-Request-Id"));
        assert!(is_valid_header_name(b"X_Custom_Header"));
        assert!(!is_valid_header_name(b"Header: Invalid"));
        assert!(!is_valid_header_name(b"Header\n"));

        // The rest of the RFC 9110 tchar set, which an alphanumeric-plus-`-_`
        // check used to reject even though it is legal on the wire.
        for &c in b"!#$%&'*+.^`|~" {
            assert!(is_valid_header_name(&[c]), "{:?} is a tchar", c as char);
        }

        // Still not tokens: separators, whitespace, controls, non-ASCII.
        for &c in b"()/@[]{}\",;=?: \t\x7f\xff" {
            assert!(
                !is_valid_header_name(&[c]),
                "{:?} is not a tchar",
                c as char
            );
        }

        // An empty name is not a token either.
        assert!(!is_valid_header_name(b""));
    }

    #[test]
    fn test_parse_request_line() {
        let result = parse_request_line(b"GET /api/users HTTP/1.1\r\n\r\n");
        assert!(result.is_ok());
        let (method, path, version) = result.unwrap();
        assert_eq!(method, "GET");
        assert_eq!(path, "/api/users");
        assert_eq!(version, 1);
    }

    #[test]
    fn truncated_input_is_an_error_not_a_fabricated_default() {
        // The request line has not arrived in full. Reporting `GET /` here
        // would be indistinguishable from a client that really sent `GET /`.
        assert!(parse_request_line(b"GET /api/us").is_err());
        assert!(parse_request_line(b"").is_err());

        // Same for a header block with no terminating blank line: the headers
        // that did arrive must not be presented as the complete set.
        assert!(parse_headers(b"GET / HTTP/1.1\r\nHost: example.com\r\n").is_err());
        assert!(parse_headers(b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n").is_ok());
    }
}
