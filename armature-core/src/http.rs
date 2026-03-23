// HTTP request and response types

use crate::body::RequestBody;
use crate::extensions::Extensions;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// HTTP request wrapper
///
/// The body is stored as `Vec<u8>` for backwards compatibility.
/// For zero-copy body handling, use `body_bytes()` or `set_body_bytes()`.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    /// Request body as raw bytes.
    ///
    /// For zero-copy access, use `body_bytes()` to get a `Bytes` view,
    /// or use `RequestBody` for efficient body handling.
    pub body: Vec<u8>,
    pub path_params: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
    /// Type-safe extensions for storing application state.
    ///
    /// Use this to pass typed data to handlers without DI container lookups.
    /// Access via the `State<T>` extractor for zero-cost state retrieval.
    pub extensions: Extensions,
    /// Optional zero-copy body storage using Bytes.
    /// When set, this takes precedence over `body` for read operations.
    body_bytes: Option<Bytes>,
}

impl HttpRequest {
    pub fn new(method: String, path: String) -> Self {
        Self {
            method,
            path,
            headers: HashMap::new(),
            body: Vec::new(),
            path_params: HashMap::new(),
            query_params: HashMap::new(),
            extensions: Extensions::new(),
            body_bytes: None,
        }
    }

    /// Create a new request with pre-allocated extensions capacity.
    #[inline]
    pub fn with_extensions_capacity(method: String, path: String, capacity: usize) -> Self {
        Self {
            method,
            path,
            headers: HashMap::new(),
            body: Vec::new(),
            path_params: HashMap::new(),
            query_params: HashMap::new(),
            extensions: Extensions::with_capacity(capacity),
            body_bytes: None,
        }
    }

    /// Create a new request with a Bytes body (zero-copy).
    ///
    /// This is the most efficient way to create a request from Hyper's body,
    /// as it avoids copying the body data.
    #[inline]
    pub fn with_bytes_body(method: String, path: String, body: Bytes) -> Self {
        Self {
            method,
            path,
            headers: HashMap::new(),
            body: Vec::new(), // Not used when body_bytes is set
            path_params: HashMap::new(),
            query_params: HashMap::new(),
            extensions: Extensions::new(),
            body_bytes: Some(body),
        }
    }

    /// Set the body using Bytes (zero-copy).
    ///
    /// This avoids copying the body data from Hyper.
    #[inline]
    pub fn set_body_bytes(&mut self, bytes: Bytes) {
        self.body_bytes = Some(bytes);
        self.body.clear(); // Clear legacy body to save memory
    }

    /// Get the body as Bytes (zero-copy if stored as Bytes).
    ///
    /// If the body was set via `set_body_bytes()` or `with_bytes_body()`,
    /// this returns a clone of the Bytes (O(1) reference count increment).
    /// Otherwise, it creates Bytes from the Vec<u8>.
    #[inline]
    pub fn body_bytes(&self) -> Bytes {
        if let Some(ref bytes) = self.body_bytes {
            bytes.clone() // O(1) - just increments ref count
        } else {
            Bytes::copy_from_slice(&self.body)
        }
    }

    /// Get a reference to the body bytes.
    ///
    /// Returns a reference to the body data without copying.
    #[inline]
    pub fn body_ref(&self) -> &[u8] {
        if let Some(ref bytes) = self.body_bytes {
            bytes.as_ref()
        } else {
            &self.body
        }
    }

    /// Get the body as a RequestBody (zero-copy wrapper).
    #[inline]
    pub fn request_body(&self) -> RequestBody {
        RequestBody::from_bytes(self.body_bytes())
    }

    /// Check if the body is using zero-copy Bytes storage.
    #[inline]
    pub fn has_bytes_body(&self) -> bool {
        self.body_bytes.is_some()
    }

    /// Set the body from a Vec<u8>.
    #[inline]
    pub fn set_body(&mut self, body: Vec<u8>) {
        self.body = body;
        self.body_bytes = None;
    }

    /// Create a request from all parts (for compatibility in tests).
    #[inline]
    pub fn from_parts(
        method: String,
        path: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
        path_params: HashMap<String, String>,
        query_params: HashMap<String, String>,
    ) -> Self {
        Self {
            method,
            path,
            headers,
            body,
            path_params,
            query_params,
            extensions: Extensions::new(),
            body_bytes: None,
        }
    }

    /// Insert a typed value into request extensions.
    ///
    /// Use this to pass application state to handlers.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut request = HttpRequest::new("GET".into(), "/".into());
    /// request.insert_extension(app_state);
    /// ```
    #[inline]
    pub fn insert_extension<T: Send + Sync + 'static>(&mut self, value: T) {
        self.extensions.insert(value);
    }

    /// Insert an Arc-wrapped value into request extensions.
    ///
    /// This is more efficient when you already have an Arc.
    #[inline]
    pub fn insert_extension_arc<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        self.extensions.insert_arc(value);
    }

    /// Get a reference to a typed extension.
    ///
    /// Returns `None` if no value of this type exists.
    #[inline]
    pub fn extension<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.extensions.get::<T>()
    }

    /// Get an Arc reference to a typed extension.
    #[inline]
    pub fn extension_arc<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.extensions.get_arc::<T>()
    }

    /// Parse the request body as JSON.
    ///
    /// With the `simd-json` feature enabled, this uses SIMD-accelerated parsing
    /// which can be 2-3x faster on modern x86_64 CPUs.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let user: CreateUser = request.json()?;
    /// ```
    #[inline]
    pub fn json<T: for<'de> Deserialize<'de>>(&self) -> Result<T, crate::Error> {
        crate::json::from_slice(self.body_ref())
            .map_err(|e| crate::Error::Deserialization(e.to_string()))
    }

    /// Parse URL-encoded form data
    pub fn form<T: for<'de> Deserialize<'de>>(&self) -> Result<T, crate::Error> {
        crate::form::parse_form(self.body_ref())
    }

    /// Parse URL-encoded form data into a HashMap
    pub fn form_map(&self) -> Result<HashMap<String, String>, crate::Error> {
        crate::form::parse_form_map(self.body_ref())
    }

    /// Parse multipart form data
    pub fn multipart(&self) -> Result<Vec<crate::form::FormField>, crate::Error> {
        let content_type = self
            .headers
            .get("Content-Type")
            .or_else(|| self.headers.get("content-type"))
            .ok_or_else(|| crate::Error::BadRequest("Missing Content-Type header".to_string()))?;

        let parser = crate::form::MultipartParser::from_content_type(content_type)?;
        parser.parse(self.body_ref())
    }

    /// Get a path parameter by name
    pub fn param(&self, name: &str) -> Option<&String> {
        self.path_params.get(name)
    }

    /// Get a query parameter by name
    pub fn query(&self, name: &str) -> Option<&String> {
        self.query_params.get(name)
    }
}

/// Lazy-initialized HashMap that doesn't allocate until first insert.
///
/// This provides the same API as HashMap but with zero allocation cost
/// for empty maps.
#[derive(Debug, Clone, Default)]
pub struct LazyHeaders {
    inner: Option<HashMap<String, String>>,
}

impl LazyHeaders {
    /// Create a new empty LazyHeaders (no allocation).
    #[inline(always)]
    pub const fn new() -> Self {
        Self { inner: None }
    }

    /// Create with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Some(HashMap::with_capacity(cap)),
        }
    }

    /// Insert a key-value pair.
    #[inline]
    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        self.inner
            .get_or_insert_with(HashMap::new)
            .insert(key, value)
    }

    /// Get a value by key.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&String> {
        self.inner.as_ref()?.get(key)
    }

    /// Check if key exists.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.inner.as_ref().is_some_and(|m| m.contains_key(key))
    }

    /// Get number of headers.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.as_ref().map_or(0, |m| m.len())
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.as_ref().is_none_or(|m| m.is_empty())
    }

    /// Iterate over headers.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.inner.iter().flat_map(|m| m.iter())
    }

    /// Convert to HashMap (for compatibility).
    #[inline]
    pub fn to_hashmap(&self) -> HashMap<String, String> {
        self.inner.clone().unwrap_or_default()
    }

    /// Remove a header by key.
    #[inline]
    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.inner.as_mut()?.remove(key)
    }

    /// Get an entry for in-place manipulation.
    #[inline]
    pub fn entry(&mut self, key: String) -> std::collections::hash_map::Entry<'_, String, String> {
        self.inner.get_or_insert_with(HashMap::new).entry(key)
    }

    /// Extend with headers from an iterator.
    #[inline]
    pub fn extend<I: IntoIterator<Item = (String, String)>>(&mut self, iter: I) {
        let map = self.inner.get_or_insert_with(HashMap::new);
        map.extend(iter);
    }

    /// Clear all headers.
    #[inline]
    pub fn clear(&mut self) {
        if let Some(ref mut map) = self.inner {
            map.clear();
        }
    }

    /// Clone the inner HashMap if present.
    #[inline]
    pub fn clone_inner(&self) -> Option<HashMap<String, String>> {
        self.inner.clone()
    }
}

impl From<HashMap<String, String>> for LazyHeaders {
    #[inline]
    fn from(map: HashMap<String, String>) -> Self {
        Self { inner: Some(map) }
    }
}

impl From<LazyHeaders> for HashMap<String, String> {
    #[inline]
    fn from(lazy: LazyHeaders) -> Self {
        lazy.inner.unwrap_or_default()
    }
}

// Allow iteration
impl<'a> IntoIterator for &'a LazyHeaders {
    type Item = (&'a String, &'a String);
    type IntoIter = std::iter::Flatten<std::option::Iter<'a, HashMap<String, String>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter().flatten()
    }
}

/// HTTP response wrapper
///
/// The body is stored as `Vec<u8>` for backwards compatibility.
/// For zero-copy body handling, use `with_bytes_body()` or `body_bytes()`.
///
/// ## Performance Note
///
/// Response creation is optimized for minimal allocation:
/// - `headers` uses `LazyHeaders` which doesn't allocate until first insert
/// - `body` uses `Option<Vec<u8>>` which doesn't allocate for empty responses
/// - Use `FastResponse` from `armature_core::fast_response` for even faster creation
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    /// Response headers with lazy allocation.
    pub headers: LazyHeaders,
    /// Set-Cookie headers (supports multiple cookies per response).
    pub cookies: Vec<String>,
    /// Response body as raw bytes (legacy field for compatibility).
    pub body: Vec<u8>,
    /// Optional zero-copy body storage using Bytes.
    /// When set, this takes precedence over `body`.
    body_bytes: Option<Bytes>,
}

/// Default pre-allocated response buffer size (512 bytes).
pub const DEFAULT_RESPONSE_CAPACITY: usize = 512;

impl HttpResponse {
    /// Create a new response with the given status code.
    ///
    /// This is optimized for minimal allocation - headers use `LazyHeaders`
    /// which doesn't allocate until first insert, and body uses `Vec::new()`
    /// which is zero-cost.
    #[inline(always)]
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: LazyHeaders::new(),
            cookies: Vec::new(),
            body: Vec::new(),
            body_bytes: None,
        }
    }

    /// Create a new response with pre-allocated body buffer.
    ///
    /// This is more efficient than `new()` when you know you'll be
    /// writing to the body, as it avoids reallocation.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Pre-allocate for typical JSON responses
    /// let response = HttpResponse::with_capacity(200, 512);
    /// ```
    #[inline]
    pub fn with_capacity(status: u16, capacity: usize) -> Self {
        Self {
            status,
            headers: LazyHeaders::with_capacity(8),
            cookies: Vec::new(),
            body: Vec::with_capacity(capacity),
            body_bytes: None,
        }
    }

    /// Create a 200 OK response.
    #[inline(always)]
    pub fn ok() -> Self {
        Self::new(200)
    }

    /// Create a 200 OK response with pre-allocated buffer (512 bytes default).
    #[inline]
    pub fn ok_preallocated() -> Self {
        Self::with_capacity(200, DEFAULT_RESPONSE_CAPACITY)
    }

    /// Create a 201 Created response.
    #[inline(always)]
    pub fn created() -> Self {
        Self::new(201)
    }

    /// Create a 204 No Content response.
    #[inline(always)]
    pub fn no_content() -> Self {
        Self::new(204)
    }

    /// Create a 400 Bad Request response.
    #[inline(always)]
    pub fn bad_request() -> Self {
        Self::new(400)
    }

    /// Create a 404 Not Found response.
    #[inline(always)]
    pub fn not_found() -> Self {
        Self::new(404)
    }

    /// Create a 500 Internal Server Error response.
    #[inline(always)]
    pub fn internal_server_error() -> Self {
        Self::new(500)
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self.body_bytes = None;
        self
    }

    /// Set the body using Bytes (zero-copy).
    ///
    /// This is the most efficient way to set response body data,
    /// as it can be passed directly to Hyper without copying.
    #[inline]
    pub fn with_bytes_body(mut self, bytes: Bytes) -> Self {
        self.body_bytes = Some(bytes);
        self.body.clear();
        self
    }

    /// Set the body from a static byte slice (zero-copy).
    #[inline]
    pub fn with_static_body(mut self, body: &'static [u8]) -> Self {
        self.body_bytes = Some(Bytes::from_static(body));
        self.body.clear();
        self
    }

    /// Get the body as Bytes (zero-copy if stored as Bytes).
    ///
    /// This is the key method for zero-copy Hyper body passthrough.
    /// If body was set via `with_bytes_body()`, returns the Bytes directly (O(1)).
    /// Otherwise, converts from Vec<u8>.
    #[inline]
    pub fn body_bytes(&self) -> Bytes {
        if let Some(ref bytes) = self.body_bytes {
            bytes.clone() // O(1) - just increments ref count
        } else {
            Bytes::copy_from_slice(&self.body)
        }
    }

    /// Consume the response and return body as Bytes (zero-copy).
    ///
    /// This is the most efficient way to get the body for Hyper,
    /// as it avoids cloning when body_bytes is set.
    #[inline]
    pub fn into_body_bytes(self) -> Bytes {
        if let Some(bytes) = self.body_bytes {
            bytes
        } else {
            Bytes::from(self.body)
        }
    }

    /// Get a reference to the body bytes.
    #[inline]
    pub fn body_ref(&self) -> &[u8] {
        if let Some(ref bytes) = self.body_bytes {
            bytes.as_ref()
        } else {
            &self.body
        }
    }

    /// Get the body length.
    #[inline]
    pub fn body_len(&self) -> usize {
        if let Some(ref bytes) = self.body_bytes {
            bytes.len()
        } else {
            self.body.len()
        }
    }

    /// Check if using zero-copy Bytes storage.
    #[inline]
    pub fn has_bytes_body(&self) -> bool {
        self.body_bytes.is_some()
    }

    /// Serialize a value as JSON and set it as the response body.
    ///
    /// With the `simd-json` feature enabled, this uses SIMD-accelerated serialization
    /// which can be 1.5-2x faster on modern x86_64 CPUs.
    ///
    /// The body is stored as `Bytes` for zero-copy passthrough to Hyper.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// HttpResponse::ok().with_json(&user)?
    /// ```
    #[inline]
    pub fn with_json<T: Serialize>(mut self, value: &T) -> Result<Self, crate::Error> {
        let vec =
            crate::json::to_vec(value).map_err(|e| crate::Error::Serialization(e.to_string()))?;
        self.body_bytes = Some(Bytes::from(vec));
        self.body.clear();
        self.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        Ok(self)
    }

    pub fn with_header(mut self, key: String, value: String) -> Self {
        self.headers.insert(key, value);
        self
    }

    /// Set multiple headers from a HashMap.
    #[inline]
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.headers = LazyHeaders::from(headers);
        self
    }

    /// Create a response with status and headers (for CORS preflight, etc.).
    #[inline]
    pub fn with_status_and_headers(status: u16, headers: HashMap<String, String>) -> Self {
        Self {
            status,
            headers: LazyHeaders::from(headers),
            cookies: Vec::new(),
            body: Vec::new(),
            body_bytes: None,
        }
    }

    /// Create a response with all components (for compatibility).
    ///
    /// This is useful when you need to construct a response with all parts at once.
    #[inline]
    pub fn from_parts(status: u16, headers: HashMap<String, String>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: LazyHeaders::from(headers),
            cookies: Vec::new(),
            body,
            body_bytes: None,
        }
    }

    // ============================================================================
    // Convenience Methods for Common Response Types
    // ============================================================================

    /// Create an accepted response (202).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::accepted();
    /// assert_eq!(response.status, 202);
    /// ```
    pub fn accepted() -> Self {
        Self::new(202)
    }

    /// Create an unauthorized response (401).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::unauthorized();
    /// assert_eq!(response.status, 401);
    /// ```
    pub fn unauthorized() -> Self {
        Self::new(401)
    }

    /// Create a forbidden response (403).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::forbidden();
    /// assert_eq!(response.status, 403);
    /// ```
    pub fn forbidden() -> Self {
        Self::new(403)
    }

    /// Create a conflict response (409).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::conflict();
    /// assert_eq!(response.status, 409);
    /// ```
    pub fn conflict() -> Self {
        Self::new(409)
    }

    /// Create a service unavailable response (503).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::service_unavailable();
    /// assert_eq!(response.status, 503);
    /// ```
    pub fn service_unavailable() -> Self {
        Self::new(503)
    }

    /// Shorthand for creating a JSON response with 200 OK status.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// use serde_json::json;
    ///
    /// let response = HttpResponse::json(&json!({"message": "Hello"})).unwrap();
    /// assert_eq!(response.status, 200);
    /// ```
    pub fn json<T: Serialize>(value: &T) -> Result<Self, crate::Error> {
        Self::ok().with_json(value)
    }

    /// Create an HTML response with 200 OK status.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::html("<h1>Hello</h1>");
    /// assert_eq!(response.status, 200);
    /// assert_eq!(response.headers.get("Content-Type"), Some(&"text/html; charset=utf-8".to_string()));
    /// ```
    pub fn html(content: impl Into<String>) -> Self {
        Self::ok()
            .with_header(
                "Content-Type".to_string(),
                "text/html; charset=utf-8".to_string(),
            )
            .with_body(content.into().into_bytes())
    }

    /// Create a plain text response with 200 OK status.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::text("Hello, World!");
    /// assert_eq!(response.status, 200);
    /// assert_eq!(response.headers.get("Content-Type"), Some(&"text/plain; charset=utf-8".to_string()));
    /// ```
    pub fn text(content: impl Into<String>) -> Self {
        Self::ok()
            .with_header(
                "Content-Type".to_string(),
                "text/plain; charset=utf-8".to_string(),
            )
            .with_body(content.into().into_bytes())
    }

    /// Create a redirect response (302 Found).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::redirect("https://example.com");
    /// assert_eq!(response.status, 302);
    /// assert_eq!(response.headers.get("Location"), Some(&"https://example.com".to_string()));
    /// ```
    pub fn redirect(url: impl Into<String>) -> Self {
        Self::new(302).with_header("Location".to_string(), url.into())
    }

    /// Create a permanent redirect response (301 Moved Permanently).
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::redirect_permanent("https://example.com");
    /// assert_eq!(response.status, 301);
    /// ```
    pub fn redirect_permanent(url: impl Into<String>) -> Self {
        Self::new(301).with_header("Location".to_string(), url.into())
    }

    /// Create a see other redirect response (303 See Other).
    /// Useful after a POST request to redirect to a GET.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::see_other("/success");
    /// assert_eq!(response.status, 303);
    /// ```
    pub fn see_other(url: impl Into<String>) -> Self {
        Self::new(303).with_header("Location".to_string(), url.into())
    }

    /// Alias for no_content() - returns 204 with empty body.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::empty();
    /// assert_eq!(response.status, 204);
    /// ```
    pub fn empty() -> Self {
        Self::no_content()
    }

    /// Set the Content-Type header.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::ok().content_type("application/xml");
    /// assert_eq!(response.headers.get("Content-Type"), Some(&"application/xml".to_string()));
    /// ```
    pub fn content_type(self, content_type: impl Into<String>) -> Self {
        self.with_header("Content-Type".to_string(), content_type.into())
    }

    /// Set the Cache-Control header.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::ok().cache_control("max-age=3600");
    /// ```
    pub fn cache_control(self, directive: impl Into<String>) -> Self {
        self.with_header("Cache-Control".to_string(), directive.into())
    }

    /// Mark the response as not cacheable.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::ok().no_cache();
    /// ```
    pub fn no_cache(self) -> Self {
        self.cache_control("no-store, no-cache, must-revalidate")
    }

    /// Set a cookie on the response. Can be called multiple times to set
    /// multiple cookies — each produces a separate `Set-Cookie` header.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::ok()
    ///     .cookie("session", "abc123; HttpOnly; Secure")
    ///     .cookie("theme", "dark; Path=/");
    /// ```
    pub fn cookie(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.cookies.push(format!("{}={}", name.into(), value.into()));
        self
    }

    /// Clear a cookie by setting it with an expired Max-Age.
    ///
    /// # Example
    /// ```
    /// use armature_core::HttpResponse;
    /// let response = HttpResponse::ok().clear_cookie("session", "/");
    /// ```
    pub fn clear_cookie(mut self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.cookies.push(format!(
            "{}=; Path={}; Max-Age=0",
            name.into(),
            path.into(),
        ));
        self
    }

    /// Get the response body as a string (lossy UTF-8 conversion).
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(self.body_ref()).to_string()
    }

    /// Check if the response is successful (2xx status code).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Check if the response is a redirect (3xx status code).
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    /// Check if the response is a client error (4xx status code).
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }

    /// Check if the response is a server error (5xx status code).
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status)
    }
}

/// JSON response helper
#[derive(Debug)]
pub struct Json<T: Serialize>(pub T);

impl<T: Serialize> Json<T> {
    pub fn into_response(self) -> Result<HttpResponse, crate::Error> {
        HttpResponse::ok().with_json(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_request_new() {
        let req = HttpRequest::new("GET".to_string(), "/test".to_string());
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/test");
        assert!(req.headers.is_empty());
        assert!(req.body.is_empty());
    }

    #[test]
    fn test_http_request_with_body() {
        let mut req = HttpRequest::new("POST".to_string(), "/api".to_string());
        req.body = vec![1, 2, 3, 4];
        assert_eq!(req.body.len(), 4);
    }

    #[test]
    fn test_http_request_json_deserialization() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct TestData {
            name: String,
            age: u32,
        }

        let mut req = HttpRequest::new("POST".to_string(), "/api".to_string());
        req.body = serde_json::to_vec(&serde_json::json!({
            "name": "John",
            "age": 30
        }))
        .unwrap();

        let data: TestData = req.json().unwrap();
        assert_eq!(data.name, "John");
        assert_eq!(data.age, 30);
    }

    #[test]
    fn test_http_request_param() {
        let mut req = HttpRequest::new("GET".to_string(), "/users/123".to_string());
        req.path_params.insert("id".to_string(), "123".to_string());

        assert_eq!(req.param("id"), Some(&"123".to_string()));
        assert_eq!(req.param("name"), None);
    }

    #[test]
    fn test_http_request_query() {
        let mut req = HttpRequest::new("GET".to_string(), "/users".to_string());
        req.query_params
            .insert("sort".to_string(), "asc".to_string());

        assert_eq!(req.query("sort"), Some(&"asc".to_string()));
        assert_eq!(req.query("limit"), None);
    }

    #[test]
    fn test_http_request_clone() {
        let req1 = HttpRequest::new("GET".to_string(), "/test".to_string());
        let req2 = req1.clone();

        assert_eq!(req1.method, req2.method);
        assert_eq!(req1.path, req2.path);
    }

    #[test]
    fn test_http_response_ok() {
        let res = HttpResponse::ok();
        assert_eq!(res.status, 200);
    }

    #[test]
    fn test_http_response_created() {
        let res = HttpResponse::created();
        assert_eq!(res.status, 201);
    }

    #[test]
    fn test_http_response_no_content() {
        let res = HttpResponse::no_content();
        assert_eq!(res.status, 204);
    }

    #[test]
    fn test_http_response_bad_request() {
        let res = HttpResponse::bad_request();
        assert_eq!(res.status, 400);
    }

    #[test]
    fn test_http_response_not_found() {
        let res = HttpResponse::not_found();
        assert_eq!(res.status, 404);
    }

    #[test]
    fn test_http_response_internal_server_error() {
        let res = HttpResponse::internal_server_error();
        assert_eq!(res.status, 500);
    }

    #[test]
    fn test_http_response_with_body() {
        let body = b"Hello, World!".to_vec();
        let res = HttpResponse::ok().with_body(body.clone());
        assert_eq!(res.body, body);
    }

    #[test]
    fn test_http_response_with_json() {
        #[derive(Serialize)]
        struct TestData {
            message: String,
        }

        let data = TestData {
            message: "test".to_string(),
        };

        let res = HttpResponse::ok().with_json(&data).unwrap();
        assert!(!res.body_ref().is_empty());
        assert_eq!(
            res.headers.get("Content-Type"),
            Some(&"application/json".to_string())
        );
    }

    #[test]
    fn test_http_response_with_header() {
        let res = HttpResponse::ok().with_header("X-Custom".to_string(), "value".to_string());

        assert_eq!(res.headers.get("X-Custom"), Some(&"value".to_string()));
    }

    #[test]
    fn test_http_response_multiple_headers() {
        let res = HttpResponse::ok()
            .with_header("X-Header-1".to_string(), "value1".to_string())
            .with_header("X-Header-2".to_string(), "value2".to_string());

        assert_eq!(res.headers.len(), 2);
    }

    #[test]
    fn test_json_helper() {
        #[derive(Serialize)]
        struct Data {
            value: i32,
        }

        let json = Json(Data { value: 42 });
        let response = json.into_response().unwrap();

        assert_eq!(response.status, 200);
        assert!(!response.body_ref().is_empty());
    }

    #[test]
    fn test_http_request_with_headers() {
        let mut req = HttpRequest::new("GET".to_string(), "/api".to_string());
        req.headers
            .insert("Authorization".to_string(), "Bearer token".to_string());
        req.headers
            .insert("Content-Type".to_string(), "application/json".to_string());

        assert_eq!(req.headers.len(), 2);
    }

    #[test]
    fn test_http_request_json_invalid() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct TestData {
            name: String,
        }

        let mut req = HttpRequest::new("POST".to_string(), "/api".to_string());
        req.body = b"invalid json".to_vec();

        let result: Result<TestData, crate::Error> = req.json();
        assert!(result.is_err());
    }

    #[test]
    fn test_http_response_new_custom_status() {
        let res = HttpResponse::new(418); // I'm a teapot
        assert_eq!(res.status, 418);
    }

    #[test]
    fn test_http_response_with_json_complex() {
        #[derive(Serialize)]
        struct ComplexData {
            nested: Vec<HashMap<String, i32>>,
        }

        let mut map = HashMap::new();
        map.insert("key".to_string(), 123);

        let data = ComplexData { nested: vec![map] };

        let res = HttpResponse::ok().with_json(&data);
        assert!(res.is_ok());
    }
}
