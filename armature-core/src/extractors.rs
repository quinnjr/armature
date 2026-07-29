//! Request parameter extractors
//!
//! This module provides types for extracting data from HTTP requests
//! in a type-safe manner, similar to NestJS decorators.
//!
//! # Example
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_core::extractors::{Body, Query, Path, Header};
//!
//! #[derive(Deserialize)]
//! struct CreateUser {
//!     name: String,
//!     email: String,
//! }
//!
//! #[derive(Deserialize)]
//! struct UserFilters {
//!     page: Option<u32>,
//!     limit: Option<u32>,
//! }
//!
//! // Extract body as JSON
//! let body: Body<CreateUser> = Body::from_request(&request)?;
//!
//! // Extract query parameters
//! let query: Query<UserFilters> = Query::from_request(&request)?;
//!
//! // Extract path parameter
//! let id: Path<u32> = Path::from_request(&request, "id")?;
//!
//! // Extract header
//! let auth: Header = Header::from_request(&request, "Authorization")?;
//! ```

use crate::{Error, HttpRequest};
use serde::de::DeserializeOwned;
use std::ops::Deref;
use std::sync::Arc;

/// Trait for extracting data from an HTTP request
pub trait FromRequest: Sized {
    /// Extract data from the request
    fn from_request(request: &HttpRequest) -> Result<Self, Error>;
}

// ========== State Extractor ==========

/// Zero-cost application state extractor.
///
/// `State<T>` provides type-safe access to application state without
/// runtime type checking overhead. State is stored in request extensions
/// and retrieved via `TypeId` lookup followed by a direct pointer cast.
///
/// # Performance
///
/// Unlike DI container lookups which use `Any::downcast`, `State<T>` uses
/// a pre-verified `TypeId` for O(1) retrieval with no runtime type checking.
///
/// # Example
///
/// ```rust,ignore
/// use armature_core::extractors::State;
/// use std::sync::Arc;
///
/// // Define your application state
/// #[derive(Clone)]
/// struct AppState {
///     db_pool: Pool,
///     config: AppConfig,
/// }
///
/// // Insert state into the application (done once at startup)
/// let state = Arc::new(AppState { db_pool, config });
/// app.with_state(state);
///
/// // Extract in handler - zero-cost after setup
/// #[get("/users")]
/// async fn list_users(state: State<AppState>) -> Result<HttpResponse, Error> {
///     let users = state.db_pool.query("SELECT * FROM users").await?;
///     HttpResponse::json(&users)
/// }
/// ```
///
/// # Notes
///
/// - State must be `Send + Sync + 'static`
/// - State should be wrapped in `Arc` for efficient cloning
/// - Multiple state types can be registered
#[derive(Debug)]
pub struct State<T: Send + Sync + 'static>(pub Arc<T>);

impl<T: Send + Sync + 'static> State<T> {
    /// Create a new State wrapper.
    #[inline]
    pub fn new(value: Arc<T>) -> Self {
        Self(value)
    }

    /// Get the inner Arc.
    #[inline]
    pub fn into_inner(self) -> Arc<T> {
        self.0
    }
}

impl<T: Send + Sync + 'static> Clone for State<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send + Sync + 'static> Deref for State<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Send + Sync + 'static> AsRef<T> for State<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T: Send + Sync + 'static> FromRequest for State<T> {
    /// Extract state from request extensions.
    ///
    /// # Errors
    ///
    /// Returns `Error::ProviderNotFound` if state of type `T` was not
    /// registered in the application.
    #[inline]
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        request.extensions.get_arc::<T>().map(State).ok_or_else(|| {
            Error::ProviderNotFound(format!(
                "State<{}> not found in request extensions. \
                     Did you forget to register it with `app.with_state()`?",
                std::any::type_name::<T>()
            ))
        })
    }
}

/// Trait for extracting named parameters from a request
pub trait FromRequestNamed: Sized {
    /// Extract a named parameter from the request
    fn from_request(request: &HttpRequest, name: &str) -> Result<Self, Error>;
}

// ========== Body Extractor ==========

/// Extracts and deserializes the request body as JSON
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Deserialize)]
/// struct CreateUser {
///     name: String,
///     email: String,
/// }
///
/// let body: Body<CreateUser> = Body::from_request(&request)?;
/// println!("Creating user: {}", body.name);
/// ```
#[derive(Debug, Clone)]
pub struct Body<T>(pub T);

impl<T> Body<T> {
    /// Create a new Body wrapper
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Body<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: DeserializeOwned> FromRequest for Body<T> {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        let value: T = request.json()?;
        Ok(Body(value))
    }
}

// ========== Query Extractor ==========

/// Extracts and deserializes query parameters
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Deserialize)]
/// struct Pagination {
///     page: Option<u32>,
///     limit: Option<u32>,
///     sort: Option<String>,
/// }
///
/// let query: Query<Pagination> = Query::from_request(&request)?;
/// let page = query.page.unwrap_or(1);
/// ```
#[derive(Debug, Clone)]
pub struct Query<T>(pub T);

impl<T> Query<T> {
    /// Create a new Query wrapper
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: DeserializeOwned> FromRequest for Query<T> {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        // Build a query string from params and deserialize
        let query_string: String = request
            .query_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let value: T = serde_urlencoded::from_str(&query_string)
            .map_err(|e| Error::Validation(format!("Invalid query parameters: {}", e)))?;

        Ok(Query(value))
    }
}

// ========== Path Extractor ==========

/// Extracts a path parameter by name
///
/// # Example
///
/// ```rust,ignore
/// // For route /users/:id
/// let id: Path<u32> = Path::from_request(&request, "id")?;
/// println!("User ID: {}", *id);
/// ```
#[derive(Debug, Clone)]
pub struct Path<T>(pub T);

impl<T> Path<T> {
    /// Create a new Path wrapper
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: std::str::FromStr> FromRequestNamed for Path<T>
where
    T::Err: std::fmt::Display,
{
    fn from_request(request: &HttpRequest, name: &str) -> Result<Self, Error> {
        let value_str = request
            .param(name)
            .ok_or_else(|| Error::Validation(format!("Missing path parameter: {}", name)))?;

        let value: T = value_str.parse().map_err(|e: T::Err| {
            Error::Validation(format!("Invalid path parameter '{}': {}", name, e))
        })?;

        Ok(Path(value))
    }
}

// ========== PathParams Extractor ==========

/// Extracts all path parameters into a struct
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Deserialize)]
/// struct UserParams {
///     user_id: u32,
///     post_id: u32,
/// }
///
/// // For route /users/:user_id/posts/:post_id
/// let params: PathParams<UserParams> = PathParams::from_request(&request)?;
/// ```
#[derive(Debug, Clone)]
pub struct PathParams<T>(pub T);

impl<T> PathParams<T> {
    /// Create a new PathParams wrapper
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for PathParams<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: DeserializeOwned> FromRequest for PathParams<T> {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        // Build a query string from path params and deserialize
        let params_string: String = request
            .path_params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        let value: T = serde_urlencoded::from_str(&params_string)
            .map_err(|e| Error::Validation(format!("Invalid path parameters: {}", e)))?;

        Ok(PathParams(value))
    }
}

// ========== Header Extractor ==========

/// Extracts a header value by name
///
/// # Example
///
/// ```rust,ignore
/// let auth: Header = Header::from_request(&request, "Authorization")?;
/// println!("Auth: {}", auth.value());
///
/// // Or as optional
/// let custom: Option<Header> = Header::optional(&request, "X-Custom-Header");
/// ```
#[derive(Debug, Clone)]
pub struct Header {
    name: String,
    value: String,
}

impl Header {
    /// Create a new Header
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Get the header name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the header value
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Get the header value, consuming self
    pub fn into_value(self) -> String {
        self.value
    }

    /// Extract a header, returning None if not present
    pub fn optional(request: &HttpRequest, name: &str) -> Option<Self> {
        request
            .headers
            .get(name)
            .or_else(|| request.headers.get(&name.to_lowercase()))
            .map(|v| Header::new(name, v.clone()))
    }
}

impl FromRequestNamed for Header {
    fn from_request(request: &HttpRequest, name: &str) -> Result<Self, Error> {
        let value = request
            .headers
            .get(name)
            .or_else(|| request.headers.get(&name.to_lowercase()))
            .ok_or_else(|| Error::Validation(format!("Missing header: {}", name)))?;

        Ok(Header::new(name, value.clone()))
    }
}

impl Deref for Header {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

// ========== Headers Extractor ==========

/// Extracts all headers as a map
#[derive(Debug, Clone)]
pub struct Headers(pub std::collections::HashMap<String, String>);

impl Headers {
    /// Get a header value by name
    pub fn get(&self, name: &str) -> Option<&String> {
        self.0
            .get(name)
            .or_else(|| self.0.get(&name.to_lowercase()))
    }

    /// Check if a header exists
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Iterate over all headers
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }
}

impl FromRequest for Headers {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        // Public API stays a HashMap; convert from the internal HeaderMap.
        Ok(Headers(request.headers.clone().into()))
    }
}

impl Deref for Headers {
    type Target = std::collections::HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ========== RawBody Extractor ==========

/// Extracts the raw request body as bytes
///
/// # Example
///
/// ```rust,ignore
/// let raw: RawBody = RawBody::from_request(&request)?;
/// println!("Body length: {} bytes", raw.len());
/// ```
#[derive(Debug, Clone)]
pub struct RawBody(pub Vec<u8>);

impl RawBody {
    /// Create a new RawBody
    pub fn new(data: Vec<u8>) -> Self {
        Self(data)
    }

    /// Get the body length
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the body is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Convert to a UTF-8 string
    pub fn to_string_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0).to_string()
    }

    /// Try to convert to a UTF-8 string
    pub fn to_string(&self) -> Result<String, std::string::FromUtf8Error> {
        String::from_utf8(self.0.clone())
    }

    /// Get the inner bytes
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl FromRequest for RawBody {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        Ok(RawBody(request.body.clone()))
    }
}

impl Deref for RawBody {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ========== Form Extractor ==========

/// Extracts and deserializes form data (application/x-www-form-urlencoded)
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Deserialize)]
/// struct LoginForm {
///     username: String,
///     password: String,
/// }
///
/// let form: Form<LoginForm> = Form::from_request(&request)?;
/// ```
#[derive(Debug, Clone)]
pub struct Form<T>(pub T);

impl<T> Form<T> {
    /// Create a new Form wrapper
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Form<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: DeserializeOwned> FromRequest for Form<T> {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        let value: T = request.form()?;
        Ok(Form(value))
    }
}

// ========== ContentType Extractor ==========

/// Extracts the Content-Type header
#[derive(Debug, Clone)]
pub struct ContentType(pub String);

impl ContentType {
    /// Check if the content type is JSON
    pub fn is_json(&self) -> bool {
        self.0.contains("application/json")
    }

    /// Check if the content type is form data
    pub fn is_form(&self) -> bool {
        self.0.contains("application/x-www-form-urlencoded")
    }

    /// Check if the content type is multipart
    pub fn is_multipart(&self) -> bool {
        self.0.contains("multipart/form-data")
    }

    /// Get the inner value
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl FromRequest for ContentType {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        let value = request
            .headers
            .get("Content-Type")
            .or_else(|| request.headers.get("content-type"))
            .cloned()
            .unwrap_or_default();

        Ok(ContentType(value))
    }
}

impl Deref for ContentType {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ========== Method Extractor ==========

/// Extracts the HTTP method
#[derive(Debug, Clone)]
pub struct Method(pub String);

impl Method {
    /// Check if the method is GET
    pub fn is_get(&self) -> bool {
        self.0 == "GET"
    }

    /// Check if the method is POST
    pub fn is_post(&self) -> bool {
        self.0 == "POST"
    }

    /// Check if the method is PUT
    pub fn is_put(&self) -> bool {
        self.0 == "PUT"
    }

    /// Check if the method is DELETE
    pub fn is_delete(&self) -> bool {
        self.0 == "DELETE"
    }

    /// Check if the method is PATCH
    pub fn is_patch(&self) -> bool {
        self.0 == "PATCH"
    }
}

impl FromRequest for Method {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        Ok(Method(request.method.clone()))
    }
}

impl Deref for Method {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ========== Extension: FromRequest for primitives ==========

impl FromRequest for HttpRequest {
    fn from_request(request: &HttpRequest) -> Result<Self, Error> {
        Ok(request.clone())
    }
}

// ========== Helper Macros ==========

/// Extract body from request as the specified type
///
/// # Example
///
/// ```rust,ignore
/// let user: CreateUser = body!(request, CreateUser)?;
/// // or with type inference if annotated
/// let user = body!(request, CreateUser)?;
/// ```
#[macro_export]
macro_rules! body {
    ($request:expr, $type:ty) => {
        <$crate::extractors::Body<$type> as $crate::extractors::FromRequest>::from_request(
            &$request,
        )
        .map(|b| b.into_inner())
    };
}

/// Extract query parameters from request as the specified type
///
/// # Example
///
/// ```rust,ignore
/// let filters = query!(request, UserFilters)?;
/// ```
#[macro_export]
macro_rules! query {
    ($request:expr, $type:ty) => {
        <$crate::extractors::Query<$type> as $crate::extractors::FromRequest>::from_request(
            &$request,
        )
        .map(|q| q.into_inner())
    };
}

/// Extract path parameter from request
///
/// # Example
///
/// ```rust,ignore
/// let id: u32 = path!(request, "id", u32)?;
/// ```
#[macro_export]
macro_rules! path {
    ($request:expr, $name:expr, $type:ty) => {
        <$crate::extractors::Path<$type> as $crate::extractors::FromRequestNamed>::from_request(
            &$request, $name,
        )
        .map(|p| p.into_inner())
    };
}

/// Extract header from request
///
/// # Example
///
/// ```rust,ignore
/// let auth: String = header!(request, "Authorization")?;
/// ```
#[macro_export]
macro_rules! header {
    ($request:expr, $name:expr) => {
        <$crate::extractors::Header as $crate::extractors::FromRequestNamed>::from_request(
            &$request, $name,
        )
        .map(|h| h.into_value())
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn create_request() -> HttpRequest {
        let mut req = HttpRequest::new("GET".to_string(), "/users/123".to_string());
        req.path_params.insert("id".to_string(), "123".to_string());
        req.query_params.insert("page".to_string(), "1".to_string());
        req.query_params
            .insert("limit".to_string(), "10".to_string());
        req.headers
            .insert("Authorization".to_string(), "Bearer token123".to_string());
        req.headers
            .insert("Content-Type".to_string(), "application/json".to_string());
        req
    }

    #[test]
    fn test_path_extraction() {
        let request = create_request();
        let id: Path<u32> = Path::from_request(&request, "id").unwrap();
        assert_eq!(*id, 123);
    }

    #[test]
    fn test_path_missing() {
        let request = create_request();
        let result: Result<Path<u32>, _> = Path::from_request(&request, "missing");
        assert!(result.is_err());
    }

    #[test]
    fn test_header_extraction() {
        let request = create_request();
        let auth: Header = Header::from_request(&request, "Authorization").unwrap();
        assert_eq!(auth.value(), "Bearer token123");
    }

    #[test]
    fn test_header_optional() {
        let request = create_request();

        let auth = Header::optional(&request, "Authorization");
        assert!(auth.is_some());

        let missing = Header::optional(&request, "X-Missing");
        assert!(missing.is_none());
    }

    #[test]
    fn test_headers_extraction() {
        let request = create_request();
        let headers: Headers = Headers::from_request(&request).unwrap();

        assert!(headers.contains("Authorization"));
        assert!(headers.contains("Content-Type"));
        assert!(!headers.contains("X-Missing"));
    }

    #[test]
    fn test_query_extraction() {
        let request = create_request();

        #[derive(Debug, Deserialize, PartialEq)]
        struct Pagination {
            page: u32,
            limit: u32,
        }

        let query: Query<Pagination> = Query::from_request(&request).unwrap();
        assert_eq!(query.page, 1);
        assert_eq!(query.limit, 10);
    }

    #[test]
    fn test_body_extraction() {
        let mut request = create_request();
        request.body = serde_json::to_vec(&serde_json::json!({
            "name": "Test",
            "email": "test@example.com"
        }))
        .unwrap();

        #[derive(Debug, Deserialize)]
        struct CreateUser {
            name: String,
            email: String,
        }

        let body: Body<CreateUser> = Body::from_request(&request).unwrap();
        assert_eq!(body.name, "Test");
        assert_eq!(body.email, "test@example.com");
    }

    #[test]
    fn test_raw_body() {
        let mut request = create_request();
        request.body = b"raw content".to_vec();

        let raw: RawBody = RawBody::from_request(&request).unwrap();
        assert_eq!(raw.len(), 11);
        assert_eq!(raw.to_string_lossy(), "raw content");
    }

    #[test]
    fn test_content_type() {
        let request = create_request();
        let ct: ContentType = ContentType::from_request(&request).unwrap();

        assert!(ct.is_json());
        assert!(!ct.is_form());
        assert!(!ct.is_multipart());
    }

    #[test]
    fn test_method() {
        let request = create_request();
        let method: Method = Method::from_request(&request).unwrap();

        assert!(method.is_get());
        assert!(!method.is_post());
    }

    #[test]
    fn test_request_extraction() {
        let request = create_request();
        let extracted: HttpRequest = HttpRequest::from_request(&request).unwrap();

        assert_eq!(extracted.method, request.method);
        assert_eq!(extracted.path, request.path);
    }
}
