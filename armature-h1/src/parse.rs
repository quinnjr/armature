//! Request head parsing.
//!
//! Parsing runs in two independent passes. [`prescan`] enforces strict CRLF
//! framing and rejects obs-fold; only then does `httparse` tokenize the head.
//!
//! The prescan exists because `httparse` is deliberately lenient about line
//! endings and will accept a bare LF as a line terminator. RFC 9112 section 2.2
//! permits that, and this crate declines it: when our leniency differs from an
//! upstream or downstream peer's leniency, the difference is a request-smuggling
//! vector. Running the strict check first and independently means the decision
//! does not depend on parser internals.

use crate::head::Head;
use crate::header::{HeaderId, HeaderVec};
use crate::limits::MAX_HEADERS_CEILING;
use crate::{ByteStr, Limits, Method, Version};
use bytes::Bytes;
use memchr::memmem;

/// A malformed or oversized request head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// A CR not followed by LF.
    #[error("bare CR in message head")]
    BareCr,
    /// An LF not preceded by CR.
    #[error("bare LF in message head")]
    BareLf,
    /// Obsolete line folding (RFC 9112 section 5.2).
    #[error("obs-fold in message head")]
    ObsFold,
    /// Whitespace between a field name and its colon.
    #[error("whitespace before colon in field name")]
    WhitespaceBeforeColon,
    /// A field name containing characters outside the token set.
    #[error("invalid field name")]
    InvalidHeaderName,
    /// A request line that does not parse.
    #[error("invalid request line")]
    InvalidRequestLine,
    /// A method token containing characters outside the token set.
    #[error("invalid method token")]
    InvalidMethod,
    /// A request target that is not valid UTF-8.
    #[error("request target is not valid UTF-8")]
    NonUtf8Target,
    /// A request target containing characters a URI may not contain.
    #[error("invalid character in request target")]
    InvalidTarget,
    /// A version other than HTTP/1.0 or HTTP/1.1.
    #[error("unsupported HTTP version")]
    UnsupportedVersion,
    /// The head exceeded `Limits::max_head_bytes`.
    #[error("message head too large")]
    HeadTooLarge,
    /// The head carried more fields than `Limits::max_headers`.
    #[error("too many header fields")]
    TooManyHeaders,
}

impl ParseError {
    /// The status code to answer with before closing the connection.
    #[inline]
    pub fn status(&self) -> u16 {
        match self {
            ParseError::HeadTooLarge | ParseError::TooManyHeaders => 431,
            ParseError::UnsupportedVersion => 505,
            _ => 400,
        }
    }
}

/// The index just past the `\r\n\r\n` terminating the head, or `None` if the
/// head is not yet complete.
///
/// Uses a SIMD-accelerated substring search, so scanning a partial head is
/// cheap enough to repeat on every read without buffering heuristics.
#[inline]
pub fn find_head_end(buf: &[u8]) -> Option<usize> {
    memmem::find(buf, b"\r\n\r\n").map(|i| i + 4)
}

/// Validate that `head` uses strict CRLF line endings and contains no obs-fold.
///
/// `head` must be exactly the head region, terminator included, as returned by
/// [`find_head_end`].
pub fn prescan(head: &[u8]) -> Result<(), ParseError> {
    // Every CR must be followed by LF, and every LF preceded by CR. Walking the
    // CR positions and the LF positions separately lets memchr vectorize both
    // scans, and the two checks together are equivalent to "line endings are
    // exactly CRLF".
    //
    // The obs-fold check rides along in the CR loop rather than taking a third
    // pass. A fold is a CRLF followed by SP or HTAB, and once the arm below has
    // confirmed the LF at `i + 1`, every CRLF in the head is a CR position here
    // — so a separate `\r\n` search would rescan exactly the ground this loop
    // already covers. The final CRLF CRLF terminator cannot be mistaken for a
    // fold, because the byte after the third CR is LF rather than SP or HTAB.
    for i in memchr::memchr_iter(b'\r', head) {
        if head.get(i + 1) != Some(&b'\n') {
            return Err(ParseError::BareCr);
        }
        if matches!(head.get(i + 2), Some(b' ') | Some(b'\t')) {
            return Err(ParseError::ObsFold);
        }
    }
    for i in memchr::memchr_iter(b'\n', head) {
        if i == 0 || head[i - 1] != b'\r' {
            return Err(ParseError::BareLf);
        }
    }

    Ok(())
}

// Compile-time assertion tying the parser scratch array to the ceiling `Limits`
// validates against.
const _: () = assert!(MAX_HEADERS_CEILING == 128);

/// Parse a request head out of `buf`.
///
/// Returns `Ok(None)` when the head is not yet complete, and `Ok(Some((head,
/// n)))` where `n` is the head length in bytes, so the caller knows where the
/// body begins.
///
/// Every `Bytes` in the returned [`Head`] is a `slice_ref` projection of `buf`,
/// so no field content is copied.
pub fn parse_head(buf: &Bytes, limits: &Limits) -> Result<Option<(Head, usize)>, ParseError> {
    debug_assert!(limits.max_headers <= MAX_HEADERS_CEILING);

    let Some(head_len) = find_head_end(buf) else {
        // Incomplete. Refuse to keep buffering an unterminated head forever.
        if buf.len() > limits.max_head_bytes {
            return Err(ParseError::HeadTooLarge);
        }
        return Ok(None);
    };

    if head_len > limits.max_head_bytes {
        return Err(ParseError::HeadTooLarge);
    }

    let region = &buf[..head_len];
    prescan(region)?;

    let mut scratch = [httparse::EMPTY_HEADER; MAX_HEADERS_CEILING];
    let mut req = httparse::Request::new(&mut scratch);
    let parsed = req.parse(region).map_err(map_httparse)?;
    let httparse::Status::Complete(consumed) = parsed else {
        // `find_head_end` located a terminator, so httparse should have
        // completed. A Partial here means the head contained something httparse
        // treats as needing more input; reject rather than loop forever.
        return Err(ParseError::InvalidRequestLine);
    };
    debug_assert_eq!(consumed, head_len);

    let version = req
        .version
        .and_then(Version::from_httparse)
        .ok_or(ParseError::UnsupportedVersion)?;

    let method_token = req.method.ok_or(ParseError::InvalidRequestLine)?;
    let method = match Method::from_bytes(method_token.as_bytes()) {
        Some(m) => m,
        None => {
            if method_token.is_empty() {
                return Err(ParseError::InvalidMethod);
            }
            Method::Other(
                ByteStr::from_utf8(buf.slice_ref(method_token.as_bytes()))
                    .map_err(|_| ParseError::InvalidMethod)?,
            )
        }
    };

    let target_token = req.path.ok_or(ParseError::InvalidRequestLine)?;
    validate_target(target_token.as_bytes())?;
    validate_target_form(target_token.as_bytes(), &method)?;
    let target = ByteStr::from_utf8(buf.slice_ref(target_token.as_bytes()))
        .map_err(|_| ParseError::NonUtf8Target)?;

    let field_count = req.headers.len();
    if field_count > limits.max_headers {
        return Err(ParseError::TooManyHeaders);
    }

    let mut headers = HeaderVec::with_capacity(field_count);
    for h in req.headers.iter() {
        let name = h.name.as_bytes();
        let id = match HeaderId::from_bytes(name) {
            Some(id) => id,
            None => {
                // Field names are case-insensitive, so a custom name must be
                // normalized here or lookups would depend on the sender's
                // capitalization. Already-lowercase names — the common case —
                // are projected without copying; only mixed-case names allocate.
                let lowered = if name.iter().any(|b| b.is_ascii_uppercase()) {
                    Bytes::from(name.to_ascii_lowercase())
                } else {
                    buf.slice_ref(name)
                };
                HeaderId::Other(
                    ByteStr::from_utf8(lowered).map_err(|_| ParseError::InvalidHeaderName)?,
                )
            }
        };
        headers.push((id, buf.slice_ref(h.value)));
    }

    Ok(Some((
        Head {
            method,
            target,
            version,
            headers,
        },
        head_len,
    )))
}

/// Reject a request target containing characters no URI may contain.
///
/// `httparse` is permissive here: it accepts control characters, backslashes, and
/// raw non-ASCII in the target. Every one of those is a normalization hazard —
/// a backslash that one hop reads as a path separator and another does not, a
/// control byte that survives into a log line or a proxied request line. So the
/// target is restricted to the RFC 3986 character set (unreserved, sub-delims,
/// plus `:/?[]@%`), which covers origin-, absolute-, authority-, and
/// asterisk-form alike.
///
/// `#` is excluded even though RFC 3986 admits it in a URI: the fragment is not
/// part of the request target (RFC 9110 section 7.1) and no form in RFC 9112
/// section 3.2 contains one. Accepting it would put a fragment in `Head::path`,
/// where routing would match on bytes the client never meant the server to see
/// and an upstream hop would have stripped.
///
/// The differential fuzz target against hyper found this gap: hyper parses the
/// target into a `http::Uri` and rejects what does not fit, while this crate was
/// passing the bytes straight through to routing.
fn validate_target(target: &[u8]) -> Result<(), ParseError> {
    if target.is_empty() {
        return Err(ParseError::InvalidTarget);
    }
    for &b in target {
        let ok = b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b':'
                    | b'/'
                    | b'?'
                    | b'['
                    | b']'
                    | b'@'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b'%'
            );
        if !ok {
            return Err(ParseError::InvalidTarget);
        }
    }
    Ok(())
}

/// Reject a request target that matches none of the four permitted forms.
///
/// RFC 9112 section 3.2 defines exactly four: origin-form (`/path`),
/// absolute-form (`scheme://...`), authority-form (`host:port`, CONNECT only), and
/// asterisk-form (`*`, OPTIONS only). A target matching none of them is not a
/// request-target at all, and passing it to routing means routing on something
/// whose meaning no two implementations need agree on.
///
/// Also found by the differential fuzz target: hyper parses the target into a
/// `http::Uri`, which enforces these forms, and rejected targets this crate was
/// accepting.
fn validate_target_form(target: &[u8], method: &Method) -> Result<(), ParseError> {
    // asterisk-form: OPTIONS only (RFC 9112 section 3.2.4).
    if target == b"*" {
        return if matches!(method, Method::Options) {
            Ok(())
        } else {
            Err(ParseError::InvalidTarget)
        };
    }

    // authority-form: CONNECT only, and CONNECT accepts nothing else
    // (RFC 9112 section 3.2.3).
    if matches!(method, Method::Connect) {
        let has_scheme = target.windows(3).any(|w| w == b"://");
        let has_colon = target.contains(&b':');
        return if !has_scheme && has_colon && !target.starts_with(b"/") {
            Ok(())
        } else {
            Err(ParseError::InvalidTarget)
        };
    }

    // origin-form: the common case.
    if target.starts_with(b"/") {
        return Ok(());
    }

    // absolute-form: a valid scheme, then `://`.
    if let Some(i) = target.windows(3).position(|w| w == b"://") {
        let scheme = &target[..i];
        let valid_scheme = scheme.first().is_some_and(|b| b.is_ascii_alphabetic())
            && scheme
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'));
        return if valid_scheme {
            Ok(())
        } else {
            Err(ParseError::InvalidTarget)
        };
    }

    Err(ParseError::InvalidTarget)
}

/// Map an `httparse` error onto our error set.
///
/// `httparse` reports a field name containing a space as `HeaderName`, which is
/// the whitespace-before-colon case RFC 9112 section 5.1 requires rejecting.
/// That variant covers other malformed-name causes too, but every one is
/// answered with 400, so collapsing them costs no correctness.
fn map_httparse(e: httparse::Error) -> ParseError {
    match e {
        httparse::Error::HeaderName => ParseError::WhitespaceBeforeColon,
        httparse::Error::Version => ParseError::UnsupportedVersion,
        httparse::Error::Token => ParseError::InvalidMethod,
        httparse::Error::TooManyHeaders => ParseError::TooManyHeaders,
        _ => ParseError::InvalidRequestLine,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &[u8] = b"GET / HTTP/1.1\r\nHost: a\r\n\r\n";

    #[test]
    fn find_head_end_locates_the_terminator() {
        assert_eq!(find_head_end(OK), Some(OK.len()));
        // Body bytes after the terminator do not move the boundary.
        let with_body = b"GET / HTTP/1.1\r\nHost: a\r\n\r\nBODY";
        assert_eq!(find_head_end(with_body), Some(with_body.len() - 4));
    }

    #[test]
    fn find_head_end_reports_incomplete() {
        assert_eq!(find_head_end(b""), None);
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\nHost: a\r\n"), None);
        // A lone trailing CR is not yet a terminator.
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\nHost: a\r\n\r"), None);
    }

    #[test]
    fn prescan_accepts_strict_crlf() {
        assert!(prescan(OK).is_ok());
        assert!(prescan(b"GET / HTTP/1.1\r\nHost: a\r\nAccept: */*\r\n\r\n").is_ok());
        assert!(prescan(b"GET / HTTP/1.1\r\n\r\n").is_ok());
    }

    /// RFC 9112 section 2.2 permits recognizing a bare LF. We decline: leniency
    /// that differs from a peer's leniency is a smuggling vector.
    #[test]
    fn prescan_rejects_bare_lf() {
        assert_eq!(
            prescan(b"GET / HTTP/1.1\nHost: a\r\n\r\n"),
            Err(ParseError::BareLf)
        );
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\n\r\n"),
            Err(ParseError::BareLf)
        );
    }

    #[test]
    fn prescan_rejects_bare_cr() {
        assert_eq!(
            prescan(b"GET / HTTP/1.1\rHost: a\r\n\r\n"),
            Err(ParseError::BareCr)
        );
        // A CR embedded in a field value is equally rejected.
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\rb\r\n\r\n"),
            Err(ParseError::BareCr)
        );
    }

    /// RFC 9112 section 5.2: obs-fold must be rejected in a request.
    #[test]
    fn prescan_rejects_obs_fold() {
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\r\n b\r\n\r\n"),
            Err(ParseError::ObsFold)
        );
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\r\n\tb\r\n\r\n"),
            Err(ParseError::ObsFold)
        );
    }

    /// The blank line terminating the head is CRLF followed by CRLF, which must
    /// not itself be mistaken for a fold.
    #[test]
    fn prescan_does_not_mistake_the_terminator_for_a_fold() {
        assert!(prescan(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n").is_ok());
    }

    #[test]
    fn status_codes_map_correctly() {
        assert_eq!(ParseError::BareCr.status(), 400);
        assert_eq!(ParseError::BareLf.status(), 400);
        assert_eq!(ParseError::ObsFold.status(), 400);
        assert_eq!(ParseError::WhitespaceBeforeColon.status(), 400);
        assert_eq!(ParseError::InvalidRequestLine.status(), 400);
        assert_eq!(ParseError::NonUtf8Target.status(), 400);
        assert_eq!(ParseError::InvalidTarget.status(), 400);
        assert_eq!(ParseError::HeadTooLarge.status(), 431);
        assert_eq!(ParseError::TooManyHeaders.status(), 431);
        assert_eq!(ParseError::UnsupportedVersion.status(), 505);
    }

    // ---- parse_head ----

    fn parse(raw: &'static [u8]) -> Result<Option<(Head, usize)>, ParseError> {
        parse_head(&Bytes::from_static(raw), &Limits::default())
    }

    #[test]
    fn parses_a_minimal_request() {
        let (head, n) = parse(b"GET / HTTP/1.1\r\nHost: a.example\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(n, 35);
        assert_eq!(head.method, Method::Get);
        assert_eq!(head.target.as_str(), "/");
        assert_eq!(head.version, Version::Http11);
        assert_eq!(head.headers.len(), 1);
        assert_eq!(head.get_str(&HeaderId::Host), Some("a.example"));
    }

    #[test]
    fn reports_incomplete_heads() {
        assert!(parse(b"GET / HTTP/1.1\r\n").unwrap().is_none());
        assert!(parse(b"GE").unwrap().is_none());
        assert!(parse(b"").unwrap().is_none());
    }

    /// The whole point of the design: nothing in the parsed head is copied.
    #[test]
    fn header_values_share_the_read_buffer() {
        let buf = Bytes::from_static(b"GET /a?b=1 HTTP/1.1\r\nHost: a.example\r\n\r\n");
        let (head, _) = parse_head(&buf, &Limits::default()).unwrap().unwrap();
        let host = head.get(&HeaderId::Host).unwrap();
        let base = buf.as_ptr() as usize;
        let addr = host.as_ptr() as usize;
        assert!(
            addr >= base && addr < base + buf.len(),
            "header value must point into the read buffer, not a copy"
        );
        let target = head.target.as_bytes().as_ptr() as usize;
        assert!(target >= base && target < base + buf.len());
    }

    #[test]
    fn splits_path_and_query() {
        let (head, _) = parse(b"GET /a/b?x=1&y=2 HTTP/1.1\r\nHost: a\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(head.path(), "/a/b");
        assert_eq!(head.query(), Some("x=1&y=2"));

        let (head, _) = parse(b"GET /a/b HTTP/1.1\r\nHost: a\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(head.path(), "/a/b");
        assert_eq!(head.query(), None);

        // An empty query is present but empty, distinct from absent.
        let (head, _) = parse(b"GET /a? HTTP/1.1\r\nHost: a\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(head.path(), "/a");
        assert_eq!(head.query(), Some(""));
    }

    #[test]
    fn unknown_method_and_header_become_other_variants() {
        let (head, _) = parse(b"PROPFIND / HTTP/1.1\r\nHost: a\r\nX-Req-Id: z\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(head.method.as_str(), "PROPFIND");
        assert!(matches!(head.method, Method::Other(_)));
        let id = HeaderId::Other(ByteStr::from_static("x-req-id"));
        assert_eq!(head.get_str(&id), Some("z"));
    }

    /// Field names are case-insensitive, so a custom name must be lowercased at
    /// parse time or lookups would depend on the sender's capitalization.
    #[test]
    fn custom_header_names_are_lowercased() {
        let (head, _) = parse(b"GET / HTTP/1.1\r\nHost: a\r\nX-Req-Id: z\r\n\r\n")
            .unwrap()
            .unwrap();
        match &head.headers[1].0 {
            HeaderId::Other(name) => assert_eq!(name.as_str(), "x-req-id"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn http_10_is_accepted_and_http_12_is_not() {
        let (head, _) = parse(b"GET / HTTP/1.0\r\n\r\n").unwrap().unwrap();
        assert_eq!(head.version, Version::Http10);
        assert_eq!(
            parse(b"GET / HTTP/1.2\r\nHost: a\r\n\r\n"),
            Err(ParseError::UnsupportedVersion)
        );
    }

    /// httparse accepts backslashes and raw non-ASCII in the target; both are
    /// normalization hazards across hops. Found by the differential fuzz target
    /// against hyper, which parses the target into a `http::Uri` and rejects what
    /// does not fit.
    #[test]
    fn rejects_invalid_target_characters() {
        // Characters our validator owns: httparse passes these through.
        for raw in [
            &b"GET /a\\b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a<b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a>b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a{b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a|b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a^b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a\"b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            // A fragment is not part of the request target (RFC 9110 section
            // 7.1) and no RFC 9112 section 3.2 form admits one, so `#` would
            // reach routing as path bytes an upstream hop would have stripped.
            &b"GET /a#frag HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a?q=1#frag HTTP/1.1\r\nHost: a\r\n\r\n"[..],
        ] {
            assert_eq!(
                parse_head(&Bytes::copy_from_slice(raw), &Limits::default()),
                Err(ParseError::InvalidTarget),
                "should have rejected: {}",
                String::from_utf8_lossy(raw)
            );
        }

        // Control bytes and raw non-ASCII are rejected too, though httparse
        // catches some of them first and reports a malformed request line. Either
        // error closes the connection with 400, so which one fires does not matter
        // — that it is rejected at all does.
        for raw in [
            &b"GET /a\x01b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a\x7fb HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a\x00b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a\xc3\xa9 HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a\xffb HTTP/1.1\r\nHost: a\r\n\r\n"[..],
        ] {
            let err = parse_head(&Bytes::copy_from_slice(raw), &Limits::default())
                .expect_err("a non-URI byte in a target must be rejected");
            assert_eq!(err.status(), 400, "{err:?}");
        }
    }

    /// RFC 9112 section 3.2 permits exactly four target forms. Anything else is
    /// not a request-target, and routing on it means routing on something whose
    /// meaning no two implementations need agree on. Found by the differential
    /// fuzz target against hyper.
    #[test]
    fn rejects_targets_matching_no_permitted_form() {
        for raw in [
            // Neither origin-, absolute-, authority-, nor asterisk-form.
            &b"GET foo/bar HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET a.example:80 HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET ://a.example/x HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET 1http://a.example/x HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            // asterisk-form is OPTIONS only.
            &b"GET * HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            // CONNECT takes authority-form and nothing else.
            &b"CONNECT /x HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"CONNECT http://a.example/x HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"CONNECT a.example HTTP/1.1\r\nHost: a\r\n\r\n"[..],
        ] {
            assert_eq!(
                parse_head(&Bytes::copy_from_slice(raw), &Limits::default()),
                Err(ParseError::InvalidTarget),
                "should have rejected: {}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    #[test]
    fn accepts_every_legitimate_target_form() {
        for raw in [
            &b"GET / HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a/b?x=1&y=2 HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a%20b HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /~user/a.b_c-d HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET /a?q=1;2,3!$&'()*+= HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"OPTIONS * HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"GET http://a.example:8080/x HTTP/1.1\r\nHost: a.example\r\n\r\n"[..],
            &b"GET http://[::1]:8080/x HTTP/1.1\r\nHost: a\r\n\r\n"[..],
            &b"CONNECT a.example:443 HTTP/1.1\r\nHost: a.example\r\n\r\n"[..],
        ] {
            assert!(
                parse_head(&Bytes::copy_from_slice(raw), &Limits::default()).is_ok(),
                "should have accepted: {}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    #[test]
    fn rejects_whitespace_before_colon() {
        assert_eq!(
            parse(b"GET / HTTP/1.1\r\nHost : a\r\n\r\n"),
            Err(ParseError::WhitespaceBeforeColon)
        );
    }

    #[test]
    fn rejects_malformed_request_lines() {
        assert!(parse(b"GET\r\nHost: a\r\n\r\n").is_err());
        assert!(parse(b"GET /\r\nHost: a\r\n\r\n").is_err());
        assert!(parse(b"/ HTTP/1.1\r\nHost: a\r\n\r\n").is_err());
    }

    #[test]
    fn prescan_runs_before_tokenization() {
        // A bare LF must be rejected as BareLf, not silently accepted by
        // httparse's lenient line-ending handling.
        assert_eq!(
            parse(b"GET / HTTP/1.1\nHost: a\r\n\r\n"),
            Err(ParseError::BareLf)
        );
    }

    #[test]
    fn enforces_head_size_limit() {
        let limits = Limits {
            max_head_bytes: 40,
            ..Default::default()
        };
        let long = Bytes::from_static(
            b"GET /aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HTTP/1.1\r\nHost: a\r\n\r\n",
        );
        assert_eq!(parse_head(&long, &limits), Err(ParseError::HeadTooLarge));
    }

    #[test]
    fn enforces_header_count_limit() {
        let limits = Limits {
            max_headers: 2,
            ..Default::default()
        };
        let raw = Bytes::from_static(b"GET / HTTP/1.1\r\nHost: a\r\nA: 1\r\nB: 2\r\n\r\n");
        assert_eq!(parse_head(&raw, &limits), Err(ParseError::TooManyHeaders));
    }

    #[test]
    fn keep_alive_defaults_follow_the_version() {
        let (head, _) = parse(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(head.is_keep_alive(), "HTTP/1.1 persists by default");

        let (head, _) = parse(b"GET / HTTP/1.0\r\n\r\n").unwrap().unwrap();
        assert!(!head.is_keep_alive(), "HTTP/1.0 closes by default");
    }

    #[test]
    fn connection_tokens_override_the_default() {
        let (head, _) = parse(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(!head.is_keep_alive());

        let (head, _) = parse(b"GET / HTTP/1.0\r\nConnection: keep-alive\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(head.is_keep_alive());
    }

    #[test]
    fn connection_token_matching_is_case_insensitive_and_list_aware() {
        let (head, _) =
            parse(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: Keep-Alive, Upgrade\r\n\r\n")
                .unwrap()
                .unwrap();
        assert!(head.connection_has_token("upgrade"));
        assert!(head.connection_has_token("keep-alive"));
        assert!(!head.connection_has_token("close"));
        assert!(head.is_keep_alive());
    }
}
