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
        if let Some(base_path) = &self.config.function.base_path
            && request.path.starts_with(base_path)
        {
            request.path = request
                .path
                .strip_prefix(base_path)
                .unwrap_or(&request.path)
                .to_string();
            if request.path.is_empty() {
                request.path = "/".to_string();
            }
        }

        // Log request if enabled. Uses `info!` so the line is actually emitted
        // under `init_tracing()`'s default `info` EnvFilter.
        if self.config.log_requests {
            info!(
                method = %request.method,
                path = %request.path,
                invocation_id = ?request.context.invocation_id,
                "Handling Azure Function request"
            );
        }

        // Handle request, applying the configured request timeout. A value of 0
        // disables the timeout.
        let timeout_seconds = self.config.function.timeout_seconds;
        let response = if timeout_seconds == 0 {
            self.app.handle(request).await
        } else {
            let path = request.path.clone();
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_seconds),
                self.app.handle(request),
            )
            .await
            {
                Ok(response) => response,
                Err(_elapsed) => {
                    error!(
                        path = %path,
                        timeout_seconds,
                        "Azure Function request timed out"
                    );
                    FunctionResponse::error(504, "Gateway Timeout")
                }
            }
        };

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
async fn handle_http_request<App, B>(
    app: Arc<App>,
    config: RuntimeConfig,
    req: hyper::Request<B>,
) -> Result<hyper::Response<http_body_util::Full<bytes::Bytes>>, std::convert::Infallible>
where
    App: RequestHandler + 'static,
    B: hyper::body::Body<Data = bytes::Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    use http_body_util::{BodyExt, LengthLimitError, Limited};

    // Convert hyper request to FunctionRequest
    let (parts, body) = req.into_parts();

    let max_request_size = config.function.max_request_size;

    // Early rejection: if the client declared a Content-Length that already
    // exceeds the cap, refuse before buffering the body.
    if let Some(len) = parts
        .headers
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        && len > max_request_size
    {
        return Ok(oversize_response(len, max_request_size));
    }

    // Bound the body *during* ingestion so a chunked request with no
    // Content-Length cannot OOM the process before any size check fires.
    // `Limited` caps buffered bytes at `max_request_size + 1`; the extra byte
    // lets the post-collect check below distinguish an at-limit body from an
    // over-limit one exactly. Ingestion is wrapped in the configured request
    // timeout (0 disables, matching the handler-timeout semantics) so a
    // slow-loris upload cannot stall forever.
    let timeout_seconds = config.function.timeout_seconds;
    let collect_fut = Limited::new(body, max_request_size + 1).collect();
    let collect_result = if timeout_seconds == 0 {
        collect_fut.await
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(timeout_seconds), collect_fut)
            .await
        {
            Ok(result) => result,
            Err(_elapsed) => return Ok(request_timeout_response(timeout_seconds)),
        }
    };

    let body_bytes = match collect_result {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            // A genuine oversize body surfaces as `LengthLimitError` -> 413. Any
            // other collect failure is a client transport error / mid-stream
            // abort -> 400. Never fall through to processing an empty body as a
            // legitimate request.
            if err.downcast_ref::<LengthLimitError>().is_some() {
                return Ok(oversize_response(max_request_size + 1, max_request_size));
            }
            return Ok(bad_request_response());
        }
    };

    // Exact-boundary enforcement: `Limited` permits up to `max_request_size + 1`
    // bytes, so a body of exactly `max_request_size + 1` collects successfully.
    // Reject it here to preserve the `> max_request_size` cap semantics (also
    // covers a lying Content-Length that under-declares the real body size).
    if body_bytes.len() > max_request_size {
        return Ok(oversize_response(body_bytes.len(), max_request_size));
    }

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

/// Build a `413 Payload Too Large` response for an oversize request body.
fn oversize_response(
    actual: usize,
    limit: usize,
) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    error!(
        actual_bytes = actual,
        limit_bytes = limit,
        "Rejecting oversize request body"
    );
    let body = serde_json::json!({
        "error": format!(
            "Request body of {actual} bytes exceeds the maximum of {limit} bytes"
        )
    })
    .to_string();
    hyper::Response::builder()
        .status(413)
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(body)))
        .unwrap_or_else(|_| {
            hyper::Response::builder()
                .status(413)
                .body(http_body_util::Full::new(bytes::Bytes::from(
                    "Payload Too Large",
                )))
                .unwrap()
        })
}

/// Build a `408 Request Timeout` response when body ingestion exceeds the
/// configured request timeout (e.g. a slow-loris upload that never completes).
fn request_timeout_response(
    timeout_seconds: u64,
) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    error!(
        timeout_seconds,
        "Request body ingestion timed out before the body was fully received"
    );
    let body = serde_json::json!({
        "error": format!(
            "Request body was not fully received within {timeout_seconds} seconds"
        )
    })
    .to_string();
    hyper::Response::builder()
        .status(408)
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(body)))
        .unwrap_or_else(|_| {
            hyper::Response::builder()
                .status(408)
                .body(http_body_util::Full::new(bytes::Bytes::from(
                    "Request Timeout",
                )))
                .unwrap()
        })
}

/// Build a `400 Bad Request` response for a client transport error or
/// mid-stream abort encountered while reading the request body. Prevents a
/// truncated/aborted upload from being silently processed as an empty request.
fn bad_request_response() -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    error!("Request body could not be read (client disconnect or transport error)");
    let body = serde_json::json!({
        "error": "Request body could not be read"
    })
    .to_string();
    hyper::Response::builder()
        .status(400)
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(body)))
        .unwrap_or_else(|_| {
            hyper::Response::builder()
                .status(400)
                .body(http_body_util::Full::new(bytes::Bytes::from("Bad Request")))
                .unwrap()
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FunctionRequest;
    use http_body_util::{BodyExt, Full};
    use std::sync::Arc;

    /// Build a runtime around a closure handler.
    fn runtime_with<F, Fut>(handler: F) -> AzureFunctionsRuntime<F>
    where
        F: Fn(FunctionRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = FunctionResponse> + Send,
    {
        AzureFunctionsRuntime::new(handler)
    }

    async fn body_bytes(resp: hyper::Response<Full<bytes::Bytes>>) -> bytes::Bytes {
        resp.into_body().collect().await.unwrap().to_bytes()
    }

    #[tokio::test]
    async fn base_path_prefix_is_stripped() {
        let mut runtime = runtime_with(|req: FunctionRequest| async move {
            FunctionResponse::with_body(200, req.path)
        });
        runtime.config.function.base_path = Some("/api".to_string());

        let resp = runtime
            .handle(FunctionRequest::new("GET", "/api/hello"))
            .await;
        assert_eq!(resp.body, "/hello");
    }

    #[tokio::test]
    async fn base_path_exact_match_becomes_root() {
        let mut runtime = runtime_with(|req: FunctionRequest| async move {
            FunctionResponse::with_body(200, req.path)
        });
        runtime.config.function.base_path = Some("/api".to_string());

        let resp = runtime.handle(FunctionRequest::new("GET", "/api")).await;
        assert_eq!(resp.body, "/");
    }

    #[tokio::test]
    async fn base_path_non_matching_path_unchanged() {
        let mut runtime = runtime_with(|req: FunctionRequest| async move {
            FunctionResponse::with_body(200, req.path)
        });
        runtime.config.function.base_path = Some("/api".to_string());

        let resp = runtime
            .handle(FunctionRequest::new("GET", "/other/x"))
            .await;
        assert_eq!(resp.body, "/other/x");
    }

    #[tokio::test(start_paused = true)]
    async fn slow_handler_times_out_with_504() {
        let mut runtime = runtime_with(|_req: FunctionRequest| async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            FunctionResponse::ok()
        });
        runtime.config.function.timeout_seconds = 1;

        let resp = runtime.handle(FunctionRequest::new("GET", "/")).await;
        assert_eq!(resp.status_code, 504);
    }

    #[tokio::test]
    async fn fast_handler_does_not_time_out() {
        let mut runtime =
            runtime_with(
                |_req: FunctionRequest| async move { FunctionResponse::with_body(201, "ok") },
            );
        runtime.config.function.timeout_seconds = 30;

        let resp = runtime.handle(FunctionRequest::new("GET", "/")).await;
        assert_eq!(resp.status_code, 201);
    }

    #[tokio::test]
    async fn oversize_body_returns_413_via_collected_length() {
        let app = Arc::new(|_req: FunctionRequest| async move { FunctionResponse::ok() });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 10;

        // No Content-Length header set by the builder for Full bodies, so this
        // exercises the post-collect size check.
        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full::new(bytes::Bytes::from(vec![0u8; 100])))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 413);
    }

    #[tokio::test]
    async fn oversize_content_length_header_returns_413_early() {
        let app = Arc::new(|_req: FunctionRequest| async move { FunctionResponse::ok() });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 10;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header("content-length", "1000000")
            .body(Full::new(bytes::Bytes::from_static(b"tiny")))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 413);
    }

    #[tokio::test]
    async fn within_limit_body_is_accepted() {
        let app = Arc::new(|req: FunctionRequest| async move {
            FunctionResponse::with_body(200, format!("{} bytes", req.body.len()))
        });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 1024;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full::new(bytes::Bytes::from_static(b"hello")))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(&body_bytes(resp).await[..], b"5 bytes");
    }

    #[tokio::test]
    async fn body_at_exact_limit_is_accepted() {
        // Streamed path (Full bodies carry no Content-Length): a body of exactly
        // `max_request_size` bytes must NOT be rejected.
        let app = Arc::new(|req: FunctionRequest| async move {
            FunctionResponse::with_body(200, format!("{} bytes", req.body.len()))
        });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 10;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full::new(bytes::Bytes::from(vec![0u8; 10])))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_ne!(resp.status(), 413);
        assert_eq!(resp.status(), 200);
        assert_eq!(&body_bytes(resp).await[..], b"10 bytes");
    }

    #[tokio::test]
    async fn body_one_over_limit_returns_413() {
        // Streamed path: a body of exactly `max_request_size + 1` bytes collects
        // under the `Limited` cap but must be rejected by the boundary check.
        let app = Arc::new(|_req: FunctionRequest| async move { FunctionResponse::ok() });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 10;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full::new(bytes::Bytes::from(vec![0u8; 11])))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 413);
    }

    #[tokio::test]
    async fn content_length_at_exact_limit_is_accepted() {
        // Content-Length header path: a declared length equal to the cap must
        // pass the early check and be handled (not 413).
        let app = Arc::new(|req: FunctionRequest| async move {
            FunctionResponse::with_body(200, format!("{} bytes", req.body.len()))
        });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 10;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header("content-length", "10")
            .body(Full::new(bytes::Bytes::from(vec![0u8; 10])))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_ne!(resp.status(), 413);
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn content_length_one_over_limit_returns_413() {
        // Content-Length header path: a declared length one byte over the cap
        // must be rejected early, before the body is buffered.
        let app = Arc::new(|_req: FunctionRequest| async move { FunctionResponse::ok() });
        let mut config = RuntimeConfig::default();
        config.function.max_request_size = 10;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .header("content-length", "11")
            .body(Full::new(bytes::Bytes::from_static(b"tiny")))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 413);
    }

    #[tokio::test]
    async fn base64_response_body_is_decoded_to_raw_bytes() {
        // Invalid UTF-8 forces the base64-encoded response path.
        let raw: Vec<u8> = vec![0u8, 159, 146, 150];
        let raw_for_handler = raw.clone();
        let app = Arc::new(move |_req: FunctionRequest| {
            let raw = raw_for_handler.clone();
            async move { FunctionResponse::ok().binary_body(&raw) }
        });
        let config = RuntimeConfig::default();

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(&body_bytes(resp).await[..], &raw[..]);
    }
}
