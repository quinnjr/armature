//! Protocol dispatch over plaintext: the h2c prior-knowledge preface.
//!
//! These run against a live `Server` on a real socket, because the thing under
//! test is the wiring — that a connection this crate will not serve actually
//! reaches the fallback with its bytes intact, rather than being fed to the
//! HTTP/1 parser.
//!
//! Deliberately not gated on the `tls` feature. `required-features` applies to a
//! whole test target, so keeping the ALPN tests here (they live in
//! `tls_dispatch.rs`) meant none of this ran under `default = []` — which is the
//! feature set CI builds.

use armature_h1::{Config, H2Fallback, Limits, Request, Response, Server, Transport};
use bytes::Bytes;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const H2C_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n";

/// Records what the fallback received, so a test can assert on it from outside.
#[derive(Clone, Default)]
struct Recorder {
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl H2Fallback for Recorder {
    fn handle(&self, io: Box<dyn Transport>, buffered: Bytes) -> Pin<Box<dyn Future<Output = ()>>> {
        let seen = self.seen.clone();
        Box::pin(async move {
            seen.lock().expect("lock").push(buffered.to_vec());
            // Reply with something recognizable so the client can tell the
            // fallback ran, rather than inferring it from silence.
            let mut io = io;
            let _ = io.write_all(b"FALLBACK").await;
            let _ = io.flush().await;
        })
    }
}

fn test_config() -> Config {
    let limits = Limits {
        idle_timeout: Duration::from_millis(300),
        header_timeout: Duration::from_millis(300),
        ..Default::default()
    };
    Config::new(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
        .workers(1)
        .limits(limits)
        .pin_cores(false)
}

async fn hello(_req: Request) -> Response {
    Response::text("hi")
}

/// Run `body` against a live server built from `cfg`, with `fallback`.
fn with_server<T>(
    cfg: Config,
    fallback: Recorder,
    body: impl FnOnce(SocketAddr) -> Pin<Box<dyn Future<Output = T> + Send>> + Send + 'static,
) -> T
where
    T: Send + 'static,
{
    let server = Server::bind(cfg).expect("bind");
    let addr = server.local_addr();
    let handle = server.handle();

    let client = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let out = rt.block_on(body(addr));
        handle.shutdown();
        out
    });

    server
        .serve_with_fallback(|| hello, move || fallback.clone())
        .expect("serve");
    client.join().expect("client thread")
}

#[test]
fn h2c_preface_reaches_the_fallback_with_its_bytes() {
    let rec = Recorder::default();
    let seen = rec.seen.clone();

    let reply = with_server(test_config().detect_h2c(true), rec, |addr| {
        Box::pin(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(H2C_PREFACE).await.unwrap();
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut out)).await;
            out
        })
    });

    assert_eq!(reply, b"FALLBACK", "the fallback must have handled it");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "exactly one connection reached the fallback");
    assert_eq!(
        seen[0], H2C_PREFACE,
        "the preface is part of the HTTP/2 stream and cannot be re-read from the \
         socket, so it must arrive in `buffered` intact"
    );
}

#[test]
fn http1_request_is_served_normally_with_h2c_detection_on() {
    let rec = Recorder::default();
    let seen = rec.seen.clone();

    let reply = with_server(test_config().detect_h2c(true), rec, |addr| {
        Box::pin(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(GET).await.unwrap();
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut out)).await;
            String::from_utf8_lossy(&out).into_owned()
        })
    });

    assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
    assert!(reply.ends_with("hi"), "{reply}");
    assert!(
        seen.lock().unwrap().is_empty(),
        "an HTTP/1 request must not reach the HTTP/2 fallback"
    );
}

/// The bytes consumed while classifying are part of the request and must be
/// handed to the HTTP/1 parser, not dropped.
#[test]
fn peeked_bytes_are_not_lost_from_the_http1_request() {
    let rec = Recorder::default();
    let reply = with_server(test_config().detect_h2c(true), rec, |addr| {
        Box::pin(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            // Dribble the request one byte at a time, forcing the classifier to
            // consume several reads before deciding.
            for b in GET {
                s.write_all(&[*b]).await.unwrap();
                s.flush().await.unwrap();
            }
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut out)).await;
            String::from_utf8_lossy(&out).into_owned()
        })
    });
    assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
}

#[test]
fn h2c_without_detection_is_parsed_as_http1_and_rejected() {
    let rec = Recorder::default();
    let seen = rec.seen.clone();

    // With detection off, `PRI * HTTP/2.0` is just an unparseable HTTP/1 request
    // line. It must be rejected, never silently accepted.
    let reply = with_server(test_config(), rec, |addr| {
        Box::pin(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(H2C_PREFACE).await.unwrap();
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut out)).await;
            String::from_utf8_lossy(&out).into_owned()
        })
    });

    assert!(
        reply.starts_with("HTTP/1.1 4") || reply.starts_with("HTTP/1.1 5") || reply.is_empty(),
        "must not be served as a valid request: {reply}"
    );
    assert!(seen.lock().unwrap().is_empty());
}
