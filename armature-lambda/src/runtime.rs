//! Lambda runtime for Armature applications.

use lambda_http::{Body, Error, Request, Response, run, service_fn};
use std::sync::Arc;
use tracing::{debug, info};

use crate::{LambdaRequest, LambdaResponse};

/// Lambda runtime configuration.
#[derive(Debug, Clone)]
pub struct LambdaConfig {
    /// Enable request logging.
    pub log_requests: bool,
    /// Enable response logging.
    pub log_responses: bool,
    /// Custom base path to strip (e.g., "/prod", "/dev").
    pub base_path: Option<String>,
}

impl Default for LambdaConfig {
    fn default() -> Self {
        Self {
            log_requests: true,
            log_responses: false,
            base_path: None,
        }
    }
}

impl LambdaConfig {
    /// Enable request logging.
    pub fn log_requests(mut self, enabled: bool) -> Self {
        self.log_requests = enabled;
        self
    }

    /// Enable response logging.
    pub fn log_responses(mut self, enabled: bool) -> Self {
        self.log_responses = enabled;
        self
    }

    /// Set a base path to strip from requests.
    pub fn base_path(mut self, path: impl Into<String>) -> Self {
        self.base_path = Some(path.into());
        self
    }
}

/// Lambda runtime for Armature applications.
///
/// This wraps an Armature application and runs it on the Lambda runtime,
/// translating API Gateway/ALB requests to Armature requests.
pub struct LambdaRuntime<App> {
    app: Arc<App>,
    config: LambdaConfig,
}

impl<App> LambdaRuntime<App>
where
    App: Send + Sync + 'static,
{
    /// Create a new Lambda runtime.
    pub fn new(app: App) -> Self {
        Self {
            app: Arc::new(app),
            config: LambdaConfig::default(),
        }
    }

    /// Set the runtime configuration.
    pub fn with_config(mut self, config: LambdaConfig) -> Self {
        self.config = config;
        self
    }

    /// Run the Lambda runtime.
    ///
    /// This function never returns under normal operation.
    pub async fn run(self) -> Result<(), Error>
    where
        App: RequestHandler,
    {
        info!("Starting Armature Lambda runtime");

        let app = self.app.clone();
        let config = self.config.clone();

        run(service_fn(move |request: Request| {
            let app = app.clone();
            let config = config.clone();
            async move { handle_request(app, config, request).await }
        }))
        .await
    }
}

/// Request handler trait for Armature applications.
///
/// This is automatically implemented for Armature Application types.
#[async_trait::async_trait]
pub trait RequestHandler: Send + Sync {
    /// Handle an HTTP request.
    async fn handle(&self, request: LambdaRequest) -> LambdaResponse;
}

/// Handle a Lambda request.
async fn handle_request<App: RequestHandler>(
    app: Arc<App>,
    config: LambdaConfig,
    request: Request,
) -> Result<Response<Body>, Error> {
    // Convert Lambda request
    let mut lambda_request = LambdaRequest::from_lambda_request(request);

    // Strip base path if configured
    if let Some(base_path) = &config.base_path {
        lambda_request.path = strip_base_path(&lambda_request.path, base_path);
    }

    // Log request if enabled
    if config.log_requests {
        debug!(
            method = %lambda_request.method,
            path = %lambda_request.path,
            request_id = ?lambda_request.request_context.request_id,
            "Handling Lambda request"
        );
    }

    // Handle request
    let response = app.handle(lambda_request).await;

    // Log response if enabled
    if config.log_responses {
        debug!(status = response.status, "Lambda response");
    }

    Ok(response.into_lambda_response())
}

/// Strip a configured base path prefix (e.g. an API Gateway stage like
/// `/prod`) from a request path. When stripping empties the path it is
/// normalized back to `/`. Paths that do not start with `base_path` are
/// returned unchanged.
pub(crate) fn strip_base_path(path: &str, base_path: &str) -> String {
    match path.strip_prefix(base_path) {
        Some("") => "/".to_string(),
        Some(stripped) => stripped.to_string(),
        None => path.to_string(),
    }
}

/// Macro to implement RequestHandler for Armature applications.
///
/// Usage:
/// ```rust,ignore
/// use armature_lambda::impl_request_handler;
///
/// impl_request_handler!(MyApplication);
/// ```
#[macro_export]
macro_rules! impl_request_handler {
    ($app_type:ty) => {
        #[async_trait::async_trait]
        impl $crate::runtime::RequestHandler for $app_type {
            async fn handle(&self, request: $crate::LambdaRequest) -> $crate::LambdaResponse {
                // Forward the full request so the application handler has
                // access to headers, query string, path parameters, stage
                // variables, and the request context (including authorizer
                // claims) — not just the method/path/body.
                match self.handle_request(request).await {
                    Ok(response) => {
                        let mut lambda_response =
                            $crate::LambdaResponse::new(response.status, response.body);
                        for (name, value) in response.headers {
                            lambda_response = lambda_response.header(name, value);
                        }
                        lambda_response
                    }
                    Err(e) => $crate::LambdaResponse::internal_error(e.to_string()),
                }
            }
        }
    };
}

/// Example implementation for a simple handler function.
#[async_trait::async_trait]
impl<F, Fut> RequestHandler for F
where
    F: Fn(LambdaRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = LambdaResponse> + Send,
{
    async fn handle(&self, request: LambdaRequest) -> LambdaResponse {
        self(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LambdaRequest;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[test]
    fn strip_base_path_removes_stage_prefix() {
        assert_eq!(strip_base_path("/prod/users", "/prod"), "/users");
    }

    #[test]
    fn strip_base_path_normalizes_empty_to_root() {
        assert_eq!(strip_base_path("/prod", "/prod"), "/");
    }

    #[test]
    fn strip_base_path_leaves_non_matching_paths() {
        assert_eq!(strip_base_path("/other/users", "/prod"), "/other/users");
    }

    // A minimal response/error/app trio mirroring the shape the
    // `impl_request_handler!` macro expects from an Armature application.
    struct MockResponse {
        status: u16,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    }

    #[derive(Debug)]
    struct MockError(String);

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    #[derive(Default)]
    struct Captured {
        method: Option<String>,
        path: Option<String>,
        headers: HashMap<String, String>,
        query_string: Option<String>,
        path_parameters: HashMap<String, String>,
        claims: HashMap<String, String>,
        body: Vec<u8>,
    }

    struct MockApp {
        captured: Mutex<Captured>,
    }

    impl MockApp {
        async fn handle_request(
            &self,
            request: LambdaRequest,
        ) -> std::result::Result<MockResponse, MockError> {
            let mut captured = self.captured.lock().unwrap();
            captured.method = Some(request.method.to_string());
            captured.path = Some(request.path.clone());
            captured.headers = request.headers.clone();
            captured.query_string = request.query_string.clone();
            captured.path_parameters = request.path_parameters.clone();
            captured.claims = request.request_context.authorizer_claims.clone();
            captured.body = request.body.to_vec();
            Ok(MockResponse {
                status: 201,
                body: b"ok".to_vec(),
                headers: vec![("x-app".to_string(), "yes".to_string())],
            })
        }
    }

    impl_request_handler!(MockApp);

    fn sample_request() -> LambdaRequest {
        let mut headers = HashMap::new();
        headers.insert("x-custom".to_string(), "value".to_string());
        let mut path_parameters = HashMap::new();
        path_parameters.insert("id".to_string(), "42".to_string());
        let mut claims = HashMap::new();
        claims.insert("sub".to_string(), "user-1".to_string());

        LambdaRequest {
            method: http::Method::POST,
            path: "/users/42".to_string(),
            query_string: Some("page=2".to_string()),
            headers,
            body: bytes::Bytes::from_static(b"payload"),
            path_parameters,
            stage_variables: HashMap::new(),
            request_context: crate::request::RequestContext {
                authorizer_claims: claims,
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn macro_forwards_full_request_to_app() {
        let app = MockApp {
            captured: Mutex::new(Captured::default()),
        };

        let response = RequestHandler::handle(&app, sample_request()).await;

        // Response mapping is preserved.
        assert_eq!(response.status, 201);
        assert_eq!(&response.body[..], b"ok");
        assert_eq!(
            response.headers.get("x-app").map(String::as_str),
            Some("yes")
        );

        // The app received every part of the request, not just method/path/body.
        let captured = app.captured.lock().unwrap();
        assert_eq!(captured.method.as_deref(), Some("POST"));
        assert_eq!(captured.path.as_deref(), Some("/users/42"));
        assert_eq!(captured.query_string.as_deref(), Some("page=2"));
        assert_eq!(
            captured.headers.get("x-custom").map(String::as_str),
            Some("value")
        );
        assert_eq!(
            captured.path_parameters.get("id").map(String::as_str),
            Some("42")
        );
        assert_eq!(
            captured.claims.get("sub").map(String::as_str),
            Some("user-1")
        );
        assert_eq!(captured.body, b"payload".to_vec());
    }
}
