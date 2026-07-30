//! Resource limits and deadlines.
//!
//! These are the slowloris and resource-exhaustion defenses. Every field has a
//! finite default; there is deliberately no "unlimited" setting for the head,
//! because an unbounded head read is a trivial memory-exhaustion vector.

use std::time::Duration;

/// The largest `max_headers` the parser's fixed scratch array can serve.
///
/// `parse::parse_head` allocates its `httparse::Header` array on the stack at
/// this size, so [`Limits::max_headers`] must never exceed it.
pub const MAX_HEADERS_CEILING: usize = 128;

/// Per-connection resource limits and deadlines.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Maximum bytes in the request line plus header section. Exceeding this
    /// yields 431 and closes.
    pub max_head_bytes: usize,
    /// Maximum header field count. Exceeding this yields 431 and closes. Must
    /// be `<= MAX_HEADERS_CEILING`.
    pub max_headers: usize,
    /// Maximum request body bytes. Exceeding this yields 413 and closes.
    pub max_body_bytes: u64,
    /// Maximum requests parsed ahead of the one being served. Reaching this
    /// stops reading (backpressure) rather than producing a response.
    pub max_pipeline_depth: usize,
    /// Deadline for the complete head to arrive once the first byte does.
    pub header_timeout: Duration,
    /// Deadline for the complete body to arrive once the head is parsed.
    pub body_timeout: Duration,
    /// Deadline for the next request to begin on an idle keep-alive connection.
    pub idle_timeout: Duration,
    /// Deadline for a response write to complete.
    pub write_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_head_bytes: 16 * 1024,
            max_headers: 96,
            max_body_bytes: 2 * 1024 * 1024,
            max_pipeline_depth: 8,
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(75),
            write_timeout: Duration::from_secs(30),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_values() {
        let l = Limits::default();
        assert_eq!(l.max_head_bytes, 16 * 1024);
        assert_eq!(l.max_headers, 96);
        assert_eq!(l.max_body_bytes, 2 * 1024 * 1024);
        assert_eq!(l.max_pipeline_depth, 8);
        assert_eq!(l.header_timeout, Duration::from_secs(10));
        assert_eq!(l.body_timeout, Duration::from_secs(30));
        assert_eq!(l.idle_timeout, Duration::from_secs(75));
        assert_eq!(l.write_timeout, Duration::from_secs(30));
    }

    /// `max_headers` must not exceed the fixed httparse scratch array in
    /// parse.rs, or heads that fit the limit would fail to parse.
    #[test]
    fn max_headers_fits_the_parser_scratch_array() {
        assert!(Limits::default().max_headers <= MAX_HEADERS_CEILING);
    }

    #[test]
    fn builder_overrides_apply() {
        let l = Limits {
            max_body_bytes: 64,
            ..Default::default()
        };
        assert_eq!(l.max_body_bytes, 64);
        assert_eq!(l.max_headers, 96, "unrelated fields keep their defaults");
    }
}
