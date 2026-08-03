//! The per-connection state machine.
//!
//! One `Connection` owns one transport and serves requests on it until the peer
//! closes, a deadline expires, framing goes wrong, or the connection is upgraded.
//!
//! The controlling rule: **any framing rejection closes the connection.** The
//! stream is never resynchronized after a malformed or ambiguous request,
//! because the bytes following one are exactly what a smuggling attack wants
//! interpreted as a fresh request.

use crate::deadline::ConnDeadline;
use crate::header::{self, HeaderId, HeaderVec};
use crate::service::{BodyIo, H1Service, ResponseBody, Upgraded};
use crate::write::{self, DateCache, OutBody, ResponseHead};
use crate::{Body, Limits, Method, Request, Response, Version, framing, parse, parse_head};
use bytes::{Bytes, BytesMut};
use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncRead, AsyncWrite};

/// How much to read per syscall.
const READ_CHUNK: usize = 8 * 1024;

/// The interim response for `Expect: 100-continue`.
const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

/// Per-connection configuration.
#[derive(Clone, Debug)]
pub struct ConnConfig {
    /// Resource limits and deadlines.
    pub limits: Limits,
    /// Deadline coarsening granularity.
    pub tick: Duration,
    /// Value for the `Server` field, or none to omit it.
    pub server_name: Option<Bytes>,
}

impl Default for ConnConfig {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            tick: Duration::from_millis(100),
            server_name: None,
        }
    }
}

/// What to do with the connection after a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Serve another request.
    KeepAlive,
    /// Close.
    Close,
    /// Hand the transport to an upgrade consumer.
    Upgrade,
}

/// Transport plus read buffer, shared between the serve loop and request bodies.
///
/// Shared through `Rc<RefCell<_>>` rather than split, because HTTP/1 serializes
/// a request against its response: while a handler holds a body, the loop is
/// awaiting that handler and touches nothing.
struct IoState<IO> {
    io: IO,
    /// Unconsumed bytes: a partial head, or body bytes read past one.
    buf: BytesMut,
    /// A `100 Continue` owed to the peer but not yet written.
    pending_continue: bool,
}

impl<IO: AsyncRead + AsyncWrite + Unpin> IoState<IO> {
    /// Read once into `buf`, appending. `Ok(0)` means EOF.
    fn poll_read_more(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        let start = self.buf.len();
        // Grow with zeroed bytes so the read target is an initialized slice.
        // Reading into uninitialized capacity would need `unsafe`, which this
        // crate forbids; the cost is one memset per syscall, which is small
        // against the syscall itself. `tokio_util::io::poll_read_buf` is the
        // alternative if a benchmark ever shows this mattering.
        self.buf.resize(start + READ_CHUNK, 0);
        let mut read_buf = tokio::io::ReadBuf::new(&mut self.buf[start..]);
        let poll = std::pin::Pin::new(&mut self.io).poll_read(cx, &mut read_buf);
        let filled = read_buf.filled().len();
        match poll {
            Poll::Pending => {
                self.buf.truncate(start);
                Poll::Pending
            }
            Poll::Ready(Err(e)) => {
                self.buf.truncate(start);
                Poll::Ready(Err(e))
            }
            Poll::Ready(Ok(())) => {
                self.buf.truncate(start + filled);
                Poll::Ready(Ok(filled))
            }
        }
    }
}

impl<IO: AsyncRead + AsyncWrite + Unpin> BodyIo for IoState<IO> {
    fn poll_fill(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Bytes>> {
        let before = self.buf.len();
        match self.poll_read_more(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(0)) => Poll::Ready(Ok(Bytes::new())),
            Poll::Ready(Ok(_)) => {
                // Hand the new bytes to the body as a slice of this same
                // allocation.
                let fresh = self.buf.split_off(before).freeze();
                Poll::Ready(Ok(fresh))
            }
        }
    }

    fn take_buffered(&mut self, max: usize) -> Bytes {
        let n = self.buf.len().min(max);
        self.buf.split_to(n).freeze()
    }

    fn push_back(&mut self, bytes: Bytes) {
        if bytes.is_empty() {
            return;
        }
        // Prepend: these bytes precede whatever is already buffered.
        let mut joined = BytesMut::with_capacity(bytes.len() + self.buf.len());
        joined.extend_from_slice(&bytes);
        joined.extend_from_slice(&self.buf);
        self.buf = joined;
    }

    fn poll_send_continue(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.pending_continue {
            return Poll::Ready(Ok(()));
        }
        match std::pin::Pin::new(&mut self.io).poll_write(cx, CONTINUE) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(_)) => {
                self.pending_continue = false;
                Poll::Ready(Ok(()))
            }
        }
    }
}

/// A single HTTP/1 connection.
pub struct Connection<IO, S> {
    shared: Rc<RefCell<IoState<IO>>>,
    service: S,
    cfg: Rc<ConnConfig>,
    date: Rc<RefCell<DateCache>>,
    deadline: ConnDeadline,
    out: BytesMut,
    /// Reused across requests: allocating a fresh flag per request would put an
    /// allocation back on the steady-state path.
    body_read: Rc<Cell<bool>>,
}

impl<IO, S> Connection<IO, S>
where
    IO: AsyncRead + AsyncWrite + Unpin + 'static,
    S: H1Service,
{
    /// Wrap `io`, serving requests through `service`.
    ///
    /// Must be called inside a tokio runtime context: the connection's reusable
    /// deadline timer is allocated here.
    pub fn new(io: IO, service: S, cfg: Rc<ConnConfig>, date: Rc<RefCell<DateCache>>) -> Self {
        Self::with_buffered(io, service, cfg, date, Bytes::new())
    }

    /// Wrap `io` with `buffered` bytes already read from it.
    ///
    /// Used by protocol dispatch, which must read the first bytes of a connection
    /// to decide whether this crate should serve it at all. Those bytes are part
    /// of the request and cannot be re-read from the socket, so they are handed
    /// back here rather than dropped.
    pub fn with_buffered(
        io: IO,
        service: S,
        cfg: Rc<ConnConfig>,
        date: Rc<RefCell<DateCache>>,
        buffered: Bytes,
    ) -> Self {
        let deadline = ConnDeadline::new(cfg.tick);
        let mut buf = BytesMut::with_capacity(READ_CHUNK.max(buffered.len()));
        buf.extend_from_slice(&buffered);
        Self {
            shared: Rc::new(RefCell::new(IoState {
                io,
                buf,
                pending_continue: false,
            })),
            service,
            cfg,
            date,
            deadline,
            out: BytesMut::with_capacity(1024),
            body_read: Rc::new(Cell::new(false)),
        }
    }

    /// Serve requests until the connection ends.
    ///
    /// `Ok(Some(_))` means the connection was upgraded and the caller must hand
    /// it to the upgrade consumer. This return value is the *only* way an
    /// upgraded transport leaves the crate: a handler signals an upgrade by
    /// answering a request that asked for one with status 101, and whoever drove
    /// this `Connection` receives the socket here. [`crate::Server`] has no
    /// upgrade consumer to hand it to and closes instead, so a service that
    /// upgrades must drive `Connection` itself.
    pub async fn serve(mut self) -> io::Result<Option<Upgraded>> {
        loop {
            // Wait for the next request to begin. An idle keep-alive connection
            // that never sends again is closed silently — no response is owed.
            match self.read_until_head_or_idle().await? {
                HeadOutcome::Eof => return Ok(None),
                HeadOutcome::IdleTimeout => return Ok(None),
                HeadOutcome::HeaderTimeout => {
                    self.write_error(Version::Http11, 408).await?;
                    return Ok(None);
                }
                HeadOutcome::Failed(status) => {
                    self.write_error(Version::Http11, status).await?;
                    return Ok(None);
                }
                HeadOutcome::Ready => {}
            }

            // Split the head off rather than cloning the buffer. `BytesMut::clone`
            // would copy every head byte and allocate to do it; `split_to` hands
            // back a view of the same allocation, and the remainder stays put for
            // the body or the next pipelined request.
            let head_bytes = {
                let mut st = self.shared.borrow_mut();
                let end = parse::find_head_end(&st.buf).unwrap_or(st.buf.len());
                st.buf.split_to(end).freeze()
            };
            let head = match parse_head(&head_bytes, &self.cfg.limits) {
                Ok(Some((head, _))) => head,
                // `read_until_head_or_idle` only returns Ready once a terminator
                // is present, so this is unreachable in practice.
                Ok(None) => return Ok(None),
                Err(e) => {
                    self.write_error(Version::Http11, e.status()).await?;
                    return Ok(None);
                }
            };

            let version = head.version;
            let keep_alive = head.is_keep_alive();

            // Framing before anything else touches the body.
            let kind = match framing::decide(&head, &self.cfg.limits) {
                Ok(k) => k,
                Err(e) => {
                    self.write_error(version, e.status()).await?;
                    return Ok(None);
                }
            };

            let wants_upgrade =
                head.count(&HeaderId::Upgrade) > 0 && head.connection_has_token("upgrade");
            let expects_continue = head
                .get_str(&HeaderId::Expect)
                .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"));
            let is_head_request = head.method == Method::Head;

            self.shared.borrow_mut().pending_continue = expects_continue;

            let dyn_io: Rc<RefCell<dyn BodyIo>> = self.shared.clone();
            // Shared with the body so the loop can tell, after the handler
            // returns, whether the body was consumed to its end.
            self.body_read.set(false);
            let body_read = self.body_read.clone();
            let body = Body::new(
                kind,
                dyn_io,
                expects_continue,
                &self.cfg.limits,
                body_read.clone(),
            );

            // The body deadline covers the handler's read of it. Nothing else
            // polls this deadline, so the handler call itself has to race it:
            // arming a timer nobody awaits would leave a handler blocked on body
            // bytes the peer never sends holding the connection forever.
            //
            // Cancelling the handler mid-await drops its `Body`, and with it the
            // last borrow of the transport, so the 408 can be written here.
            let call = self.service.call(Request { head, body });
            self.deadline.arm(self.cfg.limits.body_timeout);
            let resp = {
                let deadline = &mut self.deadline;
                tokio::select! {
                    biased;
                    () = deadline.expired() => None,
                    r = call => Some(r),
                }
            };
            self.deadline.disarm();
            let Some(resp) = resp else {
                self.write_error(version, 408).await?;
                return Ok(None);
            };

            // An unconsumed or errored body leaves an unknown number of bytes on
            // the wire. Reusing the connection would mean looking for a request
            // line inside a message body — the smuggling scenario itself — so
            // reuse requires that the body reached its end cleanly.
            //
            // Draining the remainder instead would preserve keep-alive for
            // handlers that ignore bodies, but it hands an attacker a way to make
            // the server read bytes it has no use for. Closing is the safe
            // default; a handler that wants keep-alive on a request with a body
            // must read that body.
            let body_consumed = body_read.get();
            let keep_alive = keep_alive && body_consumed;

            let disposition = self
                .write_response(version, resp, keep_alive, is_head_request, wants_upgrade)
                .await?;

            match disposition {
                Disposition::KeepAlive => continue,
                Disposition::Close => return Ok(None),
                Disposition::Upgrade => {
                    let (io, buffered) = self.into_parts();
                    return Ok(Some(Upgraded {
                        io: Box::new(io),
                        buffered,
                    }));
                }
            }
        }
    }

    /// Consume the connection, yielding the transport and unread bytes.
    fn into_parts(self) -> (IO, Bytes) {
        let shared = self.shared;
        let state = Rc::try_unwrap(shared)
            .map(RefCell::into_inner)
            .unwrap_or_else(|_| unreachable!("body handles are dropped before upgrade"));
        (state.io, state.buf.freeze())
    }

    /// Read until a complete head is buffered, or a deadline or EOF intervenes.
    async fn read_until_head_or_idle(&mut self) -> io::Result<HeadOutcome> {
        // A complete head may already be buffered from a pipelined write.
        if self.has_complete_head() {
            return Ok(HeadOutcome::Ready);
        }

        let had_bytes = !self.shared.borrow().buf.is_empty();
        // Idle applies before the first byte; the header deadline applies once
        // the request has begun. Conflating them would let a client hold a
        // connection open indefinitely mid-head.
        self.deadline.arm(if had_bytes {
            self.cfg.limits.header_timeout
        } else {
            self.cfg.limits.idle_timeout
        });
        let mut started = had_bytes;

        loop {
            let read = {
                let shared = self.shared.clone();
                let deadline = &mut self.deadline;
                tokio::select! {
                    biased;
                    () = deadline.expired() => None,
                    r = std::future::poll_fn(|cx| shared.borrow_mut().poll_read_more(cx)) => Some(r),
                }
            };

            match read {
                None => {
                    return Ok(if started {
                        HeadOutcome::HeaderTimeout
                    } else {
                        HeadOutcome::IdleTimeout
                    });
                }
                Some(Err(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(HeadOutcome::Eof);
                }
                Some(Err(e)) => return Err(e),
                Some(Ok(0)) => return Ok(HeadOutcome::Eof),
                Some(Ok(_)) => {
                    if !started {
                        started = true;
                        self.deadline.arm(self.cfg.limits.header_timeout);
                    }
                    if self.has_complete_head() {
                        return Ok(HeadOutcome::Ready);
                    }
                    // Refuse to buffer an unterminated head forever.
                    if self.shared.borrow().buf.len() > self.cfg.limits.max_head_bytes {
                        return Ok(HeadOutcome::Failed(431));
                    }
                }
            }
        }
    }

    fn has_complete_head(&self) -> bool {
        parse::find_head_end(&self.shared.borrow().buf).is_some()
    }

    /// Serialize and write a response, returning what to do next.
    async fn write_response(
        &mut self,
        version: Version,
        resp: Response,
        req_keep_alive: bool,
        is_head_request: bool,
        wants_upgrade: bool,
    ) -> io::Result<Disposition> {
        // Status 101 alone does not take the connection: the peer must have
        // asked for the upgrade, or the transport would be handed off to a
        // protocol the client is not speaking.
        let upgrading = resp.status == 101 && wants_upgrade;
        // An unread request body means the next bytes on the wire are body
        // bytes, not a request line. Rather than guess where the body ended,
        // close.
        let keep_alive = req_keep_alive && !upgrading;

        let Response {
            status,
            mut headers,
            body,
        } = resp;

        if let Some(name) = &self.cfg.server_name
            && header::get(&headers, &HeaderId::Server).is_none()
        {
            headers.push((HeaderId::Server, name.clone()));
        }

        let out_body = match &body {
            ResponseBody::Empty => OutBody::None,
            ResponseBody::Full(b) => OutBody::Fixed(b.clone()),
            ResponseBody::Stream(_) => OutBody::Chunked,
        };

        self.out.clear();
        {
            let mut date = self.date.borrow_mut();
            let now = SystemTime::now();
            let date_bytes = date.get(now);
            write::write_head(
                &mut self.out,
                version,
                &ResponseHead { status, headers },
                &out_body,
                date_bytes,
                keep_alive,
            );
        }

        // A HEAD response carries the headers a GET would, and no body
        // (RFC 9112 section 6.3). 204, 304 and 1xx likewise forbid one, and the
        // writer already suppresses their framing field — so writing a body
        // anyway would put bytes on the wire that no `Content-Length` or chunk
        // framing accounts for, and a keep-alive peer reads them as the start of
        // the next response. A handler that attaches a body to a 304 should lose
        // the body, not desync the connection.
        let body_forbidden = matches!(status, 204 | 304) || (100..200).contains(&status);
        if !is_head_request && !upgrading && !body_forbidden {
            match body {
                ResponseBody::Empty => {}
                ResponseBody::Full(b) => self.out.extend_from_slice(&b),
                ResponseBody::Stream(mut s) => {
                    // Every write in the stream loop is deadline-guarded, not
                    // just the last one: a peer that stops reading mid-stream
                    // would otherwise pin the connection and its buffer
                    // indefinitely. The deadline is re-armed per flush, so a
                    // slow but progressing consumer is never cut off.
                    //
                    // Flush the head first so the peer can begin processing.
                    self.flush().await?;
                    loop {
                        let next = std::future::poll_fn(|cx| s.as_mut().poll_next(cx)).await;
                        match next {
                            None => break,
                            Some(Ok(chunk)) => {
                                self.out.clear();
                                write::write_chunk(&mut self.out, &chunk);
                                self.flush().await?;
                            }
                            Some(Err(_)) => {
                                // The body failed mid-stream. The head is
                                // already sent, so the only honest signal left
                                // is to close without a terminating chunk.
                                return Ok(Disposition::Close);
                            }
                        }
                    }
                    self.out.clear();
                    write::write_last_chunk(&mut self.out, &HeaderVec::new());
                }
            }
        }

        self.flush().await?;

        if upgrading {
            return Ok(Disposition::Upgrade);
        }

        Ok(if keep_alive {
            Disposition::KeepAlive
        } else {
            Disposition::Close
        })
    }

    /// Write the pending buffer under the write deadline, then clear it.
    ///
    /// The deadline is armed here rather than by the caller so that every write
    /// on the response path is covered. A streaming body flushes once per chunk,
    /// and guarding only the final flush would leave all the others unbounded —
    /// which is exactly the position a peer that stops reading mid-stream puts
    /// the connection in.
    async fn flush(&mut self) -> io::Result<()> {
        self.deadline.arm(self.cfg.limits.write_timeout);
        let flushed = tokio::select! {
            biased;
            () = self.deadline.expired() => Err(io::Error::from(io::ErrorKind::TimedOut)),
            r = Self::write_all_from(&self.shared, &self.out) => r,
        };
        self.deadline.disarm();
        self.out.clear();
        flushed
    }

    /// Write `out` in full, then flush, then clear it for reuse.
    ///
    /// Writes straight from the buffer rather than `split()`ing it off. `split`
    /// leaves the buffer with zero capacity, so the next response would have to
    /// reallocate — one allocation per request, purely to avoid a borrow.
    ///
    /// Each `borrow_mut` is scoped to a single poll rather than held across the
    /// await. Holding it across would deadlock the moment anything else — a body
    /// read, say — needed the same transport, and that is the kind of latent
    /// hazard that only surfaces under a specific interleaving.
    async fn write_all_from(shared: &Rc<RefCell<IoState<IO>>>, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut written = 0;
        while written < bytes.len() {
            let n = std::future::poll_fn(|cx| {
                let mut st = shared.borrow_mut();
                std::pin::Pin::new(&mut st.io).poll_write(cx, &bytes[written..])
            })
            .await?;
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::WriteZero));
            }
            written += n;
        }
        std::future::poll_fn(|cx| {
            let mut st = shared.borrow_mut();
            std::pin::Pin::new(&mut st.io).poll_flush(cx)
        })
        .await
    }

    /// Write a bare error response and close.
    ///
    /// Always `Connection: close`: a connection whose framing we do not fully
    /// agree on is never reused.
    async fn write_error(&mut self, version: Version, status: u16) -> io::Result<()> {
        self.out.clear();
        {
            let mut date = self.date.borrow_mut();
            let now = SystemTime::now();
            let date_bytes = date.get(now);
            write::write_head(
                &mut self.out,
                version,
                &ResponseHead {
                    status,
                    headers: HeaderVec::new(),
                },
                &OutBody::None,
                date_bytes,
                false,
            );
        }
        // Best effort: the peer may already be gone, and there is nothing
        // further to do about it either way.
        let _ = Self::write_all_from(&self.shared, &self.out).await;
        self.out.clear();
        Ok(())
    }
}

/// Why head reading stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeadOutcome {
    /// A complete head is buffered.
    Ready,
    /// The peer closed cleanly between requests.
    Eof,
    /// No request began within the idle deadline.
    IdleTimeout,
    /// A request began but its head never completed.
    HeaderTimeout,
    /// The head was rejected before parsing, with this status.
    Failed(u16),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::HeaderId;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Shorten the idle and header deadlines so a test that ends on a timeout
    /// finishes in milliseconds rather than waiting out the 75s production
    /// default.
    fn quick(mut limits: Limits) -> Limits {
        limits.idle_timeout = Duration::from_millis(200);
        limits.header_timeout = Duration::from_millis(200);
        limits
    }

    fn cfg(limits: Limits) -> Rc<ConnConfig> {
        Rc::new(ConnConfig {
            limits: quick(limits),
            tick: Duration::from_millis(10),
            server_name: None,
        })
    }

    /// Drive a connection with `input`, returning everything written back.
    ///
    /// Runs on a `LocalSet` because nothing in this crate is `Send` — which is
    /// itself part of what these tests hold in place.
    async fn exchange<S>(input: &'static [u8], service: S, limits: Limits) -> String
    where
        S: H1Service + 'static,
    {
        exchange_raw(input, service, limits).await.0
    }

    async fn exchange_raw<S>(input: &'static [u8], service: S, limits: Limits) -> (String, bool)
    where
        S: H1Service + 'static,
    {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let local = tokio::task::LocalSet::new();

        let conn = Connection::new(
            server,
            service,
            cfg(limits),
            Rc::new(RefCell::new(DateCache::new())),
        );
        let server_task = local.spawn_local(async move { conn.serve().await });

        local
            .run_until(async move {
                client.write_all(input).await.unwrap();
                let mut out = Vec::new();
                // Read to EOF or until the server stops writing.
                let read =
                    tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut out))
                        .await;
                let closed = matches!(read, Ok(Ok(_)));
                let _ = server_task.await;
                (String::from_utf8_lossy(&out).into_owned(), closed)
            })
            .await
    }

    async fn ok_service(_req: Request) -> Response {
        Response::text("hi")
    }

    async fn echo_service(mut req: Request) -> Response {
        match req.body.collect(1024 * 1024).await {
            Ok(b) => Response::ok().with_body(ResponseBody::Full(b)),
            Err(e) => Response::status_only(e.status()),
        }
    }

    /// A handler that never touches the body — the common 404-on-POST shape.
    async fn ignore_body_service(_req: Request) -> Response {
        Response::status_only(404)
    }

    #[tokio::test]
    async fn serves_a_single_request() {
        let out = exchange(
            b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"), "{out}");
        assert!(out.contains("content-length: 2\r\n"), "{out}");
        assert!(out.ends_with("hi"), "{out}");
    }

    #[tokio::test]
    async fn keeps_the_connection_alive_for_sequential_requests() {
        // Both requests are written up front; the second is served on the same
        // connection.
        let out = exchange(
            b"GET /a HTTP/1.1\r\nHost: a\r\n\r\nGET /b HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert_eq!(out.matches("HTTP/1.1 200 OK").count(), 2, "{out}");
    }

    #[tokio::test]
    async fn serves_pipelined_requests_in_order() {
        async fn echo_path(req: Request) -> Response {
            let mut r = Response::new(200);
            r.body = ResponseBody::Full(Bytes::from(req.head.path().to_owned()));
            r
        }
        let out = exchange(
            b"GET /1 HTTP/1.1\r\nHost: a\r\n\r\nGET /2 HTTP/1.1\r\nHost: a\r\n\r\nGET /3 HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            echo_path,
            Limits::default(),
        )
        .await;
        let p1 = out.find("/1").expect("first response");
        let p2 = out.find("/2").expect("second response");
        let p3 = out.find("/3").expect("third response");
        assert!(
            p1 < p2 && p2 < p3,
            "responses must be in request order: {out}"
        );
    }

    #[tokio::test]
    async fn closes_on_connection_close() {
        let (out, closed) = exchange_raw(
            b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert!(out.contains("connection: close"), "{out}");
        assert!(closed, "server must close the socket");
    }

    #[tokio::test]
    async fn http_10_closes_by_default() {
        let out = exchange(b"GET / HTTP/1.0\r\n\r\n", ok_service, Limits::default()).await;
        assert!(out.starts_with("HTTP/1.0 200 OK"), "{out}");
        assert!(out.contains("connection: close"), "{out}");
    }

    #[tokio::test]
    async fn echoes_a_content_length_body() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            echo_service,
            Limits::default(),
        )
        .await;
        assert!(out.ends_with("hello"), "{out}");
    }

    #[tokio::test]
    async fn echoes_a_chunked_body() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
            echo_service,
            Limits::default(),
        )
        .await;
        assert!(out.ends_with("hello"), "{out}");
    }

    #[tokio::test]
    async fn head_response_has_headers_but_no_body() {
        let out = exchange(
            b"HEAD / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert!(
            out.contains("content-length: 2"),
            "HEAD keeps the length a GET would report: {out}"
        );
        assert!(out.ends_with("\r\n\r\n"), "no body may follow: {out}");
    }

    #[tokio::test]
    async fn parse_error_yields_400_and_closes() {
        // Bare LF: rejected by the prescan.
        let (out, closed) = exchange_raw(
            b"GET / HTTP/1.1\nHost: a\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 400 Bad Request"), "{out}");
        assert!(out.contains("connection: close"), "{out}");
        assert!(closed);
    }

    #[tokio::test]
    async fn framing_conflict_yields_400_and_closes() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\nhello",
            echo_service,
            Limits::default(),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 400 Bad Request"), "{out}");
        assert!(out.contains("connection: close"), "{out}");
    }

    #[tokio::test]
    async fn missing_host_yields_400() {
        let out = exchange(b"GET / HTTP/1.1\r\n\r\n", ok_service, Limits::default()).await;
        assert!(out.starts_with("HTTP/1.1 400 Bad Request"), "{out}");
    }

    #[tokio::test]
    async fn oversized_declared_body_yields_413() {
        let limits = Limits {
            max_body_bytes: 2,
            ..Default::default()
        };
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\nhello",
            echo_service,
            limits,
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 413"), "{out}");
    }

    #[tokio::test]
    async fn unsupported_version_yields_505() {
        let out = exchange(
            b"GET / HTTP/1.2\r\nHost: a\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 505"), "{out}");
    }

    /// The bytes after a rejected request must never be read as a new request.
    #[tokio::test]
    async fn does_not_resynchronize_after_a_framing_error() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\nhelloGET /second HTTP/1.1\r\nHost: a\r\n\r\n",
            ok_service,
            Limits::default(),
        )
        .await;
        assert_eq!(
            out.matches("HTTP/1.1").count(),
            1,
            "exactly one response; the trailing request must not be served: {out}"
        );
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    }

    /// A handler that ignores the body leaves undelimited bytes on the wire, so
    /// the connection must not be reused.
    #[tokio::test]
    async fn unread_body_forces_a_close() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\nhelloGET / HTTP/1.1\r\nHost: a\r\n\r\n",
            ignore_body_service,
            Limits::default(),
        )
        .await;
        assert_eq!(
            out.matches("HTTP/1.1").count(),
            1,
            "the unread body must not be mined for a second request: {out}"
        );
        assert!(out.contains("connection: close"), "{out}");
    }

    #[tokio::test]
    async fn sends_100_continue_when_body_is_read() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nExpect: 100-continue\r\nConnection: close\r\n\r\nhello",
            echo_service,
            Limits::default(),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 100 Continue\r\n\r\n"), "{out}");
        assert!(out.contains("HTTP/1.1 200 OK"), "{out}");
    }

    /// The value of lazy 100-continue: a rejection costs no interim response and
    /// no body transfer.
    #[tokio::test]
    async fn omits_100_continue_when_body_is_ignored() {
        let out = exchange(
            b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nExpect: 100-continue\r\n\r\nhello",
            ignore_body_service,
            Limits::default(),
        )
        .await;
        assert!(!out.contains("100 Continue"), "{out}");
        assert_eq!(out.matches("HTTP/1.1").count(), 1, "{out}");
        assert!(out.starts_with("HTTP/1.1 404"), "{out}");
    }

    #[tokio::test]
    async fn oversized_head_yields_431() {
        let limits = Limits {
            max_head_bytes: 40,
            ..Default::default()
        };
        let out = exchange(
            b"GET /aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HTTP/1.1\r\nHost: a\r\n\r\n",
            ok_service,
            limits,
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 431"), "{out}");
    }

    #[tokio::test]
    async fn server_name_is_emitted_when_configured() {
        let (mut client, server) = tokio::io::duplex(4096);
        let local = tokio::task::LocalSet::new();
        let conn = Connection::new(
            server,
            ok_service,
            Rc::new(ConnConfig {
                limits: Limits::default(),
                tick: Duration::from_millis(10),
                server_name: Some(Bytes::from_static(b"armature-h1")),
            }),
            Rc::new(RefCell::new(DateCache::new())),
        );
        let task = local.spawn_local(async move { conn.serve().await });
        let out = local
            .run_until(async move {
                client
                    .write_all(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                let mut out = Vec::new();
                let _ = tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut out))
                    .await;
                let _ = task.await;
                String::from_utf8_lossy(&out).into_owned()
            })
            .await;
        assert!(out.contains("server: armature-h1"), "{out}");
    }

    #[tokio::test]
    async fn response_headers_from_the_handler_are_emitted() {
        async fn tagged(_req: Request) -> Response {
            Response::ok().header(HeaderId::Etag, Bytes::from_static(b"v1"))
        }
        let out = exchange(
            b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n",
            tagged,
            Limits::default(),
        )
        .await;
        assert!(out.contains("etag: v1"), "{out}");
    }

    /// A peer that stops reading mid-stream must not pin the connection and its
    /// buffer. Every flush in the streaming loop is deadline-guarded, so this
    /// fails if the guard is ever narrowed back to the final write.
    #[tokio::test]
    async fn write_timeout_cuts_off_a_stalled_streamed_body() {
        const CHUNK: &[u8] = &[b'x'; 1024];

        struct Endless;

        impl crate::service::futures_stream::Stream for Endless {
            fn poll_next(
                self: std::pin::Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<Option<Result<Bytes, crate::BodyError>>> {
                Poll::Ready(Some(Ok(Bytes::from_static(CHUNK))))
            }
        }

        async fn streamer(_req: Request) -> Response {
            Response::ok().with_body(ResponseBody::Stream(Box::pin(Endless)))
        }

        let limits = Limits {
            write_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        // Small enough that the unread transport backs up within a chunk or two.
        let (client, server) = tokio::io::duplex(256);
        let local = tokio::task::LocalSet::new();
        let conn = Connection::new(
            server,
            streamer,
            cfg(limits),
            Rc::new(RefCell::new(DateCache::new())),
        );
        let task = local.spawn_local(async move { conn.serve().await });

        let served = local
            .run_until(async move {
                let mut client = client;
                client
                    .write_all(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n")
                    .await
                    .unwrap();
                // Deliberately never read. The client half is held open so the
                // failure comes from the deadline rather than from a closed peer.
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .expect("the connection must not hang on a peer that stopped reading")
                    .expect("join")
            })
            .await;

        assert_eq!(
            served.expect_err("a stalled write must fail").kind(),
            io::ErrorKind::TimedOut
        );
    }

    /// A handler awaiting body bytes the peer never sends must be cut off. The
    /// deadline being armed is not enough — nothing else polls it, so the
    /// handler call has to lose a race against it.
    #[tokio::test]
    async fn body_timeout_yields_408_and_closes() {
        let limits = Limits {
            body_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let (mut client, server) = tokio::io::duplex(4096);
        let local = tokio::task::LocalSet::new();
        let conn = Connection::new(
            server,
            echo_service,
            cfg(limits),
            Rc::new(RefCell::new(DateCache::new())),
        );
        let task = local.spawn_local(async move { conn.serve().await });

        let out = local
            .run_until(async move {
                // Declares five body bytes and sends three, then never sends the
                // rest.
                client
                    .write_all(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\nhel")
                    .await
                    .unwrap();
                let mut out = Vec::new();
                let read =
                    tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut out))
                        .await;
                assert!(matches!(read, Ok(Ok(_))), "the server must close");
                let _ = task.await;
                String::from_utf8_lossy(&out).into_owned()
            })
            .await;

        assert!(out.starts_with("HTTP/1.1 408 Request Timeout"), "{out}");
        assert!(out.contains("connection: close"), "{out}");
    }

    /// Status 101 on a request that asked for an upgrade hands the transport
    /// back to whoever drove the connection, together with the bytes the peer
    /// already sent past the head. Those bytes are the first frames of the new
    /// protocol and cannot be re-read from the socket.
    #[tokio::test]
    async fn upgrade_hands_back_the_transport_and_buffered_bytes() {
        async fn switching(_req: Request) -> Response {
            Response::new(101)
                .header(HeaderId::Upgrade, Bytes::from_static(b"raw"))
                .header(HeaderId::Connection, Bytes::from_static(b"upgrade"))
        }

        let (mut client, server) = tokio::io::duplex(4096);
        let local = tokio::task::LocalSet::new();
        let conn = Connection::new(
            server,
            switching,
            cfg(Limits::default()),
            Rc::new(RefCell::new(DateCache::new())),
        );
        let task = local.spawn_local(async move { conn.serve().await });

        let (head, upgraded) = local
            .run_until(async move {
                client
                    .write_all(
                        b"GET / HTTP/1.1\r\nHost: a\r\nConnection: upgrade\r\nUpgrade: raw\r\n\r\nFIRSTFRAME",
                    )
                    .await
                    .unwrap();
                let mut buf = [0u8; 256];
                let n = client.read(&mut buf).await.unwrap();
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let upgraded = task.await.expect("join").expect("serve");
                (head, upgraded)
            })
            .await;

        assert!(
            head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
            "{head}"
        );
        assert!(
            !head.contains("content-length"),
            "101 frames no body: {head}"
        );
        let upgraded = upgraded.expect("the transport must be handed back");
        assert_eq!(
            &upgraded.buffered[..],
            b"FIRSTFRAME",
            "bytes past the head belong to the upgraded protocol"
        );
    }

    /// 101 alone must not take the connection. Handing the transport to a
    /// protocol the client never negotiated loses every subsequent request on
    /// it.
    #[tokio::test]
    async fn status_101_without_a_client_upgrade_request_does_not_upgrade() {
        async fn switching(_req: Request) -> Response {
            Response::new(101)
        }

        let (mut client, server) = tokio::io::duplex(4096);
        let local = tokio::task::LocalSet::new();
        let conn = Connection::new(
            server,
            switching,
            cfg(Limits::default()),
            Rc::new(RefCell::new(DateCache::new())),
        );
        let task = local.spawn_local(async move { conn.serve().await });

        let upgraded = local
            .run_until(async move {
                client
                    .write_all(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                let mut out = Vec::new();
                let _ = tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut out))
                    .await;
                task.await.expect("join").expect("serve")
            })
            .await;

        assert!(upgraded.is_none(), "no upgrade was negotiated");
    }

    /// `Connection` must never become `Send`. The per-core model rests on it:
    /// the `Rc` and `RefCell` in per-core state are only sound because nothing
    /// migrates threads. Asserting a *negative* impl on stable takes the
    /// ambiguity trick — if `Connection` were `Send`, both impls below would
    /// apply and this would stop compiling.
    #[test]
    fn connection_is_not_send() {
        trait AmbiguousIfSend<A> {
            fn assert() {}
        }
        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

        type Svc = fn(Request) -> std::future::Ready<Response>;
        let _ = <Connection<tokio::io::DuplexStream, Svc> as AmbiguousIfSend<_>>::assert;
    }

    #[tokio::test]
    async fn eof_between_requests_closes_silently() {
        let (out, _) = exchange_raw(b"", ok_service, Limits::default()).await;
        assert!(
            out.is_empty(),
            "no response is owed on a silent close: {out}"
        );
    }
}
