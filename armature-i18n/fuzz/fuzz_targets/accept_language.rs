//! Fuzz `Accept-Language` parsing.
//!
//! This header arrives verbatim from the client on every request, so the parser
//! sees whatever a peer chooses to send. Beyond not panicking, the property
//! worth pinning is that every returned locale is one the input actually asked
//! for: the returned tags go on to select a translation bundle, so a parse
//! accident that invents a locale serves content nobody requested.

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

    // Quality ordering is deliberately *not* asserted here. It would need each
    // returned locale tied back to the one header entry it came from, and a
    // header may repeat a tag with several different `q` values, leaving no
    // single entry to attribute it to. Deriving the expected order properly
    // would mean reimplementing the whole parser in this file and checking it
    // agrees with itself, which proves nothing. Ordering for well-formed
    // headers is pinned by unit tests in the crate instead.

    // The languages the header named, in the same normalized form the parser
    // derives: an entry is the text before its first `;`, and its language is
    // that tag up to the first subtag separator, case folded. That much *is*
    // attributable without duplicating the parser, and it is what catches an
    // invented locale. Comparing rendered tags against the raw input would not
    // work — parsing case folds and rewrites `_` as `-`, so a returned tag is
    // often not a substring of what was sent.
    let requested: Vec<String> = header
        .split(',')
        .map(|part| {
            let tag = part.trim().split(';').next().unwrap_or("").trim();
            tag.split(['-', '_']).next().unwrap_or("").to_lowercase()
        })
        .collect();

    for locale in &locales {
        let tag = locale.tag();

        assert!(
            requested.contains(&locale.language),
            "locale {tag:?} was never asked for in {header:?}"
        );

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
