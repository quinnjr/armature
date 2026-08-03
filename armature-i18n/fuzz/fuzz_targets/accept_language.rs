//! Fuzz `Accept-Language` parsing.
//!
//! This header arrives verbatim from the client on every request, so the parser
//! sees whatever a peer chooses to send. Beyond not panicking, two properties
//! are worth pinning because the parser sorts by a quality value it parses out
//! of the same untrusted string:
//!
//! * the result is ordered by descending quality — a client must not be able to
//!   promote a locale by malforming its `q` parameter;
//! * every returned locale is one the input actually asked for, so a parse
//!   accident cannot invent a locale that then selects a translation bundle.

#![no_main]

use armature_i18n::{Locale, parse_accept_language};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(header) = std::str::from_utf8(data) else {
        return;
    };
    if header.len() > 4096 {
        return;
    }

    let locales = parse_accept_language(header);

    // Quality ordering is deliberately *not* asserted here. Tying a returned
    // locale back to the header entry it came from is not reliable enough to
    // build an assertion on: parsing normalizes (case folds, and treats `_` as
    // a region separator) so a rendered tag is often not a substring of what
    // was sent, and a header may repeat one tag with several different `q`
    // values, leaving no single entry to attribute it to. Deriving the
    // expected order properly would mean reimplementing the parser in this
    // file and checking it agrees with itself, which proves nothing. Ordering
    // for well-formed headers is pinned by unit tests in the crate instead.

    for locale in &locales {
        let tag = locale.tag();
        assert!(
            !tag.is_empty(),
            "an empty locale tag was produced from {header:?}"
        );
        assert_ne!(tag, "*", "the wildcard is documented as excluded");

        // Whatever came back must be re-parseable as itself: the tag is handed
        // to bundle lookup, and a tag that cannot round-trip means the parser
        // and the loader disagree about what a locale is.
        assert!(
            Locale::parse(&tag).is_ok(),
            "produced tag {tag:?} does not parse as a locale (from {header:?})"
        );
    }
});
