//! Compression middleware implementation

use crate::{CompressionAlgorithm, CompressionConfig};
use armature_core::{Error, HttpRequest, HttpResponse, Middleware};
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;

/// HTTP response compression middleware
///
/// This middleware compresses HTTP response bodies based on the client's
/// `Accept-Encoding` header and the configured compression algorithm.
///
/// # Example
///
/// ```rust,no_run
/// use armature_compression::{CompressionMiddleware, CompressionConfig};
///
/// // Create with default settings (auto-select algorithm)
/// let middleware = CompressionMiddleware::new();
///
/// // Create with custom configuration
/// let config = CompressionConfig::builder()
///     .min_size(1024)
///     .level(6)
///     .build();
/// let middleware = CompressionMiddleware::with_config(config);
/// ```
#[derive(Debug, Clone)]
pub struct CompressionMiddleware {
    config: CompressionConfig,
}

impl CompressionMiddleware {
    /// Create a new compression middleware with default settings
    pub fn new() -> Self {
        Self {
            config: CompressionConfig::default(),
        }
    }

    /// Create a compression middleware with custom configuration
    pub fn with_config(config: CompressionConfig) -> Self {
        Self { config }
    }

    /// Get a reference to the configuration
    pub fn config(&self) -> &CompressionConfig {
        &self.config
    }

    /// Determine the compression algorithm to use for a request
    fn select_algorithm(&self, accept_encoding: Option<&str>) -> CompressionAlgorithm {
        match self.config.algorithm {
            CompressionAlgorithm::Auto => {
                if let Some(encoding) = accept_encoding {
                    CompressionAlgorithm::select_from_accept_encoding(encoding)
                } else {
                    CompressionAlgorithm::None
                }
            }
            algo => algo,
        }
    }

    /// Check if a response should be compressed
    fn should_compress(&self, response: &HttpResponse) -> bool {
        // Don't compress error responses or empty bodies.
        // Use `body_ref()`/`body_len()` so zero-copy responses (JSON/static payloads
        // stored in `body_bytes`, e.g. via `with_json`) are inspected correctly.
        if response.status >= 400 || response.body_ref().is_empty() {
            return false;
        }

        // Check size threshold
        if !self.config.should_compress_size(response.body_len()) {
            return false;
        }

        // Check if already encoded
        if !self.config.compress_encoded
            && let Some(encoding) = response.headers.get("Content-Encoding")
            && !encoding.is_empty()
            && encoding != "identity"
        {
            return false;
        }

        // Check content type
        if let Some(content_type) = response.headers.get("Content-Type") {
            self.config.should_compress_content_type(content_type)
        } else {
            // No content type header, compress by default for non-empty bodies
            true
        }
    }

    /// Compress the response body
    fn compress_response(
        &self,
        mut response: HttpResponse,
        algorithm: CompressionAlgorithm,
    ) -> HttpResponse {
        let level = self.config.effective_level();

        // Read the body via `body_ref()` so zero-copy responses (payload in
        // `body_bytes`) are compressed rather than silently skipped.
        match algorithm.compress(response.body_ref(), level) {
            Ok(compressed) => {
                // Only use compressed data if it's smaller
                if compressed.len() < response.body_len() {
                    let compressed_len = compressed.len();
                    // Store the compressed payload via the zero-copy setter, which
                    // clears the legacy `body` Vec and populates `body_bytes`.
                    response = response.with_bytes_body(bytes::Bytes::from(compressed));

                    // Set Content-Encoding header
                    if let Some(encoding) = algorithm.encoding_name() {
                        response
                            .headers
                            .insert("Content-Encoding".to_string(), encoding.to_string());
                    }

                    // Update Content-Length
                    response
                        .headers
                        .insert("Content-Length".to_string(), compressed_len.to_string());

                    // Add Vary header to indicate response varies by Accept-Encoding
                    let vary = response.headers.entry("Vary".to_string()).or_default();
                    if !vary.contains("Accept-Encoding") {
                        if !vary.is_empty() {
                            vary.push_str(", ");
                        }
                        vary.push_str("Accept-Encoding");
                    }
                }
            }
            Err(e) => {
                // Log compression error but return original response
                tracing::warn!("Compression failed: {}", e);
            }
        }

        response
    }
}

impl Default for CompressionMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for CompressionMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: Box<
            dyn FnOnce(
                    HttpRequest,
                )
                    -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
                + Send,
        >,
    ) -> Result<HttpResponse, Error> {
        // Get Accept-Encoding header before passing request
        let accept_encoding = req
            .headers
            .get("Accept-Encoding")
            .or_else(|| req.headers.get("accept-encoding"))
            .cloned();

        // Call the next handler
        let response = next(req).await?;

        // Determine compression algorithm
        let algorithm = self.select_algorithm(accept_encoding.as_deref());

        // Check if we should compress
        if algorithm == CompressionAlgorithm::None || !self.should_compress(&response) {
            return Ok(response);
        }

        // Compress and return
        Ok(self.compress_response(response, algorithm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_response(body: &str, content_type: &str) -> HttpResponse {
        HttpResponse::new(200)
            .with_header("Content-Type".to_string(), content_type.to_string())
            .with_header("Content-Length".to_string(), body.len().to_string())
            .with_body(body.as_bytes().to_vec())
    }

    #[test]
    fn test_middleware_creation() {
        let middleware = CompressionMiddleware::new();
        assert_eq!(middleware.config().algorithm, CompressionAlgorithm::Auto);
    }

    #[test]
    fn test_should_compress() {
        let middleware = CompressionMiddleware::new();

        // Large JSON response should compress
        let response = create_response(&"x".repeat(1000), "application/json");
        assert!(middleware.should_compress(&response));

        // Small response should not compress
        let response = create_response("small", "application/json");
        assert!(!middleware.should_compress(&response));

        // Binary content should not compress
        let response = create_response(&"x".repeat(1000), "image/png");
        assert!(!middleware.should_compress(&response));

        // Error response should not compress
        let mut response = create_response(&"x".repeat(1000), "application/json");
        response.status = 500;
        assert!(!middleware.should_compress(&response));

        // Empty body should not compress
        let mut response = create_response("", "application/json");
        response.body = Vec::new();
        assert!(!middleware.should_compress(&response));
    }

    #[test]
    fn test_select_algorithm_auto() {
        let middleware = CompressionMiddleware::new();

        // No Accept-Encoding
        assert_eq!(
            middleware.select_algorithm(None),
            CompressionAlgorithm::None
        );

        #[cfg(feature = "gzip")]
        {
            assert_eq!(
                middleware.select_algorithm(Some("gzip")),
                CompressionAlgorithm::Gzip
            );
        }

        #[cfg(feature = "brotli")]
        {
            assert_eq!(
                middleware.select_algorithm(Some("br")),
                CompressionAlgorithm::Brotli
            );
        }

        #[cfg(all(feature = "gzip", feature = "brotli"))]
        {
            // Brotli should be preferred over gzip
            assert_eq!(
                middleware.select_algorithm(Some("gzip, br")),
                CompressionAlgorithm::Brotli
            );
        }
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn test_select_algorithm_specific() {
        let config = CompressionConfig::builder().gzip().build();
        let middleware = CompressionMiddleware::with_config(config);

        // Should always use gzip regardless of Accept-Encoding
        assert_eq!(
            middleware.select_algorithm(Some("br")),
            CompressionAlgorithm::Gzip
        );
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn test_compress_response() {
        let middleware = CompressionMiddleware::with_config(
            CompressionConfig::builder().gzip().min_size(10).build(),
        );

        // Use a large repetitive string that compresses well
        let body = "Hello, World! ".repeat(100);
        let response = create_response(&body, "text/plain");

        let compressed = middleware.compress_response(response, CompressionAlgorithm::Gzip);

        // Check headers
        assert_eq!(
            compressed.headers.get("Content-Encoding"),
            Some(&"gzip".to_string())
        );
        assert!(
            compressed
                .headers
                .get("Vary")
                .unwrap()
                .contains("Accept-Encoding")
        );

        // Compressed body should be smaller
        assert!(compressed.body_len() < body.len());
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn test_compress_json_body_bytes() {
        // Responses built with `with_json` store their payload in `body_bytes`
        // (the legacy `body` Vec is cleared). Verify such responses are actually
        // compressed rather than silently skipped.
        let middleware = CompressionMiddleware::with_config(
            CompressionConfig::builder().gzip().min_size(10).build(),
        );

        // Use a value whose `Serialize` impl is already available through
        // armature-core's dependency graph (no direct serde dep needed here).
        let payload: Vec<String> = vec!["armature".to_string(); 100];
        let response = HttpResponse::new(200).with_json(&payload).unwrap();

        // Sanity: payload lives in body_bytes, not the legacy Vec, and exceeds min_size.
        assert!(response.has_bytes_body());
        assert!(response.body.is_empty());
        assert!(response.body_len() > 10);

        let original_len = response.body_len();
        assert!(middleware.should_compress(&response));

        let compressed = middleware.compress_response(response, CompressionAlgorithm::Gzip);

        assert_eq!(
            compressed.headers.get("Content-Encoding"),
            Some(&"gzip".to_string())
        );
        assert!(compressed.body_len() < original_len);
    }

    // Build a boxed `next` handler returning a fixed response, matching the
    // `Middleware::handle` signature.
    #[allow(clippy::type_complexity)] // mirrors the trait's boxed-closure signature
    fn next_returning(
        response: HttpResponse,
    ) -> Box<
        dyn FnOnce(HttpRequest) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
            + Send,
    > {
        Box::new(move |_req| Box::pin(async move { Ok(response) }))
    }

    /// End-to-end: Accept-Encoding drives algorithm selection, and the response
    /// body comes back gzip-compressed and byte-for-byte recoverable.
    #[cfg(feature = "gzip")]
    #[tokio::test]
    async fn test_handle_end_to_end_gzip() {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let middleware = CompressionMiddleware::with_config(
            CompressionConfig::builder().gzip().min_size(10).build(),
        );

        let mut req = HttpRequest::new("GET".to_string(), "/data".to_string());
        req.headers
            .insert("Accept-Encoding".to_string(), "gzip".to_string());

        let body = "Hello, World! ".repeat(100);
        let next = next_returning(create_response(&body, "text/plain"));

        let response = middleware.handle(req, next).await.unwrap();

        assert_eq!(
            response.headers.get("Content-Encoding"),
            Some(&"gzip".to_string())
        );
        assert!(
            response
                .headers
                .get("Vary")
                .unwrap()
                .contains("Accept-Encoding")
        );
        assert_eq!(
            response.headers.get("Content-Length"),
            Some(&response.body_len().to_string())
        );
        assert!(response.body_len() < body.len());

        let mut decoder = GzDecoder::new(response.body_ref());
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        assert_eq!(decompressed, body.as_bytes());
    }

    /// End-to-end: with no Accept-Encoding, the middleware selects `None` and
    /// passes the body through untouched.
    #[tokio::test]
    async fn test_handle_end_to_end_no_accept_encoding() {
        let middleware =
            CompressionMiddleware::with_config(CompressionConfig::builder().min_size(10).build());

        let req = HttpRequest::new("GET".to_string(), "/data".to_string());
        let body = "Hello, World! ".repeat(100);
        let next = next_returning(create_response(&body, "text/plain"));

        let response = middleware.handle(req, next).await.unwrap();

        assert!(response.headers.get("Content-Encoding").is_none());
        assert_eq!(response.body_ref(), body.as_bytes());
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn test_vary_header_appended() {
        let middleware = CompressionMiddleware::with_config(
            CompressionConfig::builder().gzip().min_size(10).build(),
        );

        // Use a repetitive string that compresses well
        let body = "Hello, World! ".repeat(100);
        let mut response = create_response(&body, "text/plain");
        response
            .headers
            .insert("Vary".to_string(), "Origin".to_string());

        let compressed = middleware.compress_response(response, CompressionAlgorithm::Gzip);
        let vary = compressed.headers.get("Vary").unwrap();
        assert!(vary.contains("Origin"));
        assert!(vary.contains("Accept-Encoding"));
    }
}
