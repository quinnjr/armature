//! Global Exception Filters
//!
//! This module provides a NestJS-style exception filter system for centralized
//! error handling and transformation.
//!
//! # Features
//!
//! - Global exception filters that catch all errors
//! - Type-specific exception filters (catch specific error types)
//! - Filter chaining with priority support
//! - Integration with Problem Details (RFC 7807)
//! - Built-in filters for common scenarios
//!
//! # Examples
//!
//! ## Basic Exception Filter
//!
//! ```ignore
//! use armature_core::exception_filter::{ExceptionFilter, ExceptionContext};
//!
//! struct MyExceptionFilter;
//!
//! #[async_trait]
//! impl ExceptionFilter for MyExceptionFilter {
//!     async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
//!         if let Error::NotFound(_) = error {
//!             Some(HttpResponse::not_found()
//!                 .with_json(&json!({"error": "Resource not found"}))
//!                 .unwrap())
//!         } else {
//!             None // Let other filters handle it
//!         }
//!     }
//! }
//! ```
//!
//! ## Using the `#[catch]` Decorator
//!
//! ```ignore
//! use armature_proc_macro::catch;
//!
//! #[catch(NotFound)]
//! async fn handle_not_found(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
//!     HttpResponse::not_found()
//!         .with_json(&json!({"error": error.to_string()}))
//!         .unwrap()
//! }
//! ```
//!
//! ## Registering Global Filters
//!
//! ```
//! use armature_core::{Application, Module};
//! use armature_core::exception_filter::AllExceptionsFilter;
//! use std::any::TypeId;
//!
//! #[derive(Default)]
//! struct AppModule;
//! impl Module for AppModule {
//!     fn providers(&self) -> Vec<armature_core::ProviderRegistration> { vec![] }
//!     fn controllers(&self) -> Vec<armature_core::ControllerRegistration> { vec![] }
//!     fn imports(&self) -> Vec<Box<dyn Module>> { vec![] }
//!     fn exports(&self) -> Vec<TypeId> { vec![] }
//! }
//!
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! let app = Application::create::<AppModule>()
//!     .await
//!     .use_global_filter(AllExceptionsFilter::new());
//! # let _ = app;
//! # });
//! ```

use crate::error_transform::{ErrorContext, ErrorResponse, ErrorTransformer, ResponseFormat};
use crate::{Error, HttpRequest, HttpResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Exception Context
// ============================================================================

/// Context information passed to exception filters.
#[derive(Debug, Clone)]
pub struct ExceptionContext {
    /// The original HTTP request
    pub request: HttpRequest,
    /// Request ID for tracing
    pub request_id: Option<String>,
    /// User ID (if authenticated)
    pub user_id: Option<String>,
    /// The path that was being accessed
    pub path: String,
    /// The HTTP method
    pub method: String,
    /// Additional context data
    pub data: HashMap<String, serde_json::Value>,
    /// Whether we're in production mode
    pub production_mode: bool,
}

impl ExceptionContext {
    /// Create a new exception context from an HTTP request.
    pub fn from_request(request: HttpRequest) -> Self {
        let request_id = request
            .headers
            .get("x-request-id")
            .or_else(|| request.headers.get("X-Request-Id"))
            .cloned();

        let path = request.path.clone();
        let method = request.method.clone();

        Self {
            request,
            request_id,
            user_id: None,
            path,
            method,
            data: HashMap::new(),
            production_mode: true,
        }
    }

    /// Set the user ID.
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set production mode.
    pub fn with_production_mode(mut self, production: bool) -> Self {
        self.production_mode = production;
        self
    }

    /// Add context data.
    pub fn with_data(mut self, key: impl Into<String>, value: impl serde::Serialize) -> Self {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.data.insert(key.into(), json_value);
        }
        self
    }

    /// Convert to ErrorContext for compatibility with ErrorTransformer.
    pub fn to_error_context(&self) -> ErrorContext {
        ErrorContext {
            request: self.request.clone(),
            request_id: self.request_id.clone(),
            user_id: self.user_id.clone(),
            data: self.data.clone(),
        }
    }
}

// ============================================================================
// Exception Filter Trait
// ============================================================================

/// Trait for exception filters that catch and transform errors.
///
/// Exception filters are called in order of registration, and the first
/// filter that returns `Some(HttpResponse)` wins.
#[async_trait]
pub trait ExceptionFilter: Send + Sync + 'static {
    /// Catch and handle an exception.
    ///
    /// Return `Some(HttpResponse)` to handle the error, or `None` to pass
    /// it to the next filter in the chain.
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse>;

    /// Get the error types this filter handles.
    ///
    /// Return `None` to handle all errors, or `Some(vec![...])` to only
    /// handle specific error variants.
    fn handles(&self) -> Option<Vec<&'static str>> {
        None // Handle all errors by default
    }

    /// Get the filter's priority (higher = earlier in chain).
    fn priority(&self) -> i32 {
        0
    }

    /// Get the filter's name for debugging.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

// ============================================================================
// Exception Filter Chain
// ============================================================================

/// A chain of exception filters.
pub struct ExceptionFilterChain {
    filters: Vec<Arc<dyn ExceptionFilter>>,
    default_transformer: ErrorTransformer,
    response_format: ResponseFormat,
}

impl ExceptionFilterChain {
    /// Create a new empty filter chain.
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            default_transformer: ErrorTransformer::production(),
            response_format: ResponseFormat::Json,
        }
    }

    /// Add a filter to the chain.
    pub fn add_filter<F: ExceptionFilter>(mut self, filter: F) -> Self {
        self.filters.push(Arc::new(filter));
        // Sort by priority (descending)
        self.filters
            .sort_by_key(|f| std::cmp::Reverse(f.priority()));
        self
    }

    /// Add a filter as Arc.
    pub fn add_filter_arc(mut self, filter: Arc<dyn ExceptionFilter>) -> Self {
        self.filters.push(filter);
        self.filters
            .sort_by_key(|f| std::cmp::Reverse(f.priority()));
        self
    }

    /// Set the default error transformer.
    pub fn with_transformer(mut self, transformer: ErrorTransformer) -> Self {
        self.default_transformer = transformer;
        self
    }

    /// Set the response format.
    pub fn with_format(mut self, format: ResponseFormat) -> Self {
        self.response_format = format;
        self
    }

    /// Use Problem Details (RFC 7807) format.
    pub fn use_problem_details(mut self) -> Self {
        self.response_format = ResponseFormat::ProblemDetails;
        self.default_transformer = self
            .default_transformer
            .format(ResponseFormat::ProblemDetails);
        self
    }

    /// Handle an error through the filter chain.
    pub async fn handle(&self, error: &Error, request: &HttpRequest) -> HttpResponse {
        let ctx = ExceptionContext::from_request(request.clone());
        self.handle_with_context(error, &ctx).await
    }

    /// Handle an error with full context.
    pub async fn handle_with_context(&self, error: &Error, ctx: &ExceptionContext) -> HttpResponse {
        let error_type = get_error_type_name(error);

        // Try each filter in order
        for filter in &self.filters {
            // Check if filter handles this error type
            if let Some(handled_types) = filter.handles()
                && !handled_types.contains(&error_type)
            {
                continue;
            }

            // Try to catch the error
            if let Some(response) = filter.catch(error, ctx).await {
                tracing::debug!(
                    filter = filter.name(),
                    error_type = error_type,
                    "Exception caught by filter"
                );
                return response;
            }
        }

        // No filter handled it, use default transformer
        tracing::debug!(
            error_type = error_type,
            "No filter caught exception, using default transformer"
        );
        self.default_transformer
            .transform_with_context(error, &ctx.to_error_context())
    }
}

impl Default for ExceptionFilterChain {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ExceptionFilterChain {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            default_transformer: self.default_transformer.clone(),
            response_format: self.response_format,
        }
    }
}

// ============================================================================
// Built-in Exception Filters
// ============================================================================

/// A catch-all exception filter that transforms all errors.
pub struct AllExceptionsFilter {
    transformer: ErrorTransformer,
}

impl AllExceptionsFilter {
    /// Create a new catch-all filter.
    pub fn new() -> Self {
        Self {
            transformer: ErrorTransformer::production(),
        }
    }

    /// Use a custom transformer.
    pub fn with_transformer(mut self, transformer: ErrorTransformer) -> Self {
        self.transformer = transformer;
        self
    }

    /// Use development mode (verbose errors).
    pub fn development() -> Self {
        Self {
            transformer: ErrorTransformer::development(),
        }
    }

    /// Use Problem Details format.
    pub fn problem_details() -> Self {
        Self {
            transformer: ErrorTransformer::api(),
        }
    }
}

impl Default for AllExceptionsFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for AllExceptionsFilter {
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        Some(
            self.transformer
                .transform_with_context(error, &ctx.to_error_context()),
        )
    }

    fn priority(&self) -> i32 {
        -1000 // Very low priority - should be last
    }

    fn name(&self) -> &str {
        "AllExceptionsFilter"
    }
}

/// HTTP exception filter that handles specific HTTP error types.
pub struct HttpExceptionFilter {
    format: ResponseFormat,
    production_mode: bool,
}

impl HttpExceptionFilter {
    /// Create a new HTTP exception filter.
    pub fn new() -> Self {
        Self {
            format: ResponseFormat::Json,
            production_mode: true,
        }
    }

    /// Set the response format.
    pub fn with_format(mut self, format: ResponseFormat) -> Self {
        self.format = format;
        self
    }

    /// Set production mode.
    pub fn production_mode(mut self, production: bool) -> Self {
        self.production_mode = production;
        self
    }
}

impl Default for HttpExceptionFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for HttpExceptionFilter {
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        let status = error.status_code();
        let error_type = get_error_type_name(error);

        let mut response = ErrorResponse::new(status)
            .error_type(error_type)
            .path(&ctx.path);

        // Add message
        if self.production_mode && error.is_server_error() {
            response = response.message("An internal server error occurred");
        } else {
            response = response.message(error.to_string());
        }

        // Add request ID
        if let Some(ref request_id) = ctx.request_id {
            response = response.request_id(request_id);
        }

        Some(response.into_http_response(self.format))
    }

    fn handles(&self) -> Option<Vec<&'static str>> {
        Some(vec![
            "BadRequest",
            "Unauthorized",
            "Forbidden",
            "NotFound",
            "Conflict",
            "TooManyRequests",
            "Internal",
            "ServiceUnavailable",
        ])
    }

    fn name(&self) -> &str {
        "HttpExceptionFilter"
    }
}

/// Validation exception filter for handling validation errors.
pub struct ValidationExceptionFilter {
    format: ResponseFormat,
}

impl ValidationExceptionFilter {
    /// Create a new validation exception filter.
    pub fn new() -> Self {
        Self {
            format: ResponseFormat::Json,
        }
    }

    /// Use Problem Details format.
    pub fn problem_details() -> Self {
        Self {
            format: ResponseFormat::ProblemDetails,
        }
    }
}

impl Default for ValidationExceptionFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for ValidationExceptionFilter {
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        if let Error::Validation(msg) | Error::UnprocessableEntity(msg) = error {
            let response = ErrorResponse::new(422)
                .message("Validation failed")
                .error_type("VALIDATION_ERROR")
                .details(msg)
                .path(&ctx.path);

            return Some(response.into_http_response(self.format));
        }
        None
    }

    fn handles(&self) -> Option<Vec<&'static str>> {
        Some(vec!["Validation", "UnprocessableEntity"])
    }

    fn priority(&self) -> i32 {
        100 // High priority
    }

    fn name(&self) -> &str {
        "ValidationExceptionFilter"
    }
}

/// Not found exception filter with custom messaging.
pub struct NotFoundExceptionFilter {
    custom_message: Option<String>,
    format: ResponseFormat,
}

impl NotFoundExceptionFilter {
    /// Create a new not found exception filter.
    pub fn new() -> Self {
        Self {
            custom_message: None,
            format: ResponseFormat::Json,
        }
    }

    /// Set a custom message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.custom_message = Some(message.into());
        self
    }

    /// Use Problem Details format.
    pub fn problem_details() -> Self {
        Self {
            custom_message: None,
            format: ResponseFormat::ProblemDetails,
        }
    }
}

impl Default for NotFoundExceptionFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for NotFoundExceptionFilter {
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        if let Error::NotFound(_) | Error::RouteNotFound(_) = error {
            let message = self
                .custom_message
                .clone()
                .unwrap_or_else(|| error.to_string());

            let response = ErrorResponse::new(404)
                .message(message)
                .error_type("NOT_FOUND")
                .path(&ctx.path);

            return Some(response.into_http_response(self.format));
        }
        None
    }

    fn handles(&self) -> Option<Vec<&'static str>> {
        Some(vec!["NotFound", "RouteNotFound"])
    }

    fn priority(&self) -> i32 {
        50
    }

    fn name(&self) -> &str {
        "NotFoundExceptionFilter"
    }
}

/// Unauthorized exception filter.
pub struct UnauthorizedExceptionFilter {
    realm: Option<String>,
    format: ResponseFormat,
}

impl UnauthorizedExceptionFilter {
    /// Create a new unauthorized exception filter.
    pub fn new() -> Self {
        Self {
            realm: None,
            format: ResponseFormat::Json,
        }
    }

    /// Set the WWW-Authenticate realm.
    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = Some(realm.into());
        self
    }
}

impl Default for UnauthorizedExceptionFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for UnauthorizedExceptionFilter {
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        if let Error::Unauthorized(msg) = error {
            let response = ErrorResponse::new(401)
                .message(msg)
                .error_type("UNAUTHORIZED")
                .path(&ctx.path);

            let mut http_response = response.into_http_response(self.format);

            // Add WWW-Authenticate header
            if let Some(ref realm) = self.realm {
                http_response.headers.insert(
                    "WWW-Authenticate".to_string(),
                    format!("Bearer realm=\"{}\"", realm),
                );
            }

            return Some(http_response);
        }
        None
    }

    fn handles(&self) -> Option<Vec<&'static str>> {
        Some(vec!["Unauthorized"])
    }

    fn priority(&self) -> i32 {
        50
    }

    fn name(&self) -> &str {
        "UnauthorizedExceptionFilter"
    }
}

/// Rate limit exception filter.
pub struct RateLimitExceptionFilter {
    retry_after: Option<u32>,
    format: ResponseFormat,
}

impl RateLimitExceptionFilter {
    /// Create a new rate limit exception filter.
    pub fn new() -> Self {
        Self {
            retry_after: None,
            format: ResponseFormat::Json,
        }
    }

    /// Set the Retry-After header value in seconds.
    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retry_after = Some(seconds);
        self
    }
}

impl Default for RateLimitExceptionFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExceptionFilter for RateLimitExceptionFilter {
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        if let Error::TooManyRequests(msg) = error {
            let response = ErrorResponse::new(429)
                .message(msg)
                .error_type("RATE_LIMITED")
                .path(&ctx.path);

            let mut http_response = response.into_http_response(self.format);

            // Add Retry-After header
            if let Some(seconds) = self.retry_after {
                http_response
                    .headers
                    .insert("Retry-After".to_string(), seconds.to_string());
            }

            return Some(http_response);
        }
        None
    }

    fn handles(&self) -> Option<Vec<&'static str>> {
        Some(vec!["TooManyRequests"])
    }

    fn priority(&self) -> i32 {
        50
    }

    fn name(&self) -> &str {
        "RateLimitExceptionFilter"
    }
}

// ============================================================================
// Function-based Exception Filter
// ============================================================================

/// A function-based exception filter for simple cases.
pub struct FnExceptionFilter<F>
where
    F: Fn(&Error, &ExceptionContext) -> Option<HttpResponse> + Send + Sync + 'static,
{
    handler: F,
    error_types: Option<Vec<&'static str>>,
    priority: i32,
    name: String,
}

impl<F> FnExceptionFilter<F>
where
    F: Fn(&Error, &ExceptionContext) -> Option<HttpResponse> + Send + Sync + 'static,
{
    /// Create a new function-based filter.
    pub fn new(handler: F) -> Self {
        Self {
            handler,
            error_types: None,
            priority: 0,
            name: "FnExceptionFilter".to_string(),
        }
    }

    /// Set the error types to handle.
    pub fn handles(mut self, types: Vec<&'static str>) -> Self {
        self.error_types = Some(types);
        self
    }

    /// Set the priority.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Set the name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

#[async_trait]
impl<F> ExceptionFilter for FnExceptionFilter<F>
where
    F: Fn(&Error, &ExceptionContext) -> Option<HttpResponse> + Send + Sync + 'static,
{
    async fn catch(&self, error: &Error, ctx: &ExceptionContext) -> Option<HttpResponse> {
        (self.handler)(error, ctx)
    }

    fn handles(&self) -> Option<Vec<&'static str>> {
        self.error_types.clone()
    }

    fn priority(&self) -> i32 {
        self.priority
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Get the error type name as a string.
fn get_error_type_name(error: &Error) -> &'static str {
    match error {
        Error::Http(_) => "Http",
        Error::RouteNotFound(_) => "RouteNotFound",
        Error::MethodNotAllowed(_) => "MethodNotAllowed",
        Error::DependencyInjection(_) => "DependencyInjection",
        Error::ProviderNotFound(_) => "ProviderNotFound",
        Error::Serialization(_) => "Serialization",
        Error::Deserialization(_) => "Deserialization",
        Error::Validation(_) => "Validation",
        Error::Internal(_) => "Internal",
        Error::Forbidden(_) => "Forbidden",
        Error::Io(_) => "Io",
        Error::BadRequest(_) => "BadRequest",
        Error::Unauthorized(_) => "Unauthorized",
        Error::PaymentRequired(_) => "PaymentRequired",
        Error::NotFound(_) => "NotFound",
        Error::NotAcceptable(_) => "NotAcceptable",
        Error::ProxyAuthenticationRequired(_) => "ProxyAuthenticationRequired",
        Error::RequestTimeout(_) => "RequestTimeout",
        Error::Conflict(_) => "Conflict",
        Error::Gone(_) => "Gone",
        Error::LengthRequired(_) => "LengthRequired",
        Error::PreconditionFailed(_) => "PreconditionFailed",
        Error::PayloadTooLarge(_) => "PayloadTooLarge",
        Error::UriTooLong(_) => "UriTooLong",
        Error::UnsupportedMediaType(_) => "UnsupportedMediaType",
        Error::RangeNotSatisfiable(_) => "RangeNotSatisfiable",
        Error::ExpectationFailed(_) => "ExpectationFailed",
        Error::ImATeapot(_) => "ImATeapot",
        Error::MisdirectedRequest(_) => "MisdirectedRequest",
        Error::UnprocessableEntity(_) => "UnprocessableEntity",
        Error::Locked(_) => "Locked",
        Error::FailedDependency(_) => "FailedDependency",
        Error::TooEarly(_) => "TooEarly",
        Error::UpgradeRequired(_) => "UpgradeRequired",
        Error::PreconditionRequired(_) => "PreconditionRequired",
        Error::TooManyRequests(_) => "TooManyRequests",
        Error::RequestHeaderFieldsTooLarge(_) => "RequestHeaderFieldsTooLarge",
        Error::UnavailableForLegalReasons(_) => "UnavailableForLegalReasons",
        Error::NotImplemented(_) => "NotImplemented",
        Error::BadGateway(_) => "BadGateway",
        Error::ServiceUnavailable(_) => "ServiceUnavailable",
        Error::GatewayTimeout(_) => "GatewayTimeout",
        Error::HttpVersionNotSupported(_) => "HttpVersionNotSupported",
        Error::VariantAlsoNegotiates(_) => "VariantAlsoNegotiates",
        Error::InsufficientStorage(_) => "InsufficientStorage",
        Error::LoopDetected(_) => "LoopDetected",
        Error::NotExtended(_) => "NotExtended",
        Error::NetworkAuthenticationRequired(_) => "NetworkAuthenticationRequired",
    }
}

// ============================================================================
// Presets
// ============================================================================

impl ExceptionFilterChain {
    /// Create a development filter chain (verbose errors).
    pub fn development() -> Self {
        Self::new()
            .add_filter(ValidationExceptionFilter::new())
            .add_filter(AllExceptionsFilter::development())
    }

    /// Create a production filter chain (safe errors).
    pub fn production() -> Self {
        Self::new()
            .add_filter(ValidationExceptionFilter::new())
            .add_filter(NotFoundExceptionFilter::new())
            .add_filter(UnauthorizedExceptionFilter::new())
            .add_filter(RateLimitExceptionFilter::new())
            .add_filter(HttpExceptionFilter::new())
            .add_filter(AllExceptionsFilter::new())
    }

    /// Create an API filter chain (Problem Details format).
    pub fn api() -> Self {
        Self::new()
            .use_problem_details()
            .add_filter(ValidationExceptionFilter::problem_details())
            .add_filter(NotFoundExceptionFilter::problem_details())
            .add_filter(AllExceptionsFilter::problem_details())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exception_context() {
        let request = HttpRequest::new("GET".to_string(), "/api/users".to_string());
        let ctx = ExceptionContext::from_request(request)
            .with_user_id("user-123")
            .with_production_mode(false);

        assert_eq!(ctx.path, "/api/users");
        assert_eq!(ctx.method, "GET");
        assert_eq!(ctx.user_id, Some("user-123".to_string()));
        assert!(!ctx.production_mode);
    }

    #[tokio::test]
    async fn test_http_exception_filter() {
        let filter = HttpExceptionFilter::new();
        let error = Error::BadRequest("Invalid input".to_string());
        let request = HttpRequest::new("POST".to_string(), "/api/users".to_string());
        let ctx = ExceptionContext::from_request(request);

        let response = filter.catch(&error, &ctx).await;
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.status, 400);
    }

    #[tokio::test]
    async fn test_validation_exception_filter() {
        let filter = ValidationExceptionFilter::new();
        let error = Error::Validation("Email is required".to_string());
        let request = HttpRequest::new("POST".to_string(), "/api/users".to_string());
        let ctx = ExceptionContext::from_request(request);

        let response = filter.catch(&error, &ctx).await;
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.status, 422);
    }

    #[tokio::test]
    async fn test_not_found_filter() {
        let filter = NotFoundExceptionFilter::new().with_message("Resource not found");
        let error = Error::NotFound("User not found".to_string());
        let request = HttpRequest::new("GET".to_string(), "/api/users/123".to_string());
        let ctx = ExceptionContext::from_request(request);

        let response = filter.catch(&error, &ctx).await;
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.status, 404);
    }

    #[tokio::test]
    async fn test_filter_chain() {
        let chain = ExceptionFilterChain::new()
            .add_filter(ValidationExceptionFilter::new())
            .add_filter(NotFoundExceptionFilter::new())
            .add_filter(AllExceptionsFilter::new());

        // Test validation error
        let error = Error::Validation("Invalid".to_string());
        let request = HttpRequest::new("POST".to_string(), "/api".to_string());
        let response = chain.handle(&error, &request).await;
        assert_eq!(response.status, 422);

        // Test not found error
        let error = Error::NotFound("Not found".to_string());
        let response = chain.handle(&error, &request).await;
        assert_eq!(response.status, 404);

        // Test internal error
        let error = Error::Internal("Server error".to_string());
        let response = chain.handle(&error, &request).await;
        assert_eq!(response.status, 500);
    }

    #[tokio::test]
    async fn test_fn_exception_filter() {
        let filter = FnExceptionFilter::new(|error, _ctx| {
            if let Error::BadRequest(_) = error {
                Some(
                    HttpResponse::new(400)
                        .with_json(&serde_json::json!({"custom": "response"}))
                        .unwrap(),
                )
            } else {
                None
            }
        })
        .handles(vec!["BadRequest"])
        .with_priority(100)
        .with_name("CustomFilter");

        let error = Error::BadRequest("Bad".to_string());
        let request = HttpRequest::new("GET".to_string(), "/".to_string());
        let ctx = ExceptionContext::from_request(request);

        let response = filter.catch(&error, &ctx).await;
        assert!(response.is_some());
        assert_eq!(filter.priority(), 100);
        assert_eq!(filter.name(), "CustomFilter");
    }

    #[tokio::test]
    async fn test_unauthorized_filter_with_realm() {
        let filter = UnauthorizedExceptionFilter::new().with_realm("api");
        let error = Error::Unauthorized("Invalid token".to_string());
        let request = HttpRequest::new("GET".to_string(), "/api/protected".to_string());
        let ctx = ExceptionContext::from_request(request);

        let response = filter.catch(&error, &ctx).await;
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.status, 401);
        assert!(resp.headers.contains_key("WWW-Authenticate"));
    }

    #[tokio::test]
    async fn test_rate_limit_filter_with_retry_after() {
        let filter = RateLimitExceptionFilter::new().with_retry_after(60);
        let error = Error::TooManyRequests("Rate limited".to_string());
        let request = HttpRequest::new("GET".to_string(), "/api".to_string());
        let ctx = ExceptionContext::from_request(request);

        let response = filter.catch(&error, &ctx).await;
        assert!(response.is_some());

        let resp = response.unwrap();
        assert_eq!(resp.status, 429);
        assert_eq!(resp.headers.get("Retry-After"), Some(&"60".to_string()));
    }

    #[tokio::test]
    async fn test_preset_chains() {
        let dev_chain = ExceptionFilterChain::development();
        let prod_chain = ExceptionFilterChain::production();
        let api_chain = ExceptionFilterChain::api();

        let error = Error::NotFound("Not found".to_string());
        let request = HttpRequest::new("GET".to_string(), "/".to_string());

        // All chains should handle the error
        let response = dev_chain.handle(&error, &request).await;
        assert_eq!(response.status, 404);

        let response = prod_chain.handle(&error, &request).await;
        assert_eq!(response.status, 404);

        let response = api_chain.handle(&error, &request).await;
        assert_eq!(response.status, 404);
        // API chain should return Problem Details content type
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&"application/problem+json".to_string())
        );
    }
}
