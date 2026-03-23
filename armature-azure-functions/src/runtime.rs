//! Azure Functions runtime for Armature applications.

use std::sync::Arc;
use tracing::{debug, error, info};

use crate::{FunctionConfig, FunctionRequest, FunctionResponse};

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Function configuration.
    pub function: FunctionConfig,
    /// Enable request logging.
    pub log_requests: bool,
    /// Enable response logging.
    pub log_responses: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            function: FunctionConfig::from_env(),
            log_requests: true,
            log_responses: false,
        }
    }
}

impl RuntimeConfig {
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

    /// Set function configuration.
    pub fn function_config(mut self, config: FunctionConfig) -> Self {
        self.function = config;
        self
    }
}

/// Azure Functions runtime for Armature applications.
///
/// This wraps an Armature application and runs it on Azure Functions,
/// translating HTTP trigger requests to Armature requests.
pub struct AzureFunctionsRuntime<App> {
    app: Arc<App>,
    config: RuntimeConfig,
}

impl<App> AzureFunctionsRuntime<App>
where
    App: Send + Sync + 'static,
{
    /// Create a new Azure Functions runtime.
    pub fn new(app: App) -> Self {
        Self {
            app: Arc::new(app),
            config: RuntimeConfig::default(),
        }
    }

    /// Set the runtime configuration.
    pub fn with_config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }

    /// Handle a function request.
    ///
    /// This is called for each HTTP trigger invocation.
    pub async fn handle(&self, request: FunctionRequest) -> FunctionResponse
    where
        App: RequestHandler,
    {
        let mut request = request;

        // Strip base path if configured
        if let Some(base_path) = &self.config.function.base_path {
            if request.path.starts_with(base_path) {
                request.path = request
                    .path
                    .strip_prefix(base_path)
                    .unwrap_or(&request.path)
                    .to_string();
                if request.path.is_empty() {
                    request.path = "/".to_string();
                }
            }
        }

        // Log request if enabled
        if self.config.log_requests {
            debug!(
                method = %request.method,
                path = %request.path,
                invocation_id = ?request.context.invocation_id,
                "Handling Azure Function request"
            );
        }

        // Handle request
        let response = self.app.handle(request).await;

        // Log response if enabled
        if self.config.log_responses {
            debug!(status = response.status_code, "Azure Function response");
        }

        response
    }

    /// Run the Azure Functions runtime.
    ///
    /// This starts the custom handler HTTP server that Azure Functions
    /// uses to communicate with the worker.
    pub async fn run(self) -> Result<(), crate::AzureFunctionsError>
    where
        App: RequestHandler,
    {
        info!("Starting Armature Azure Functions runtime");

        // Get port from environment (Azure Functions sets this)
        let port: u16 = std::env::var("FUNCTIONS_CUSTOMHANDLER_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(7071);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

        info!("Listening on {}", addr);

        // Create HTTP server for custom handler
        use hyper::server::conn::http1;
        use hyper::service::service_fn;
        use hyper_util::rt::TokioIo;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| crate::AzureFunctionsError::Runtime(e.to_string()))?;

        let app = self.app.clone();
        let config = self.config.clone();

        loop {
            let (stream, _) = listener
                .accept()
                .await
                .map_err(|e| crate::AzureFunctionsError::Runtime(e.to_string()))?;

            let io = TokioIo::new(stream);
            let app = app.clone();
            let config = config.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let app = app.clone();
                    let config = config.clone();
                    async move { handle_http_request(app, config, req).await }
                });

                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    error!("Connection error: {}", e);
                }
            });
        }
    }
}

/// Handle an HTTP request from the custom handler.
async fn handle_http_request<App: RequestHandler + 'static>(
    app: Arc<App>,
    config: RuntimeConfig,
    req: hyper::Request<hyper::body::Incoming>,
) -> Result<hyper::Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible> {
    use http_body_util::BodyExt;

    // Convert hyper request to FunctionRequest
    let (parts, body) = req.into_parts();

    let body_bytes = body
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();

    let mut headers = std::collections::HashMap::new();
    for (name, value) in parts.headers.iter() {
        if let Ok(v) = value.to_str() {
            headers.insert(name.to_string(), v.to_string());
        }
    }

    let mut query = std::collections::HashMap::new();
    if let Some(q) = parts.uri.query() {
        for pair in q.split('&') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                query.insert(
                    urlencoding::decode(key).unwrap_or_default().to_string(),
                    urlencoding::decode(value).unwrap_or_default().to_string(),
                );
            }
        }
    }

    let function_request = FunctionRequest {
        method: parts.method.to_string(),
        url: parts.uri.to_string(),
        path: parts.uri.path().to_string(),
        query,
        headers,
        body: body_bytes,
        params: std::collections::HashMap::new(),
        context: crate::request::RequestContext {
            invocation_id: std::env::var("AZURE_FUNCTIONS_INVOCATION_ID").ok(),
            function_name: std::env::var("AZURE_FUNCTIONS_FUNCTION_NAME").ok(),
            ..Default::default()
        },
    };

    // Handle request
    let runtime = AzureFunctionsRuntime { app, config };
    let response = runtime.handle(function_request).await;

    // Convert FunctionResponse to hyper response
    let mut builder = hyper::Response::builder().status(response.status_code);

    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }

    let body = if response.is_base64_encoded {
        use base64::Engine;
        bytes::Bytes::from(
            base64::engine::general_purpose::STANDARD
                .decode(&response.body)
                .unwrap_or_default(),
        )
    } else {
        bytes::Bytes::from(response.body)
    };

    Ok(builder
        .body(http_body_util::Full::new(body))
        .unwrap_or_else(|_| {
            hyper::Response::builder()
                .status(500)
                .body(http_body_util::Full::new(bytes::Bytes::from(
                    "Internal Server Error",
                )))
                .unwrap()
        }))
}

/// Request handler trait for Armature applications.
#[async_trait::async_trait]
pub trait RequestHandler: Send + Sync {
    /// Handle an HTTP request.
    async fn handle(&self, request: FunctionRequest) -> FunctionResponse;
}

/// Example implementation for a simple handler function.
#[async_trait::async_trait]
impl<F, Fut> RequestHandler for F
where
    F: Fn(FunctionRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = FunctionResponse> + Send,
{
    async fn handle(&self, request: FunctionRequest) -> FunctionResponse {
        self(request).await
    }
}

/// Macro to implement RequestHandler for Armature applications.
#[macro_export]
macro_rules! impl_request_handler {
    ($app_type:ty) => {
        #[async_trait::async_trait]
        impl $crate::runtime::RequestHandler for $app_type {
            async fn handle(&self, request: $crate::FunctionRequest) -> $crate::FunctionResponse {
                match self
                    .handle_request(request.http_method(), &request.path, request.body)
                    .await
                {
                    Ok(response) => {
                        let mut func_response =
                            $crate::FunctionResponse::with_body(response.status, response.body);
                        for (name, value) in response.headers {
                            func_response = func_response.header(name, value);
                        }
                        func_response
                    }
                    Err(e) => $crate::FunctionResponse::internal_error(e.to_string()),
                }
            }
        }
    };
}
