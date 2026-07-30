//! Request method and protocol version.

use crate::ByteStr;

/// An HTTP request method.
///
/// Well-known methods are unit variants, so dispatch is a discriminant
/// comparison rather than a string comparison. Unrecognized methods carry a
/// [`ByteStr`] slice of the read buffer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `HEAD`
    Head,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `DELETE`
    Delete,
    /// `CONNECT`
    Connect,
    /// `OPTIONS`
    Options,
    /// `TRACE`
    Trace,
    /// `PATCH`
    Patch,
    /// `QUERY` — a safe method that carries a request body.
    Query,
    /// Any other valid method token.
    Other(ByteStr),
}

impl Method {
    /// Match a method token against the well-known set.
    ///
    /// Returns `None` when the token is not well-known; the caller then builds
    /// [`Method::Other`] from the read buffer. Keeping `Bytes` out of this
    /// signature is what makes the function trivially unit-testable.
    ///
    /// Methods are case-sensitive (RFC 9110 section 9.1), so this compares
    /// exactly. Dispatching on length first means most calls do one integer
    /// comparison and one short memcmp.
    #[inline]
    pub fn from_bytes(token: &[u8]) -> Option<Method> {
        match token.len() {
            3 => match token {
                b"GET" => Some(Method::Get),
                b"PUT" => Some(Method::Put),
                _ => None,
            },
            4 => match token {
                b"HEAD" => Some(Method::Head),
                b"POST" => Some(Method::Post),
                _ => None,
            },
            5 => match token {
                b"PATCH" => Some(Method::Patch),
                b"TRACE" => Some(Method::Trace),
                b"QUERY" => Some(Method::Query),
                _ => None,
            },
            6 => match token {
                b"DELETE" => Some(Method::Delete),
                _ => None,
            },
            7 => match token {
                b"CONNECT" => Some(Method::Connect),
                b"OPTIONS" => Some(Method::Options),
                _ => None,
            },
            _ => None,
        }
    }

    /// The method token as a string.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Connect => "CONNECT",
            Method::Options => "OPTIONS",
            Method::Trace => "TRACE",
            Method::Patch => "PATCH",
            Method::Query => "QUERY",
            Method::Other(s) => s.as_str(),
        }
    }

    /// Whether this method is safe per RFC 9110 section 9.2.1.
    ///
    /// Unrecognized methods are conservatively treated as unsafe.
    #[inline]
    pub fn is_safe(&self) -> bool {
        matches!(
            self,
            Method::Get | Method::Head | Method::Options | Method::Trace | Method::Query
        )
    }

    /// Whether a response to this method may carry a body.
    ///
    /// `HEAD` responses carry headers only, including the `Content-Length` the
    /// equivalent `GET` would have produced (RFC 9112 section 6.3).
    #[inline]
    pub fn expects_response_body(&self) -> bool {
        !matches!(self, Method::Head)
    }
}

/// The HTTP/1 protocol version of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Version {
    /// `HTTP/1.0` — connections close by default.
    Http10,
    /// `HTTP/1.1` — connections persist by default.
    Http11,
}

impl Version {
    /// Map `httparse`'s minor-version byte.
    ///
    /// Anything other than 0 or 1 is not HTTP/1.x and must be answered with 505
    /// rather than guessed at.
    #[inline]
    pub fn from_httparse(minor: u8) -> Option<Version> {
        match minor {
            0 => Some(Version::Http10),
            1 => Some(Version::Http11),
            _ => None,
        }
    }

    /// The version token for a status line.
    #[inline]
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            Version::Http10 => b"HTTP/1.0",
            Version::Http11 => b"HTTP/1.1",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_methods_parse() {
        assert_eq!(Method::from_bytes(b"GET"), Some(Method::Get));
        assert_eq!(Method::from_bytes(b"HEAD"), Some(Method::Head));
        assert_eq!(Method::from_bytes(b"POST"), Some(Method::Post));
        assert_eq!(Method::from_bytes(b"PUT"), Some(Method::Put));
        assert_eq!(Method::from_bytes(b"DELETE"), Some(Method::Delete));
        assert_eq!(Method::from_bytes(b"CONNECT"), Some(Method::Connect));
        assert_eq!(Method::from_bytes(b"OPTIONS"), Some(Method::Options));
        assert_eq!(Method::from_bytes(b"TRACE"), Some(Method::Trace));
        assert_eq!(Method::from_bytes(b"PATCH"), Some(Method::Patch));
        assert_eq!(Method::from_bytes(b"QUERY"), Some(Method::Query));
    }

    /// Methods are case-sensitive per RFC 9110 section 9.1.
    #[test]
    fn methods_are_case_sensitive() {
        assert_eq!(Method::from_bytes(b"get"), None);
        assert_eq!(Method::from_bytes(b"Get"), None);
    }

    #[test]
    fn unknown_method_is_not_well_known() {
        assert_eq!(Method::from_bytes(b"PROPFIND"), None);
        assert_eq!(Method::from_bytes(b""), None);
        assert_eq!(Method::from_bytes(b"GETX"), None);
        assert_eq!(Method::from_bytes(b"GE"), None);
    }

    #[test]
    fn as_str_round_trips() {
        for m in [
            Method::Get,
            Method::Head,
            Method::Post,
            Method::Put,
            Method::Delete,
            Method::Connect,
            Method::Options,
            Method::Trace,
            Method::Patch,
            Method::Query,
        ] {
            assert_eq!(Method::from_bytes(m.as_str().as_bytes()), Some(m.clone()));
        }
        assert_eq!(
            Method::Other(ByteStr::from_static("PROPFIND")).as_str(),
            "PROPFIND"
        );
    }

    #[test]
    fn head_expects_no_response_body() {
        assert!(!Method::Head.expects_response_body());
        assert!(Method::Get.expects_response_body());
    }

    #[test]
    fn safe_methods_classified() {
        assert!(Method::Get.is_safe());
        assert!(Method::Head.is_safe());
        assert!(Method::Options.is_safe());
        assert!(Method::Trace.is_safe());
        assert!(Method::Query.is_safe());
        assert!(!Method::Post.is_safe());
        assert!(!Method::Delete.is_safe());
        assert!(!Method::Other(ByteStr::from_static("PROPFIND")).is_safe());
    }

    #[test]
    fn versions_map_from_httparse() {
        assert_eq!(Version::from_httparse(0), Some(Version::Http10));
        assert_eq!(Version::from_httparse(1), Some(Version::Http11));
        assert_eq!(Version::from_httparse(2), None);
        assert_eq!(Version::Http11.as_bytes(), b"HTTP/1.1");
        assert_eq!(Version::Http10.as_bytes(), b"HTTP/1.0");
    }
}
