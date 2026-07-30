//! End-to-end benchmark: one full request/response cycle through `Connection`.
//!
//! Over `tokio::io::duplex` rather than a real socket, on purpose. A loopback TCP
//! benchmark measures the kernel's network stack more than it measures this
//! crate — syscall and scheduling cost dominates, and the numbers move with
//! unrelated kernel settings. What is interesting here is the crate's own cost per
//! request: parse, dispatch, serialize, and the state-machine transition back to
//! reading. `scripts/bench-h1.sh` covers the socket path against a real client.
//!
//! The server runs on its own thread with its own current-thread runtime and
//! `LocalSet`, and the benchmark drives the client half from the criterion thread.
//! That is not an implementation detail to skip past: `Connection` is `!Send` (it
//! holds `Rc`s), and driving both halves from one current-thread runtime would
//! mean either nesting `block_on` inside `block_on` or spinning on a thread whose
//! `LocalSet` owns the server task — the second starves the very task the
//! benchmark is waiting on. Two threads costs a cross-thread wakeup per request,
//! which is real and included in the numbers below; the microbenchmarks in
//! `parse.rs`/`write.rs` are the place to read per-stage cost without it.
//!
//! Each iteration drives an already-warm keep-alive connection, so pool growth and
//! task setup sit outside the measurement — the same steady state the allocation
//! regression test asserts costs zero allocations.

use armature_h1::{ConnConfig, Connection, DateCache, Limits, Request, Response};
use criterion::{Criterion, criterion_group, criterion_main};
use std::cell::RefCell;
use std::hint::black_box;
use std::rc::Rc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const MINIMAL_GET: &[u8] = b"GET / HTTP/1.1\r\nHost: a.example\r\n\r\n";

const BROWSER_GET: &[u8] = b"GET /index.html HTTP/1.1\r\n\
Host: a.example\r\n\
User-Agent: Mozilla/5.0\r\n\
Accept: text/html,application/xhtml+xml\r\n\
Accept-Language: en-US,en;q=0.9\r\n\
Accept-Encoding: gzip, deflate, br\r\n\
Connection: keep-alive\r\n\
Cache-Control: max-age=0\r\n\
\r\n";

const FIXED_BODY_POST: &[u8] =
    b"POST / HTTP/1.1\r\nHost: a.example\r\nContent-Length: 5\r\n\r\nhello";

const CHUNKED_POST: &[u8] = b"POST / HTTP/1.1\r\nHost: a.example\r\n\
Transfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";

/// Four requests back to back in one write, to price pipelining: the connection
/// should parse all four out of one read rather than paying a read per request.
const PIPELINED_GETS: &[u8] = b"GET / HTTP/1.1\r\nHost: a.example\r\n\r\n\
GET / HTTP/1.1\r\nHost: a.example\r\n\r\n\
GET / HTTP/1.1\r\nHost: a.example\r\n\r\n\
GET / HTTP/1.1\r\nHost: a.example\r\n\r\n";

async fn hello(_req: Request) -> Response {
    Response::text("hi")
}

async fn drain(mut req: Request) -> Response {
    let _ = req.body.collect(64 * 1024).await;
    Response::text("hi")
}

fn limits() -> Limits {
    Limits {
        // Long enough that a slow criterion warm-up never races the idle timer,
        // short enough that a wedged harness fails instead of hanging.
        idle_timeout: Duration::from_secs(60),
        header_timeout: Duration::from_secs(60),
        ..Default::default()
    }
}

/// Serve one connection on a dedicated thread, returning the client half.
///
/// The returned handle must be joined after the client is dropped; the connection
/// ends when its peer closes.
fn spawn_server<S, Fut>(service: S) -> (DuplexStream, std::thread::JoinHandle<()>)
where
    S: Fn(Request) -> Fut + Copy + Send + 'static,
    Fut: std::future::Future<Output = Response> + 'static,
{
    let (client, server) = tokio::io::duplex(1024 * 1024);
    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            let conn = Connection::new(
                server,
                service,
                Rc::new(ConnConfig {
                    limits: limits(),
                    tick: Duration::from_millis(50),
                    server_name: None,
                }),
                Rc::new(RefCell::new(DateCache::new())),
            );
            let _ = conn.serve().await;
        });
    });
    (client, handle)
}

/// Read `count` responses off `client`, tolerating any read split.
///
/// Counts `HTTP/1.1 ` occurrences rather than assuming one read per response:
/// pipelined replies legitimately coalesce into one write, and a fixed
/// one-read-per-response assumption would silently measure the wrong thing. The
/// marker cannot straddle a read boundary here because the server writes each
/// response head in a single `write_all` into a buffer larger than any response
/// this file sends.
async fn read_responses(client: &mut DuplexStream, scratch: &mut [u8], count: usize) {
    let mut seen = 0;
    while seen < count {
        let n = client.read(scratch).await.expect("read");
        assert!(n > 0, "server closed unexpectedly");
        seen += scratch[..n]
            .windows(9)
            .filter(|w| *w == b"HTTP/1.1 ")
            .count();
    }
}

/// Benchmark `request` served on a warm keep-alive connection.
///
/// `per_iter` is how many responses one write produces — one for a single
/// request, four for the pipelined case.
fn bench_serve<S, Fut>(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    request: &'static [u8],
    per_iter: usize,
    service: S,
) where
    S: Fn(Request) -> Fut + Copy + Send + 'static,
    Fut: std::future::Future<Output = Response> + 'static,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("client runtime");
    let (mut client, server_thread) = spawn_server(service);
    let mut scratch = vec![0u8; 256 * 1024];

    // Warm the pools and both connections' buffers so the measured iterations are
    // steady-state rather than first-request cost.
    rt.block_on(async {
        for _ in 0..100 {
            client.write_all(request).await.expect("write");
            read_responses(&mut client, &mut scratch, per_iter).await;
        }
    });

    g.throughput(criterion::Throughput::Elements(per_iter as u64));
    g.bench_function(name, |b| {
        b.iter(|| {
            rt.block_on(async {
                client.write_all(black_box(request)).await.expect("write");
                read_responses(&mut client, &mut scratch, per_iter).await;
            })
        })
    });

    drop(client);
    server_thread.join().expect("server thread");
}

fn benches(c: &mut Criterion) {
    let mut g = c.benchmark_group("serve");
    bench_serve(&mut g, "keepalive_get", MINIMAL_GET, 1, hello);
    bench_serve(&mut g, "browser_get", BROWSER_GET, 1, hello);
    bench_serve(&mut g, "fixed_body_post", FIXED_BODY_POST, 1, drain);
    bench_serve(&mut g, "chunked_post", CHUNKED_POST, 1, drain);
    bench_serve(&mut g, "pipelined_4_gets", PIPELINED_GETS, 4, hello);
    g.finish();
}

criterion_group!(e2e, benches);
criterion_main!(e2e);
