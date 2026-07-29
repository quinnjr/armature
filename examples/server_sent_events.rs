#![allow(
    dead_code,
    unused_imports,
    clippy::default_constructed_unit_structs,
    clippy::needless_borrow,
    clippy::unnecessary_lazy_evaluations
)]
// Server-Sent Events (SSE) example

use armature::prelude::*;
use armature_core::sse::{ServerSentEvent, SseStream};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_stream::StreamExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StockPrice {
    symbol: String,
    price: f64,
    timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NewsItem {
    title: String,
    content: String,
    timestamp: u64,
}

/// Stock ticker service using SSE
#[injectable]
#[derive(Clone, Default)]
struct StockTickerService;

impl StockTickerService {
    fn get_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "service": "Stock Ticker",
            "status": "running",
            "symbols": ["AAPL", "GOOGL", "MSFT", "AMZN"]
        })
    }
}

/// News service using SSE
#[injectable]
#[derive(Clone, Default)]
struct NewsService;

impl NewsService {
    fn get_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "service": "News Feed",
            "status": "running"
        })
    }
}

/// SSE controller
///
/// Only `/events/stats` is registered here. `/stocks` and `/news` are served
/// by a dedicated raw SSE listener (see [`run_sse_server`] below) rather than
/// through this controller — see that function's doc comment for why.
#[controller("/events")]
#[derive(Default, Clone)]
struct EventsController;

#[routes]
impl EventsController {
    #[get("/stats")]
    async fn get_stats() -> Result<HttpResponse, Error> {
        let stock_service = StockTickerService::default();
        let news_service = NewsService::default();

        HttpResponse::json(&serde_json::json!({
            "stock_ticker": stock_service.get_stats(),
            "news": news_service.get_stats(),
            "stream_port": SSE_PORT
        }))
    }
}

#[module(
    providers: [StockTickerService, NewsService],
    controllers: [EventsController]
)]
#[derive(Default, Clone)]
struct AppModule;

// ============================================================================
// Genuine Server-Sent Events streaming
// ============================================================================
//
// `armature_core::sse::{ServerSentEvent, SseStream}` produce correctly framed
// `text/event-stream` payloads (`data: ...\n\n`, with optional `id:`/`event:`/
// `retry:` lines). However, `Application::listen` currently turns every
// handler's returned `HttpResponse` into a single, fully buffered
// `http_body_util::Full<Bytes>` hyper body (see `to_hyper_response` in
// `armature-core/src/application.rs`): the handler future must resolve to a
// *complete* response before hyper writes a single byte to the socket, and
// once it resolves the connection's response is done. There is currently no
// chunked/streamed-body path wired into Armature's HTTP transport, so a
// `#[routes]` handler cannot push additional events to a client after its
// initial response — the earlier version of this example silently relied on
// that fact and just returned one static "connected" event.
//
// Real, periodic server push therefore needs a connection that stays open
// while the framework produces events over time. Until Armature's transport
// gains a streaming-body mode, this example demonstrates *genuine* SSE by
// running a small dedicated HTTP/1.1 listener that writes each
// `ServerSentEvent` to the socket as soon as it is produced, built directly
// on top of the real `armature_core::sse` primitives.

/// Port serving the genuinely-streaming `/stocks` and `/news` SSE endpoints.
const SSE_PORT: u16 = 3016;

const SYMBOLS: [&str; 4] = ["AAPL", "GOOGL", "MSFT", "AMZN"];
const BASE_PRICES: [f64; 4] = [190.15, 164.80, 414.60, 178.25];

const NEWS_HEADLINES: [(&str, &str); 5] = [
    (
        "Markets Rally on Strong Tech Earnings",
        "Major indices climbed today as tech giants posted better-than-expected quarterly results.",
    ),
    (
        "Central Bank Holds Interest Rates Steady",
        "Policymakers opted to maintain current rates, citing balanced inflation and employment data.",
    ),
    (
        "New Chip Architecture Promises Efficiency Gains",
        "Semiconductor manufacturers unveiled a next-generation architecture aimed at cutting power draw.",
    ),
    (
        "Cloud Providers Expand Real-Time Data Offerings",
        "Streaming infrastructure investment continues to grow as demand for live data rises.",
    ),
    (
        "Startup Raises Series B to Scale Event-Driven Platform",
        "The funding will be used to expand engineering headcount and infrastructure.",
    ),
];

/// Minimal xorshift64 PRNG so the simulated feeds don't need an external
/// `rand` dependency.
struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Random value in `[-1.0, 1.0)`.
    fn next_signed_unit(&mut self) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
        unit * 2.0 - 1.0
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Run the raw SSE listener. Accepts connections forever; each connection is
/// handled on its own task so multiple clients can stream concurrently.
async fn run_sse_server(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    serve_sse(listener).await
}

/// Accepts connections off an already-bound listener forever; each
/// connection is handled on its own task so multiple clients can stream
/// concurrently. Split out from [`run_sse_server`] so tests can bind an
/// ephemeral port (`127.0.0.1:0`), read back the actual bound port, and then
/// drive the same accept loop the real server uses.
async fn serve_sse(listener: TcpListener) -> std::io::Result<()> {
    loop {
        let (socket, _) = listener.accept().await?;
        tokio::spawn(handle_sse_connection(socket));
    }
}

async fn handle_sse_connection(mut socket: TcpStream) {
    let Some(path) = read_request_path(&mut socket).await else {
        return;
    };

    const SSE_HEADERS: &str = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream\r\n",
        "Cache-Control: no-cache\r\n",
        "Connection: keep-alive\r\n",
        "Access-Control-Allow-Origin: *\r\n",
        "\r\n",
    );

    match path.as_str() {
        "/stocks" => {
            if socket.write_all(SSE_HEADERS.as_bytes()).await.is_err() {
                return;
            }
            stream_stock_prices(socket).await;
        }
        "/news" => {
            if socket.write_all(SSE_HEADERS.as_bytes()).await.is_err() {
                return;
            }
            stream_news(socket).await;
        }
        other => {
            let body = format!("Unknown SSE stream: {other}\n");
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    }
}

/// Reads request headers off the socket (discarding them) and returns the
/// requested path from the request line.
async fn read_request_path(socket: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];

    let read_headers = async {
        loop {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.len() > 8192 {
                return None;
            }
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                return Some(());
            }
        }
    };

    match tokio::time::timeout(Duration::from_secs(5), read_headers).await {
        Ok(Some(())) => {}
        _ => return None,
    }

    let text = String::from_utf8_lossy(&buf);
    let request_line = text.lines().next()?;
    let path = request_line.split_whitespace().nth(1)?;
    Some(path.to_string())
}

/// Streams simulated stock ticks for every symbol in [`SYMBOLS`] once a
/// second, using the real `armature_core::sse` API to build each event.
async fn stream_stock_prices(mut socket: TcpStream) {
    let (sse, mut events) = SseStream::new();

    let producer = tokio::spawn(async move {
        let mut rng = SimpleRng::new(now_millis() ^ 0xA5A5_A5A5_A5A5_A5A5);
        let mut prices = BASE_PRICES;
        let mut event_id: u64 = 0;

        loop {
            for (symbol, price) in SYMBOLS.iter().zip(prices.iter_mut()) {
                let delta = rng.next_signed_unit() * (*price * 0.004);
                *price = (*price + delta).max(1.0);

                let payload = StockPrice {
                    symbol: symbol.to_string(),
                    price: (*price * 100.0).round() / 100.0,
                    timestamp: now_millis(),
                };
                let Ok(json) = serde_json::to_string(&payload) else {
                    continue;
                };

                event_id += 1;
                let event = ServerSentEvent::full(
                    event_id.to_string(),
                    "price_update".to_string(),
                    json,
                    3000,
                );
                if sse.send(event).await.is_err() {
                    // Receiver dropped (client disconnected) — stop producing.
                    return;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    while let Some(item) = events.next().await {
        match item {
            Ok(frame) => {
                if !write_frame(&mut socket, frame.as_bytes()).await {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    producer.abort();
}

/// Writes an SSE frame to the socket, bounding both the write and the flush
/// with a timeout so a slow/stalled client can't block the streaming task
/// (and leak its producer/channel) indefinitely. Returns `false` if the
/// write, flush, or timeout failed, in which case the caller should stop
/// streaming.
async fn write_frame(socket: &mut TcpStream, frame: &[u8]) -> bool {
    /// SSE writes can legitimately be slower than a plain request read (an
    /// open streaming connection sits idle between events), so this is a
    /// bit looser than the 5s request-line read timeout in
    /// [`read_request_path`].
    const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

    match tokio::time::timeout(WRITE_TIMEOUT, socket.write_all(frame)).await {
        Ok(Ok(())) => {}
        _ => return false,
    }

    matches!(
        tokio::time::timeout(WRITE_TIMEOUT, socket.flush()).await,
        Ok(Ok(()))
    )
}

/// Streams simulated news headlines every 2.5 seconds, using the real
/// `armature_core::sse` API to build each event.
async fn stream_news(mut socket: TcpStream) {
    let (sse, mut events) = SseStream::new();

    let producer = tokio::spawn(async move {
        let mut event_id: u64 = 0;

        loop {
            let (title, content) = NEWS_HEADLINES[(event_id as usize) % NEWS_HEADLINES.len()];
            let payload = NewsItem {
                title: title.to_string(),
                content: content.to_string(),
                timestamp: now_millis(),
            };
            let Ok(json) = serde_json::to_string(&payload) else {
                continue;
            };

            event_id += 1;
            let event =
                ServerSentEvent::full(event_id.to_string(), "news_update".to_string(), json, 5000);
            if sse.send(event).await.is_err() {
                return;
            }

            tokio::time::sleep(Duration::from_millis(2500)).await;
        }
    });

    while let Some(item) = events.next().await {
        match item {
            Ok(frame) => {
                if !write_frame(&mut socket, frame.as_bytes()).await {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    producer.abort();
}

#[tokio::main]
async fn main() {
    println!("📡 Armature Server-Sent Events Example");
    println!("=======================================\n");

    println!("Available endpoints:");
    println!("  GET /events/stats                - Service statistics (Armature, port 3006)");
    println!(
        "  GET /stocks                       - Stock price updates, genuine SSE stream (port {SSE_PORT})"
    );
    println!(
        "  GET /news                         - News updates, genuine SSE stream (port {SSE_PORT})"
    );

    println!("\n💡 Usage with curl:");
    println!("  curl -N http://localhost:{SSE_PORT}/stocks");
    println!("  curl -N http://localhost:{SSE_PORT}/news");
    println!("  curl http://localhost:3006/events/stats");

    println!("\n💡 Usage in JavaScript:");
    println!("  const source = new EventSource('http://localhost:{SSE_PORT}/stocks');");
    println!("  source.addEventListener('price_update', (e) => {{");
    println!("    const data = JSON.parse(e.data);");
    println!("    console.log('Stock price:', data);");
    println!("  }});");
    println!();

    tokio::spawn(async move {
        if let Err(e) = run_sse_server(SSE_PORT).await {
            eprintln!("SSE server error: {}", e);
        }
    });

    let app = Application::create::<AppModule>().await;

    if let Err(e) = app.listen(3006).await {
        eprintln!("Server error: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bounded overall time budget for every socket read in these tests, so
    /// a framing regression fails the test instead of hanging the suite.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// Binds `serve_sse` to an ephemeral port and returns the actual bound
    /// address, so tests don't race for a fixed port.
    async fn spawn_test_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = serve_sse(listener).await;
        });
        addr
    }

    /// Reads from `stream` until `pattern` has been seen in the accumulated
    /// bytes (or the timeout elapses), returning everything read so far as a
    /// lossy UTF-8 string.
    async fn read_until(stream: &mut TcpStream, pattern: &[u8]) -> Option<String> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];

        let read_loop = async {
            loop {
                let n = stream.read(&mut chunk).await.ok()?;
                if n == 0 {
                    return Some(());
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(pattern.len()).any(|w| w == pattern) {
                    return Some(());
                }
            }
        };

        match tokio::time::timeout(TEST_TIMEOUT, read_loop).await {
            Ok(Some(())) => Some(String::from_utf8_lossy(&buf).into_owned()),
            _ => None,
        }
    }

    /// Connects to `addr`, issues a raw `GET <path> HTTP/1.1` request, and
    /// returns the response bytes read up through the end of the first SSE
    /// frame (i.e. through the first bare `\n\n`, which only appears as an
    /// event terminator — the header block itself ends in `\r\n\r\n`).
    async fn get_first_sse_frame(addr: std::net::SocketAddr, path: &str) -> String {
        let mut stream = tokio::time::timeout(TEST_TIMEOUT, TcpStream::connect(addr))
            .await
            .expect("connect timed out")
            .expect("connect failed");

        let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n");
        tokio::time::timeout(TEST_TIMEOUT, stream.write_all(request.as_bytes()))
            .await
            .expect("write timed out")
            .expect("write failed");

        read_until(&mut stream, b"\n\n")
            .await
            .expect("response (headers + first SSE frame) before timeout")
    }

    #[tokio::test]
    async fn stocks_stream_sends_well_formed_sse_frames() {
        let addr = spawn_test_server().await;
        let response = get_first_sse_frame(addr, "/stocks").await;

        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "expected 200 OK status line, got: {response}"
        );
        assert!(
            response.contains("Content-Type: text/event-stream"),
            "expected SSE content type header, got: {response}"
        );
        assert!(
            response.contains("event: price_update"),
            "expected price_update event, got: {response}"
        );
        assert!(
            response.contains("data: "),
            "expected an SSE `data:` line, got: {response}"
        );
        assert!(
            response.ends_with("\n\n"),
            "expected the frame to be terminated by a blank line, got: {response}"
        );
    }

    #[tokio::test]
    async fn news_stream_sends_well_formed_sse_frames() {
        let addr = spawn_test_server().await;
        let response = get_first_sse_frame(addr, "/news").await;

        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "expected 200 OK status line, got: {response}"
        );
        assert!(
            response.contains("Content-Type: text/event-stream"),
            "expected SSE content type header, got: {response}"
        );
        assert!(
            response.contains("event: news_update"),
            "expected news_update event, got: {response}"
        );
        assert!(
            response.contains("data: "),
            "expected an SSE `data:` line, got: {response}"
        );
        assert!(
            response.ends_with("\n\n"),
            "expected the frame to be terminated by a blank line, got: {response}"
        );
    }

    #[tokio::test]
    async fn unknown_path_returns_404() {
        let addr = spawn_test_server().await;

        let mut stream = tokio::time::timeout(TEST_TIMEOUT, TcpStream::connect(addr))
            .await
            .expect("connect timed out")
            .expect("connect failed");

        let request = "GET /unknown HTTP/1.1\r\nHost: localhost\r\n\r\n";
        tokio::time::timeout(TEST_TIMEOUT, stream.write_all(request.as_bytes()))
            .await
            .expect("write timed out")
            .expect("write failed");

        // The 404 handler closes the connection after writing the response,
        // so reading to EOF gives us the full response.
        let response = read_until(&mut stream, b"\0\0\0\0")
            .await
            .expect("404 response before timeout");

        assert!(
            response.starts_with("HTTP/1.1 404 Not Found"),
            "expected 404 status line, got: {response}"
        );
        assert!(
            response.contains("Unknown SSE stream: /unknown"),
            "expected the unknown path echoed in the body, got: {response}"
        );
    }
}
