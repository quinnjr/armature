//! Request, response, body, and the service trait.
//!
//! Nothing here carries a `Send` bound. That is the point of the thread-per-core
//! model: a connection never migrates cores, so requiring handler futures to be
//! `Send` would buy nothing and cost the ability to hold `Rc` and `RefCell` in
//! per-core state.

use crate::chunked::{ChunkEvent, ChunkedDecoder, ChunkedError};
use crate::header::{HeaderId, HeaderVec};
use crate::{BodyKind, Head};
use bytes::Bytes;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite};

/// Any byte stream a connection can be served over.
///
/// Object-safe so that [`Upgraded`] and [`Response`] need not be generic over
/// the transport, which would otherwise infect the service trait with a type
/// parameter every handler would have to name.
pub trait Transport: AsyncRead + AsyncWrite + Unpin {}

impl<T: AsyncRead + AsyncWrite + Unpin> Transport for T {}

/// A connection handed off after a successful protocol upgrade.
pub struct Upgraded {
    /// The raw transport, positioned after the upgrade request's head.
    pub io: Box<dyn Transport>,
    /// Bytes already read past the head.
    ///
    /// The upgrade consumer **must** process these before reading `io`.
    /// Discarding them silently loses the first frames the peer sent, which is a
    /// data-loss bug that only shows up under load, so this field is public and
    /// its contract stated here rather than left implicit.
    pub buffered: Bytes,
}

impl std::fmt::Debug for Upgraded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upgraded")
            .field("buffered", &self.buffered.len())
            .finish_non_exhaustive()
    }
}

/// A failure while reading a request body.
#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    /// The chunked coding was malformed or oversized.
    #[error("chunked body error: {0}")]
    Chunked(#[from] ChunkedError),
    /// The transport failed.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    /// The peer closed before the declared body arrived.
    ///
    /// Distinct from a successful short read on purpose: silently handing a
    /// handler a truncated body is how a partial request gets processed as if it
    /// were whole.
    #[error("connection closed before the declared body arrived")]
    Incomplete,
    /// The body exceeded the cap given to [`Body::collect`].
    #[error("body exceeded the requested cap")]
    TooLarge,
}

impl BodyError {
    /// The status code to answer with.
    pub fn status(&self) -> u16 {
        match self {
            BodyError::Chunked(e) => e.status(),
            BodyError::TooLarge => 413,
            BodyError::Incomplete => 400,
            BodyError::Io(_) => 400,
        }
    }
}

/// The connection's side of body reading.
///
/// Implemented by the connection over its concrete transport, so [`Body`] can
/// stay free of a type parameter. HTTP/1 serializes a request against its
/// response — the connection loop awaits the handler future — so while a handler
/// holds a `Body`, nothing else touches the transport.
pub trait BodyIo {
    /// Read more bytes from the transport, returning what arrived.
    ///
    /// An empty return means EOF.
    fn poll_fill(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Bytes>>;

    /// Flush a pending `Expect: 100-continue` interim response.
    ///
    /// Called on the first body read and nowhere else, which is what makes the
    /// interim response lazy: a handler that rejects a request without reading
    /// its body never causes one to be sent.
    fn poll_send_continue(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>>;

    /// Take up to `max` already-buffered bytes.
    ///
    /// The body pulls from the connection's single buffer rather than being
    /// handed a private copy of it. That distinction is load-bearing: bytes past
    /// the end of this body belong to the *next* pipelined request, and a body
    /// that swallowed them would make the following request vanish.
    fn take_buffered(&mut self, max: usize) -> Bytes;

    /// Return bytes the body read but did not consume.
    ///
    /// Used after a chunked body's terminating chunk, where the decoder may have
    /// been handed bytes belonging to the next request.
    fn push_back(&mut self, bytes: Bytes);
}

/// Shared body-reading state.
struct BodyState {
    kind: BodyKind,
    /// Bytes read past the head but not yet consumed by the body.
    buffered: Bytes,
    /// Bytes still expected, for a `Content-Length` body.
    remaining: u64,
    decoder: Option<ChunkedDecoder>,
    trailers: Option<HeaderVec>,
    io: Option<Rc<RefCell<dyn BodyIo>>>,
    continue_sent: bool,
    needs_continue: bool,
    done: bool,
    /// Shared with the connection, set when the body is fully consumed.
    ///
    /// The connection must know this: if a handler returns without reading the
    /// body, the bytes still on the wire are body bytes, and treating them as
    /// the next request line is precisely the smuggling scenario this crate
    /// exists to prevent.
    fully_read: Rc<Cell<bool>>,
}

impl BodyState {
    /// Mark the body complete, publishing that fact to the connection.
    ///
    /// An error path calls this too: a body that failed is not "fully read", but
    /// it is finished, and the connection closes on error regardless.
    #[inline]
    fn finish(&mut self) {
        self.done = true;
        self.fully_read.set(true);
    }

    /// Mark the body finished *without* claiming it was fully read.
    ///
    /// An errored body leaves an unknown number of bytes on the wire, so the
    /// connection must close rather than try to find the next request line in
    /// them. Keeping this distinct from [`finish`](Self::finish) is what stops a
    /// handler that swallows a body error from silently enabling connection
    /// reuse over undelimited bytes.
    #[inline]
    fn fail(&mut self) {
        self.done = true;
        self.fully_read.set(false);
    }
}

/// A request body.
///
/// Chunks are [`Bytes`] slices of the connection's read buffer, so reading a
/// body copies nothing.
pub struct Body {
    state: BodyState,
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Body")
            .field("kind", &self.state.kind)
            .field("done", &self.state.done)
            .finish_non_exhaustive()
    }
}

impl Body {
    /// A body with no content.
    pub fn empty() -> Self {
        Self {
            state: BodyState {
                kind: BodyKind::None,
                buffered: Bytes::new(),
                remaining: 0,
                decoder: None,
                trailers: Some(HeaderVec::new()),
                io: None,
                continue_sent: true,
                needs_continue: false,
                done: true,
                fully_read: Rc::new(Cell::new(true)),
            },
        }
    }

    /// A body of `kind`, reading through `io`.
    ///
    /// `needs_continue` requests a lazy `100 Continue`, sent on the first read.
    /// The body pulls already-buffered bytes from `io` on demand rather than
    /// taking a copy, so bytes belonging to the next pipelined request stay where
    /// the connection can still see them.
    pub fn new(
        kind: BodyKind,
        io: Rc<RefCell<dyn BodyIo>>,
        needs_continue: bool,
        limits: &crate::Limits,
        fully_read: Rc<Cell<bool>>,
    ) -> Self {
        let (remaining, decoder, done) = match kind {
            BodyKind::None => (0, None, true),
            BodyKind::Length(n) => (n, None, n == 0),
            BodyKind::Chunked => (0, Some(ChunkedDecoder::new(limits)), false),
        };
        fully_read.set(done);
        Self {
            state: BodyState {
                kind,
                buffered: Bytes::new(),
                remaining,
                decoder,
                trailers: if done { Some(HeaderVec::new()) } else { None },
                io: Some(io),
                continue_sent: !needs_continue,
                needs_continue,
                done,
                fully_read,
            },
        }
    }

    /// A body over a fixed byte string, for tests and synthetic requests.
    pub fn from_bytes(data: Bytes) -> Self {
        let len = data.len() as u64;
        Self {
            state: BodyState {
                kind: BodyKind::Length(len),
                buffered: data,
                remaining: len,
                decoder: None,
                trailers: Some(HeaderVec::new()),
                io: None,
                continue_sent: true,
                needs_continue: false,
                done: len == 0,
                fully_read: Rc::new(Cell::new(true)),
            },
        }
    }

    /// How this body is framed.
    #[inline]
    pub fn kind(&self) -> BodyKind {
        self.state.kind
    }

    /// Whether the body is fully read.
    #[inline]
    pub fn is_end(&self) -> bool {
        self.state.done
    }

    /// Whether a `100 Continue` was requested and not yet sent.
    #[inline]
    pub fn continue_pending(&self) -> bool {
        self.state.needs_continue && !self.state.continue_sent
    }

    /// Whether the body was read to its end cleanly.
    ///
    /// False after an error, and false if the handler simply never read it. The
    /// connection uses this to decide whether reuse is safe.
    #[inline]
    pub fn was_fully_read(&self) -> bool {
        self.state.fully_read.get()
    }

    /// The trailer section, once the body is fully read.
    #[inline]
    pub fn trailers(&self) -> Option<&HeaderVec> {
        self.state.trailers.as_ref()
    }

    /// Poll for the next body chunk.
    pub fn poll_chunk(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Bytes, BodyError>>> {
        let s = &mut self.state;

        if s.done {
            return Poll::Ready(None);
        }

        // Lazily flush the interim response before the first byte is read.
        if !s.continue_sent {
            // The RefMut is scoped tightly so the error arm can touch `s` again.
            let sent = match &s.io {
                Some(io) => io.borrow_mut().poll_send_continue(cx),
                None => Poll::Ready(Ok(())),
            };
            match sent {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => {
                    s.fail();
                    return Poll::Ready(Some(Err(BodyError::Io(e))));
                }
                Poll::Ready(Ok(())) => {}
            }
            s.continue_sent = true;
        }

        loop {
            match s.kind {
                BodyKind::None => {
                    s.finish();
                    return Poll::Ready(None);
                }

                BodyKind::Length(_) => {
                    if s.buffered.is_empty() && s.remaining > 0 {
                        // Take at most what this body is owed, leaving anything
                        // beyond it for the next request.
                        let want = usize::try_from(s.remaining).unwrap_or(usize::MAX);
                        if let Some(io) = &s.io {
                            let got = io.borrow_mut().take_buffered(want);
                            if !got.is_empty() {
                                s.buffered = got;
                            }
                        }
                    }
                    if !s.buffered.is_empty() {
                        let take = std::cmp::min(s.buffered.len() as u64, s.remaining) as usize;
                        let chunk = s.buffered.split_to(take);
                        s.remaining -= take as u64;
                        if s.remaining == 0 {
                            s.finish();
                            s.trailers = Some(HeaderVec::new());
                        }
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                    if s.remaining == 0 {
                        s.finish();
                        s.trailers = Some(HeaderVec::new());
                        return Poll::Ready(None);
                    }
                    match Self::fill(s, cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(e)) => {
                            s.fail();
                            return Poll::Ready(Some(Err(e)));
                        }
                        Poll::Ready(Ok(false)) => {
                            // EOF with bytes still owed. Never report success on
                            // a truncated body.
                            s.fail();
                            return Poll::Ready(Some(Err(BodyError::Incomplete)));
                        }
                        Poll::Ready(Ok(true)) => continue,
                    }
                }

                BodyKind::Chunked => {
                    if s.buffered.is_empty()
                        && let Some(io) = &s.io
                    {
                        let got = io.borrow_mut().take_buffered(usize::MAX);
                        if !got.is_empty() {
                            s.buffered = got;
                        }
                    }
                    let decoder = s.decoder.as_mut().expect("chunked body has a decoder");
                    match decoder.poll(&mut s.buffered) {
                        Err(e) => {
                            s.fail();
                            return Poll::Ready(Some(Err(BodyError::Chunked(e))));
                        }
                        Ok(Some(ChunkEvent::Data(d))) => return Poll::Ready(Some(Ok(d))),
                        Ok(Some(ChunkEvent::Trailers(t))) => {
                            s.trailers = Some(*t);
                            continue;
                        }
                        Ok(Some(ChunkEvent::End)) => {
                            s.finish();
                            if s.trailers.is_none() {
                                s.trailers = Some(HeaderVec::new());
                            }
                            // Anything left belongs to the next pipelined
                            // request; give it back rather than dropping it.
                            if !s.buffered.is_empty() {
                                let leftover = std::mem::take(&mut s.buffered);
                                if let Some(io) = &s.io {
                                    io.borrow_mut().push_back(leftover);
                                }
                            }
                            return Poll::Ready(None);
                        }
                        Ok(None) => match Self::fill(s, cx) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(Err(e)) => {
                                s.fail();
                                return Poll::Ready(Some(Err(e)));
                            }
                            Poll::Ready(Ok(false)) => {
                                s.fail();
                                return Poll::Ready(Some(Err(BodyError::Incomplete)));
                            }
                            Poll::Ready(Ok(true)) => continue,
                        },
                    }
                }
            }
        }
    }

    /// Read more bytes into `buffered`. `Ok(false)` means EOF.
    fn fill(s: &mut BodyState, cx: &mut Context<'_>) -> Poll<Result<bool, BodyError>> {
        let Some(io) = &s.io else {
            return Poll::Ready(Ok(false));
        };
        match io.borrow_mut().poll_fill(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(BodyError::Io(e))),
            Poll::Ready(Ok(more)) => {
                if more.is_empty() {
                    return Poll::Ready(Ok(false));
                }
                if s.buffered.is_empty() {
                    s.buffered = more;
                } else {
                    // Only reached when a chunk straddles two reads.
                    let mut joined = Vec::with_capacity(s.buffered.len() + more.len());
                    joined.extend_from_slice(&s.buffered);
                    joined.extend_from_slice(&more);
                    s.buffered = Bytes::from(joined);
                }
                Poll::Ready(Ok(true))
            }
        }
    }

    /// The next body chunk, or `None` at the end.
    pub async fn chunk(&mut self) -> Option<Result<Bytes, BodyError>> {
        std::future::poll_fn(|cx| self.poll_chunk(cx)).await
    }

    /// Read the whole body, refusing to buffer more than `cap` bytes.
    ///
    /// The cap is enforced as bytes accumulate, not against the declared length,
    /// so a body that lies about its size cannot drive unbounded buffering.
    pub async fn collect(&mut self, cap: u64) -> Result<Bytes, BodyError> {
        let mut acc: Option<Vec<u8>> = None;
        let mut first: Option<Bytes> = None;
        let mut total: u64 = 0;

        while let Some(chunk) = self.chunk().await {
            let chunk = chunk?;
            total += chunk.len() as u64;
            if total > cap {
                return Err(BodyError::TooLarge);
            }
            match (&mut acc, &first) {
                // Single-chunk bodies — the common case — pass straight through
                // without a copy.
                (None, None) => first = Some(chunk),
                (None, Some(_)) => {
                    let f = first.take().expect("checked");
                    let mut v = Vec::with_capacity(f.len() + chunk.len());
                    v.extend_from_slice(&f);
                    v.extend_from_slice(&chunk);
                    acc = Some(v);
                }
                (Some(v), _) => v.extend_from_slice(&chunk),
            }
        }

        Ok(match acc {
            Some(v) => Bytes::from(v),
            None => first.unwrap_or_default(),
        })
    }
}

/// An incoming request.
pub struct Request {
    /// The parsed request line and headers.
    pub head: Head,
    /// The request body.
    pub body: Body,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.head.method)
            .field("target", &self.head.target)
            .finish_non_exhaustive()
    }
}

/// A response body.
///
/// Note the absence of `Send` on the streaming variant.
pub enum ResponseBody {
    /// No body.
    Empty,
    /// A complete body.
    Full(Bytes),
    /// A body produced incrementally.
    Stream(Pin<Box<dyn futures_stream::Stream>>),
}

/// A minimal local stream abstraction.
///
/// Defined here rather than depending on `futures` so the crate keeps a small
/// dependency surface; the connection only ever polls it.
pub mod futures_stream {
    use super::{BodyError, Bytes, Context, Poll};

    /// An incrementally-produced response body.
    pub trait Stream {
        /// Poll for the next piece, or `None` when finished.
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Bytes, BodyError>>>;
    }
}

impl std::fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResponseBody::Empty => f.write_str("Empty"),
            ResponseBody::Full(b) => write!(f, "Full({} bytes)", b.len()),
            ResponseBody::Stream(_) => f.write_str("Stream"),
        }
    }
}

/// An outgoing response.
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Response headers. Framing, `Date`, and `Connection` are added by the
    /// writer unless set here.
    pub headers: HeaderVec,
    /// The response body.
    pub body: ResponseBody,
    /// Set alongside status 101 to take over the connection.
    pub upgrade: Option<Box<dyn FnOnce(Upgraded)>>,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("body", &self.body)
            .field("upgrade", &self.upgrade.is_some())
            .finish()
    }
}

impl Response {
    /// An empty response with `status`.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: HeaderVec::new(),
            body: ResponseBody::Empty,
            upgrade: None,
        }
    }

    /// An empty 200.
    pub fn ok() -> Self {
        Self::new(200)
    }

    /// An empty response with `status`, for error replies.
    pub fn status_only(status: u16) -> Self {
        Self::new(status)
    }

    /// A `text/plain` 200.
    pub fn text(body: &'static str) -> Self {
        let mut r = Self::new(200);
        r.headers.push((
            HeaderId::ContentType,
            Bytes::from_static(b"text/plain; charset=utf-8"),
        ));
        r.body = ResponseBody::Full(Bytes::from_static(body.as_bytes()));
        r
    }

    /// An `application/json` 200.
    pub fn json(body: Bytes) -> Self {
        let mut r = Self::new(200);
        r.headers.push((
            HeaderId::ContentType,
            Bytes::from_static(b"application/json"),
        ));
        r.body = ResponseBody::Full(body);
        r
    }

    /// Add a header.
    pub fn header(mut self, id: HeaderId, value: Bytes) -> Self {
        self.headers.push((id, value));
        self
    }

    /// Replace the body.
    pub fn with_body(mut self, body: ResponseBody) -> Self {
        self.body = body;
        self
    }
}

/// Something that turns a [`Request`] into a [`Response`].
///
/// Deliberately without a `Send` bound: under thread-per-core a connection never
/// migrates, so handlers may hold `Rc` and `RefCell` freely.
pub trait H1Service {
    /// The future returned by [`call`](Self::call).
    type Future: Future<Output = Response>;

    /// Handle one request.
    fn call(&self, req: Request) -> Self::Future;
}

impl<F, Fut> H1Service for F
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Response>,
{
    type Future = Fut;

    #[inline]
    fn call(&self, req: Request) -> Fut {
        (self)(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    /// A `BodyIo` that hands out canned reads and records interim responses.
    struct MockIo {
        reads: Vec<Bytes>,
        buffered: Bytes,
        continues: usize,
    }

    impl MockIo {
        fn with_buffered(buffered: &'static [u8], reads: Vec<&'static [u8]>) -> Rc<RefCell<Self>> {
            Rc::new(RefCell::new(Self {
                reads: reads.into_iter().rev().map(Bytes::from_static).collect(),
                buffered: Bytes::from_static(buffered),
                continues: 0,
            }))
        }
    }

    impl BodyIo for MockIo {
        fn poll_fill(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<Bytes>> {
            Poll::Ready(Ok(self.reads.pop().unwrap_or_default()))
        }

        fn poll_send_continue(&mut self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.continues += 1;
            Poll::Ready(Ok(()))
        }

        fn take_buffered(&mut self, max: usize) -> Bytes {
            let n = self.buffered.len().min(max);
            self.buffered.split_to(n)
        }

        fn push_back(&mut self, bytes: Bytes) {
            let mut joined = Vec::with_capacity(bytes.len() + self.buffered.len());
            joined.extend_from_slice(&bytes);
            joined.extend_from_slice(&self.buffered);
            self.buffered = Bytes::from(joined);
        }
    }

    fn body(kind: BodyKind, buffered: &'static [u8], reads: Vec<&'static [u8]>) -> Body {
        Body::new(
            kind,
            MockIo::with_buffered(buffered, reads),
            false,
            &Limits::default(),
            Rc::new(Cell::new(false)),
        )
    }

    #[tokio::test]
    async fn empty_body_ends_immediately() {
        let mut b = Body::empty();
        assert!(b.is_end());
        assert!(b.chunk().await.is_none());
        assert_eq!(b.collect(1024).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn fixed_body_yields_declared_bytes() {
        let mut b = body(BodyKind::Length(5), b"hello", vec![]);
        assert_eq!(&b.collect(1024).await.unwrap()[..], b"hello");
        assert!(b.is_end());
    }

    #[tokio::test]
    async fn fixed_body_reads_across_fills() {
        let mut b = body(BodyKind::Length(10), b"hel", vec![b"lo wor", b"ld"]);
        assert_eq!(&b.collect(1024).await.unwrap()[..], &b"hello world"[..10]);
    }

    /// A truncated body must never be reported as a success.
    #[tokio::test]
    async fn fixed_body_short_read_is_incomplete() {
        let mut b = body(BodyKind::Length(10), b"hel", vec![]);
        let err = b.collect(1024).await.unwrap_err();
        assert!(matches!(err, BodyError::Incomplete), "got {err:?}");
    }

    #[tokio::test]
    async fn chunked_body_yields_chunks_and_trailers() {
        let mut b = body(
            BodyKind::Chunked,
            b"5\r\nhello\r\n0\r\nEtag: x\r\n\r\n",
            vec![],
        );
        assert_eq!(&b.collect(1024).await.unwrap()[..], b"hello");
        let t = b.trailers().expect("trailers after end");
        assert_eq!(crate::header::get_str(t, &HeaderId::Etag), Some("x"));
    }

    #[tokio::test]
    async fn chunked_body_reads_across_fills() {
        let mut b = body(BodyKind::Chunked, b"5\r\nhel", vec![b"lo\r\n0\r\n\r\n"]);
        assert_eq!(&b.collect(1024).await.unwrap()[..], b"hello");
    }

    #[tokio::test]
    async fn chunked_error_surfaces() {
        let mut b = body(BodyKind::Chunked, b"zz\r\n", vec![]);
        let err = b.collect(1024).await.unwrap_err();
        assert!(matches!(err, BodyError::Chunked(_)), "got {err:?}");
        assert_eq!(err.status(), 400);
    }

    #[tokio::test]
    async fn collect_enforces_its_cap() {
        let mut b = body(BodyKind::Length(5), b"hello", vec![]);
        let err = b.collect(4).await.unwrap_err();
        assert!(matches!(err, BodyError::TooLarge));
        assert_eq!(err.status(), 413);
    }

    /// The cap applies to bytes actually accumulated, so a lying
    /// `Content-Length` cannot drive unbounded buffering.
    #[tokio::test]
    async fn collect_ignores_a_lying_content_length() {
        let mut b = body(BodyKind::Length(5), b"hello world, much longer", vec![]);
        // Declared 5, so only 5 are read regardless of what arrived.
        assert_eq!(&b.collect(1024).await.unwrap()[..], b"hello");
    }

    /// The interim response is sent on the first read and nowhere else.
    #[tokio::test]
    async fn continue_is_sent_lazily_on_first_read() {
        let io = MockIo::with_buffered(b"hello", vec![]);
        let mut b = Body::new(
            BodyKind::Length(5),
            io.clone(),
            true,
            &Limits::default(),
            Rc::new(Cell::new(false)),
        );
        assert!(b.continue_pending());
        assert_eq!(io.borrow().continues, 0, "nothing sent before a read");

        b.collect(1024).await.unwrap();
        assert_eq!(io.borrow().continues, 1, "sent exactly once");
        assert!(!b.continue_pending());
    }

    /// A handler that rejects a request without reading its body must not cause
    /// an interim response — that is the whole value of lazy 100-continue.
    #[tokio::test]
    async fn continue_is_not_sent_when_body_is_ignored() {
        let io = MockIo::with_buffered(b"hello", vec![]);
        let b = Body::new(
            BodyKind::Length(5),
            io.clone(),
            true,
            &Limits::default(),
            Rc::new(Cell::new(false)),
        );
        drop(b);
        assert_eq!(io.borrow().continues, 0);
    }

    #[test]
    fn response_builders_set_expected_fields() {
        let r = Response::text("hi");
        assert_eq!(r.status, 200);
        assert!(matches!(r.body, ResponseBody::Full(_)));
        assert_eq!(
            crate::header::get_str(&r.headers, &HeaderId::ContentType),
            Some("text/plain; charset=utf-8")
        );

        let r = Response::json(Bytes::from_static(b"{}"));
        assert_eq!(
            crate::header::get_str(&r.headers, &HeaderId::ContentType),
            Some("application/json")
        );

        let r = Response::status_only(404);
        assert_eq!(r.status, 404);
        assert!(matches!(r.body, ResponseBody::Empty));

        let r = Response::ok().header(HeaderId::Etag, Bytes::from_static(b"v1"));
        assert_eq!(
            crate::header::get_str(&r.headers, &HeaderId::Etag),
            Some("v1")
        );
    }

    /// The load-bearing compile-time check: if anyone reintroduces a `Send`
    /// bound anywhere in the service chain, this stops compiling.
    #[tokio::test]
    async fn service_future_need_not_be_send() {
        use std::cell::Cell;

        let counter = Rc::new(Cell::new(0u32));
        let svc = {
            let counter = counter.clone();
            move |_req: Request| {
                let counter = counter.clone();
                async move {
                    // An Rc<Cell> captured across an await point makes this
                    // future decisively !Send.
                    counter.set(counter.get() + 1);
                    tokio::task::yield_now().await;
                    counter.set(counter.get() + 1);
                    Response::ok()
                }
            }
        };

        let req = Request {
            head: crate::parse_head(
                &Bytes::from_static(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n"),
                &Limits::default(),
            )
            .unwrap()
            .unwrap()
            .0,
            body: Body::empty(),
        };

        // Fully qualified: on a closure, `.call()` would collide with the
        // unstable `Fn::call`.
        let resp = H1Service::call(&svc, req).await;
        assert_eq!(resp.status, 200);
        assert_eq!(counter.get(), 2);
    }

    #[tokio::test]
    async fn from_bytes_body_round_trips() {
        let mut b = Body::from_bytes(Bytes::from_static(b"payload"));
        assert_eq!(&b.collect(1024).await.unwrap()[..], b"payload");
    }
}
