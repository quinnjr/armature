//! Response serialization.
//!
//! Everything here writes into a caller-supplied [`BytesMut`] — the connection's
//! write buffer — so a response is assembled in place and leaves in one
//! `writev` rather than being built up through intermediate allocations.

use crate::Version;
use crate::header::{HeaderId, HeaderVec};
use bytes::{BufMut, Bytes, BytesMut};
use std::time::SystemTime;

/// The length of an IMF-fixdate: `Sun, 06 Nov 1994 08:49:37 GMT`.
const DATE_LEN: usize = 29;

/// A once-per-second cache of the `Date` field value.
///
/// Formatting a date is comparatively expensive and the result only changes on
/// the second, so it is reformatted at most once per second per worker. `now` is
/// a parameter rather than read internally so the cache is testable without a
/// clock.
#[derive(Debug)]
pub struct DateCache {
    secs: u64,
    buf: [u8; DATE_LEN],
    valid: bool,
}

impl Default for DateCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DateCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            secs: 0,
            buf: [0; DATE_LEN],
            valid: false,
        }
    }

    /// The IMF-fixdate for `now`, reformatting only when the second changed.
    pub fn get(&mut self, now: SystemTime) -> &[u8] {
        let secs = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if !self.valid || secs != self.secs {
            let formatted = httpdate::fmt_http_date(now);
            debug_assert_eq!(formatted.len(), DATE_LEN);
            let bytes = formatted.as_bytes();
            let n = bytes.len().min(DATE_LEN);
            self.buf[..n].copy_from_slice(&bytes[..n]);
            self.secs = secs;
            self.valid = true;
        }
        &self.buf
    }
}

/// How a response body is framed on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutBody {
    /// No body.
    None,
    /// A body of known length.
    Fixed(Bytes),
    /// A chunked body.
    Chunked,
}

/// The status and headers of a response.
#[derive(Clone, Debug, Default)]
pub struct ResponseHead {
    /// The status code.
    pub status: u16,
    /// Headers supplied by the handler.
    pub headers: HeaderVec,
}

/// The reason phrase for `status`, or `""` if unregistered.
///
/// An empty phrase is valid on the wire (RFC 9112 section 4), so an unknown code
/// needs no invented text.
pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

/// Append `v` as decimal digits.
///
/// Avoids `format!`, which would allocate a `String` per call on a path that
/// runs at least once per response.
pub fn write_u64(out: &mut BytesMut, v: u64) {
    // u64::MAX is 20 digits.
    let mut scratch = [0u8; 20];
    let mut i = scratch.len();
    let mut n = v;
    loop {
        i -= 1;
        scratch[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.put_slice(&scratch[i..]);
}

/// Whether a status code forbids any body framing field.
///
/// 204 and 304 must carry neither `Content-Length` nor `Transfer-Encoding`
/// (RFC 9110 sections 15.3.5 and 15.4.5).
#[inline]
fn forbids_body_framing(status: u16) -> bool {
    matches!(status, 204 | 304) || (100..200).contains(&status)
}

/// Whether a field value can be written without splitting the response.
///
/// CR, LF, or NUL in a value terminates the field early and lets whatever
/// follows be read as further header fields or as a body. That is response
/// splitting — the mirror image of the request smuggling this crate is built to
/// reject — and a handler that reflects request data into a header (a computed
/// `Location`, an echoed correlation id) is one bad input away from it. The
/// check lives here, at the last point before the bytes reach the wire, rather
/// than at each of the call sites that could produce one.
#[inline]
fn valid_field_value(value: &[u8]) -> bool {
    !value.iter().any(|b| matches!(b, b'\r' | b'\n' | 0))
}

/// Whether a field name is a token (RFC 9110 section 5.6.2).
///
/// Well-known [`HeaderId`]s render to fixed tokens by construction.
/// [`HeaderId::Other`] carries whatever the parser or the handler put in it, and
/// a name containing a colon, a space, or CRLF splits the response exactly as a
/// value does.
#[inline]
fn valid_field_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Whether this field may be emitted as-is.
#[inline]
fn writable(id: &HeaderId, value: &[u8]) -> bool {
    let name_ok = match id {
        HeaderId::Other(name) => valid_field_name(name.as_str()),
        _ => true,
    };
    name_ok && valid_field_value(value)
}

/// The first value for `id` that will actually be emitted.
///
/// The framing, `Date`, and `Connection` decisions below must agree with what
/// was written, not with what the handler supplied: a `Content-Length` dropped
/// for containing CRLF would otherwise suppress the framing field too and leave
/// the response undelimited — trading one splitting bug for another.
#[inline]
fn emitted<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a Bytes> {
    v.iter()
        .find(|(k, val)| k == id && writable(k, val))
        .map(|(_, val)| val)
}

/// Serialize a status line and header section into `out`.
///
/// Emits `Date`, `Connection`, and the body framing field itself, but never
/// duplicates one the handler already supplied. A duplicate `Content-Length` on
/// a response is the mirror image of the request ambiguity this crate rejects,
/// so it must not be possible to produce one by accident.
pub fn write_head(
    out: &mut BytesMut,
    version: Version,
    resp: &ResponseHead,
    body: &OutBody,
    date: &[u8],
    keep_alive: bool,
) {
    out.put_slice(version.as_bytes());
    out.put_u8(b' ');
    write_u64(out, resp.status as u64);
    let phrase = reason_phrase(resp.status);
    if !phrase.is_empty() {
        out.put_u8(b' ');
        out.put_slice(phrase.as_bytes());
    }
    out.put_slice(b"\r\n");

    // Handler-supplied headers first, so the checks below can see them.
    for (id, value) in resp.headers.iter() {
        if !writable(id, value) {
            // The status line is already in `out`, so there is no error left to
            // return; dropping the field is the only outcome that does not put
            // attacker-chosen bytes on the wire.
            tracing::warn!(
                field = id.as_str(),
                "dropping response header with an invalid field name or value"
            );
            continue;
        }
        out.put_slice(id.as_str().as_bytes());
        out.put_slice(b": ");
        out.put_slice(value);
        out.put_slice(b"\r\n");
    }

    if emitted(&resp.headers, &HeaderId::Date).is_none() {
        out.put_slice(b"date: ");
        out.put_slice(date);
        out.put_slice(b"\r\n");
    }

    // Body framing, unless the handler already framed it or the status forbids
    // framing entirely.
    let handler_framed = emitted(&resp.headers, &HeaderId::ContentLength).is_some()
        || emitted(&resp.headers, &HeaderId::TransferEncoding).is_some();
    if !handler_framed && !forbids_body_framing(resp.status) {
        match body {
            OutBody::Fixed(b) => {
                out.put_slice(b"content-length: ");
                write_u64(out, b.len() as u64);
                out.put_slice(b"\r\n");
            }
            OutBody::Chunked => {
                out.put_slice(b"transfer-encoding: chunked\r\n");
            }
            OutBody::None => {
                out.put_slice(b"content-length: 0\r\n");
            }
        }
    }

    if emitted(&resp.headers, &HeaderId::Connection).is_none() {
        match (version, keep_alive) {
            // Persistence is the HTTP/1.1 default; saying so would be noise.
            (Version::Http11, true) => {}
            (Version::Http10, true) => out.put_slice(b"connection: keep-alive\r\n"),
            (_, false) => out.put_slice(b"connection: close\r\n"),
        }
    }

    out.put_slice(b"\r\n");
}

/// Append one chunk of a chunked body.
pub fn write_chunk(out: &mut BytesMut, data: &[u8]) {
    write_hex(out, data.len() as u64);
    out.put_slice(b"\r\n");
    out.put_slice(data);
    out.put_slice(b"\r\n");
}

/// Append the terminating zero-length chunk and trailer section.
///
/// Trailers are held to the same field-name and field-value rules as headers: a
/// CRLF in a trailer value ends the trailer section early, and the bytes after
/// it become the start of whatever the peer reads next.
pub fn write_last_chunk(out: &mut BytesMut, trailers: &HeaderVec) {
    out.put_slice(b"0\r\n");
    for (id, value) in trailers.iter() {
        if !writable(id, value) {
            tracing::warn!(
                field = id.as_str(),
                "dropping trailer with an invalid field name or value"
            );
            continue;
        }
        out.put_slice(id.as_str().as_bytes());
        out.put_slice(b": ");
        out.put_slice(value);
        out.put_slice(b"\r\n");
    }
    out.put_slice(b"\r\n");
}

/// Append `v` as lowercase hex digits.
fn write_hex(out: &mut BytesMut, v: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut scratch = [0u8; 16];
    let mut i = scratch.len();
    let mut n = v;
    loop {
        i -= 1;
        scratch[i] = HEX[(n % 16) as usize];
        n /= 16;
        if n == 0 {
            break;
        }
    }
    out.put_slice(&scratch[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const EPOCH_DATE: &[u8] = b"Thu, 01 Jan 1970 00:00:00 GMT";

    fn head_of(status: u16, headers: &[(HeaderId, &'static str)]) -> ResponseHead {
        ResponseHead {
            status,
            headers: headers
                .iter()
                .map(|(id, v)| (id.clone(), Bytes::from_static(v.as_bytes())))
                .collect(),
        }
    }

    fn render(version: Version, resp: &ResponseHead, body: &OutBody, keep_alive: bool) -> String {
        let mut out = BytesMut::new();
        write_head(&mut out, version, resp, body, EPOCH_DATE, keep_alive);
        String::from_utf8(out.to_vec()).unwrap()
    }

    #[test]
    fn writes_a_minimal_200() {
        let got = render(
            Version::Http11,
            &head_of(200, &[]),
            &OutBody::Fixed(Bytes::from_static(b"hello")),
            true,
        );
        assert_eq!(
            got,
            "HTTP/1.1 200 OK\r\n\
             date: Thu, 01 Jan 1970 00:00:00 GMT\r\n\
             content-length: 5\r\n\
             \r\n"
        );
    }

    #[test]
    fn write_u64_matches_to_string() {
        for v in [0u64, 1, 9, 10, 99, 100, 12345, u64::MAX] {
            let mut out = BytesMut::new();
            write_u64(&mut out, v);
            assert_eq!(String::from_utf8(out.to_vec()).unwrap(), v.to_string());
        }
    }

    #[test]
    fn reason_phrases_are_correct() {
        assert_eq!(reason_phrase(200), "OK");
        assert_eq!(reason_phrase(201), "Created");
        assert_eq!(reason_phrase(204), "No Content");
        assert_eq!(reason_phrase(301), "Moved Permanently");
        assert_eq!(reason_phrase(304), "Not Modified");
        assert_eq!(reason_phrase(400), "Bad Request");
        assert_eq!(reason_phrase(404), "Not Found");
        assert_eq!(reason_phrase(408), "Request Timeout");
        assert_eq!(reason_phrase(413), "Content Too Large");
        assert_eq!(reason_phrase(431), "Request Header Fields Too Large");
        assert_eq!(reason_phrase(500), "Internal Server Error");
        assert_eq!(reason_phrase(501), "Not Implemented");
        assert_eq!(reason_phrase(505), "HTTP Version Not Supported");
        assert_eq!(reason_phrase(599), "", "unregistered codes invent nothing");
    }

    #[test]
    fn fixed_body_emits_content_length() {
        let got = render(
            Version::Http11,
            &head_of(200, &[]),
            &OutBody::Fixed(Bytes::from_static(b"hello")),
            true,
        );
        assert!(got.contains("content-length: 5\r\n"));
        assert!(!got.contains("transfer-encoding"));
    }

    #[test]
    fn chunked_body_emits_transfer_encoding() {
        let got = render(Version::Http11, &head_of(200, &[]), &OutBody::Chunked, true);
        assert!(got.contains("transfer-encoding: chunked\r\n"));
        assert!(!got.contains("content-length"));
    }

    #[test]
    fn empty_body_emits_zero_length() {
        let got = render(Version::Http11, &head_of(200, &[]), &OutBody::None, true);
        assert!(got.contains("content-length: 0\r\n"));
    }

    /// RFC 9110 15.3.5 and 15.4.5: neither status may carry body framing.
    #[test]
    fn no_framing_fields_on_204_or_304() {
        for status in [204u16, 304] {
            let got = render(Version::Http11, &head_of(status, &[]), &OutBody::None, true);
            assert!(
                !got.contains("content-length"),
                "{status} must not frame a body: {got}"
            );
            assert!(
                !got.contains("transfer-encoding"),
                "{status} must not frame a body: {got}"
            );
        }
    }

    #[test]
    fn does_not_duplicate_handler_supplied_date() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::Date, "Mon, 01 Jan 2001 00:00:00 GMT")]),
            &OutBody::None,
            true,
        );
        assert_eq!(got.matches("date: ").count(), 1);
        assert!(got.contains("Mon, 01 Jan 2001"));
    }

    /// A duplicate Content-Length on a response is the mirror image of the
    /// request ambiguity this crate rejects, so it must be impossible to emit.
    #[test]
    fn does_not_duplicate_handler_supplied_content_length() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::ContentLength, "5")]),
            &OutBody::Fixed(Bytes::from_static(b"hello")),
            true,
        );
        assert_eq!(got.matches("content-length").count(), 1);
    }

    #[test]
    fn does_not_add_content_length_when_handler_set_transfer_encoding() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::TransferEncoding, "chunked")]),
            &OutBody::Chunked,
            true,
        );
        assert_eq!(got.matches("transfer-encoding").count(), 1);
        assert!(!got.contains("content-length"));
    }

    #[test]
    fn emits_connection_close_when_not_keep_alive() {
        let got = render(Version::Http11, &head_of(200, &[]), &OutBody::None, false);
        assert!(got.contains("connection: close\r\n"));
    }

    #[test]
    fn omits_connection_header_when_keep_alive_on_http11() {
        let got = render(Version::Http11, &head_of(200, &[]), &OutBody::None, true);
        assert!(
            !got.contains("connection:"),
            "persistence is the HTTP/1.1 default; stating it is noise"
        );
    }

    #[test]
    fn emits_connection_keep_alive_on_http10() {
        let got = render(Version::Http10, &head_of(200, &[]), &OutBody::None, true);
        assert!(got.contains("connection: keep-alive\r\n"));
        assert!(got.starts_with("HTTP/1.0 200 OK\r\n"));
    }

    #[test]
    fn does_not_duplicate_handler_supplied_connection() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::Connection, "close")]),
            &OutBody::None,
            false,
        );
        assert_eq!(got.matches("connection").count(), 1);
    }

    /// The mirror image of request smuggling: a CRLF in a handler-supplied value
    /// ends the header section early, and everything after it is read as more
    /// header fields or as a body.
    #[test]
    fn drops_header_values_that_would_split_the_response() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::Location, "/a\r\nX-Injected: 1")]),
            &OutBody::None,
            true,
        );
        assert!(!got.contains("X-Injected"), "{got}");
        assert!(
            !got.contains("location"),
            "the whole field goes, not just the tail: {got}"
        );
    }

    #[test]
    fn drops_values_containing_bare_cr_lf_or_nul() {
        for bad in ["a\rb", "a\nb", "a\0b"] {
            let got = render(
                Version::Http11,
                &head_of(200, &[(HeaderId::Etag, bad)]),
                &OutBody::None,
                true,
            );
            assert!(!got.contains("etag"), "{bad:?} must be dropped: {got}");
        }
    }

    /// `HeaderId::Other` carries whatever a handler put in it; a name with a
    /// space or a colon splits the response just as a value does.
    #[test]
    fn drops_custom_field_names_that_are_not_tokens() {
        let mut headers = HeaderVec::new();
        for name in ["x bad", "x:bad", "x\r\nbad", ""] {
            headers.push((
                HeaderId::Other(crate::ByteStr::from(name)),
                Bytes::from_static(b"1"),
            ));
        }
        headers.push((
            HeaderId::Other(crate::ByteStr::from_static("x-good")),
            Bytes::from_static(b"1"),
        ));
        let got = render(
            Version::Http11,
            &ResponseHead {
                status: 200,
                headers,
            },
            &OutBody::None,
            true,
        );
        assert!(!got.contains("bad"), "{got}");
        assert!(got.contains("x-good: 1\r\n"), "{got}");
    }

    /// Dropping a field must not leave the response undelimited. The framing
    /// decision follows what was written, so a rejected `Content-Length` puts the
    /// writer back in charge of framing rather than suppressing it.
    #[test]
    fn a_dropped_content_length_does_not_suppress_framing() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::ContentLength, "5\r\nX-Injected: 1")]),
            &OutBody::Fixed(Bytes::from_static(b"hello")),
            true,
        );
        assert!(!got.contains("X-Injected"), "{got}");
        assert_eq!(got.matches("content-length").count(), 1, "{got}");
        assert!(got.contains("content-length: 5\r\n"), "{got}");
    }

    #[test]
    fn write_last_chunk_drops_an_injecting_trailer() {
        let mut trailers = HeaderVec::new();
        trailers.push((HeaderId::Etag, Bytes::from_static(b"x\r\nX-Injected: 1")));
        let mut out = BytesMut::new();
        write_last_chunk(&mut out, &trailers);
        assert_eq!(&out[..], b"0\r\n\r\n");
    }

    #[test]
    fn date_cache_reformats_only_on_second_change() {
        let mut c = DateCache::new();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_millis(1_500);
        let first = c.get(t0).to_vec();
        let same_second = c.get(t0 + Duration::from_millis(400)).to_vec();
        assert_eq!(first, same_second);
        let next_second = c.get(t0 + Duration::from_secs(1)).to_vec();
        assert_ne!(first, next_second);
    }

    #[test]
    fn date_format_is_imf_fixdate() {
        let mut c = DateCache::new();
        let d = c.get(SystemTime::UNIX_EPOCH);
        assert_eq!(d.len(), DATE_LEN);
        assert_eq!(d, EPOCH_DATE);
    }

    #[test]
    fn write_chunk_frames_correctly() {
        let mut out = BytesMut::new();
        write_chunk(&mut out, b"hello");
        assert_eq!(&out[..], b"5\r\nhello\r\n");

        let mut out = BytesMut::new();
        write_chunk(&mut out, &[0u8; 31]);
        assert!(out.starts_with(b"1f\r\n"), "sizes are lowercase hex");
    }

    #[test]
    fn write_last_chunk_without_trailers() {
        let mut out = BytesMut::new();
        write_last_chunk(&mut out, &HeaderVec::new());
        assert_eq!(&out[..], b"0\r\n\r\n");
    }

    #[test]
    fn write_last_chunk_with_trailers() {
        let mut trailers = HeaderVec::new();
        trailers.push((HeaderId::Etag, Bytes::from_static(b"x")));
        let mut out = BytesMut::new();
        write_last_chunk(&mut out, &trailers);
        assert_eq!(&out[..], b"0\r\netag: x\r\n\r\n");
    }

    /// The round trip that matters: what we write, our own parser must accept.
    #[test]
    fn written_head_parses_as_a_valid_message() {
        let got = render(
            Version::Http11,
            &head_of(200, &[(HeaderId::ContentType, "text/plain")]),
            &OutBody::Fixed(Bytes::from_static(b"hello")),
            true,
        );
        // Strict CRLF throughout, and exactly one blank-line terminator.
        assert!(crate::parse::prescan(got.as_bytes()).is_ok());
        assert_eq!(crate::parse::find_head_end(got.as_bytes()), Some(got.len()));
    }
}
