//! A hyper-based HTTP stub server for scripting responses in tests.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A request captured by the stub server.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl RecordedRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body decoded as UTF-8 (lossy).
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// A scripted HTTP response.
#[derive(Clone, Debug)]
pub struct StubResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl StubResponse {
    /// Response with a raw body and no extra headers.
    pub fn new(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// JSON response (sets `content-type: application/json`).
    pub fn json(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into(),
        }
    }

    /// Add a response header.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// A running stub HTTP server. The accept loop is aborted on drop.
pub struct StubServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: JoinHandle<()>,
}

impl StubServer {
    /// Start a server that returns `resp` for every request.
    pub async fn start_single(resp: StubResponse) -> StubServer {
        Self::builder().default_response(resp).start().await
    }

    /// The server's base URL, e.g. `http://127.0.0.1:PORT`.
    pub fn url(&self) -> &str {
        &self.base_url
    }

    /// Start building a multi-route stub server.
    pub fn builder() -> StubServerBuilder {
        StubServerBuilder {
            routes: HashMap::new(),
            default: StubResponse::new(404, Bytes::new()),
        }
    }

    /// Snapshot of all requests received so far.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Return the first recorded request matching `method` + `path`, or panic.
    pub fn assert_received(&self, method: &str, path: &str) -> RecordedRequest {
        let m = method.to_ascii_uppercase();
        self.requests()
            .into_iter()
            .find(|r| r.method == m && r.path == path)
            .unwrap_or_else(|| {
                panic!(
                    "stub server received no {method} {path}; got: {:?}",
                    self.requests()
                )
            })
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Builder for a stub server with per-route responses.
pub struct StubServerBuilder {
    routes: HashMap<(String, String), StubResponse>,
    default: StubResponse,
}

impl StubServerBuilder {
    /// Add a response for an exact method + path.
    pub fn route(mut self, method: &str, path: &str, resp: StubResponse) -> Self {
        self.routes
            .insert((method.to_ascii_uppercase(), path.to_string()), resp);
        self
    }

    /// Set the response returned when no route matches.
    pub fn default_response(mut self, resp: StubResponse) -> Self {
        self.default = resp;
        self
    }

    /// Start the server.
    pub async fn start(self) -> StubServer {
        let routes = Arc::new(self.routes);
        let default = Arc::new(self.default);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind stub server");
        let addr: SocketAddr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let requests_for_server = requests.clone();
        let handle = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let routes = routes.clone();
                let default = default.clone();
                let requests = requests.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |req: Request<Incoming>| {
                        let routes = routes.clone();
                        let default = default.clone();
                        let requests = requests.clone();
                        async move {
                            let (parts, body) = req.into_parts();
                            let method = parts.method.as_str().to_ascii_uppercase();
                            let path = parts.uri.path().to_string();
                            let query = parts.uri.query().map(|q| q.to_string());
                            let headers = parts
                                .headers
                                .iter()
                                .map(|(k, v)| {
                                    (
                                        k.as_str().to_string(),
                                        String::from_utf8_lossy(v.as_bytes()).into_owned(),
                                    )
                                })
                                .collect();
                            let bytes = body
                                .collect()
                                .await
                                .map(|c| c.to_bytes())
                                .unwrap_or_default();
                            requests.lock().unwrap().push(RecordedRequest {
                                method: method.clone(),
                                path: path.clone(),
                                query,
                                headers,
                                body: bytes,
                            });
                            let resp = routes
                                .get(&(method, path))
                                .cloned()
                                .unwrap_or_else(|| (*default).clone());
                            Ok::<_, Infallible>(build_response(&resp))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
        });

        StubServer {
            base_url,
            requests: requests_for_server,
            handle,
        }
    }
}

fn build_response(resp: &StubResponse) -> Response<Full<Bytes>> {
    let mut builder = Response::builder().status(resp.status);
    for (k, v) in &resp.headers {
        builder = builder.header(k, v);
    }
    builder
        .body(Full::new(resp.body.clone()))
        .unwrap_or_else(|e| {
            Response::builder()
                .status(500)
                .body(Full::new(Bytes::from(format!("stub build error: {e}"))))
                .unwrap()
        })
}

#[cfg(test)]
mod tests {
    /// A parsed raw HTTP/1.1 response.
    struct RawResponse {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    #[tokio::test]
    async fn serves_a_single_scripted_response() {
        let server =
            super::StubServer::start_single(super::StubResponse::json(200, r#"{"ok":true}"#)).await;

        let resp = raw_request(server.url(), "GET", "/", "").await;
        assert_eq!(resp.body, r#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn routes_by_method_and_path() {
        let server = super::StubServer::builder()
            .route("GET", "/health", super::StubResponse::new(200, "ok"))
            .route(
                "POST",
                "/token",
                super::StubResponse::json(201, r#"{"id":1}"#),
            )
            .default_response(super::StubResponse::new(404, "missing"))
            .start()
            .await;

        let health = raw_request(server.url(), "GET", "/health", "").await;
        assert_eq!(health.status, 200);
        assert_eq!(health.body, "ok");

        let token = raw_request(server.url(), "POST", "/token", "").await;
        assert_eq!(token.status, 201);
        assert_eq!(token.body, r#"{"id":1}"#);

        let nope = raw_request(server.url(), "GET", "/nope", "").await;
        assert_eq!(nope.status, 404);
        assert_eq!(nope.body, "missing");
    }

    #[tokio::test]
    async fn json_response_sets_content_type() {
        let server =
            super::StubServer::start_single(super::StubResponse::json(200, r#"{"ok":true}"#)).await;

        let resp = raw_request(server.url(), "GET", "/", "").await;
        assert!(
            resp.headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("content-type") && v == "application/json"),
            "expected content-type: application/json header, got: {:?}",
            resp.headers
        );
    }

    #[tokio::test]
    async fn records_requests_for_assertions() {
        let server = super::StubServer::builder()
            .route(
                "POST",
                "/introspect",
                super::StubResponse::json(200, r#"{"active":true}"#),
            )
            .start()
            .await;

        let _ = raw_request(server.url(), "POST", "/introspect?trace=1", "token=abc").await;

        let rec = server.assert_received("POST", "/introspect");
        assert_eq!(rec.body_string(), "token=abc");
        assert_eq!(rec.header("content-length"), Some("9"));
        assert_eq!(rec.header("Content-Length"), Some("9"));
        assert_eq!(rec.header("CONTENT-LENGTH"), Some("9"));
        assert_eq!(rec.path, "/introspect");
        assert_eq!(rec.query.as_deref(), Some("trace=1"));
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn default_404_when_no_default_response_configured() {
        let server = super::StubServer::builder()
            .route("GET", "/x", super::StubResponse::new(200, "x"))
            .start()
            .await;

        let resp = raw_request(server.url(), "GET", "/unmatched", "").await;
        assert_eq!(resp.status, 404);
        assert_eq!(resp.body, "");
    }

    #[tokio::test]
    #[should_panic(expected = "received no")]
    async fn assert_received_panics_when_not_found() {
        let server = super::StubServer::builder()
            .route("POST", "/missing", super::StubResponse::new(200, "ok"))
            .start()
            .await;

        let _ = raw_request(server.url(), "GET", "/other", "").await;

        server.assert_received("POST", "/missing");
    }

    // Minimal dependency-free HTTP/1.1 client for tests: parses the status
    // line, headers, and body.
    async fn raw_request(url: &str, method: &str, path: &str, body: &str) -> RawResponse {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = url.trim_start_matches("http://");
        let (host, port) = addr.split_once(':').unwrap();
        let mut s = tokio::net::TcpStream::connect((host, port.parse::<u16>().unwrap()))
            .await
            .unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_ref(), ""));
        let mut lines = head.split("\r\n");
        let status_line = lines.next().unwrap_or_default();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        RawResponse {
            status,
            headers,
            body: body.to_string(),
        }
    }

    #[tokio::test]
    async fn port_is_released_after_drop() {
        let addr = {
            let server = super::StubServer::start_single(super::StubResponse::new(200, "x")).await;
            server.url().trim_start_matches("http://").to_string()
        }; // server dropped here

        // The abort() call schedules cancellation but the OS may need a
        // moment to release the socket; retry rebinding for up to ~2s.
        let socket_addr = addr.parse::<std::net::SocketAddr>().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut bound = tokio::net::TcpListener::bind(socket_addr).await;
        while bound.is_err() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            bound = tokio::net::TcpListener::bind(socket_addr).await;
        }
        assert!(bound.is_ok(), "port was not released after StubServer drop");
    }
}
