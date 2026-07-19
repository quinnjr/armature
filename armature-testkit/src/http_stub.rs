//! A hyper-based HTTP stub server for scripting responses in tests.

use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

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
        Self { status, headers: Vec::new(), body: body.into() }
    }

    /// JSON response (sets `content-type: application/json`).
    pub fn json(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into(),
        }
    }
}

/// A running stub HTTP server. The accept loop is aborted on drop.
pub struct StubServer {
    base_url: String,
    handle: JoinHandle<()>,
}

impl StubServer {
    /// Start a server that returns `resp` for every request.
    pub async fn start_single(resp: StubResponse) -> StubServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind stub server");
        let addr: SocketAddr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { break };
                let resp = resp.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |_req: Request<Incoming>| {
                        let resp = resp.clone();
                        async move { Ok::<_, Infallible>(build_response(&resp)) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
        });

        StubServer { base_url, handle }
    }

    /// The server's base URL, e.g. `http://127.0.0.1:PORT`.
    pub fn url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn build_response(resp: &StubResponse) -> Response<Full<Bytes>> {
    let mut builder = Response::builder().status(resp.status);
    for (k, v) in &resp.headers {
        builder = builder.header(k, v);
    }
    builder.body(Full::new(resp.body.clone())).unwrap()
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn serves_a_single_scripted_response() {
        let server = super::StubServer::start_single(super::StubResponse::json(200, r#"{"ok":true}"#)).await;

        let body = reqwest_get(server.url()).await;
        assert_eq!(body, r#"{"ok":true}"#);
    }

    // Minimal dependency-free HTTP/1.1 GET client for tests.
    async fn reqwest_get(url: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = url.trim_start_matches("http://");
        let (host, port) = addr.split_once(':').unwrap();
        let mut s = tokio::net::TcpStream::connect((host, port.parse::<u16>().unwrap()))
            .await
            .unwrap();
        s.write_all(format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
    }
}
