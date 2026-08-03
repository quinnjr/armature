//! Chunked transfer-coding decoder (RFC 9112 section 7.1).
//!
//! The decoder consumes from the front of a [`Bytes`] and emits [`Data`] events
//! that are slices of that same buffer, so decoding copies nothing.
//!
//! Line endings are strict CRLF here for the same reason they are in
//! [`crate::parse`]: a chunk-size line a peer reads differently from us is a
//! request-smuggling vector.
//!
//! [`Data`]: ChunkEvent::Data

use crate::header::{HeaderId, HeaderVec};
use crate::{ByteStr, Limits};
use bytes::{Buf, Bytes};

/// The maximum hex digits allowed in a chunk size.
///
/// A `u64` tops out at 16 hex digits; more than that cannot be represented and
/// is treated as an attack rather than rounded down.
const MAX_SIZE_DIGITS: usize = 16;

/// Something the decoder produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkEvent {
    /// Body bytes. May be a partial chunk when the whole chunk has not arrived.
    Data(Bytes),
    /// A non-empty trailer section, after the terminating zero-length chunk.
    ///
    /// **Only emitted when trailers are actually present.** An empty trailer
    /// section — the overwhelmingly common case — goes straight to [`End`], because
    /// boxing an empty `HeaderVec` to announce that there is nothing in it would
    /// put an allocation on the steady-state path for no information.
    ///
    /// Boxed because an inline [`HeaderVec`] is about a kilobyte, and paying that
    /// on every `Data` event would be backwards. Trailers are off the hot path;
    /// `Data` is not.
    ///
    /// [`End`]: ChunkEvent::End
    Trailers(Box<HeaderVec>),
    /// The body is complete. Terminal.
    End,
}

/// A malformed or oversized chunked body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChunkedError {
    /// A chunk size that was empty or not hexadecimal.
    #[error("malformed chunk size")]
    BadSize,
    /// A chunk size too large to represent.
    #[error("chunk size overflow")]
    SizeOverflow,
    /// A line not terminated by CRLF.
    #[error("missing CRLF")]
    MissingCrlf,
    /// The decoded body exceeded `Limits::max_body_bytes`.
    #[error("body too large")]
    BodyTooLarge,
    /// A malformed trailer field.
    #[error("malformed trailer field")]
    BadTrailer,
    /// A trailer field RFC 9110 section 6.5.1 forbids.
    #[error("forbidden trailer field")]
    ForbiddenTrailer,
    /// A chunk extension containing invalid bytes.
    #[error("malformed chunk extension")]
    BadExtension,
    /// The trailer section exceeded `Limits::max_head_bytes`.
    #[error("trailer section too large")]
    TrailerTooLarge,
}

impl ChunkedError {
    /// The status code to answer with before closing the connection.
    #[inline]
    pub fn status(&self) -> u16 {
        match self {
            ChunkedError::BodyTooLarge => 413,
            _ => 400,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Awaiting a chunk-size line.
    Size,
    /// Awaiting `remaining` more body bytes.
    Data,
    /// Awaiting the CRLF that follows chunk data.
    DataCrlf,
    /// Awaiting trailer lines.
    Trailers,
    /// Trailers emitted; only `End` remains.
    Ending,
    /// Terminal.
    Done,
}

/// An incremental chunked-body decoder.
///
/// Feed it bytes with [`poll`](Self::poll); it consumes what it can and returns
/// `Ok(None)` when it needs more.
#[derive(Debug)]
pub struct ChunkedDecoder {
    state: State,
    /// Body bytes still expected in the current chunk.
    remaining: u64,
    /// Total body bytes decoded so far.
    decoded: u64,
    max_body: u64,
    max_trailer: usize,
    trailer_bytes: usize,
    trailers: HeaderVec,
}

impl ChunkedDecoder {
    /// Create a decoder bounded by `limits`.
    pub fn new(limits: &Limits) -> Self {
        Self {
            state: State::Size,
            remaining: 0,
            decoded: 0,
            max_body: limits.max_body_bytes,
            max_trailer: limits.max_head_bytes,
            trailer_bytes: 0,
            trailers: HeaderVec::new(),
        }
    }

    /// Total body bytes decoded so far.
    ///
    /// Checked against `Limits::max_body_bytes` as data arrives, not merely as
    /// declared, so a body that lies about its chunk sizes is still bounded.
    #[inline]
    pub fn decoded_len(&self) -> u64 {
        self.decoded
    }

    /// Whether the body is complete.
    #[inline]
    pub fn is_done(&self) -> bool {
        self.state == State::Done
    }

    /// Consume from the front of `buf` and produce the next event.
    ///
    /// Returns `Ok(None)` when more input is needed. `Data` payloads are slices
    /// of `buf`, so no body byte is copied.
    pub fn poll(&mut self, buf: &mut Bytes) -> Result<Option<ChunkEvent>, ChunkedError> {
        loop {
            match self.state {
                State::Done => return Ok(Some(ChunkEvent::End)),

                State::Ending => {
                    self.state = State::Done;
                    return Ok(Some(ChunkEvent::End));
                }

                State::Size => {
                    let Some((content_len, consumed)) =
                        find_line(buf, MAX_SIZE_DIGITS + 256, ChunkedError::MissingCrlf)?
                    else {
                        return Ok(None);
                    };
                    let size = parse_chunk_size(&buf[..content_len])?;
                    buf.advance(consumed);

                    // The declared size is checked before any of it is read, so
                    // an oversized chunk is rejected without buffering it.
                    if self
                        .decoded
                        .checked_add(size)
                        .is_none_or(|total| total > self.max_body)
                    {
                        return Err(ChunkedError::BodyTooLarge);
                    }

                    if size == 0 {
                        self.state = State::Trailers;
                    } else {
                        self.remaining = size;
                        self.state = State::Data;
                    }
                }

                State::Data => {
                    if buf.is_empty() {
                        return Ok(None);
                    }
                    // Emit whatever has arrived, up to the rest of this chunk.
                    let take = std::cmp::min(buf.len() as u64, self.remaining) as usize;
                    let data = buf.slice(..take);
                    buf.advance(take);
                    self.remaining -= take as u64;
                    self.decoded += take as u64;
                    if self.remaining == 0 {
                        self.state = State::DataCrlf;
                    }
                    return Ok(Some(ChunkEvent::Data(data)));
                }

                State::DataCrlf => {
                    if buf.len() < 2 {
                        return Ok(None);
                    }
                    if &buf[..2] != b"\r\n" {
                        return Err(ChunkedError::MissingCrlf);
                    }
                    buf.advance(2);
                    self.state = State::Size;
                }

                State::Trailers => {
                    let Some((content_len, consumed)) =
                        find_line(buf, self.max_trailer, ChunkedError::TrailerTooLarge)?
                    else {
                        return Ok(None);
                    };

                    self.trailer_bytes += consumed;
                    if self.trailer_bytes > self.max_trailer {
                        return Err(ChunkedError::TrailerTooLarge);
                    }

                    if content_len == 0 {
                        // Empty line ends the trailer section.
                        buf.advance(consumed);
                        self.state = State::Ending;
                        if self.trailers.is_empty() {
                            // Nothing to report: skip the event rather than
                            // allocating a box to say so.
                            continue;
                        }
                        return Ok(Some(ChunkEvent::Trailers(Box::new(std::mem::take(
                            &mut self.trailers,
                        )))));
                    }

                    let line = buf.slice(..content_len);
                    buf.advance(consumed);
                    self.push_trailer(&line)?;
                }
            }
        }
    }

    /// Parse and record one trailer field.
    fn push_trailer(&mut self, line: &Bytes) -> Result<(), ChunkedError> {
        // obs-fold: a continuation line has no field name of its own.
        if matches!(line.first(), Some(b' ') | Some(b'\t')) {
            return Err(ChunkedError::BadTrailer);
        }

        let colon = memchr::memchr(b':', line).ok_or(ChunkedError::BadTrailer)?;
        let name = &line[..colon];
        if name.is_empty() {
            return Err(ChunkedError::BadTrailer);
        }
        // Whitespace between the field name and the colon is rejected in a
        // trailer for the same reason as in a header (RFC 9112 section 5.1).
        if name.last().is_some_and(|b| b.is_ascii_whitespace()) {
            return Err(ChunkedError::BadTrailer);
        }

        let id = match HeaderId::from_bytes(name) {
            Some(id) => id,
            None => {
                let lowered = if name.iter().any(|b| b.is_ascii_uppercase()) {
                    Bytes::from(name.to_ascii_lowercase())
                } else {
                    line.slice_ref(name)
                };
                HeaderId::Other(ByteStr::from_utf8(lowered).map_err(|_| ChunkedError::BadTrailer)?)
            }
        };

        // Framing was decided before this trailer was read, so honoring a
        // framing field from here would be a smuggling vector.
        if id.forbidden_in_trailers() {
            return Err(ChunkedError::ForbiddenTrailer);
        }

        // Trim optional whitespace around the value without copying.
        let mut start = colon + 1;
        let mut end = line.len();
        while start < end && (line[start] == b' ' || line[start] == b'\t') {
            start += 1;
        }
        while end > start && (line[end - 1] == b' ' || line[end - 1] == b'\t') {
            end -= 1;
        }

        self.trailers.push((id, line.slice(start..end)));
        Ok(())
    }
}

/// Locate a CRLF-terminated line at the front of `buf`.
///
/// Returns `(content_len, consumed)` where `content_len` excludes the CRLF and
/// `consumed` includes it. `Ok(None)` means the line is incomplete.
///
/// Strict CRLF: a bare LF is [`ChunkedError::MissingCrlf`], matching the policy
/// [`crate::parse::prescan`] applies to the message head.
///
/// `max` bounds the line's content and `too_long` is the verdict when it is
/// exceeded. The bound is enforced whether or not the terminator has arrived,
/// and both paths raise the same error, so the outcome cannot depend on how the
/// peer packetized its writes. Checking only the "no LF yet" path would invert
/// the protection: a line dribbled a byte at a time trips the limit while the
/// buffer grows, but the identical line delivered in one segment arrives with
/// its LF already present and skips the check entirely — and one segment is the
/// easy case for a sender, so the limit would constrain only the slow ones.
///
/// The caller names the error because the two line kinds answer differently: an
/// over-long size line is malformed framing, while an over-long trailer line is
/// the head-size budget running out.
fn find_line(
    buf: &Bytes,
    max: usize,
    too_long: ChunkedError,
) -> Result<Option<(usize, usize)>, ChunkedError> {
    match memchr::memchr(b'\n', buf) {
        None => {
            if buf.len() > max {
                return Err(too_long);
            }
            Ok(None)
        }
        Some(i) => {
            if i == 0 || buf[i - 1] != b'\r' {
                return Err(ChunkedError::MissingCrlf);
            }
            if i - 1 > max {
                return Err(too_long);
            }
            Ok(Some((i - 1, i + 1)))
        }
    }
}

/// Parse a chunk-size line: hex digits, optionally followed by `;extensions`.
fn parse_chunk_size(line: &[u8]) -> Result<u64, ChunkedError> {
    let digits = match memchr::memchr(b';', line) {
        Some(i) => {
            // Extensions are not interpreted, but they must be printable ASCII;
            // control bytes here mean the framing is not what we think it is.
            if line[i + 1..]
                .iter()
                .any(|&b| b < 0x20 && b != b'\t' || b == 0x7f)
            {
                return Err(ChunkedError::BadExtension);
            }
            &line[..i]
        }
        None => line,
    };

    if digits.is_empty() {
        return Err(ChunkedError::BadSize);
    }
    if digits.len() > MAX_SIZE_DIGITS {
        return Err(ChunkedError::SizeOverflow);
    }

    let mut size: u64 = 0;
    for &b in digits {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return Err(ChunkedError::BadSize),
        };
        size = size
            .checked_mul(16)
            .and_then(|s| s.checked_add(d as u64))
            .ok_or(ChunkedError::SizeOverflow)?;
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec() -> ChunkedDecoder {
        ChunkedDecoder::new(&Limits::default())
    }

    /// Drain every event the decoder can produce from `raw` in one pass.
    fn drain(d: &mut ChunkedDecoder, raw: &'static [u8]) -> Result<Vec<ChunkEvent>, ChunkedError> {
        let mut buf = Bytes::from_static(raw);
        let mut out = Vec::new();
        while let Some(ev) = d.poll(&mut buf)? {
            let end = ev == ChunkEvent::End;
            out.push(ev);
            if end {
                break;
            }
        }
        Ok(out)
    }

    fn data(s: &'static str) -> ChunkEvent {
        ChunkEvent::Data(Bytes::from_static(s.as_bytes()))
    }

    #[test]
    fn decodes_a_single_chunk() {
        let events = drain(&mut dec(), b"5\r\nhello\r\n0\r\n\r\n").unwrap();
        assert_eq!(events, vec![data("hello"), ChunkEvent::End]);
    }

    #[test]
    fn decodes_multiple_chunks() {
        let events = drain(&mut dec(), b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!(events, vec![data("abc"), data("de"), ChunkEvent::End]);
    }

    /// Data events must be slices of the input, not copies.
    #[test]
    fn data_events_share_the_input_buffer() {
        let raw = Bytes::from_static(b"5\r\nhello\r\n0\r\n\r\n");
        let base = raw.as_ptr() as usize;
        let mut buf = raw.clone();
        let mut d = dec();
        let ChunkEvent::Data(payload) = d.poll(&mut buf).unwrap().unwrap() else {
            panic!("expected Data");
        };
        let addr = payload.as_ptr() as usize;
        assert!(
            addr >= base && addr < base + raw.len(),
            "chunk data must point into the input buffer, not a copy"
        );
    }

    #[test]
    fn handles_split_across_reads() {
        let raw = b"5\r\nhello\r\n0\r\n\r\n";
        let mut d = dec();
        let mut buf = Bytes::new();
        let mut collected = Vec::new();
        let mut payload = Vec::new();

        for &byte in raw {
            // Append one byte at a time, mimicking a socket dribbling input.
            let mut next = Vec::from(&buf[..]);
            next.push(byte);
            buf = Bytes::from(next);
            while let Some(ev) = d.poll(&mut buf).unwrap() {
                if let ChunkEvent::Data(b) = &ev {
                    payload.extend_from_slice(b);
                }
                let end = ev == ChunkEvent::End;
                collected.push(ev);
                if end {
                    break;
                }
            }
        }

        assert_eq!(payload, b"hello");
        assert!(collected.contains(&ChunkEvent::End));
    }

    #[test]
    fn hex_size_is_case_insensitive() {
        let upper = drain(
            &mut dec(),
            b"1F\r\n0123456789012345678901234567890\r\n0\r\n\r\n",
        );
        let lower = drain(
            &mut dec(),
            b"1f\r\n0123456789012345678901234567890\r\n0\r\n\r\n",
        );
        assert_eq!(upper.unwrap(), lower.unwrap());
    }

    #[test]
    fn skips_chunk_extensions() {
        let events = drain(&mut dec(), b"5;name=value\r\nhello\r\n0\r\n\r\n").unwrap();
        assert_eq!(events[0], data("hello"));
    }

    /// The size-line bound must not depend on how the peer packetized its
    /// writes. Feeding an over-long chunk extension one byte at a time trips the
    /// limit while the buffer grows; delivering the identical bytes in a single
    /// segment must reach the same verdict, not sail past because the LF is
    /// already present when the line is first examined.
    #[test]
    fn over_long_chunk_extension_is_rejected_however_it_is_split() {
        let mut raw = Vec::from(*b"0;");
        raw.resize(2 + MAX_SIZE_DIGITS + 256 + 1, b'x');
        raw.extend_from_slice(b"\r\n\r\n");

        // `poll` reports `End` for as long as it is asked once the body is
        // complete, so both loops stop there rather than spinning on it.
        let whole = {
            let mut d = dec();
            let mut buf = Bytes::from(raw.clone());
            loop {
                match d.poll(&mut buf) {
                    Err(e) => break Err(e),
                    Ok(Some(ChunkEvent::End)) | Ok(None) => break Ok(()),
                    Ok(Some(_)) => {}
                }
            }
        };

        let dribbled = {
            let mut d = dec();
            let mut buf = Bytes::new();
            let mut fed = 0;
            loop {
                match d.poll(&mut buf) {
                    Err(e) => break Err(e),
                    Ok(Some(ChunkEvent::End)) => break Ok(()),
                    Ok(Some(_)) => continue,
                    Ok(None) => {}
                }
                if fed == raw.len() {
                    break Ok(());
                }
                let mut next = Vec::with_capacity(buf.len() + 1);
                next.extend_from_slice(&buf);
                next.push(raw[fed]);
                buf = Bytes::from(next);
                fed += 1;
            }
        };

        assert_eq!(
            dribbled,
            Err(ChunkedError::MissingCrlf),
            "a size line longer than the bound must be rejected"
        );
        assert_eq!(
            whole, dribbled,
            "the same bytes in one segment must reach the same verdict"
        );
    }

    #[test]
    fn parses_trailers() {
        let events = drain(&mut dec(), b"0\r\nEtag: x\r\n\r\n").unwrap();
        let ChunkEvent::Trailers(t) = &events[0] else {
            panic!("expected Trailers, got {:?}", events[0]);
        };
        assert_eq!(crate::header::get_str(t, &HeaderId::Etag), Some("x"));
    }

    #[test]
    fn rejects_non_hex_size() {
        assert_eq!(
            drain(&mut dec(), b"zz\r\nhello\r\n"),
            Err(ChunkedError::BadSize)
        );
    }

    #[test]
    fn rejects_empty_size() {
        assert_eq!(
            drain(&mut dec(), b"\r\nhello\r\n"),
            Err(ChunkedError::BadSize)
        );
    }

    #[test]
    fn rejects_size_overflow() {
        assert_eq!(
            drain(&mut dec(), b"11111111111111111\r\n"),
            Err(ChunkedError::SizeOverflow)
        );
    }

    #[test]
    fn rejects_missing_crlf_after_data() {
        assert_eq!(
            drain(&mut dec(), b"5\r\nhelloXX\r\n0\r\n\r\n"),
            Err(ChunkedError::MissingCrlf)
        );
    }

    /// Strict CRLF, matching the message-head policy: a bare LF in framing is a
    /// smuggling vector, not a leniency to extend.
    #[test]
    fn rejects_bare_lf_in_framing() {
        assert_eq!(
            drain(&mut dec(), b"5\nhello\r\n0\r\n\r\n"),
            Err(ChunkedError::MissingCrlf)
        );
    }

    #[test]
    fn enforces_running_body_limit() {
        let limits = Limits {
            max_body_bytes: 4,
            ..Default::default()
        };
        let mut d = ChunkedDecoder::new(&limits);
        assert_eq!(
            drain(&mut d, b"5\r\nhello\r\n0\r\n\r\n"),
            Err(ChunkedError::BodyTooLarge)
        );
    }

    #[test]
    fn enforces_body_limit_across_chunks() {
        let limits = Limits {
            max_body_bytes: 4,
            ..Default::default()
        };
        let mut d = ChunkedDecoder::new(&limits);
        assert_eq!(
            drain(&mut d, b"2\r\nab\r\n2\r\ncd\r\n2\r\nef\r\n0\r\n\r\n"),
            Err(ChunkedError::BodyTooLarge)
        );
    }

    /// RFC 9110 section 6.5.1. Framing was already decided before the trailer
    /// section was read, so honoring one from here is a smuggling vector.
    #[test]
    fn rejects_forbidden_trailer_fields() {
        assert_eq!(
            drain(&mut dec(), b"0\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Err(ChunkedError::ForbiddenTrailer)
        );
        assert_eq!(
            drain(&mut dec(), b"0\r\nContent-Length: 5\r\n\r\n"),
            Err(ChunkedError::ForbiddenTrailer)
        );
        assert_eq!(
            drain(&mut dec(), b"0\r\nHost: evil\r\n\r\n"),
            Err(ChunkedError::ForbiddenTrailer)
        );
    }

    #[test]
    fn rejects_obs_fold_in_trailers() {
        assert_eq!(
            drain(&mut dec(), b"0\r\nEtag: a\r\n b\r\n\r\n"),
            Err(ChunkedError::BadTrailer)
        );
    }

    #[test]
    fn rejects_whitespace_before_colon_in_trailers() {
        assert_eq!(
            drain(&mut dec(), b"0\r\nEtag : a\r\n\r\n"),
            Err(ChunkedError::BadTrailer)
        );
    }

    #[test]
    fn rejects_trailer_without_colon() {
        assert_eq!(
            drain(&mut dec(), b"0\r\nnonsense\r\n\r\n"),
            Err(ChunkedError::BadTrailer)
        );
    }

    #[test]
    fn enforces_trailer_size_limit() {
        let limits = Limits {
            max_head_bytes: 8,
            ..Default::default()
        };
        let mut d = ChunkedDecoder::new(&limits);
        assert_eq!(
            drain(&mut d, b"0\r\nEtag: aaaaaaaaaaaaaaaaaaaa\r\n\r\n"),
            Err(ChunkedError::TrailerTooLarge)
        );
    }

    #[test]
    fn end_is_terminal() {
        let mut d = dec();
        let mut buf = Bytes::from_static(b"0\r\n\r\n");
        assert_eq!(d.poll(&mut buf).unwrap(), Some(ChunkEvent::End));
        assert!(d.is_done());
        // Further polls stay at End and consume nothing.
        let mut extra = Bytes::from_static(b"GET / HTTP/1.1\r\n\r\n");
        assert_eq!(d.poll(&mut extra).unwrap(), Some(ChunkEvent::End));
        assert_eq!(extra.len(), 18, "must not consume the next request");
    }

    #[test]
    fn zero_length_body_decodes() {
        let events = drain(&mut dec(), b"0\r\n\r\n").unwrap();
        assert_eq!(
            events,
            vec![ChunkEvent::End],
            "an empty trailer section produces no event"
        );
        assert_eq!(dec().decoded_len(), 0);
    }

    #[test]
    fn decoded_len_tracks_body_bytes() {
        let mut d = dec();
        drain(&mut d, b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n").unwrap();
        assert_eq!(d.decoded_len(), 5);
    }
}
