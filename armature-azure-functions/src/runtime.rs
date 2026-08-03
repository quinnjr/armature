//! Azure Functions runtime for Armature applications.
use std::sync::Arc;
use tracing::{debug, error, info, warn};

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

    /// Handle a function request, starting the configured timeout now.
    pub async fn handle(&self, request: FunctionRequest) -> FunctionResponse
    where
        App: RequestHandler,
    {
        self.handle_with_deadline(request, deadline_from_config(&self.config))
            .await
    }

    /// Handle a function request against an already-established deadline.
    ///
    /// The HTTP entry point spends part of the request budget reading the body,
    /// so it establishes one deadline up front and passes it here. Restarting
    /// the timeout at this point would hand the invocation a second full budget
    /// and let a request outlive twice the configured `timeout_seconds`.
    ///
    /// `None` means no deadline (a configured timeout of 0).
    pub async fn handle_with_deadline(
        &self,
        request: FunctionRequest,
        deadline: Option<tokio::time::Instant>,
    ) -> FunctionResponse
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

        // Handle the request against the deadline, whatever is left of it.
        let response = match deadline {
            None => self.app.handle(request).await,
            Some(deadline) => {
                let path = request.path.clone();
                match tokio::time::timeout_at(deadline, self.app.handle(request)).await {
                    Ok(response) => response,
                    Err(_elapsed) => {
                        error!(
                            path = %path,
                            timeout_seconds = self.config.function.timeout_seconds,
                            "Azure Function request timed out"
                        );
                        FunctionResponse::error(504, "Gateway Timeout")
                    }
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
    // over-limit one exactly.
    //
    // The deadline is established once, here, and reused for the handler call
    // below: body ingestion and handling share a single `timeout_seconds`
    // budget rather than getting one each. `None` means no timeout (a
    // configured value of 0).
    let timeout_seconds = config.function.timeout_seconds;
    let deadline = deadline_from_config(&config);
    let collect_fut = Limited::new(body, max_request_size + 1).collect();
    let collect_result = match deadline {
        None => collect_fut.await,
        Some(deadline) => match tokio::time::timeout_at(deadline, collect_fut).await {
            Ok(result) => result,
            Err(_elapsed) => return Ok(request_timeout_response(timeout_seconds)),
        },
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

    // `HeaderMap::iter` yields one entry per value, so repeated field names
    // (`Cookie`, `X-Forwarded-For`) survive into the list intact.
    let mut headers = Vec::with_capacity(parts.headers.len());
    for (name, value) in parts.headers.iter() {
        match value.to_str() {
            Ok(v) => headers.push((name.as_str().to_string(), v.to_string())),
            // The facade hands out `&str`, so a value that is not UTF-8 cannot
            // be represented. Say so instead of dropping it in silence — an
            // unexplained missing header is very hard to debug.
            Err(_) => warn!(
                header = %name,
                "Dropping request header with a non-UTF-8 value"
            ),
        }
    }

    let query = parse_query(parts.uri.query().unwrap_or(""));

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

    // Handle the request on the remainder of the deadline established above.
    let runtime = AzureFunctionsRuntime { app, config };
    let response = runtime
        .handle_with_deadline(function_request, deadline)
        .await;

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

/// Turn the configured request timeout into a single absolute deadline.
///
/// Returns `None` when the timeout is disabled (`timeout_seconds == 0`). Every
/// stage of an invocation measures against the one deadline this produces, so
/// the total time a request can occupy is the configured value, not a multiple
/// of it.
fn deadline_from_config(config: &RuntimeConfig) -> Option<tokio::time::Instant> {
    match config.function.timeout_seconds {
        0 => None,
        seconds => Some(tokio::time::Instant::now() + std::time::Duration::from_secs(seconds)),
    }
}

/// Parse a raw query string into ordered key/value pairs.
///
/// Wire order and repeated keys are both preserved, matching
/// `armature_core`'s own query view: `?tag=a&tag=b` is meaningful and a handler
/// must be able to see both values in the order sent. A pair with no `=` keeps
/// its key with an empty value (`?flag` is a query parameter, not noise); a
/// pair with an empty key is dropped, since there is nothing to look it up by.
fn parse_query(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => (pair, ""),
        };
        if raw_key.is_empty() {
            continue;
        }
        out.push((decode_component(raw_key), decode_component(raw_value)));
    }
    out
}

/// Percent-decode one query component.
///
/// A component whose escapes decode to bytes that are not UTF-8 is passed
/// through verbatim rather than replaced with an empty string, so the handler
/// sees what the client actually sent and can decide for itself. The failure is
/// reported at `warn` — the same channel the rest of the runtime uses for
/// malformed input it chooses not to reject outright.
fn decode_component(raw: &str) -> String {
    match urlencoding::decode(raw) {
        Ok(decoded) => decoded.into_owned(),
        Err(err) => {
            warn!(
                component = %raw,
                error = %err,
                "Query component did not percent-decode; passing it through verbatim"
            );
            raw.to_string()
        }
    }
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

/// The contract [`AzureFunctionsRuntime`] drives.
///
/// Implemented here for any
/// `Fn(FunctionRequest) -> Future<Output = FunctionResponse>`. Other types
/// implement it themselves, or via `impl_azure_function_handler!`; there is
/// no blanket implementation for an Armature `Application`.
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

/// Implement [`RequestHandler`] for a type that already exposes an inherent
/// async `handle_request` method.
///
/// This macro knows nothing about `armature_core::Application`; there is no
/// conversion in this crate between `armature_core`'s `HttpRequest`/
/// `HttpResponse` and the Azure Functions types. What it targets is a
/// **user-supplied** shape:
///
/// ```rust,ignore
/// impl MyApp {
///     async fn handle_request(
///         &self,
///         request: armature_azure_functions::FunctionRequest,
///     ) -> Result<MyResponse, MyError> { /* ... */ }
/// }
/// ```
///
/// where `MyResponse` has the fields
/// - `status: u16`,
/// - `body: impl Into<String>`,
/// - `headers: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>`,
///
/// and `MyError: std::fmt::Display`. Given that, expand the macro once per type:
///
/// ```rust,ignore
/// use armature_azure_functions::impl_azure_function_handler;
///
/// impl_azure_function_handler!(MyApp);
/// ```
///
/// The whole [`FunctionRequest`] is forwarded — headers, query string, route
/// parameters and invocation context included — so the application can see
/// `Authorization`, cookies and the query, not just the method, path and body.
#[macro_export]
macro_rules! impl_azure_function_handler {
    ($app_type:ty) => {
        #[$crate::async_trait::async_trait]
        impl $crate::RequestHandler for $app_type {
            async fn handle(&self, request: $crate::FunctionRequest) -> $crate::FunctionResponse {
                match self.handle_request(request).await {
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
    async fn headers_and_query_reach_the_handler() {
        // Regression guard: the conversion used to forward only method, path
        // and body, so an application could never see Authorization, cookies or
        // any query string.
        let app = Arc::new(|req: FunctionRequest| async move {
            let tags = req.query_params("tag").collect::<Vec<_>>().join(",");
            let cookies = req.header_values("cookie").collect::<Vec<_>>().join(",");
            FunctionResponse::with_body(
                200,
                format!(
                    "{}|{}|{}|{}|{}",
                    req.authorization().unwrap_or("-"),
                    req.query_param("q").unwrap_or("-"),
                    tags,
                    cookies,
                    // A bare key keeps its key with an empty value.
                    req.query_param("flag").map_or("absent", |_| "present"),
                ),
            )
        });

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/x?q=hello%20world&tag=a&tag=b&flag")
            .header("authorization", "Bearer token")
            .header("cookie", "a=1")
            .header("cookie", "b=2")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_http_request(app, RuntimeConfig::default(), req)
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            &body_bytes(resp).await[..],
            b"Bearer token|hello world|a,b|a=1,b=2|present"
        );
    }

    #[tokio::test]
    async fn duplicate_response_headers_are_all_emitted() {
        // A handler must be able to set two cookies in one response.
        let app = Arc::new(|_req: FunctionRequest| async move {
            FunctionResponse::ok()
                .header("set-cookie", "session=abc")
                .header("set-cookie", "csrf=xyz")
        });

        let req = hyper::Request::builder()
            .method("GET")
            .uri("/")
            .body(Full::new(bytes::Bytes::new()))
            .unwrap();

        let resp = handle_http_request(app, RuntimeConfig::default(), req)
            .await
            .unwrap();
        let cookies: Vec<_> = resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(cookies, vec!["session=abc", "csrf=xyz"]);
    }

    #[test]
    fn query_parsing_keeps_order_bare_keys_and_repeats() {
        let parsed = parse_query("flag&tag=a&empty=&tag=b&=novalue&q=x%20y");
        assert_eq!(
            parsed,
            vec![
                ("flag".to_string(), String::new()),
                ("tag".to_string(), "a".to_string()),
                ("empty".to_string(), String::new()),
                ("tag".to_string(), "b".to_string()),
                ("q".to_string(), "x y".to_string()),
            ]
        );
    }

    #[test]
    fn query_component_that_does_not_decode_is_passed_through() {
        // %FF alone is not valid UTF-8; the raw text is preserved rather than
        // collapsing the value to an empty string.
        let parsed = parse_query("bad=%FF");
        assert_eq!(parsed, vec![("bad".to_string(), "%FF".to_string())]);
    }

    #[tokio::test(start_paused = true)]
    async fn ingestion_and_handling_share_one_timeout_budget() {
        // The handler alone sleeps past the configured budget, and body
        // ingestion has already consumed part of it, so the invocation must
        // still be cut off at the single deadline rather than being granted a
        // fresh one per stage.
        let app = Arc::new(|_req: FunctionRequest| async move {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            FunctionResponse::ok()
        });
        let mut config = RuntimeConfig::default();
        config.function.timeout_seconds = 2;

        let req = hyper::Request::builder()
            .method("POST")
            .uri("/")
            .body(Full::new(bytes::Bytes::from_static(b"x")))
            .unwrap();

        let start = tokio::time::Instant::now();
        let resp = handle_http_request(app, config, req).await.unwrap();
        assert_eq!(resp.status(), 504);
        // Comfortably under the 4s two-budgets-serially behavior this guards.
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
    }

    // A minimal response/error/app trio mirroring the shape the
    // `impl_azure_function_handler!` macro documents.
    struct MockResponse {
        status: u16,
        body: String,
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
        headers: Vec<(String, String)>,
        query: Vec<(String, String)>,
        params: std::collections::HashMap<String, String>,
        invocation_id: Option<String>,
        body: Vec<u8>,
    }

    struct MockApp {
        captured: std::sync::Mutex<Captured>,
    }

    impl MockApp {
        async fn handle_request(
            &self,
            request: FunctionRequest,
        ) -> std::result::Result<MockResponse, MockError> {
            let mut captured = self.captured.lock().unwrap();
            captured.method = Some(request.method.clone());
            captured.path = Some(request.path.clone());
            captured.headers = request.headers.clone();
            captured.query = request.query.clone();
            captured.params = request.params.clone();
            captured.invocation_id = request.context.invocation_id.clone();
            captured.body = request.body.to_vec();
            Ok(MockResponse {
                status: 201,
                body: "ok".to_string(),
                headers: vec![("x-app".to_string(), "yes".to_string())],
            })
        }
    }

    impl_azure_function_handler!(MockApp);

    #[tokio::test]
    async fn macro_forwards_full_request_to_app() {
        let mut request = FunctionRequest::new("POST", "/users/42");
        request
            .headers
            .push(("authorization".to_string(), "Bearer token".to_string()));
        request
            .headers
            .push(("x-custom".to_string(), "value".to_string()));
        request.query.push(("page".to_string(), "2".to_string()));
        request.query.push(("tag".to_string(), "a".to_string()));
        request.query.push(("tag".to_string(), "b".to_string()));
        request.params.insert("id".to_string(), "42".to_string());
        request.context.invocation_id = Some("inv-1".to_string());
        request.body = bytes::Bytes::from_static(b"payload");

        let app = MockApp {
            captured: std::sync::Mutex::new(Captured::default()),
        };

        let response = RequestHandler::handle(&app, request).await;

        // Response mapping is preserved.
        assert_eq!(response.status_code, 201);
        assert_eq!(response.body, "ok");
        assert_eq!(response.header_value("x-app"), Some("yes"));

        // The app received every part of the request, not just method/path/body.
        let captured = app.captured.lock().unwrap();
        assert_eq!(captured.method.as_deref(), Some("POST"));
        assert_eq!(captured.path.as_deref(), Some("/users/42"));
        assert_eq!(captured.body, b"payload".to_vec());
        assert_eq!(
            captured
                .headers
                .iter()
                .find(|(name, _)| name == "authorization")
                .map(|(_, value)| value.as_str()),
            Some("Bearer token")
        );
        assert_eq!(
            captured
                .headers
                .iter()
                .find(|(name, _)| name == "x-custom")
                .map(|(_, value)| value.as_str()),
            Some("value")
        );
        assert_eq!(
            captured
                .query
                .iter()
                .filter(|(key, _)| key == "tag")
                .map(|(_, value)| value.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(
            captured.query.iter().find(|(key, _)| key == "page"),
            Some(&("page".to_string(), "2".to_string()))
        );
        assert_eq!(captured.params.get("id").map(String::as_str), Some("42"));
        assert_eq!(captured.invocation_id.as_deref(), Some("inv-1"));
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
