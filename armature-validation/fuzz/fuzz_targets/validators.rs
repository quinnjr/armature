//! Fuzz the string validators.
//!
//! These run on request bodies and query parameters, so every one of them sees
//! attacker-chosen text. Several are regex-backed and one (`IsUrl`) delegates
//! to a real URL parser, which is where the interesting behaviour is: a
//! validator that *accepts* something the rest of the system then treats as
//! trusted is the failure that matters, not one that rejects too much.
//!
//! Beyond absence of panics, the checks here are cross-validations — places
//! where two independent implementations must agree about the same string:
//!
//! * anything `IsUuid` accepts must parse as a UUID;
//! * anything `IsUrl` accepts must parse as an absolute `http`/`https` URL,
//!   which is the property its doc claims and the reason it exists rather than
//!   a bare regex;
//! * `IsAlpha`, `IsAlphanumeric` and `IsNumeric` must agree with Rust's own
//!   character classification, and must not disagree with each other.

#![no_main]

use armature_validation::{
    IsAlpha, IsAlphanumeric, IsEmail, IsNumeric, IsUrl, IsUuid, MaxLength, MinLength, NotEmpty,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(value) = std::str::from_utf8(data) else {
        return;
    };
    if value.len() > 4096 {
        return;
    }

    const FIELD: &str = "field";

    // None of these may panic on arbitrary text.
    let is_email = IsEmail::validate(value, FIELD).is_ok();
    let is_url = IsUrl::validate(value, FIELD).is_ok();
    let is_uuid = IsUuid::validate(value, FIELD).is_ok();
    let is_alpha = IsAlpha::validate(value, FIELD).is_ok();
    let is_alnum = IsAlphanumeric::validate(value, FIELD).is_ok();
    let is_numeric = IsNumeric::validate(value, FIELD).is_ok();

    // A UUID this validator accepts must be one the UUID parser accepts.
    // Downstream code calls `Uuid::parse_str` on validated input, so a
    // disagreement means a value passes validation and then fails to parse.
    if is_uuid {
        assert!(
            uuid::Uuid::parse_str(value).is_ok(),
            "IsUuid accepted {value:?}, which does not parse as a UUID",
        );
    }

    // `IsUrl` documents that it accepts only absolute http/https URLs, and
    // that it rejects control characters. The second half matters more than it
    // looks: the WHATWG parser strips C0 controls *before* parsing, so a value
    // carrying a CRLF parses cleanly while the caller keeps the original —
    // approving a string that was never examined, and handing on a CRLF that
    // becomes response splitting in a `Location` header.
    if is_url {
        assert!(
            !value.chars().any(|c| c.is_control()),
            "IsUrl accepted {value:?}, which carries a control character",
        );
        // Re-parsed here rather than compared against a hand-rolled split:
        // matching `://` textually is not what the parser does, and asserting
        // it would report the difference between the two as a validator bug.
        let scheme = url::Url::parse(value).ok().map(|u| u.scheme().to_owned());
        assert!(
            matches!(scheme.as_deref(), Some("http") | Some("https")),
            "IsUrl accepted {value:?}, whose parsed scheme is {scheme:?}",
        );
    }

    // Character-class validators must agree with the standard library, and are
    // documented as requiring a non-empty run of the relevant class.
    if is_alpha {
        assert!(
            !value.is_empty() && value.chars().all(|c| c.is_alphabetic()),
            "IsAlpha accepted {value:?}",
        );
        assert!(is_alnum, "IsAlpha accepted {value:?} but IsAlphanumeric did not");
    }
    if is_numeric {
        assert!(
            !value.is_empty() && value.chars().all(|c| c.is_numeric()),
            "IsNumeric accepted {value:?}",
        );
    }
    if is_alnum {
        assert!(
            !value.is_empty() && value.chars().all(|c| c.is_alphanumeric()),
            "IsAlphanumeric accepted {value:?}",
        );
    }

    // An email must at least be non-empty; the regex is the definition of the
    // rest, so asserting a second pattern here would only restate it.
    if is_email {
        assert!(!value.is_empty(), "IsEmail accepted the empty string");
    }

    // Length bounds are exact arithmetic and must never disagree with `chars`.
    let len = value.chars().count();
    assert_eq!(
        NotEmpty::validate(value, FIELD).is_ok(),
        !value.trim().is_empty(),
        "NotEmpty disagreed with `trim` on {value:?}",
    );
    assert_eq!(
        MinLength(len).validate(value, FIELD).is_ok(),
        true,
        "MinLength({len}) rejected a value of exactly that length: {value:?}",
    );
    assert_eq!(
        MaxLength(len).validate(value, FIELD).is_ok(),
        true,
        "MaxLength({len}) rejected a value of exactly that length: {value:?}",
    );
    if len > 0 {
        assert!(
            MaxLength(len - 1).validate(value, FIELD).is_err(),
            "MaxLength({}) accepted a longer value: {value:?}",
            len - 1,
        );
    }
});
