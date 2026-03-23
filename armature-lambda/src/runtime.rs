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
        if lambda_request.path.starts_with(base_path) {
            lambda_request.path = lambda_request
                .path
                .strip_prefix(base_path)
                .unwrap_or(&lambda_request.path)
                .to_string();
            if lambda_request.path.is_empty() {
                lambda_request.path = "/".to_string();
            }
        }
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
                // Convert to Armature HttpRequest and handle
                // This is a simplified implementation - full version would
                // properly convert all request data
                match self
                    .handle_request(request.method, &request.path, request.body)
                    .await
                {
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
