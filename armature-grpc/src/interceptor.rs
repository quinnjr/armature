//! gRPC interceptors for request/response processing.

use async_trait::async_trait;
use std::sync::Arc;
use tonic::{Request, Status};
use tracing::debug;

/// Type alias for custom auth validator function.
pub type AuthValidatorFn =
    Arc<dyn Fn(&tonic::metadata::MetadataMap) -> Result<(), Status> + Send + Sync>;

/// Type alias for metrics callback function.
pub type MetricsCallbackFn = Arc<dyn Fn(&str, u64, bool) + Send + Sync>;

/// Interceptor trait for gRPC requests.
///
/// **This trait is a standalone helper, NOT an automatically-applied
/// pipeline.** Implementing it does not wire the interceptor into any request
/// path — the crate's server/client code never calls
/// [`Interceptor::intercept`]. To use an implementation you must invoke
/// `.intercept(request).await` yourself at the point you want it to run
/// (typically inside your own service method or client wrapper). The
/// [`LoggingInterceptor`], [`MetricsInterceptor`] and [`RequestIdInterceptor`]
/// impls below are inert until called this way.
///
/// The one interceptor that IS wired into a request path is
/// [`AuthInterceptor::server_interceptor`], which returns a
/// `tonic::service::Interceptor` closure you can hand to
/// `InterceptedService`/`with_interceptor`; that is a separate mechanism from
/// this trait.
#[async_trait]
pub trait Interceptor: Send + Sync {
    /// Intercept an incoming request. Call this manually — see the trait-level
    /// note: nothing invokes it for you.
    async fn intercept<T>(&self, request: Request<T>) -> Result<Request<T>, Status>
    where
        T: Send;
}

/// Request-only interceptor.
#[async_trait]
pub trait RequestInterceptor: Send + Sync {
    /// Intercept and modify the request.
    async fn intercept<T>(&self, request: Request<T>) -> Result<Request<T>, Status>
    where
        T: Send;
}

/// Response interceptor (for streaming responses).
#[async_trait]
pub trait ResponseInterceptor: Send + Sync {
    /// Called when a response is ready.
    async fn on_response<T>(&self, response: &T);
}

/// Logging interceptor that logs requests.
///
/// Standalone helper: it does nothing until you call
/// [`Interceptor::intercept`] on it yourself. It is not part of any
/// automatically-applied request pipeline (see the [`Interceptor`] trait note).
pub struct LoggingInterceptor {
    log_metadata: bool,
}

impl LoggingInterceptor {
    /// Create a new logging interceptor.
    pub fn new() -> Self {
        Self {
            log_metadata: false,
        }
    }

    /// Enable logging of metadata.
    pub fn with_metadata(mut self) -> Self {
        self.log_metadata = true;
        self
    }
}

impl Default for LoggingInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Interceptor for LoggingInterceptor {
    async fn intercept<T>(&self, request: Request<T>) -> Result<Request<T>, Status>
    where
        T: Send,
    {
        debug!("gRPC request received");

        if self.log_metadata {
            for key_value in request.metadata().iter() {
                match key_value {
                    tonic::metadata::KeyAndValueRef::Ascii(key, value) => {
                        debug!(
                            key = %key,
                            value = ?value,
                            "Request metadata (ASCII)"
                        );
                    }
                    tonic::metadata::KeyAndValueRef::Binary(key, value) => {
                        debug!(
                            key = %key,
                            value_len = value.as_ref().len(),
                            "Request metadata (Binary)"
                        );
                    }
                }
            }
        }

        Ok(request)
    }
}

/// Compare two byte strings for equality in constant time (with respect to
/// their content — not their length), to avoid leaking how many leading
/// bytes of a submitted credential matched the expected value via a timing
/// side-channel. No `subtle`-equivalent crate is used elsewhere in this
/// workspace, so this is a small manual implementation rather than a new
/// dependency.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Authentication interceptor.
///
/// This type is deliberately direction-agnostic: it can attach credentials to
/// outgoing (client-side) requests via [`AuthInterceptor::add_auth`], or
/// validate incoming (server-side) requests via [`AuthInterceptor::validate`]
/// / [`AuthInterceptor::server_interceptor`].
/// The blanket [`Interceptor`] impl below is the **client-side** behavior only —
/// it attaches credentials and never rejects a request. Do not use it as a
/// `tonic::service::Interceptor` on the server; use [`AuthInterceptor::server_interceptor`]
/// for that, which actually validates and rejects unauthenticated requests.
#[derive(Clone)]
pub struct AuthInterceptor {
    auth_type: AuthType,
}

#[derive(Clone)]
enum AuthType {
    Bearer(String),
    ApiKey { header: String, key: String },
    Custom(AuthValidatorFn),
}

impl AuthInterceptor {
    /// Create a bearer token interceptor.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            auth_type: AuthType::Bearer(token.into()),
        }
    }

    /// Create an API key interceptor.
    pub fn api_key(header: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            auth_type: AuthType::ApiKey {
                header: header.into(),
                key: key.into(),
            },
        }
    }

    /// Create a custom auth interceptor.
    pub fn custom<F>(validator: F) -> Self
    where
        F: Fn(&tonic::metadata::MetadataMap) -> Result<(), Status> + Send + Sync + 'static,
    {
        Self {
            auth_type: AuthType::Custom(Arc::new(validator)),
        }
    }

    /// Validate request authentication (for server-side use).
    pub fn validate(&self, metadata: &tonic::metadata::MetadataMap) -> Result<(), Status> {
        match &self.auth_type {
            AuthType::Bearer(expected) => {
                let token = metadata
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .ok_or_else(|| Status::unauthenticated("Missing bearer token"))?;

                if !constant_time_eq(token.as_bytes(), expected.as_bytes()) {
                    return Err(Status::unauthenticated("Invalid token"));
                }
                Ok(())
            }
            AuthType::ApiKey { header, key } => {
                let provided = metadata
                    .get(header.as_str())
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| Status::unauthenticated("Missing API key"))?;

                if !constant_time_eq(provided.as_bytes(), key.as_bytes()) {
                    return Err(Status::unauthenticated("Invalid API key"));
                }
                Ok(())
            }
            AuthType::Custom(validator) => validator(metadata),
        }
    }

    /// Add authentication to a request (for client-side use).
    pub fn add_auth<T>(&self, mut request: Request<T>) -> Request<T> {
        match &self.auth_type {
            AuthType::Bearer(token) => {
                if let Ok(value) = format!("Bearer {}", token).parse() {
                    request.metadata_mut().insert("authorization", value);
                }
            }
            AuthType::ApiKey { header, key } => {
                if let (Ok(header_key), Ok(value)) = (
                    tonic::metadata::MetadataKey::from_bytes(header.as_bytes()),
                    key.parse(),
                ) {
                    request.metadata_mut().insert(header_key, value);
                }
            }
            AuthType::Custom(_) => {
                // Custom validators are for server-side; client should use bearer or api_key
            }
        }
        request
    }

    /// Build a `tonic::service::Interceptor` suitable for **server-side** use
    /// (e.g. via `InterceptedService::new` or a generated server's
    /// `with_interceptor` constructor). Unlike the client-side [`Interceptor`]
    /// impl on this type, this closure actually validates the request's
    /// metadata and rejects it with `Status::unauthenticated` on failure.
    pub fn server_interceptor(
        self,
    ) -> impl FnMut(Request<()>) -> Result<Request<()>, Status> + Clone {
        move |request: Request<()>| -> Result<Request<()>, Status> {
            self.validate(request.metadata())?;
            Ok(request)
        }
    }
}

/// **Client-side** behavior: attaches credentials to the outgoing request.
/// This impl never rejects a request — it is not authentication, it is
/// credential attachment. For server-side authentication/rejection, use
/// [`AuthInterceptor::server_interceptor`] instead.
#[async_trait]
impl Interceptor for AuthInterceptor {
    async fn intercept<T>(&self, request: Request<T>) -> Result<Request<T>, Status>
    where
        T: Send,
    {
        Ok(self.add_auth(request))
    }
}

/// Metrics interceptor that records request timing.
///
/// Standalone helper: it does nothing until you call
/// [`Interceptor::intercept`] (or [`MetricsInterceptor::record`]) on it
/// yourself. It is not part of any automatically-applied request pipeline (see
/// the [`Interceptor`] trait note).
pub struct MetricsInterceptor {
    on_complete: MetricsCallbackFn,
}

impl MetricsInterceptor {
    /// Create a new metrics interceptor with a callback.
    /// Callback receives: (method_name, duration_ms, success)
    pub fn new<F>(on_complete: F) -> Self
    where
        F: Fn(&str, u64, bool) + Send + Sync + 'static,
    {
        Self {
            on_complete: Arc::new(on_complete),
        }
    }

    /// Record a completed request.
    pub fn record(&self, method: &str, duration_ms: u64, success: bool) {
        (self.on_complete)(method, duration_ms, success);
    }
}

#[async_trait]
impl Interceptor for MetricsInterceptor {
    async fn intercept<T>(&self, request: Request<T>) -> Result<Request<T>, Status>
    where
        T: Send,
    {
        // Note: Actual timing should be done in the service implementation
        // This interceptor just passes through and can be used to inject timing context
        Ok(request)
    }
}

/// Request ID interceptor that adds/propagates request IDs.
///
/// Standalone helper: it does nothing until you call
/// [`Interceptor::intercept`] on it yourself. It is not part of any
/// automatically-applied request pipeline (see the [`Interceptor`] trait note).
pub struct RequestIdInterceptor {
    header_name: String,
}

impl RequestIdInterceptor {
    /// Create a new request ID interceptor.
    pub fn new() -> Self {
        Self {
            header_name: "x-request-id".to_string(),
        }
    }

    /// Create with a custom header name.
    pub fn with_header(header: impl Into<String>) -> Self {
        Self {
            header_name: header.into(),
        }
    }

    /// Extract request ID from metadata.
    pub fn extract_request_id(&self, metadata: &tonic::metadata::MetadataMap) -> Option<String> {
        metadata
            .get(self.header_name.as_str())
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    /// Generate a new request ID.
    pub fn generate_request_id() -> String {
        format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    }
}

impl Default for RequestIdInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Interceptor for RequestIdInterceptor {
    async fn intercept<T>(&self, mut request: Request<T>) -> Result<Request<T>, Status>
    where
        T: Send,
    {
        // Check if request already has an ID
        let request_id = self
            .extract_request_id(request.metadata())
            .unwrap_or_else(Self::generate_request_id);

        // Ensure the ID is in the metadata
        if let (Ok(key), Ok(value)) = (
            tonic::metadata::MetadataKey::from_bytes(self.header_name.as_bytes()),
            request_id.parse(),
        ) {
            request.metadata_mut().insert(key, value);
        }

        Ok(request)
    }
}
