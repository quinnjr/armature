//! Lazy query-string parsing.
//!
//! The old path parsed *and* percent-decoded the whole query into a
//! `HashMap<String, String>` on every request that had one, whether or not any
//! handler read it. This parses on first access and memoizes, so a handler that
//! ignores the query pays nothing beyond carrying the raw bytes it already had.

use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::HashMap;

/// Parsed query pairs. Eight inline slots covers essentially all real queries.
pub type QueryPairs = SmallVec<[(String, String); 8]>;

/// A parsed view over a request's query string.
///
/// Borrowed from the request, so it cannot outlive it: the view hands out
/// `&str` into the request's memoized pairs, never copies of them. The pairs
/// themselves are owned — percent-decoding has to produce new bytes — but they
/// are built once, on first access, not per lookup.
#[derive(Debug, Clone, Copy)]
pub struct QueryView<'a> {
    pairs: &'a [(String, String)],
}

impl<'a> QueryView<'a> {
    #[inline]
    pub(crate) fn new(pairs: &'a [(String, String)]) -> Self {
        Self { pairs }
    }

    /// The first value for `key`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Whether `key` appears at all.
    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.pairs.iter().any(|(k, _)| k == key)
    }

    /// Every value for `key`, in the order the client sent them.
    #[inline]
    pub fn get_all(&self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.pairs
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Every pair, in the order the client sent them.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The number of pairs, counting repeated keys separately.
    #[inline]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Whether the query carried any pairs.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// An owned copy, for the call sites that genuinely need one.
    ///
    /// This is the allocation the lazy path exists to avoid — reach for it only
    /// when a `HashMap` is actually required. Repeated keys collapse to the last
    /// one, matching `HashMap`'s own insert semantics.
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        self.pairs.iter().cloned().collect()
    }
}

impl<'a> IntoIterator for QueryView<'a> {
    type Item = (&'a str, &'a str);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (String, String)>,
        fn(&'a (String, String)) -> (&'a str, &'a str),
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn as_strs(pair: &(String, String)) -> (&str, &str) {
            (pair.0.as_str(), pair.1.as_str())
        }
        self.pairs.iter().map(as_strs as fn(_) -> _)
    }
}

/// Parse `query` into key/value pairs, percent-decoding both sides.
///
/// Malformed input degrades rather than erroring: a bare key gets an empty
/// value, and an escape that does not decode is preserved verbatim so the
/// handler sees what the client sent. Rejecting a request over a stray `%` would
/// break clients for no security gain — nothing downstream trusts these bytes.
pub(crate) fn parse(query: &str) -> QueryPairs {
    let mut out = QueryPairs::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        if raw_key.is_empty() {
            continue;
        }
        out.push((decode(raw_key).into_owned(), decode(raw_value).into_owned()));
    }
    out
}

/// Percent- and plus-decode one component.
///
/// Returns `Cow::Borrowed` when there is nothing to decode, which is the common
/// case, so the copy happens only for values that need it.
fn decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') && !s.contains('+') {
        return Cow::Borrowed(s);
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                // `get` does the bounds check, so a truncated escape at the end
                // of the input falls into the `None` arm rather than panicking.
                match bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        // Not a valid escape. Keep it as written.
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }

    match String::from_utf8(out) {
        Ok(decoded) => Cow::Owned(decoded),
        // Decoded to non-UTF-8: hand back the raw form rather than lossy text.
        Err(_) => Cow::Borrowed(s),
    }
}

#[cfg(test)]
mod tests {
    use crate::HttpRequest;

    #[test]
    fn parses_on_first_access_and_decodes() {
        let req = HttpRequest::new("GET", "/s?q=hello%20world&page=2");
        let q = req.query();
        assert_eq!(q.get("q"), Some("hello world"));
        assert_eq!(q.get("page"), Some("2"));
        assert_eq!(q.get("absent"), None);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn plus_is_a_space_and_percent_escapes_decode() {
        let req = HttpRequest::new("GET", "/s?a=x+y&b=%2Fpath&c=%E2%9C%93");
        let q = req.query();
        assert_eq!(q.get("a"), Some("x y"));
        assert_eq!(q.get("b"), Some("/path"));
        assert_eq!(q.get("c"), Some("✓"));
    }

    #[test]
    fn repeated_keys_are_all_reachable_and_get_returns_the_first() {
        let req = HttpRequest::new("GET", "/s?tag=a&tag=b");
        let q = req.query();
        assert_eq!(q.get("tag"), Some("a"));
        assert_eq!(q.get_all("tag").collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn no_query_string_is_an_empty_view_not_a_panic() {
        let req = HttpRequest::new("GET", "/s");
        assert!(req.query().is_empty());
        assert_eq!(req.query_string(), None);
        assert_eq!(req.query_param("x"), None);
    }

    #[test]
    fn malformed_input_degrades_rather_than_failing() {
        // A bare key, an empty value, a stray '=', and a truncated escape. None of
        // these is worth rejecting a request over, and all of them appear in real
        // traffic.
        let req = HttpRequest::new("GET", "/s?flag&empty=&=novalue&bad=%zz&trunc=%2");
        let q = req.query();
        assert_eq!(q.get("flag"), Some(""));
        assert_eq!(q.get("empty"), Some(""));
        // An undecodable escape is preserved verbatim rather than dropped, so a
        // handler sees what the client actually sent.
        assert_eq!(q.get("bad"), Some("%zz"));
        assert_eq!(q.get("trunc"), Some("%2"));
        // A pair with an empty key is dropped: there is nothing to look it up by.
        assert_eq!(q.len(), 4);
    }

    #[test]
    fn the_view_is_memoized_across_calls() {
        let req = HttpRequest::new("GET", "/s?a=1");
        let first = req.query().get("a").map(str::to_owned);
        let second = req.query().get("a").map(str::to_owned);
        assert_eq!(first, second);
        // Same backing storage both times: the second call must not re-parse.
        let p1 = req.query().iter().next().map(|(k, _)| k.as_ptr());
        let p2 = req.query().iter().next().map(|(k, _)| k.as_ptr());
        assert_eq!(p1, p2);
    }

    #[test]
    fn cloning_a_request_does_not_carry_a_stale_cache() {
        let req = HttpRequest::new("GET", "/s?a=1");
        assert_eq!(req.query().get("a"), Some("1"));
        let mut clone = req.clone();
        clone.path = crate::ByteStr::from("/s?a=2");
        // The clone's path changed, so its cache must not answer for the old one.
        assert_eq!(clone.query().get("a"), Some("2"));
    }
}
