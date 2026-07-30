//! Message body framing: the decision of how long the request body is.
//!
//! This is the most security-critical code in the crate. Request smuggling is,
//! in essence, two HTTP implementations disagreeing about where one message ends
//! and the next begins — so this module resolves nothing ambiguously. Every
//! unclear case is an error, and every error closes the connection.
//!
//! The function is deliberately pure and synchronous so it can be exhaustively
//! unit-tested and differentially fuzzed against another implementation.

use crate::header::HeaderId;
use crate::{Head, Limits, Version};

/// How to read the request body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// No body. A request without `Content-Length` or `Transfer-Encoding` has no
    /// body — unlike a response, whose length may run to end-of-stream.
    None,
    /// A body of exactly this many bytes.
    Length(u64),
    /// A chunked body, terminated by a zero-length chunk.
    Chunked,
}

/// An unresolvable or unacceptable framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FramingError {
    /// Both `Content-Length` and `Transfer-Encoding` were present.
    #[error("both Content-Length and Transfer-Encoding present")]
    LengthAndTransferEncoding,
    /// `Content-Length` appeared more than once with differing values.
    #[error("conflicting Content-Length values")]
    DuplicateContentLength,
    /// `Content-Length` was not a bare decimal integer.
    #[error("malformed Content-Length")]
    InvalidContentLength,
    /// `chunked` was present but was not the final transfer coding.
    #[error("chunked is not the final transfer coding")]
    ChunkedNotFinal,
    /// A transfer coding other than `chunked` was requested.
    #[error("unsupported transfer coding")]
    UnsupportedTransferEncoding,
    /// HTTP/1.1 requires exactly one `Host`.
    #[error("missing Host")]
    MissingHost,
    /// More than one `Host` is ambiguous at any version.
    #[error("multiple Host fields")]
    MultipleHost,
    /// The declared body exceeded `Limits::max_body_bytes`.
    #[error("declared body too large")]
    BodyTooLarge,
}

impl FramingError {
    /// The status code to answer with before closing the connection.
    #[inline]
    pub fn status(&self) -> u16 {
        match self {
            FramingError::BodyTooLarge => 413,
            FramingError::UnsupportedTransferEncoding => 501,
            _ => 400,
        }
    }
}

/// Decide how to frame the request body.
///
/// Rule order is fixed and load-bearing:
///
/// 1. `Host` validity
/// 2. `Content-Length`-with-`Transfer-Encoding` conflict
/// 3. `Transfer-Encoding` analysis
/// 4. `Content-Length` analysis
/// 5. Declared-size limit
///
/// Step 2 precedes 3 and 4 so that a request carrying a *valid* value of each is
/// rejected outright rather than silently resolved in favor of one. That silent
/// resolution — and disagreement between peers about which one wins — is the
/// smuggling vector.
pub fn decide(head: &Head, limits: &Limits) -> Result<BodyKind, FramingError> {
    // 1. Host. More than one is ambiguous at any version; HTTP/1.1 additionally
    //    requires at least one (RFC 9112 section 3.2).
    match head.count(&HeaderId::Host) {
        0 if head.version == Version::Http11 => return Err(FramingError::MissingHost),
        n if n > 1 => return Err(FramingError::MultipleHost),
        _ => {}
    }

    let has_len = head.count(&HeaderId::ContentLength) > 0;
    let has_te = head.count(&HeaderId::TransferEncoding) > 0;

    // 2. Conflict. Checked before either field is analyzed.
    if has_len && has_te {
        return Err(FramingError::LengthAndTransferEncoding);
    }

    // 3. Transfer-Encoding. Codings accumulate across repeated fields in wire
    //    order, exactly as if they had been sent as one comma list.
    if has_te {
        let mut codings: smallvec::SmallVec<[&str; 4]> = smallvec::SmallVec::new();
        for value in head.all(&HeaderId::TransferEncoding) {
            let s = std::str::from_utf8(value).map_err(|_| FramingError::ChunkedNotFinal)?;
            for coding in s.split(',') {
                let coding = coding.trim();
                if !coding.is_empty() {
                    codings.push(coding);
                }
            }
        }

        let chunked_count = codings
            .iter()
            .filter(|c| c.eq_ignore_ascii_case("chunked"))
            .count();

        return match chunked_count {
            // A single chunked coding, and it must be last.
            1 if codings
                .last()
                .is_some_and(|c| c.eq_ignore_ascii_case("chunked")) =>
            {
                if codings.len() == 1 {
                    Ok(BodyKind::Chunked)
                } else {
                    // e.g. `gzip, chunked`: chunked frames the message, but an
                    // inner coding we cannot decode remains. 501 rather than
                    // silently handing the handler compressed bytes.
                    Err(FramingError::UnsupportedTransferEncoding)
                }
            }
            // Present but not final, or present more than once: the message
            // length is undetermined.
            n if n >= 1 => Err(FramingError::ChunkedNotFinal),
            // No chunked coding at all: some coding we do not implement.
            _ => Err(FramingError::UnsupportedTransferEncoding),
        };
    }

    // 4. Content-Length. Every value across every field, and every element of
    //    every comma list, must parse identically (RFC 9112 section 6.3).
    if has_len {
        let mut agreed: Option<u64> = None;
        for value in head.all(&HeaderId::ContentLength) {
            let s = std::str::from_utf8(value).map_err(|_| FramingError::InvalidContentLength)?;
            for element in s.split(',') {
                let n = parse_content_length(element.trim())?;
                match agreed {
                    None => agreed = Some(n),
                    Some(prev) if prev == n => {}
                    Some(_) => return Err(FramingError::DuplicateContentLength),
                }
            }
        }
        let len = agreed.ok_or(FramingError::InvalidContentLength)?;

        // 5. Declared-size limit, before a single body byte is read.
        if len > limits.max_body_bytes {
            return Err(FramingError::BodyTooLarge);
        }
        return Ok(BodyKind::Length(len));
    }

    // Neither field: a request has no body.
    Ok(BodyKind::None)
}

/// Parse one `Content-Length` element as a bare decimal integer.
///
/// Rejects the empty string, signs, whitespace, non-digits, and overflow.
/// `str::parse::<u64>` would accept a leading `+`, which RFC 9112 does not, so
/// digits are checked explicitly.
#[inline]
fn parse_content_length(s: &str) -> Result<u64, FramingError> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii_digit()) {
        return Err(FramingError::InvalidContentLength);
    }
    s.parse::<u64>()
        .map_err(|_| FramingError::InvalidContentLength)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Limits, parse_head};
    use bytes::Bytes;

    /// Build a head from a raw request, asserting it parses.
    fn head(raw: &'static [u8]) -> crate::Head {
        parse_head(&Bytes::from_static(raw), &Limits::default())
            .expect("must parse")
            .expect("must be complete")
            .0
    }

    fn decide_raw(raw: &'static [u8]) -> Result<BodyKind, FramingError> {
        decide(&head(raw), &Limits::default())
    }

    // ---- positive cases ----

    #[test]
    fn no_framing_headers_means_no_body() {
        assert_eq!(
            decide_raw(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n"),
            Ok(BodyKind::None)
        );
    }

    #[test]
    fn content_length_gives_a_fixed_body() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\n"),
            Ok(BodyKind::Length(5))
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\n\r\n"),
            Ok(BodyKind::Length(0))
        );
    }

    #[test]
    fn transfer_encoding_chunked_gives_a_chunked_body() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Ok(BodyKind::Chunked)
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: CHUNKED\r\n\r\n"),
            Ok(BodyKind::Chunked),
            "transfer codings are case-insensitive"
        );
    }

    /// RFC 9112 section 6.3: duplicate Content-Length fields with identical
    /// values may be treated as one.
    #[test]
    fn identical_duplicate_content_length_is_accepted() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n"
            ),
            Ok(BodyKind::Length(5))
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5, 5\r\n\r\n"),
            Ok(BodyKind::Length(5)),
            "a comma list of identical values is the same case"
        );
    }

    #[test]
    fn http_10_needs_no_host() {
        assert_eq!(decide_raw(b"GET / HTTP/1.0\r\n\r\n"), Ok(BodyKind::None));
    }

    // ---- the rejection table from the spec ----

    /// RFC 9112 section 6.1: both present is unresolvable ambiguity.
    #[test]
    fn rejects_content_length_with_transfer_encoding() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding)
        );
        // Order on the wire must not change the outcome.
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding)
        );
    }

    #[test]
    fn rejects_conflicting_content_length() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n"
            ),
            Err(FramingError::DuplicateContentLength)
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5, 6\r\n\r\n"),
            Err(FramingError::DuplicateContentLength)
        );
    }

    #[test]
    fn rejects_malformed_content_length() {
        for raw in [
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: abc\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: \r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: +5\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: -5\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5x\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 0x5\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5 5\r\n\r\n"[..],
            // u64 overflow must not wrap.
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 99999999999999999999999\r\n\r\n"[..],
        ] {
            let h = parse_head(&Bytes::copy_from_slice(raw), &Limits::default())
                .unwrap()
                .unwrap()
                .0;
            assert_eq!(
                decide(&h, &Limits::default()),
                Err(FramingError::InvalidContentLength),
                "should have rejected: {}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    /// RFC 9112 section 6.1: chunked must be the final coding, or the message
    /// length is undetermined.
    #[test]
    fn rejects_chunked_not_final() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked, gzip\r\n\r\n"),
            Err(FramingError::ChunkedNotFinal)
        );
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: gzip\r\n\r\n"
            ),
            Err(FramingError::ChunkedNotFinal),
            "codings accumulate across repeated fields"
        );
    }

    /// Two chunked codings would mean two framing layers; reject rather than
    /// pick one.
    #[test]
    fn rejects_repeated_chunked() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked, chunked\r\n\r\n"
            ),
            Err(FramingError::ChunkedNotFinal)
        );
    }

    #[test]
    fn rejects_unsupported_transfer_coding() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: gzip\r\n\r\n"),
            Err(FramingError::UnsupportedTransferEncoding)
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: identity\r\n\r\n"),
            Err(FramingError::UnsupportedTransferEncoding)
        );
    }

    #[test]
    fn rejects_missing_or_multiple_host_on_http_11() {
        assert_eq!(
            decide_raw(b"GET / HTTP/1.1\r\n\r\n"),
            Err(FramingError::MissingHost)
        );
        assert_eq!(
            decide_raw(b"GET / HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n"),
            Err(FramingError::MultipleHost)
        );
    }

    /// Multiple Host fields are ambiguous regardless of version.
    #[test]
    fn rejects_multiple_host_on_http_10_too() {
        assert_eq!(
            decide_raw(b"GET / HTTP/1.0\r\nHost: a\r\nHost: b\r\n\r\n"),
            Err(FramingError::MultipleHost)
        );
    }

    #[test]
    fn rejects_oversized_declared_body() {
        let limits = Limits {
            max_body_bytes: 4,
            ..Default::default()
        };
        let h = head(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\n");
        assert_eq!(decide(&h, &limits), Err(FramingError::BodyTooLarge));
    }

    // ---- ordering: the conflict check must win ----

    /// A request carrying both a *valid* Content-Length and a *valid*
    /// Transfer-Encoding must be rejected as a conflict, never resolved in favor
    /// of one. This ordering is the smuggling defense.
    #[test]
    fn conflict_check_precedes_individual_analysis() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding),
            "must not report ChunkedNotFinal or a Length body"
        );
    }

    /// An invalid Content-Length alongside a Transfer-Encoding is still first
    /// and foremost a conflict.
    #[test]
    fn conflict_reported_even_when_content_length_is_garbage() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: abc\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding)
        );
    }

    #[test]
    fn status_codes_map_correctly() {
        assert_eq!(FramingError::LengthAndTransferEncoding.status(), 400);
        assert_eq!(FramingError::DuplicateContentLength.status(), 400);
        assert_eq!(FramingError::InvalidContentLength.status(), 400);
        assert_eq!(FramingError::ChunkedNotFinal.status(), 400);
        assert_eq!(FramingError::MissingHost.status(), 400);
        assert_eq!(FramingError::MultipleHost.status(), 400);
        assert_eq!(FramingError::UnsupportedTransferEncoding.status(), 501);
        assert_eq!(FramingError::BodyTooLarge.status(), 413);
    }
}
