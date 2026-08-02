//! The hyper baseline for `scripts/bench-h1.sh`.
//!
//! The same response as `armature-h1`'s `hello` example: the same 13-byte body,
//! and a `server` header on both sides so neither wins by omitting one. The header
//! *values* differ by six bytes (`armature-h1` against `hyper`), which is the only
//! difference in what goes on the wire — noise next to a response head, but stated
//! rather than hidden.
//!
//! Deployment shape: hyper on tokio's multi-threaded runtime with a task per
//! connection, which is how hyper is actually run. `armature-h1` is
//! thread-per-core with `SO_REUSEPORT`. Those are different models on purpose;
//! pinning hyper to one thread per core would benchmark a configuration nobody
//! deploys, and running `armature-h1` on a work-stealing runtime is not something
//! it supports. The comparison is between two servers as each is meant to be run.
//!
//! ```sh
//! cargo run --release --manifest-path benches/comparison_servers/hyper_h1_server/Cargo.toml
//! ```

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::{HeaderValue, SERVER};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;

const BODY: &[u8] = b"Hello, world!";

async fn hello(_req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let mut resp = Response::new(Full::new(Bytes::from_static(BODY)));
    resp.headers_mut()
        .insert(SERVER, HeaderValue::from_static("hyper"));
    Ok(resp)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8081);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("hyper listening on http://{addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        // Nagle off, matching what armature-h1's TcpConfig sets. Left on, it adds
        // up to 40ms of latency to small responses on one side of the comparison
        // and not the other.
        stream.set_nodelay(true)?;
        let io = TokioIo::new(stream);
        tokio::task::spawn(async move {
            if let Err(err) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service_fn(hello))
                .await
            {
                eprintln!("connection error: {err:?}");
            }
        });
    }
}
