//! HTTP integration for TOON responses.

use crate::{TOON_CONTENT_TYPE, ToonError, to_string};
use armature_core::http::HttpResponse;
use bytes::Bytes;
use serde::Serialize;

/// TOON response wrapper for Armature HTTP responses.
pub struct Toon<T>(pub T);

impl<T: Serialize> Toon<T> {
    /// Create a new TOON response.
    pub fn new(value: T) -> Self {
        Toon(value)
    }

    /// Convert to HTTP response.
    pub fn into_response(self) -> Result<HttpResponse, ToonError> {
        let body = to_string(&self.0).map_err(ToonError::from_serialize)?;
        let mut response = HttpResponse::new(200).with_bytes_body(Bytes::from(body));
        response
            .headers
            .insert("Content-Type".to_string(), TOON_CONTENT_TYPE.to_string());
        Ok(response)
    }
}

/// Extension trait for HttpResponse to add TOON support.
pub trait ToonResponseExt {
    /// Create a TOON response from a serializable value.
    fn toon<T: Serialize>(value: T) -> Result<HttpResponse, ToonError>;

    /// Create a TOON response with a custom status code.
    fn toon_with_status<T: Serialize>(status: u16, value: T) -> Result<HttpResponse, ToonError>;
}

impl ToonResponseExt for HttpResponse {
    fn toon<T: Serialize>(value: T) -> Result<HttpResponse, ToonError> {
        let body = to_string(&value).map_err(ToonError::from_serialize)?;
        let mut response = HttpResponse::new(200).with_bytes_body(Bytes::from(body));
        response
            .headers
            .insert("Content-Type".to_string(), TOON_CONTENT_TYPE.to_string());
        Ok(response)
    }

    fn toon_with_status<T: Serialize>(status: u16, value: T) -> Result<HttpResponse, ToonError> {
        let body = to_string(&value).map_err(ToonError::from_serialize)?;
        let mut response = HttpResponse::new(status).with_bytes_body(Bytes::from(body));
        response
            .headers
            .insert("Content-Type".to_string(), TOON_CONTENT_TYPE.to_string());
        Ok(response)
    }
}

/// A single parsed entry from an `Accept` header: a media range paired with
/// its `q=` quality value (defaulting to `1.0` when absent) and its
/// left-to-right position in the original header (used as a tie-breaker,
/// since RFC 7231 does not define an ordering for equal-quality entries).
struct AcceptEntry<'a> {
    media_range: &'a str,
    quality: f32,
    position: usize,
}

/// Parse an `Accept` header into its constituent media ranges with their
/// quality values, in the order they appeared in the header.
///
/// This duplicates the `split(',')` → per-entry `q=` extraction algorithm
/// used by `armature_i18n::parse_accept_language`
/// (`armature-i18n/src/locale.rs`) for `Accept-Language` headers, adapted
/// for media ranges (`type/subtype;q=value`) instead of language tags. The
/// two have since diverged further: this module's `quality_for` also does
/// RFC 7231 §5.3.2 specificity tiering (exact match / `application/*` /
/// `*/*`) that the language parser has no equivalent of (it drops the `*`
/// wildcard entirely rather than ranking it). The duplication is small
/// (~70 lines) and each crate is independently optional, so it isn't
/// extracted here; a future pass could pull a shared "parse an RFC 7231
/// quality-value list" primitive into a common crate, parameterized over
/// how the "tag" (locale vs. media-range) is matched and ranked.
fn parse_accept_header(header: &str) -> Vec<AcceptEntry<'_>> {
    header
        .split(',')
        .enumerate()
        .filter_map(|(position, part)| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }

            let mut split = part.splitn(2, ';');
            let media_range = split.next()?.trim();
            if media_range.is_empty() {
                return None;
            }

            // RFC 7231 §5.3.1 defines valid `q` values as `[0.000, 1.000]`.
            // A value that fails to parse (e.g. `q=abc`) falls back to the
            // same default as an absent `q=` parameter (1.0) -- treated as
            // leniently as a missing parameter, since that can never let a
            // client claim more than the maximum quality. A value that
            // parses but is out of range (e.g. `q=2.0` or `q=-0.5`) is
            // clamped into range so a misbehaving or malicious client can't
            // use an inflated value to force a tie-break win, or a negative
            // value to dodge the `q=0` exclusion rule.
            let quality = split
                .next()
                .and_then(|params| {
                    params.split(';').find_map(|param| {
                        let param = param.trim();
                        param
                            .strip_prefix("q=")
                            .and_then(|v| v.trim().parse::<f32>().ok())
                    })
                })
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);

            Some(AcceptEntry {
                media_range,
                quality,
                position,
            })
        })
        .collect()
}

/// Find the quality value associated with `media_type` in a parsed Accept
/// header, matching exact media types as well as `application/*` and `*/*`
/// wildcards. Returns the quality of the best (highest-priority) match, or
/// `None` if the media type is not accepted at all (i.e. only matched by a
/// `q=0` entry or not matched at all).
fn quality_for(entries: &[AcceptEntry<'_>], media_type: &str) -> Option<(f32, usize)> {
    // RFC 7231 §5.3.2: a more specific media range takes precedence over a
    // less specific one, regardless of where either appears in the header.
    // So rather than pooling all matching entries (exact match,
    // `application/*`, and `*/*`) together before applying the `q=0`
    // filter, walk tiers from most to least specific and stop at the first
    // tier that has any matching entry at all. That tier's own quality
    // value decides the outcome -- including an explicit `q=0`, which means
    // "not accepted" even if a less specific wildcard elsewhere in the
    // header carries a nonzero quality.
    for candidate in [media_type, "application/*", "*/*"] {
        let best = entries
            .iter()
            .filter(|entry| entry.media_range == candidate)
            // Prefer the highest quality within this tier; on a tie,
            // prefer whichever appeared first (lowest position) in the
            // header.
            .max_by(|a, b| {
                a.quality
                    .partial_cmp(&b.quality)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.position.cmp(&a.position))
            });

        if let Some(entry) = best {
            return if entry.quality > 0.0 {
                Some((entry.quality, entry.position))
            } else {
                None
            };
        }
    }

    None
}

/// Content negotiation helper for TOON.
pub struct ToonContentNegotiator;

impl ToonContentNegotiator {
    /// Check if the request accepts TOON format.
    pub fn accepts_toon(accept_header: Option<&str>) -> bool {
        match accept_header {
            Some(accept) => {
                let entries = parse_accept_header(accept);
                quality_for(&entries, TOON_CONTENT_TYPE).is_some()
            }
            None => false,
        }
    }

    /// Check if the request prefers TOON over JSON.
    ///
    /// This parses `q=` quality values from the `Accept` header (per
    /// RFC 7231 §5.3.2) rather than relying on the order in which the
    /// media types textually appear, so e.g.
    /// `application/json;q=0.1, application/toon;q=0.9` correctly prefers
    /// TOON even though `application/json` appears first in the string.
    pub fn prefers_toon(accept_header: Option<&str>) -> bool {
        match accept_header {
            Some(accept) => {
                let entries = parse_accept_header(accept);
                let toon = quality_for(&entries, TOON_CONTENT_TYPE);
                let json = quality_for(&entries, "application/json");

                match (toon, json) {
                    (Some((tq, tp)), Some((jq, jp))) => {
                        // Higher quality wins; on an exact tie, whichever
                        // media type appeared first in the header wins.
                        match tq.partial_cmp(&jq) {
                            Some(std::cmp::Ordering::Greater) => true,
                            Some(std::cmp::Ordering::Less) => false,
                            _ => tp < jp,
                        }
                    }
                    (Some(_), None) => true,
                    _ => false,
                }
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestData {
        name: String,
        value: i32,
    }

    #[test]
    fn test_toon_response() {
        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let response = HttpResponse::toon(data).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&TOON_CONTENT_TYPE.to_string())
        );
    }

    #[test]
    fn test_toon_wrapper() {
        let data = TestData {
            name: "test".to_string(),
            value: 42,
        };

        let toon = Toon::new(data);
        let response = toon.into_response().unwrap();
        assert_eq!(response.status, 200);
    }

    #[test]
    fn test_accepts_toon() {
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "application/toon"
        )));
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "application/json, application/toon"
        )));
        assert!(ToonContentNegotiator::accepts_toon(Some("*/*")));
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/json"
        )));
        assert!(!ToonContentNegotiator::accepts_toon(None));
    }

    #[test]
    fn test_prefers_toon() {
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/toon, application/json"
        )));
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/json, application/toon"
        )));
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/json"
        )));
    }

    /// q-values must win over textual position: JSON appears first in the
    /// header but is given a low quality, while TOON appears second but is
    /// given a high quality, so TOON should be preferred.
    #[test]
    fn test_prefers_toon_respects_q_values_over_position() {
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/json;q=0.1, application/toon;q=0.9"
        )));

        // And the inverse: TOON appears first textually but with a lower
        // quality than JSON, so JSON should win.
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/toon;q=0.2, application/json;q=0.8"
        )));
    }

    #[test]
    fn test_prefers_toon_ignores_q_zero() {
        // TOON is explicitly disabled (q=0), even though it appears first
        // in the header, so JSON should be preferred.
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/toon;q=0, application/json"
        )));
    }

    #[test]
    fn test_accepts_toon_ignores_q_zero() {
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/toon;q=0"
        )));
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/toon;q=0, application/json"
        )));
    }

    #[test]
    fn test_prefers_toon_equal_quality_keeps_positional_tiebreak() {
        // When qualities are equal, the media type that appears first in
        // the header should still win (matches prior positional behavior).
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/toon;q=0.9, application/json;q=0.9"
        )));
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/json;q=0.9, application/toon;q=0.9"
        )));
    }

    /// A more specific media range must win over a co-present wildcard,
    /// regardless of order, per RFC 7231 §5.3.2. In particular, an explicit
    /// `q=0` on the exact media type must exclude it even when a nonzero
    /// `*/*` (or `application/*`) entry is also present in the header --
    /// the wildcard's quality is never consulted once the exact type has
    /// its own entry.
    #[test]
    fn test_accepts_toon_specific_q_zero_overrides_wildcard() {
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "*/*, application/toon;q=0"
        )));
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/toon;q=0, */*"
        )));
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/*, application/toon;q=0"
        )));
        // Sanity check: without the specific exclusion, the wildcard alone
        // still grants acceptance.
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "*/*, application/json;q=0"
        )));
    }

    /// A malformed `q=` value (fails to parse as a float) currently falls
    /// back to the same default used for an absent `q=` parameter: quality
    /// 1.0. This mirrors ordinary HTTP header leniency ("be liberal in what
    /// you accept") and is safe even though it's generous: unlike an
    /// out-of-range value such as `q=2.0`, a malformed value can never let a
    /// client claim *more* than the maximum quality a well-formed entry
    /// could have, since 1.0 is already the ceiling.
    #[test]
    fn test_prefers_toon_treats_malformed_q_as_default() {
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "application/toon;q=abc"
        )));

        // Malformed TOON quality falls back to 1.0, so it beats an
        // explicit lower-quality JSON entry even though JSON appears first
        // in the header.
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/json;q=0.5, application/toon;q=abc"
        )));
    }

    /// RFC 7231 §5.3.1 defines valid `q` values as `[0.000, 1.000]`.
    /// Out-of-range values are clamped into range rather than trusted
    /// as-is, so a misbehaving or malicious client can't set e.g. `q=2.0`
    /// to force an unfair tie-break win against a legitimately maximal
    /// competing entry, nor use a negative value to dodge the `q=0`
    /// exclusion rule.
    #[test]
    fn test_prefers_toon_out_of_range_q() {
        // q=2.0 clamps down to 1.0, so it only *ties* (rather than beats)
        // an explicit q=1.0 JSON entry -- and since JSON appears first,
        // JSON wins the tie.
        assert!(!ToonContentNegotiator::prefers_toon(Some(
            "application/json;q=1.0, application/toon;q=2.0"
        )));
        // With the positions swapped, TOON (now first) wins the same tie.
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/toon;q=2.0, application/json;q=1.0"
        )));

        // q=-0.5 clamps up to 0.0, which is treated as an explicit
        // exclusion, identical to a literal `q=0`.
        assert!(!ToonContentNegotiator::accepts_toon(Some(
            "application/toon;q=-0.5"
        )));
    }

    /// A bare media range with no `;q=` segment defaults to quality 1.0
    /// (the maximum), per RFC 7231 §5.3.1.
    #[test]
    fn test_accepts_toon_default_quality_when_q_absent() {
        assert!(ToonContentNegotiator::accepts_toon(Some(
            "application/toon"
        )));

        // Confirm the default is treated as a genuine 1.0 (not e.g. 0.0 or
        // some lesser fallback): bare TOON beats an explicit low-quality
        // JSON entry that appears earlier in the header.
        assert!(ToonContentNegotiator::prefers_toon(Some(
            "application/json;q=0.5, application/toon"
        )));
    }
}
