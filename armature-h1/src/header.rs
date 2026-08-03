//! Header name interning and the header list type.
//!
//! Well-known field names collapse to a discriminant, so a lookup is an integer
//! comparison rather than the case-insensitive string comparison a conventional
//! header map performs.

use crate::ByteStr;
use bytes::Bytes;
use smallvec::SmallVec;

/// An interned HTTP field name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HeaderId {
    /// `host`
    Host,
    /// `content-length`
    ContentLength,
    /// `content-type`
    ContentType,
    /// `transfer-encoding`
    TransferEncoding,
    /// `connection`
    Connection,
    /// `expect`
    Expect,
    /// `upgrade`
    Upgrade,
    /// `date`
    Date,
    /// `server`
    Server,
    /// `accept`
    Accept,
    /// `accept-encoding`
    AcceptEncoding,
    /// `accept-language`
    AcceptLanguage,
    /// `authorization`
    Authorization,
    /// `cache-control`
    CacheControl,
    /// `cookie`
    Cookie,
    /// `set-cookie`
    SetCookie,
    /// `etag`
    Etag,
    /// `if-none-match`
    IfNoneMatch,
    /// `if-modified-since`
    IfModifiedSince,
    /// `last-modified`
    LastModified,
    /// `location`
    Location,
    /// `referer`
    Referer,
    /// `user-agent`
    UserAgent,
    /// `vary`
    Vary,
    /// `allow`
    Allow,
    /// `trailer`
    Trailer,
    /// `te`
    Te,
    /// `content-encoding`
    ContentEncoding,
    /// `content-language`
    ContentLanguage,
    /// `range`
    Range,
    /// `if-match`
    IfMatch,
    /// `if-unmodified-since`
    IfUnmodifiedSince,
    /// `origin`
    Origin,
    /// Any other valid field name, lowercased at parse time.
    Other(ByteStr),
}

impl HeaderId {
    /// Every well-known variant, for round-trip testing and diagnostics.
    pub const WELL_KNOWN: &'static [HeaderId] = &[
        HeaderId::Host,
        HeaderId::ContentLength,
        HeaderId::ContentType,
        HeaderId::TransferEncoding,
        HeaderId::Connection,
        HeaderId::Expect,
        HeaderId::Upgrade,
        HeaderId::Date,
        HeaderId::Server,
        HeaderId::Accept,
        HeaderId::AcceptEncoding,
        HeaderId::AcceptLanguage,
        HeaderId::Authorization,
        HeaderId::CacheControl,
        HeaderId::Cookie,
        HeaderId::SetCookie,
        HeaderId::Etag,
        HeaderId::IfNoneMatch,
        HeaderId::IfModifiedSince,
        HeaderId::LastModified,
        HeaderId::Location,
        HeaderId::Referer,
        HeaderId::UserAgent,
        HeaderId::Vary,
        HeaderId::Allow,
        HeaderId::Trailer,
        HeaderId::Te,
        HeaderId::ContentEncoding,
        HeaderId::ContentLanguage,
        HeaderId::Range,
        HeaderId::IfMatch,
        HeaderId::IfUnmodifiedSince,
        HeaderId::Origin,
    ];

    /// Match a field name against the well-known set, case-insensitively.
    ///
    /// Returns `None` when the name is not well-known; the caller then builds
    /// [`HeaderId::Other`] from the read buffer.
    ///
    /// Dispatching on length first bounds each call to one integer comparison
    /// plus a short case-insensitive compare. Field names are case-insensitive
    /// per RFC 9110 section 5.1.
    #[inline]
    pub fn from_bytes(name: &[u8]) -> Option<HeaderId> {
        macro_rules! m {
            ($($lit:literal => $variant:expr),+ $(,)?) => {{
                $(if name.eq_ignore_ascii_case($lit) { return Some($variant); })+
                None
            }};
        }

        match name.len() {
            2 => m!(b"te" => HeaderId::Te),
            4 => m!(
                b"host" => HeaderId::Host,
                b"date" => HeaderId::Date,
                b"vary" => HeaderId::Vary,
                b"etag" => HeaderId::Etag,
            ),
            5 => m!(b"allow" => HeaderId::Allow, b"range" => HeaderId::Range),
            6 => m!(
                b"accept" => HeaderId::Accept,
                b"expect" => HeaderId::Expect,
                b"cookie" => HeaderId::Cookie,
                b"server" => HeaderId::Server,
                b"origin" => HeaderId::Origin,
            ),
            7 => m!(
                b"upgrade" => HeaderId::Upgrade,
                b"trailer" => HeaderId::Trailer,
                b"referer" => HeaderId::Referer,
            ),
            8 => m!(b"if-match" => HeaderId::IfMatch, b"location" => HeaderId::Location),
            10 => m!(
                b"connection" => HeaderId::Connection,
                b"set-cookie" => HeaderId::SetCookie,
                b"user-agent" => HeaderId::UserAgent,
            ),
            12 => m!(b"content-type" => HeaderId::ContentType),
            13 => m!(
                b"authorization" => HeaderId::Authorization,
                b"cache-control" => HeaderId::CacheControl,
                b"if-none-match" => HeaderId::IfNoneMatch,
                b"last-modified" => HeaderId::LastModified,
            ),
            14 => m!(b"content-length" => HeaderId::ContentLength),
            15 => m!(
                b"accept-encoding" => HeaderId::AcceptEncoding,
                b"accept-language" => HeaderId::AcceptLanguage,
            ),
            16 => m!(
                b"content-encoding" => HeaderId::ContentEncoding,
                b"content-language" => HeaderId::ContentLanguage,
            ),
            17 => m!(
                b"transfer-encoding" => HeaderId::TransferEncoding,
                b"if-modified-since" => HeaderId::IfModifiedSince,
            ),
            19 => m!(b"if-unmodified-since" => HeaderId::IfUnmodifiedSince),
            _ => None,
        }
    }

    /// The canonical lowercase field name.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            HeaderId::Host => "host",
            HeaderId::ContentLength => "content-length",
            HeaderId::ContentType => "content-type",
            HeaderId::TransferEncoding => "transfer-encoding",
            HeaderId::Connection => "connection",
            HeaderId::Expect => "expect",
            HeaderId::Upgrade => "upgrade",
            HeaderId::Date => "date",
            HeaderId::Server => "server",
            HeaderId::Accept => "accept",
            HeaderId::AcceptEncoding => "accept-encoding",
            HeaderId::AcceptLanguage => "accept-language",
            HeaderId::Authorization => "authorization",
            HeaderId::CacheControl => "cache-control",
            HeaderId::Cookie => "cookie",
            HeaderId::SetCookie => "set-cookie",
            HeaderId::Etag => "etag",
            HeaderId::IfNoneMatch => "if-none-match",
            HeaderId::IfModifiedSince => "if-modified-since",
            HeaderId::LastModified => "last-modified",
            HeaderId::Location => "location",
            HeaderId::Referer => "referer",
            HeaderId::UserAgent => "user-agent",
            HeaderId::Vary => "vary",
            HeaderId::Allow => "allow",
            HeaderId::Trailer => "trailer",
            HeaderId::Te => "te",
            HeaderId::ContentEncoding => "content-encoding",
            HeaderId::ContentLanguage => "content-language",
            HeaderId::Range => "range",
            HeaderId::IfMatch => "if-match",
            HeaderId::IfUnmodifiedSince => "if-unmodified-since",
            HeaderId::Origin => "origin",
            HeaderId::Other(s) => s.as_str(),
        }
    }

    /// Whether this field is hop-by-hop and must not be forwarded.
    #[inline]
    pub fn is_hop_by_hop(&self) -> bool {
        matches!(
            self,
            HeaderId::Connection
                | HeaderId::TransferEncoding
                | HeaderId::Te
                | HeaderId::Trailer
                | HeaderId::Upgrade
        )
    }

    /// Whether RFC 9110 section 6.5.1 forbids this field in a trailer section.
    ///
    /// Framing fields are the critical entries: a `Transfer-Encoding` or
    /// `Content-Length` accepted from a trailer is a request-smuggling vector,
    /// because framing was already decided before the trailer was read.
    #[inline]
    pub fn forbidden_in_trailers(&self) -> bool {
        matches!(
            self,
            HeaderId::TransferEncoding
                | HeaderId::ContentLength
                | HeaderId::Host
                | HeaderId::Connection
                | HeaderId::Expect
                | HeaderId::Te
                | HeaderId::Trailer
                | HeaderId::Upgrade
                | HeaderId::CacheControl
                | HeaderId::Authorization
                | HeaderId::SetCookie
        )
    }
}

/// A request or response header list.
///
/// Sixteen inline slots covers the overwhelming majority of real requests, so
/// the list itself does not allocate. Values are [`Bytes`] slices of the
/// connection read buffer.
pub type HeaderVec = SmallVec<[(HeaderId, Bytes); 16]>;

/// The first value for `id`, or `None`.
///
/// A linear scan over at most a few dozen entries beats hashing a field name,
/// and it never allocates.
#[inline]
pub fn get<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a Bytes> {
    v.iter().find(|(k, _)| k == id).map(|(_, val)| val)
}

/// The first value for `id` as a string, or `None` if absent or not UTF-8.
#[inline]
pub fn get_str<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a str> {
    get(v, id).and_then(|b| std::str::from_utf8(b).ok())
}

/// Every value for `id`, in wire order.
#[inline]
pub fn all<'a>(v: &'a HeaderVec, id: &'a HeaderId) -> impl Iterator<Item = &'a Bytes> + 'a {
    v.iter().filter(move |(k, _)| k == id).map(|(_, val)| val)
}

/// How many times `id` appears.
///
/// Framing correctness depends on this: exactly one `Host` is required on
/// HTTP/1.1, and duplicate `Content-Length` fields must be rejected.
#[inline]
pub fn count(v: &HeaderVec, id: &HeaderId) -> usize {
    v.iter().filter(|(k, _)| k == id).count()
}

/// Intern a field name: a well-known variant, or a lowercased [`HeaderId::Other`].
///
/// Lowercasing here means every later comparison is a plain byte compare instead
/// of a case-insensitive one. It allocates only for a name that is not
/// well-known, which is the uncommon case.
#[inline]
pub fn intern(name: &str) -> HeaderId {
    if let Some(id) = HeaderId::from_bytes(name.as_bytes()) {
        return id;
    }
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        return HeaderId::Other(ByteStr::from(name.to_ascii_lowercase()));
    }
    HeaderId::Other(ByteStr::from(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_names_intern_case_insensitively() {
        assert_eq!(HeaderId::from_bytes(b"host"), Some(HeaderId::Host));
        assert_eq!(HeaderId::from_bytes(b"Host"), Some(HeaderId::Host));
        assert_eq!(HeaderId::from_bytes(b"HOST"), Some(HeaderId::Host));
        assert_eq!(HeaderId::from_bytes(b"hOsT"), Some(HeaderId::Host));
        assert_eq!(
            HeaderId::from_bytes(b"content-length"),
            Some(HeaderId::ContentLength)
        );
        assert_eq!(
            HeaderId::from_bytes(b"Transfer-Encoding"),
            Some(HeaderId::TransferEncoding)
        );
        assert_eq!(HeaderId::from_bytes(b"TE"), Some(HeaderId::Te));
    }

    #[test]
    fn unknown_names_are_not_well_known() {
        assert_eq!(HeaderId::from_bytes(b"x-request-id"), None);
        assert_eq!(HeaderId::from_bytes(b""), None);
        // A prefix of a known name must not match it.
        assert_eq!(HeaderId::from_bytes(b"hos"), None);
        assert_eq!(HeaderId::from_bytes(b"hostx"), None);
    }

    #[test]
    fn as_str_uses_canonical_casing_and_round_trips() {
        assert_eq!(HeaderId::Host.as_str(), "host");
        assert_eq!(HeaderId::ContentLength.as_str(), "content-length");
        assert_eq!(HeaderId::Te.as_str(), "te");
        for id in HeaderId::WELL_KNOWN {
            assert_eq!(
                HeaderId::from_bytes(id.as_str().as_bytes()).as_ref(),
                Some(id),
                "{} failed to round-trip",
                id.as_str()
            );
        }
    }

    #[test]
    fn hop_by_hop_classified() {
        assert!(HeaderId::Connection.is_hop_by_hop());
        assert!(HeaderId::TransferEncoding.is_hop_by_hop());
        assert!(HeaderId::Te.is_hop_by_hop());
        assert!(HeaderId::Trailer.is_hop_by_hop());
        assert!(HeaderId::Upgrade.is_hop_by_hop());
        assert!(!HeaderId::ContentType.is_hop_by_hop());
        assert!(!HeaderId::Host.is_hop_by_hop());
    }

    /// RFC 9110 section 6.5.1: a trailer section must not carry framing,
    /// routing, or request-modifier fields. Accepting Transfer-Encoding or
    /// Content-Length in a trailer is a smuggling vector.
    #[test]
    fn framing_headers_forbidden_in_trailers() {
        assert!(HeaderId::TransferEncoding.forbidden_in_trailers());
        assert!(HeaderId::ContentLength.forbidden_in_trailers());
        assert!(HeaderId::Host.forbidden_in_trailers());
        assert!(HeaderId::Connection.forbidden_in_trailers());
        assert!(HeaderId::Expect.forbidden_in_trailers());
        assert!(HeaderId::Te.forbidden_in_trailers());
        assert!(HeaderId::Trailer.forbidden_in_trailers());
        assert!(HeaderId::Upgrade.forbidden_in_trailers());
        assert!(!HeaderId::Etag.forbidden_in_trailers());
        assert!(!HeaderId::ContentType.forbidden_in_trailers());
    }

    fn vec_of(pairs: &[(HeaderId, &'static str)]) -> HeaderVec {
        pairs
            .iter()
            .map(|(id, v)| (id.clone(), Bytes::from_static(v.as_bytes())))
            .collect()
    }

    #[test]
    fn get_returns_first_match() {
        let v = vec_of(&[
            (HeaderId::Host, "a.example"),
            (HeaderId::ContentLength, "5"),
            (HeaderId::Host, "b.example"),
        ]);
        assert_eq!(get_str(&v, &HeaderId::Host), Some("a.example"));
        assert_eq!(get_str(&v, &HeaderId::ContentLength), Some("5"));
        assert_eq!(get(&v, &HeaderId::ContentType), None);
    }

    #[test]
    fn count_and_all_see_every_occurrence() {
        let v = vec_of(&[
            (HeaderId::Host, "a.example"),
            (HeaderId::Host, "b.example"),
            (HeaderId::ContentLength, "5"),
        ]);
        assert_eq!(count(&v, &HeaderId::Host), 2);
        assert_eq!(count(&v, &HeaderId::ContentLength), 1);
        assert_eq!(count(&v, &HeaderId::Date), 0);
        let hosts: Vec<_> = all(&v, &HeaderId::Host).collect();
        assert_eq!(hosts.len(), 2);
        assert_eq!(&hosts[1][..], b"b.example");
    }

    #[test]
    fn custom_names_compare_by_value() {
        let x = HeaderId::Other(ByteStr::from_static("x-request-id"));
        let mut v = HeaderVec::new();
        v.push((x.clone(), Bytes::from_static(b"abc")));
        assert_eq!(get_str(&v, &x), Some("abc"));
        assert_eq!(
            get(&v, &HeaderId::Other(ByteStr::from_static("x-other"))),
            None
        );
    }

    #[test]
    fn get_str_rejects_non_utf8_values() {
        let mut v = HeaderVec::new();
        v.push((HeaderId::ContentType, Bytes::from_static(&[0xff, 0xfe])));
        assert_eq!(get_str(&v, &HeaderId::ContentType), None);
        assert!(get(&v, &HeaderId::ContentType).is_some());
    }

    /// Sixteen inline slots covers the overwhelming majority of real requests,
    /// so the header list itself never allocates.
    #[test]
    fn typical_request_stays_inline() {
        let mut v = HeaderVec::new();
        for _ in 0..16 {
            v.push((HeaderId::Accept, Bytes::from_static(b"*/*")));
        }
        assert!(!v.spilled(), "16 headers must stay on the stack");
        v.push((HeaderId::Accept, Bytes::from_static(b"*/*")));
        assert!(v.spilled(), "17 headers is expected to spill");
    }

    #[test]
    fn intern_prefers_well_known_and_lowercases_the_rest() {
        assert_eq!(intern("Content-Length"), HeaderId::ContentLength);
        assert_eq!(intern("content-length"), HeaderId::ContentLength);
        // A custom name is lowercased once here so every later comparison is a
        // plain byte compare rather than a case-insensitive one.
        assert_eq!(
            intern("X-Tenant-Id"),
            HeaderId::Other(ByteStr::from_static("x-tenant-id"))
        );
        // Already lowercase: no allocation beyond the copy into Bytes.
        assert_eq!(
            intern("x-req-id"),
            HeaderId::Other(ByteStr::from_static("x-req-id"))
        );
    }
}
