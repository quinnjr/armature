# Changelog — `armature-validation`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Security — `IsUrl` rejects embedded control characters.** The WHATWG URL
  parser strips C0 controls (tab, CR, LF among them) *before* parsing, so
  `https://a.example/\r\nX-Injected: 1` parsed cleanly and was accepted. The
  caller keeps the original string, so the validator was approving text it had
  never actually examined, and that text still carried a CRLF into wherever it
  was used next — a `Location` header being the obvious route to response
  splitting.

### Fixed

- **Breaking:** a field with rules that is absent from the input now fails validation. It was silently skipped, so a form missing `email` entirely passed `NotEmpty` and `IsEmail`; `ValidationRules::optional()` opts out.
- `IsUuid` accepts uppercase and checks version/variant; `IsUrl` parses with the `url` crate instead of a regex that rejected `https://a` and accepted `https://!!`.

### Fixed

- A field with rules but **absent** from the input was silently skipped, so a form that simply omitted `email` passed `NotEmpty` and `IsEmail`. A missing required field is now validated as `""`; mark genuinely optional fields with the new `ValidationRules::optional()`. Applies to both `validate` and `validate_parallel`.
- `IsUuid` accepted only lowercase and checked neither version nor variant. It is now case-insensitive per RFC 9562 §4, constrains the version to 1–8 and the variant to 8/9/a/b, and permits the nil and max UUIDs.
- `IsUrl` parses with the `url` crate instead of pattern matching. It previously rejected valid URLs with a single-label host (`https://a`) and accepted invalid ones (`https://!!`).

### Added

- `ValidationRules::optional()`, `is_optional()` and `field_name()`.

### Changed — `0.1.3` → `0.1.4`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
