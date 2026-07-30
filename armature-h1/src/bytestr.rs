//! A `Bytes` handle carrying a UTF-8 invariant.
//!
//! This is the crate's string type. It exists so that a request target or
//! header value can be a slice of the connection's read buffer — a refcount
//! bump — rather than a freshly allocated `String`.

use bytes::Bytes;
use std::fmt;
use std::ops::Deref;
use std::str::Utf8Error;

/// An immutable UTF-8 string backed by [`Bytes`].
///
/// Cloning is a refcount increment. Slicing a `ByteStr` out of a larger
/// buffer does not copy.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct ByteStr(Bytes);

impl ByteStr {
    /// Wrap `bytes`, validating UTF-8.
    #[inline]
    pub fn from_utf8(bytes: Bytes) -> Result<Self, Utf8Error> {
        std::str::from_utf8(&bytes)?;
        Ok(Self(bytes))
    }

    /// Wrap a static string without allocating.
    #[inline]
    pub fn from_static(s: &'static str) -> Self {
        Self(Bytes::from_static(s.as_bytes()))
    }

    /// The contents as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        // Invariant established in `from_utf8`; `from_static` is UTF-8 by type.
        // Checked rather than unchecked because the crate forbids unsafe code.
        std::str::from_utf8(&self.0).unwrap_or("")
    }

    /// The contents as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Unwrap into the underlying [`Bytes`].
    #[inline]
    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    /// The length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the string is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Copy the contents into an owned `String`.
    ///
    /// The escape hatch for code that retains request data past the response:
    /// holding a `ByteStr` pins the whole pooled buffer it slices (see this
    /// crate's README on buffer pinning), and this breaks that link.
    #[inline]
    pub fn into_owned(&self) -> String {
        self.as_str().to_owned()
    }
}

impl From<&str> for ByteStr {
    /// Copies. A borrowed `&str` has no `Bytes` to share.
    #[inline]
    fn from(s: &str) -> Self {
        Self(Bytes::copy_from_slice(s.as_bytes()))
    }
}

impl From<String> for ByteStr {
    /// Takes the existing allocation rather than copying it.
    #[inline]
    fn from(s: String) -> Self {
        Self(Bytes::from(s.into_bytes()))
    }
}

impl From<Bytes> for ByteStr {
    /// Non-UTF-8 input yields an empty string.
    ///
    /// The `From` contract is infallible, and the alternatives are worse: a panic
    /// puts a remote client in control of process liveness, and an unchecked
    /// conversion would break `#![forbid(unsafe_code)]`. Use
    /// [`ByteStr::from_utf8`] when the distinction matters.
    #[inline]
    fn from(bytes: Bytes) -> Self {
        Self::from_utf8(bytes).unwrap_or_default()
    }
}

impl Deref for ByteStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for ByteStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for ByteStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl PartialEq<str> for ByteStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<&str> for ByteStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_utf8_accepts_valid() {
        let s = ByteStr::from_utf8(Bytes::from_static(b"/index.html")).unwrap();
        assert_eq!(s.as_str(), "/index.html");
        assert_eq!(&s, "/index.html");
        assert_eq!(s.len(), 11);
    }

    #[test]
    fn from_utf8_rejects_invalid() {
        assert!(ByteStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).is_err());
    }

    #[test]
    fn from_static_is_cheap_and_correct() {
        let s = ByteStr::from_static("GET");
        assert_eq!(s.as_str(), "GET");
    }

    /// The load-bearing property: a ByteStr carved out of a larger buffer
    /// shares the allocation rather than copying.
    #[test]
    fn shares_the_parent_allocation() {
        let parent = Bytes::from_static(b"GET /a/b HTTP/1.1");
        let target = ByteStr::from_utf8(parent.slice(4..8)).unwrap();
        assert_eq!(target.as_str(), "/a/b");
        assert_eq!(parent.len(), 17, "parent must be untouched");
    }

    #[test]
    fn deref_gives_str_methods() {
        let s = ByteStr::from_static("/a/b?x=1");
        assert!(s.starts_with("/a"));
        assert_eq!(s.split('?').next(), Some("/a/b"));
    }

    #[test]
    fn empty_is_valid() {
        let s = ByteStr::from_utf8(Bytes::new()).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn from_string_takes_the_allocation_and_from_str_copies() {
        let owned = String::from("/a/b");
        let s = ByteStr::from(owned);
        assert_eq!(s.as_str(), "/a/b");

        let borrowed = ByteStr::from("/c");
        assert_eq!(borrowed.as_str(), "/c");
        assert_eq!(borrowed.into_owned(), "/c".to_string());
    }

    #[test]
    fn from_non_utf8_bytes_is_empty_rather_than_panicking() {
        // Infallible by signature, so invalid input has to go somewhere. Empty is
        // the only answer that cannot be mistaken for real data; callers that need
        // to know use `from_utf8`.
        let s = ByteStr::from(Bytes::from_static(&[0xff, 0xfe]));
        assert!(s.is_empty());
        assert!(ByteStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).is_err());
    }
}
