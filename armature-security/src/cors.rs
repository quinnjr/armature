//! CORS (Cross-Origin Resource Sharing) Configuration
//!
//! Provides granular control over CORS policies for secure cross-origin requests.
//!
//! # Features
//!
//! - Allowed origins (wildcard, specific, regex)
//! - Allowed methods
//! - Allowed headers
//! - Exposed headers
//! - Credentials support
//! - Max age configuration
//! - Preflight request handling
//!
//! # Quick Start
//!
//! ```
//! use armature_security::cors::CorsConfig;
//!
//! // Simple CORS - allow all origins (dev only!)
//! let cors = CorsConfig::permissive();
//!
//! // Production CORS - specific origins
//! let cors = CorsConfig::new()
//!     .allow_origin("https://example.com")
//!     .allow_origin("https://app.example.com")
//!     .allow_methods(vec!["GET", "POST", "PUT", "DELETE"])
//!     .allow_headers(vec!["Content-Type", "Authorization"])
//!     .allow_credentials(true)
//!     .max_age(3600);
//! ```

use armature_core::{Error, HttpRequest, HttpResponse};
use regex::Regex;
use std::collections::HashSet;

/// CORS configuration
#[derive(Debug, Clone)]
pub struct CorsConfig {
    /// Allowed origins (None = all, Some = specific)
    allowed_origins: Option<HashSet<String>>,

    /// Allow all origins (wildcard)
    allow_any_origin: bool,

    /// Allowed origin patterns (regex)
    origin_patterns: Vec<Regex>,

    /// Allowed HTTP methods
    allowed_methods: HashSet<String>,

    /// Allowed request headers
    allowed_headers: Option<HashSet<String>>,

    /// Allow all headers
    allow_any_header: bool,

    /// Exposed response headers
    exposed_headers: Vec<String>,

    /// Allow credentials
    allow_credentials: bool,

    /// Max age for preflight cache (seconds)
    max_age: Option<u64>,
}

impl CorsConfig {
    /// Create a new CORS configuration with strict defaults
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .allow_origin("https://example.com")
    ///     .allow_methods(vec!["GET", "POST"]);
    /// ```
    pub fn new() -> Self {
        Self {
            allowed_origins: Some(HashSet::new()),
            allow_any_origin: false,
            origin_patterns: Vec::new(),
            allowed_methods: ["GET", "POST", "HEAD"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            allowed_headers: Some(HashSet::new()),
            allow_any_header: false,
            exposed_headers: Vec::new(),
            allow_credentials: false,
            max_age: Some(3600),
        }
    }

    /// Permissive CORS (allow all origins) - USE ONLY IN DEVELOPMENT
    ///
    /// # Security Warning
    ///
    /// This allows ALL origins and is NOT SECURE for production use!
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// // Development only!
    /// let cors = CorsConfig::permissive();
    /// ```
    pub fn permissive() -> Self {
        Self {
            allowed_origins: None,
            allow_any_origin: true,
            origin_patterns: Vec::new(),
            allowed_methods: ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            allowed_headers: None,
            allow_any_header: true,
            exposed_headers: Vec::new(),
            allow_credentials: false,
            max_age: Some(3600),
        }
    }

    /// Allow specific origin
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .allow_origin("https://example.com")
    ///     .allow_origin("https://app.example.com");
    /// ```
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        if let Some(ref mut origins) = self.allowed_origins {
            origins.insert(origin.into());
        } else {
            let mut origins = HashSet::new();
            origins.insert(origin.into());
            self.allowed_origins = Some(origins);
        }
        self
    }

    /// Allow origins matching regex pattern
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .allow_origin_regex(r"https://.*\.example\.com").unwrap();
    /// ```
    pub fn allow_origin_regex(mut self, pattern: &str) -> Result<Self, regex::Error> {
        let regex = Regex::new(pattern)?;
        self.origin_patterns.push(regex);
        Ok(self)
    }

    /// Allow all origins (wildcard) - NOT RECOMMENDED FOR PRODUCTION
    ///
    /// Wildcard origins and credentials are mutually exclusive (see
    /// [`allow_credentials`](Self::allow_credentials)); enabling the wildcard
    /// therefore turns credentials back off.
    pub fn allow_any_origin(mut self) -> Self {
        self.allow_any_origin = true;
        self.allowed_origins = None;
        self.allow_credentials = false;
        self
    }

    /// Set allowed HTTP methods
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .allow_methods(vec!["GET", "POST", "PUT", "DELETE"]);
    /// ```
    pub fn allow_methods(mut self, methods: Vec<impl Into<String>>) -> Self {
        self.allowed_methods = methods
            .into_iter()
            .map(|m| m.into().to_uppercase())
            .collect();
        self
    }

    /// Add allowed method
    pub fn allow_method(mut self, method: impl Into<String>) -> Self {
        self.allowed_methods.insert(method.into().to_uppercase());
        self
    }

    /// Set allowed request headers
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .allow_headers(vec!["Content-Type", "Authorization", "X-Custom-Header"]);
    /// ```
    pub fn allow_headers(mut self, headers: Vec<impl Into<String>>) -> Self {
        self.allowed_headers = Some(
            headers
                .into_iter()
                .map(|h| h.into().to_lowercase())
                .collect(),
        );
        self.allow_any_header = false;
        self
    }

    /// Add allowed header
    pub fn allow_header(mut self, header: impl Into<String>) -> Self {
        if let Some(ref mut headers) = self.allowed_headers {
            headers.insert(header.into().to_lowercase());
        } else {
            let mut headers = HashSet::new();
            headers.insert(header.into().to_lowercase());
            self.allowed_headers = Some(headers);
        }
        self.allow_any_header = false;
        self
    }

    /// Allow all headers
    pub fn allow_any_header(mut self) -> Self {
        self.allow_any_header = true;
        self.allowed_headers = None;
        self
    }

    /// Set exposed response headers
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .expose_headers(vec!["X-Total-Count", "X-Page-Number"]);
    /// ```
    pub fn expose_headers(mut self, headers: Vec<impl Into<String>>) -> Self {
        self.exposed_headers = headers.into_iter().map(|h| h.into()).collect();
        self
    }

    /// Allow credentials (cookies, authorization headers)
    ///
    /// # Security Note
    ///
    /// Cannot be used with `allow_any_origin`. Reflecting an arbitrary origin
    /// alongside `Access-Control-Allow-Credentials: true` lets any site read
    /// authenticated responses, so this setter revokes the wildcard rather
    /// than trusting callers to keep the two apart. The origin allowlist is
    /// left empty by that revocation, which fails closed: no origin is
    /// echoed until one is added explicitly.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .allow_origin("https://example.com")
    ///     .allow_credentials(true);
    /// ```
    pub fn allow_credentials(mut self, allow: bool) -> Self {
        self.allow_credentials = allow;
        if allow {
            self.allow_any_origin = false;
        }
        self
    }

    /// Set max age for preflight cache (seconds)
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_security::cors::CorsConfig;
    ///
    /// let cors = CorsConfig::new()
    ///     .max_age(7200); // 2 hours
    /// ```
    pub fn max_age(mut self, seconds: u64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    /// Check if origin is allowed
    pub fn is_origin_allowed(&self, origin: &str) -> bool {
        // Allow any origin
        if self.allow_any_origin {
            return true;
        }

        // Check exact match
        if let Some(ref origins) = self.allowed_origins
            && origins.contains(origin)
        {
            return true;
        }

        // Check regex patterns
        for pattern in &self.origin_patterns {
            if pattern.is_match(origin) {
                return true;
            }
        }

        false
    }

    /// Check if method is allowed
    ///
    /// Entries are stored canonically upper-cased, so the comparison is done
    /// case-insensitively against the (small) set rather than allocating a
    /// re-cased copy of `method` on every call.
    pub fn is_method_allowed(&self, method: &str) -> bool {
        self.allowed_methods
            .iter()
            .any(|m| m.eq_ignore_ascii_case(method))
    }

    /// Check if header is allowed
    ///
    /// Preflight calls this once per requested header, so it compares against
    /// the canonically lower-cased set in place instead of allocating.
    pub fn is_header_allowed(&self, header: &str) -> bool {
        if self.allow_any_header {
            return true;
        }

        if let Some(ref headers) = self.allowed_headers {
            return headers.iter().any(|h| h.eq_ignore_ascii_case(header));
        }

        false
    }

    /// Handle CORS preflight request (OPTIONS)
    ///
    /// Returns a response with appropriate CORS headers.
    pub fn handle_preflight(&self, request: &HttpRequest) -> Result<HttpResponse, Error> {
        let origin = request
            .headers
            .get("origin")
            .ok_or_else(|| Error::BadRequest("Missing Origin header".to_string()))?;

        // Check if origin is allowed
        if !self.is_origin_allowed(origin) {
            return Err(Error::Forbidden("Origin not allowed".to_string()));
        }

        let mut response = HttpResponse::new(204); // No Content

        // Add CORS headers
        self.add_cors_headers(&mut response, origin);

        // Add preflight-specific headers. A method the policy does not permit
        // is a rejected preflight, not a successful one missing a header:
        // answering 204 makes the failure look like a transport problem to the
        // browser and hides the misconfiguration from the operator.
        if let Some(method) = request.headers.get("access-control-request-method") {
            if !self.is_method_allowed(method) {
                return Err(Error::Forbidden("Method not allowed".to_string()));
            }
            response.headers.insert(
                "Access-Control-Allow-Methods".to_string(),
                self.allowed_methods
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }

        if let Some(headers) = request.headers.get("access-control-request-headers") {
            let requested: Vec<&str> = headers.split(',').map(|h| h.trim()).collect();
            let allowed: Vec<String> = requested
                .iter()
                .filter(|h| self.is_header_allowed(h))
                .map(|h| h.to_string())
                .collect();

            if !allowed.is_empty() || self.allow_any_header {
                response.headers.insert(
                    "Access-Control-Allow-Headers".to_string(),
                    if self.allow_any_header {
                        headers.to_owned()
                    } else {
                        allowed.join(", ")
                    },
                );
            }
        }

        if let Some(max_age) = self.max_age {
            response
                .headers
                .insert("Access-Control-Max-Age".to_string(), max_age.to_string());
        }

        Ok(response)
    }

    /// Add CORS headers to response
    pub fn add_cors_headers(&self, response: &mut HttpResponse, origin: &str) {
        // Wildcard + credentials is origin reflection with credentials: the
        // builder refuses to produce that combination, and if it is reached
        // any other way we emit no origin at all rather than the caller's.
        if self.allow_any_origin && self.allow_credentials {
            return;
        }

        // Origin
        if self.allow_any_origin {
            response
                .headers
                .insert("Access-Control-Allow-Origin".to_string(), "*".to_string());
        } else if self.is_origin_allowed(origin) {
            response.headers.insert(
                "Access-Control-Allow-Origin".to_string(),
                origin.to_string(),
            );

            // Vary header for caching. Appended, not inserted: the handler may
            // already vary on Accept-Encoding or similar and overwriting that
            // poisons shared caches.
            append_vary(response, "Origin");
        }

        // Credentials
        if self.allow_credentials {
            response.headers.insert(
                "Access-Control-Allow-Credentials".to_string(),
                "true".to_string(),
            );
        }

        // Exposed headers
        if !self.exposed_headers.is_empty() {
            response.headers.insert(
                "Access-Control-Expose-Headers".to_string(),
                self.exposed_headers.join(", "),
            );
        }
    }

    /// Apply CORS to a response
    pub fn apply(&self, request: &HttpRequest, mut response: HttpResponse) -> HttpResponse {
        if let Some(origin) = request.headers.get("origin") {
            self.add_cors_headers(&mut response, origin);
        }
        response
    }
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Add `value` to the response's `Vary` field without dropping existing entries.
fn append_vary(response: &mut HttpResponse, value: &str) {
    let merged = match response.headers.get("Vary") {
        Some(existing) => {
            if existing
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case(value))
            {
                return;
            }
            format!("{}, {}", existing, value)
        }
        None => value.to_string(),
    };
    response.headers.insert("Vary".to_string(), merged);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cors_new() {
        let cors = CorsConfig::new();
        assert!(!cors.allow_any_origin);
        assert!(!cors.allow_credentials);
    }

    #[test]
    fn test_cors_permissive() {
        let cors = CorsConfig::permissive();
        assert!(cors.allow_any_origin);
        assert!(cors.allow_any_header);
    }

    #[test]
    fn test_allow_origin() {
        let cors = CorsConfig::new().allow_origin("https://example.com");

        assert!(cors.is_origin_allowed("https://example.com"));
        assert!(!cors.is_origin_allowed("https://evil.com"));
    }

    #[test]
    fn test_allow_origin_regex() {
        let cors = CorsConfig::new()
            .allow_origin_regex(r"https://.*\.example\.com")
            .unwrap();

        assert!(cors.is_origin_allowed("https://app.example.com"));
        assert!(cors.is_origin_allowed("https://api.example.com"));
        assert!(!cors.is_origin_allowed("https://example.com"));
        assert!(!cors.is_origin_allowed("https://evil.com"));
    }

    #[test]
    fn test_allow_methods() {
        let cors = CorsConfig::new().allow_methods(vec!["GET", "POST", "put"]);

        assert!(cors.is_method_allowed("GET"));
        assert!(cors.is_method_allowed("POST"));
        assert!(cors.is_method_allowed("PUT"));
        assert!(!cors.is_method_allowed("DELETE"));
    }

    #[test]
    fn test_allow_headers() {
        let cors = CorsConfig::new().allow_headers(vec!["Content-Type", "Authorization"]);

        assert!(cors.is_header_allowed("content-type"));
        assert!(cors.is_header_allowed("authorization"));
        assert!(!cors.is_header_allowed("X-Custom-Header"));
    }

    #[test]
    fn test_allow_any_header() {
        let cors = CorsConfig::new().allow_any_header();

        assert!(cors.is_header_allowed("Content-Type"));
        assert!(cors.is_header_allowed("X-Custom-Header"));
        assert!(cors.is_header_allowed("anything"));
    }

    #[test]
    fn credentials_never_reflect_an_arbitrary_origin() {
        // `permissive()` + credentials is the two-call path into the classic
        // origin-reflection-with-credentials hole: the attacker's origin echoed
        // back next to `Allow-Credentials: true` grants any site authenticated
        // cross-origin reads.
        let cors = CorsConfig::permissive().allow_credentials(true);

        assert!(!cors.allow_any_origin);
        assert!(!cors.is_origin_allowed("https://evil.com"));

        let mut response = HttpResponse::ok();
        cors.add_cors_headers(&mut response, "https://evil.com");

        assert_eq!(response.headers.get("Access-Control-Allow-Origin"), None);
    }

    #[test]
    fn wildcard_revokes_credentials() {
        let cors = CorsConfig::new()
            .allow_origin("https://example.com")
            .allow_credentials(true)
            .allow_any_origin();

        let mut response = HttpResponse::ok();
        cors.add_cors_headers(&mut response, "https://evil.com");

        assert_eq!(
            response.headers.get("Access-Control-Allow-Origin"),
            Some(&"*".to_string())
        );
        assert_eq!(
            response.headers.get("Access-Control-Allow-Credentials"),
            None
        );
    }

    #[test]
    fn vary_is_appended_not_overwritten() {
        let cors = CorsConfig::new().allow_origin("https://example.com");

        let mut response = HttpResponse::ok();
        response
            .headers
            .insert("Vary".to_string(), "Accept-Encoding".to_string());
        cors.add_cors_headers(&mut response, "https://example.com");

        assert_eq!(
            response.headers.get("Vary"),
            Some(&"Accept-Encoding, Origin".to_string())
        );
    }

    #[test]
    fn preflight_rejects_disallowed_method() {
        let cors = CorsConfig::new()
            .allow_origin("https://example.com")
            .allow_methods(vec!["GET"]);

        let mut request = HttpRequest::new("OPTIONS", "/api/users".to_string());
        request.headers.insert("origin", "https://example.com");
        request
            .headers
            .insert("access-control-request-method", "DELETE");

        assert!(matches!(
            cors.handle_preflight(&request),
            Err(Error::Forbidden(_))
        ));
    }

    #[test]
    fn test_add_cors_headers() {
        let cors = CorsConfig::new()
            .allow_origin("https://example.com")
            .allow_credentials(true)
            .expose_headers(vec!["X-Total-Count"]);

        let mut response = HttpResponse::ok();
        cors.add_cors_headers(&mut response, "https://example.com");

        assert_eq!(
            response.headers.get("Access-Control-Allow-Origin"),
            Some(&"https://example.com".to_string())
        );
        assert_eq!(
            response.headers.get("Access-Control-Allow-Credentials"),
            Some(&"true".to_string())
        );
        assert_eq!(
            response.headers.get("Access-Control-Expose-Headers"),
            Some(&"X-Total-Count".to_string())
        );
    }
}
