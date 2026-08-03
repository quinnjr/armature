//! Fuzz locale-tag parsing.
//!
//! `Locale::parse` runs on tags taken from headers, query strings, cookies and
//! stored user preferences, so it must survive arbitrary text. The property
//! worth holding beyond that is idempotence: a tag the parser accepts and
//! renders back out must parse again to the same locale. Bundle lookup keys off
//! the rendered tag, so a parse that does not round-trip means a locale can be
//! stored under one name and looked up under another.

#![no_main]

use armature_i18n::Locale;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(tag) = std::str::from_utf8(data) else {
        return;
    };
    if tag.len() > 512 {
        return;
    }

    let Ok(locale) = Locale::parse(tag) else {
        return;
    };

    let rendered = locale.tag().to_owned();
    assert!(
        !rendered.is_empty(),
        "parsed {tag:?} into a locale that renders empty"
    );

    let reparsed = Locale::parse(&rendered)
        .unwrap_or_else(|e| panic!("{tag:?} rendered as {rendered:?}, which no longer parses: {e}"));

    assert_eq!(
        reparsed.tag(),
        rendered,
        "parsing is not idempotent: {tag:?} -> {rendered:?} -> {:?}",
        reparsed.tag()
    );
    // `Locale` is `PartialEq`, so compare the whole value rather than the
    // fields that happen to be interesting: a new field would otherwise be
    // free to drift across a round trip without any test noticing.
    assert_eq!(
        reparsed, locale,
        "locale drifted across a round trip of {tag:?}"
    );
});
