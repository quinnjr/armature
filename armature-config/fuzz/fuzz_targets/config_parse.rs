//! Fuzz configuration parsing across every supported format.
//!
//! Config content is not always trusted input, but it is frequently
//! *operator-supplied and machine-generated* — rendered by Helm, injected by a
//! secrets manager, or assembled from environment. A parser that panics on
//! malformed content turns a templating mistake into a crash loop rather than
//! a startup error, so the property worth holding is that every format returns
//! `Err` rather than unwinding, for any input at all.
//!
//! The format is chosen by the fuzzer rather than sniffed, so each parser is
//! driven with bytes intended for the others — the case a real deployment hits
//! when a file is renamed or an extension is wrong.

#![no_main]

use arbitrary::Arbitrary;
use armature_config::{ConfigLoader, FileFormat};
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
enum Format {
    Json,
    Toml,
    Env,
}

impl Format {
    fn as_file_format(&self) -> FileFormat {
        match self {
            Format::Json => FileFormat::Json,
            Format::Toml => FileFormat::Toml,
            Format::Env => FileFormat::Env,
        }
    }
}

#[derive(Debug, Arbitrary)]
struct Case<'a> {
    format: Format,
    content: &'a str,
}

fuzz_target!(|case: Case<'_>| {
    if case.content.len() > 16 * 1024 {
        return;
    }

    let loader = ConfigLoader::new(case.format.as_file_format());

    // The only contract: a verdict, not a panic.
    let Ok(value) = loader.parse(case.content) else {
        return;
    };

    // Every loader is documented as producing a JSON object, and
    // `ConfigManager` indexes into the result by key. A parse that succeeded
    // while producing a bare scalar would make key lookup silently find
    // nothing rather than report a malformed file.
    assert!(
        value.is_object(),
        "{:?} parsed {:?} into a non-object: {value}",
        case.format,
        case.content,
    );

    // The parsed value is handed to `serde_json` downstream, so it must at
    // least be serializable — a value that is not would fail later, far from
    // the file that caused it.
    //
    // Serializability only; *equality* across a round trip is deliberately not
    // asserted. A TOML float such as `688888888888888e88` reparses to a
    // `Number` that compares unequal to the one it was written from, which is
    // `serde_json`'s float behaviour and applies to any JSON document. Nothing
    // in this crate can act on it, so asserting it here would report a
    // dependency's rounding as a config bug.
    serde_json::to_string(&value).expect("a parsed config must serialize");
});
