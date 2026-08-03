//! The parsed request head.

use crate::header::{self, HeaderId, HeaderVec};
use crate::{ByteStr, Method, Version};
use bytes::Bytes;

/// A parsed request line and header section.
///
/// Every [`Bytes`] within shares the connection's read buffer allocation, so
/// constructing a `Head` copies nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// The request method.
    pub method: Method,
    /// The request target, exactly as received.
    pub target: ByteStr,
    /// The protocol version.
    pub version: Version,
    /// The header fields, in wire order.
    pub headers: HeaderVec,
}

impl Head {
    /// The first value for `id`.
    #[inline]
    pub fn get(&self, id: &HeaderId) -> Option<&Bytes> {
        header::get(&self.headers, id)
    }

    /// The first value for `id` as a string, or `None` if absent or not UTF-8.
    #[inline]
    pub fn get_str(&self, id: &HeaderId) -> Option<&str> {
        header::get_str(&self.headers, id)
    }

    /// Every value for `id`, in wire order.
    #[inline]
    pub fn all<'a>(&'a self, id: &'a HeaderId) -> impl Iterator<Item = &'a Bytes> + 'a {
        header::all(&self.headers, id)
    }

    /// How many times `id` appears.
    #[inline]
    pub fn count(&self, id: &HeaderId) -> usize {
        header::count(&self.headers, id)
    }

    /// The target up to, but excluding, the first `?`.
    #[inline]
    pub fn path(&self) -> &str {
        let t = self.target.as_str();
        match t.as_bytes().iter().position(|&b| b == b'?') {
            Some(i) => &t[..i],
            None => t,
        }
    }

    /// The target after the first `?`, or `None` when there is no `?`.
    ///
    /// A trailing `?` yields `Some("")`, which is distinct from `None`. This is
    /// deliberate: it lets a caller distinguish "no query" from "empty query"
    /// without re-examining the target.
    #[inline]
    pub fn query(&self) -> Option<&str> {
        let t = self.target.as_str();
        t.as_bytes()
            .iter()
            .position(|&b| b == b'?')
            .map(|i| &t[i + 1..])
    }

    /// Whether `Connection` carries `token`, compared case-insensitively
    /// against each comma-separated element.
    pub fn connection_has_token(&self, token: &str) -> bool {
        self.all(&HeaderId::Connection).any(|v| {
            std::str::from_utf8(v)
                .map(|s| s.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
                .unwrap_or(false)
        })
    }

    /// Whether the connection should persist after this request.
    ///
    /// HTTP/1.1 persists unless `Connection: close`; HTTP/1.0 closes unless
    /// `Connection: keep-alive`.
    #[inline]
    pub fn is_keep_alive(&self) -> bool {
        match self.version {
            Version::Http11 => !self.connection_has_token("close"),
            Version::Http10 => self.connection_has_token("keep-alive"),
        }
    }
}
