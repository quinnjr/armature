# Changelog — `armature-i18n`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Behaviour — `parse_accept_language`:** an entry with `q=0` is dropped
  rather than ranked last. RFC 9110 section 12.5.4 defines `q=0` as "not
  acceptable", so such an entry names a locale the client is *refusing*.
  Returning it inverted that: `negotiate_locale` walks the list in order and
  would select the refused locale whenever nothing the client accepted was
  available.
- A quality outside `0..=1` is clamped into it — above `1` becomes `1`, and
  anything negative (including `-inf`) becomes `0` and is therefore refused
  like an explicit `q=0`. A quality that is not a number, or is `NaN`, is
  ignored and behaves as though no `q` had been sent. `str::parse::<f32>`
  accepts `NaN`, `inf` and any magnitude, none of which is a qvalue.
- Accept-Language entries are ordered with `f32::total_cmp`. The previous
  `partial_cmp(..).unwrap_or(Equal)` reported every pair involving a `NaN` as
  equal, which is not transitive, and `sort` is entitled to produce arbitrary
  output from a comparator that is not a total order.

- **Breaking:** the fallback chain is evaluated per key, not per bundle, so a key present in `fr.json` but missing from `fr-CA.json` no longer skips `fr` entirely; `has` follows the same chain `t` does.
- Plural and bundle lookups no longer allocate per probe.
