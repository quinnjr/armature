//! The thread-per-core server.
//!
//! N pinned OS threads, each running a `current_thread` runtime. On Unix each
//! thread owns its own `SO_REUSEPORT` listener, so the kernel load-balances
//! accepts and a connection never migrates cores — which is what makes per-core
//! date caches and service state safe to keep non-atomic.

use crate::Limits;
use crate::conn::{ConnConfig, Connection};
use crate::service::{H1Service, Transport};
use crate::tls::{H2Fallback, Preface, is_h2c_preface};
use crate::write::DateCache;
use bytes::{Bytes, BytesMut};
use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Socket-level tuning.
#[derive(Clone, Debug)]
pub struct TcpConfig {
    /// Disable Nagle's algorithm. On for a request/response protocol, where
    /// delaying a small response to coalesce it helps nobody.
    pub nodelay: bool,
    /// Listen backlog.
    pub backlog: i32,
    /// Use `SO_REUSEPORT` so each worker owns its own listener.
    ///
    /// Ignored on platforms without it, which fall back to one shared listener.
    pub reuse_port: bool,
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            nodelay: true,
            backlog: 1024,
            reuse_port: true,
        }
    }
}

/// Server configuration.
#[derive(Clone, Debug)]
pub struct Config {
    /// Address to bind.
    pub addr: SocketAddr,
    /// Worker threads. Defaults to the available parallelism.
    pub workers: usize,
    /// Per-connection limits and deadlines.
    pub limits: Limits,
    /// Socket tuning.
    pub tcp: TcpConfig,
    /// Deadline coarsening granularity.
    pub tick: Duration,
    /// Pin each worker to a core.
    pub pin_cores: bool,
    /// Value for the `Server` field, or none to omit it.
    pub server_name: Option<Bytes>,
    /// How long to let in-flight connections finish after a shutdown signal.
    pub shutdown_grace: Duration,
    /// Detect the h2c prior-knowledge preface on plaintext connections and route
    /// it to the HTTP/2 fallback.
    ///
    /// Costs one small read before the first request is parsed, so it is opt-in.
    pub detect_h2c: bool,
    /// TLS, when serving HTTPS.
    #[cfg(feature = "tls")]
    pub tls: Option<std::sync::Arc<rustls::ServerConfig>>,
}

impl Config {
    /// A default configuration for `addr`.
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            workers: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            limits: Limits::default(),
            tcp: TcpConfig::default(),
            tick: Duration::from_millis(100),
            pin_cores: true,
            server_name: None,
            shutdown_grace: Duration::from_secs(10),
            detect_h2c: false,
            #[cfg(feature = "tls")]
            tls: None,
        }
    }

    /// Detect the h2c prior-knowledge preface on plaintext connections.
    pub fn detect_h2c(mut self, on: bool) -> Self {
        self.detect_h2c = on;
        self
    }

    /// Serve TLS with this rustls configuration.
    #[cfg(feature = "tls")]
    pub fn with_tls(mut self, tls: std::sync::Arc<rustls::ServerConfig>) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Set the worker count.
    pub fn workers(mut self, n: usize) -> Self {
        self.workers = n.max(1);
        self
    }

    /// Set the per-connection limits.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Set the `Server` field value.
    pub fn server_name(mut self, name: Bytes) -> Self {
        self.server_name = Some(name);
        self
    }

    /// Enable or disable core pinning.
    pub fn pin_cores(mut self, on: bool) -> Self {
        self.pin_cores = on;
        self
    }
}

/// A handle for stopping a running server.
///
/// `Clone + Send`, since it must cross into the thread that decides to stop.
#[derive(Clone, Debug)]
pub struct ServerHandle {
    tx: watch::Sender<bool>,
}

impl ServerHandle {
    /// Signal every worker to stop accepting and drain.
    pub fn shutdown(&self) {
        // Idempotent by construction: sending `true` twice is the same state.
        let _ = self.tx.send(true);
    }

    /// Whether shutdown has been signalled.
    pub fn is_shutting_down(&self) -> bool {
        *self.tx.borrow()
    }
}

/// How listeners are distributed across workers.
enum Listeners {
    /// One listener per worker, via `SO_REUSEPORT`.
    PerWorker(Vec<std::net::TcpListener>),
    /// One shared listener, for platforms without `SO_REUSEPORT`.
    ///
    /// `TcpListener::accept` takes `&self`, so this needs no lock.
    Shared(Arc<std::net::TcpListener>),
}

/// A bound, not-yet-serving server.
pub struct Server {
    cfg: Config,
    listeners: Listeners,
    local_addr: SocketAddr,
    tx: watch::Sender<bool>,
}

impl Server {
    /// Bind according to `cfg`.
    ///
    /// Binding happens here rather than in [`serve`](Self::serve) so that
    /// [`local_addr`](Self::local_addr) is available before serving starts —
    /// which is what lets a test bind port 0 and then connect to it.
    pub fn bind(cfg: Config) -> io::Result<Self> {
        let (listeners, local_addr) = if cfg.tcp.reuse_port && reuse_port_supported() {
            let mut v = Vec::with_capacity(cfg.workers);
            let mut addr = cfg.addr;
            for i in 0..cfg.workers {
                let l = bind_one(addr, &cfg.tcp, true)?;
                if i == 0 {
                    // With port 0, the first bind picks the port; the rest must
                    // join that same port or they would each get their own.
                    addr = l.local_addr()?;
                }
                v.push(l);
            }
            (Listeners::PerWorker(v), addr)
        } else {
            let l = bind_one(cfg.addr, &cfg.tcp, false)?;
            let addr = l.local_addr()?;
            (Listeners::Shared(Arc::new(l)), addr)
        };

        let (tx, _) = watch::channel(false);
        Ok(Self {
            cfg,
            listeners,
            local_addr,
            tx,
        })
    }

    /// The concrete bound address, with port 0 resolved.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// A handle for stopping this server.
    pub fn handle(&self) -> ServerHandle {
        ServerHandle {
            tx: self.tx.clone(),
        }
    }

    /// Serve until shutdown, blocking the calling thread.
    ///
    /// Connections this crate will not serve — a negotiated `h2`, or an h2c
    /// preface — are closed. Use [`serve_with_fallback`](Self::serve_with_fallback)
    /// to route them somewhere instead.
    ///
    /// A connection a handler upgrades with status 101 is *also* closed: there
    /// is no upgrade-consumer hook here. Drive [`Connection`] yourself and take
    /// the `Upgraded` from [`Connection::serve`] if you need the raw socket.
    ///
    /// `make` runs once per worker thread to produce that worker's service. Note
    /// the bounds: the *factory* is `Send`, because it crosses thread boundaries
    /// at startup; the *service* it produces is not, because it never does. That
    /// asymmetry is what lets per-core service state be non-atomic.
    pub fn serve<F, S>(self, make: F) -> io::Result<()>
    where
        F: Fn() -> S + Send + Clone + 'static,
        S: H1Service + 'static,
    {
        self.serve_with_fallback(make, || CloseH2)
    }

    /// Serve until shutdown, routing HTTP/2 connections to a fallback.
    ///
    /// `make_fallback` runs once per worker, like `make`, and for the same reason:
    /// the factory crosses thread boundaries at startup, the fallback it produces
    /// never does — so a fallback may hold non-`Send` state.
    pub fn serve_with_fallback<F, S, G, H>(self, make: F, make_fallback: G) -> io::Result<()>
    where
        F: Fn() -> S + Send + Clone + 'static,
        S: H1Service + 'static,
        G: Fn() -> H + Send + Clone + 'static,
        H: H2Fallback + 'static,
    {
        let Server {
            cfg, listeners, tx, ..
        } = self;

        let core_ids = if cfg.pin_cores {
            core_affinity::get_core_ids().unwrap_or_default()
        } else {
            Vec::new()
        };

        // Each worker gets its own listener under SO_REUSEPORT, or a dup of the
        // one shared listener where that option does not exist.
        let mut per_worker: Vec<Option<std::net::TcpListener>> = match listeners {
            Listeners::PerWorker(v) => v.into_iter().map(Some).collect(),
            Listeners::Shared(shared) => {
                (0..cfg.workers).map(|_| shared.try_clone().ok()).collect()
            }
        };

        let mut handles = Vec::with_capacity(cfg.workers);
        for (worker, slot) in per_worker.iter_mut().enumerate() {
            let cfg = cfg.clone();
            let make = make.clone();
            let make_fallback = make_fallback.clone();
            let rx = tx.subscribe();
            let core = core_ids.get(worker).copied();
            let Some(std_listener) = slot.take() else {
                continue;
            };

            handles.push(
                std::thread::Builder::new()
                    .name(format!("h1-{worker}"))
                    .spawn(move || {
                        if let Some(core) = core {
                            // Best effort: a container may forbid it, and failing
                            // to pin costs locality, not correctness.
                            core_affinity::set_for_current(core);
                        }
                        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        else {
                            return;
                        };
                        rt.block_on(worker_loop(std_listener, make, make_fallback, cfg, rx));
                    })?,
            );
        }

        for h in handles {
            let _ = h.join();
        }
        Ok(())
    }
}

/// One worker's accept loop.
async fn worker_loop<F, S, G, H>(
    std_listener: std::net::TcpListener,
    make: F,
    make_fallback: G,
    cfg: Config,
    mut rx: watch::Receiver<bool>,
) where
    F: Fn() -> S,
    S: H1Service + 'static,
    G: Fn() -> H,
    H: H2Fallback + 'static,
{
    std_listener.set_nonblocking(true).ok();
    let Ok(listener) = TcpListener::from_std(std_listener) else {
        return;
    };

    // Per-core state: created once here, shared by every connection on this
    // thread, and never touched by another. No atomics, no locks.
    let conn_cfg = Rc::new(ConnConfig {
        limits: cfg.limits.clone(),
        tick: cfg.tick,
        server_name: cfg.server_name.clone(),
    });
    let date = Rc::new(RefCell::new(DateCache::new()));
    let service = Rc::new(make());
    let fallback = Rc::new(make_fallback());

    let local = tokio::task::LocalSet::new();

    local
        .run_until(async {
            loop {
                tokio::select! {
                    _ = rx.changed() => {
                        if *rx.borrow() {
                            break;
                        }
                    }
                    accepted = listener.accept() => {
                        let Ok((stream, _peer)) = accepted else { continue };
                        if cfg.tcp.nodelay {
                            let _ = stream.set_nodelay(true);
                        }
                        let conn_cfg = conn_cfg.clone();
                        let date = date.clone();
                        let service = service.clone();
                        let fallback = fallback.clone();
                        let cfg = cfg.clone();
                        tokio::task::spawn_local(async move {
                            dispatch(stream, service, fallback, conn_cfg, date, cfg).await;
                        });
                    }
                }
            }
        })
        .await;

    // Drain: give in-flight connections a bounded window to finish.
    let _ = tokio::time::timeout(cfg.shutdown_grace, local).await;
}

/// Decide what protocol a connection speaks, then serve or hand it off.
///
/// With TLS off and h2c detection off — the default — this reduces to serving
/// HTTP/1 directly, with no extra read on the fast path.
async fn dispatch<S, H>(
    stream: tokio::net::TcpStream,
    service: Rc<S>,
    fallback: Rc<H>,
    conn_cfg: Rc<ConnConfig>,
    date: Rc<RefCell<DateCache>>,
    cfg: Config,
) where
    S: H1Service + 'static,
    H: H2Fallback + 'static,
{
    #[cfg(feature = "tls")]
    if let Some(tls) = cfg.tls.clone() {
        let acceptor = tokio_rustls::TlsAcceptor::from(tls);
        let Ok(tls_stream) = acceptor.accept(stream).await else {
            // A failed handshake is not a protocol error we can report over
            // HTTP; the peer gets a TLS alert from rustls and the socket closes.
            return;
        };
        let is_h2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");
        if is_h2 {
            // Nothing has been read past the handshake, so there is no buffered
            // application data to forward.
            fallback.handle(Box::new(tls_stream), Bytes::new()).await;
        } else {
            serve_h1(tls_stream, service, conn_cfg, date, Bytes::new()).await;
        }
        return;
    }

    if cfg.detect_h2c {
        match peek_preface(stream, &cfg).await {
            Some((stream, buffered, Preface::Http2)) => {
                // The preface is part of the HTTP/2 stream and cannot be re-read
                // from the socket, so it must travel with the connection.
                fallback.handle(Box::new(stream), buffered).await;
            }
            Some((stream, buffered, _)) => {
                serve_h1(stream, service, conn_cfg, date, buffered).await;
            }
            None => {}
        }
        return;
    }

    serve_h1(stream, service, conn_cfg, date, Bytes::new()).await;
}

/// Read just enough to classify a plaintext connection.
///
/// Returns `None` if the peer closed or errored before deciding. `NeedMore` at
/// EOF is reported as `Http1`, so a client that opens a connection and says
/// nothing is handled by the HTTP/1 path's idle timeout rather than being routed
/// to a fallback that has no bytes to work with.
async fn peek_preface(
    mut stream: tokio::net::TcpStream,
    cfg: &Config,
) -> Option<(tokio::net::TcpStream, Bytes, Preface)> {
    let mut buf = BytesMut::with_capacity(crate::tls::H2C_PREFACE.len());
    loop {
        match is_h2c_preface(&buf) {
            Preface::NeedMore => {}
            decided => return Some((stream, buf.freeze(), decided)),
        }
        let deadline = tokio::time::timeout(cfg.limits.header_timeout, stream.read_buf(&mut buf));
        match deadline.await {
            Ok(Ok(0)) => {
                // EOF mid-prefix: let the HTTP/1 path deal with it.
                return Some((stream, buf.freeze(), Preface::Http1));
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => return None,
        }
    }
}

/// Serve one HTTP/1 connection to completion.
async fn serve_h1<IO, S>(
    io: IO,
    service: Rc<S>,
    conn_cfg: Rc<ConnConfig>,
    date: Rc<RefCell<DateCache>>,
    buffered: Bytes,
) where
    IO: AsyncRead + AsyncWrite + Unpin + 'static,
    S: H1Service + 'static,
{
    let conn = Connection::with_buffered(io, RcService(service), conn_cfg, date, buffered);
    // A clean close and an I/O error are the same outcome here: the connection is
    // over and there is nobody left to tell.
    //
    // An upgraded transport is dropped, which closes the socket. `Server` has no
    // upgrade consumer to hand it to — unlike the HTTP/2 fallback, which is
    // pluggable — so a service that answers 101 must drive `Connection` itself
    // and take the `Upgraded` from `Connection::serve`. Closing is the honest
    // outcome for a handoff this server cannot complete; the alternative is to
    // leave a socket open that nothing will ever read.
    if let Ok(Some(upgraded)) = conn.serve().await {
        drop(upgraded);
    }
}

/// An [`H2Fallback`] that closes the connection.
///
/// The default. Closing is the honest outcome: mis-serving an HTTP/2 stream
/// through an HTTP/1 parser would produce nonsense, and there is nothing useful
/// to reply with over a protocol we do not speak.
#[derive(Clone, Copy, Debug, Default)]
pub struct CloseH2;

impl H2Fallback for CloseH2 {
    fn handle(&self, io: Box<dyn Transport>, buffered: Bytes) -> Pin<Box<dyn Future<Output = ()>>> {
        Box::pin(async move {
            drop(buffered);
            drop(io);
        })
    }
}

/// Shares one service across every connection on a worker.
struct RcService<S>(Rc<S>);

impl<S: H1Service> H1Service for RcService<S> {
    type Future = S::Future;

    #[inline]
    fn call(&self, req: crate::Request) -> Self::Future {
        self.0.call(req)
    }
}

/// Whether this platform load-balances accepts across `SO_REUSEPORT` sockets.
fn reuse_port_supported() -> bool {
    cfg!(all(
        unix,
        not(target_os = "solaris"),
        not(target_os = "illumos")
    ))
}

/// Bind one listener.
fn bind_one(
    addr: SocketAddr,
    tcp: &TcpConfig,
    reuse_port: bool,
) -> io::Result<std::net::TcpListener> {
    let domain = match addr {
        SocketAddr::V4(_) => socket2::Domain::IPV4,
        SocketAddr::V6(_) => socket2::Domain::IPV6,
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    socket.set_reuse_address(true)?;

    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    if reuse_port {
        socket.set_reuse_port(true)?;
    }
    #[cfg(not(all(unix, not(target_os = "solaris"), not(target_os = "illumos"))))]
    let _ = reuse_port;

    // Nagle is disabled on each accepted stream in `worker_loop`, which is where
    // it actually governs response latency.
    socket.bind(&addr.into())?;
    socket.listen(tcp.backlog)?;
    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Request, Response};
    use std::net::{Ipv4Addr, SocketAddrV4};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn loopback() -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
    }

    fn test_config(workers: usize) -> Config {
        // Short deadlines so a test that ends on a timeout finishes in
        // milliseconds rather than waiting out the production default.
        let limits = Limits {
            idle_timeout: Duration::from_millis(300),
            header_timeout: Duration::from_millis(300),
            ..Default::default()
        };
        Config::new(loopback())
            .workers(workers)
            .limits(limits)
            // Pinning is pointless in a test and fails inside some containers.
            .pin_cores(false)
    }

    async fn hello(_req: Request) -> Response {
        Response::text("hi")
    }

    /// Send one request over a fresh connection and return the response bytes.
    async fn request(addr: SocketAddr, raw: &[u8]) -> io::Result<String> {
        let mut s = tokio::net::TcpStream::connect(addr).await?;
        s.write_all(raw).await?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).await?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n";

    /// Run `body` against a live server, then shut it down.
    fn with_server<Fut, T>(
        workers: usize,
        body: impl FnOnce(SocketAddr) -> Fut + Send + 'static,
    ) -> T
    where
        Fut: std::future::Future<Output = T>,
        T: Send + 'static,
    {
        let server = Server::bind(test_config(workers)).expect("bind");
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

        server.serve(|| hello).expect("serve");
        client.join().expect("client thread")
    }

    #[test]
    fn binds_and_serves_on_an_ephemeral_port() {
        let out = with_server(1, |addr| async move {
            assert_ne!(addr.port(), 0, "port 0 must resolve to a real port");
            request(addr, GET).await.unwrap()
        });
        assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
        assert!(out.ends_with("hi"), "{out}");
    }

    #[test]
    fn serves_concurrent_connections() {
        let results = with_server(2, |addr| async move {
            let mut set = tokio::task::JoinSet::new();
            for _ in 0..64 {
                set.spawn(async move { request(addr, GET).await });
            }
            let mut ok = 0;
            while let Some(r) = set.join_next().await {
                if r.unwrap().unwrap().starts_with("HTTP/1.1 200 OK") {
                    ok += 1;
                }
            }
            ok
        });
        assert_eq!(results, 64);
    }

    #[test]
    fn serves_across_multiple_workers() {
        let results = with_server(4, |addr| async move {
            let mut ok = 0;
            for _ in 0..40 {
                if request(addr, GET)
                    .await
                    .unwrap()
                    .starts_with("HTTP/1.1 200 OK")
                {
                    ok += 1;
                }
            }
            ok
        });
        assert_eq!(results, 40);
    }

    #[test]
    fn single_worker_config_works() {
        let out = with_server(1, |addr| async move { request(addr, GET).await.unwrap() });
        assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
    }

    #[test]
    fn shutdown_stops_accepting() {
        let server = Server::bind(test_config(1)).expect("bind");
        let addr = server.local_addr();
        let handle = server.handle();

        let client = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            // One good request proves the server is up.
            let first = rt.block_on(request(addr, GET));
            assert!(first.unwrap().starts_with("HTTP/1.1 200 OK"));

            handle.shutdown();
            assert!(handle.is_shutting_down());
            // Idempotent.
            handle.shutdown();

            // After shutdown, connections stop being served. Whether the OS
            // refuses the connect or accepts it into a closed backlog is
            // platform-dependent, so assert on the absence of a response rather
            // than on a specific errno.
            std::thread::sleep(Duration::from_millis(300));
            rt.block_on(async {
                match tokio::time::timeout(Duration::from_millis(500), request(addr, GET)).await {
                    Err(_) => true,
                    Ok(Err(_)) => true,
                    Ok(Ok(body)) => body.is_empty(),
                }
            })
        });

        server.serve(|| hello).expect("serve");
        assert!(
            client.join().unwrap(),
            "no request may be served after shutdown"
        );
    }

    #[test]
    fn config_defaults_are_sane() {
        let c = Config::new(loopback());
        assert!(c.workers >= 1);
        assert!(c.tcp.nodelay);
        assert!(c.tcp.reuse_port);
        assert_eq!(c.tick, Duration::from_millis(100));
        assert_eq!(c.shutdown_grace, Duration::from_secs(10));
        assert_eq!(Config::new(loopback()).workers(0).workers, 1, "never zero");
    }

    #[test]
    fn bind_reports_the_resolved_port() {
        let s = Server::bind(test_config(3)).expect("bind");
        let addr = s.local_addr();
        assert_ne!(addr.port(), 0);
        // Every per-worker listener must share the one resolved port, or three
        // workers would be listening on three different ports.
        if let Listeners::PerWorker(v) = &s.listeners {
            for l in v {
                assert_eq!(l.local_addr().unwrap().port(), addr.port());
            }
        }
    }
}
