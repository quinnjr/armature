//! Locale Detection and Accept-Language Parsing
//!
//! Provides locale representation and parsing of Accept-Language headers.

use crate::{I18nError, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Represents a locale (language + optional region).
///
/// # Examples
///
/// ```
/// use armature_i18n::Locale;
/// use std::str::FromStr;
///
/// let en = Locale::new("en", None::<&str>);
/// let en_us = Locale::new("en", Some("US"));
/// let fr_fr = Locale::from_str("fr-FR").unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Locale {
    /// Language code (ISO 639-1, e.g., "en", "fr", "de")
    pub language: String,
    /// Optional region code (ISO 3166-1, e.g., "US", "GB", "FR")
    pub region: Option<String>,
    /// Optional script (e.g., "Latn", "Hans")
    pub script: Option<String>,
}

impl Locale {
    /// Create a new locale.
    pub fn new(language: impl Into<String>, region: Option<impl Into<String>>) -> Self {
        Self {
            language: language.into().to_lowercase(),
            region: region.map(|r| r.into().to_uppercase()),
            script: None,
        }
    }

    /// Create a locale with script.
    pub fn with_script(
        language: impl Into<String>,
        script: Option<impl Into<String>>,
        region: Option<impl Into<String>>,
    ) -> Self {
        Self {
            language: language.into().to_lowercase(),
            region: region.map(|r| r.into().to_uppercase()),
            script: script.map(|s| {
                let s = s.into();
                // Title case for script
                let mut chars = s.chars();
                match chars.next() {
                    Some(first) => first
                        .to_uppercase()
                        .chain(chars.flat_map(|c| c.to_lowercase()))
                        .collect(),
                    None => String::new(),
                }
            }),
        }
    }

    /// Parse from BCP 47 tag (e.g., "en-US", "zh-Hans-CN").
    pub fn parse(tag: &str) -> Result<Self> {
        let parts: Vec<&str> = tag.split(['-', '_']).collect();

        if parts.is_empty() || parts[0].is_empty() {
            return Err(I18nError::InvalidLocale(tag.to_string()));
        }

        let language = parts[0].to_lowercase();

        // Validate language code (2-3 letters)
        if language.len() < 2
            || language.len() > 3
            || !language.chars().all(|c| c.is_ascii_alphabetic())
        {
            return Err(I18nError::InvalidLocale(tag.to_string()));
        }

        let mut script = None;
        let mut region = None;

        for part in parts.iter().skip(1) {
            if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
                // Script (4 letters, e.g., "Hans")
                script = Some(part.to_string());
            } else if part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()) {
                // Region (2 letters, e.g., "US")
                region = Some(part.to_uppercase());
            } else if part.len() == 3 && part.chars().all(|c| c.is_ascii_digit()) {
                // UN M.49 code (3 digits)
                region = Some(part.to_string());
            }
        }

        Ok(Self {
            language,
            script,
            region,
        })
    }

    /// Get the language tag (e.g., "en-US").
    pub fn tag(&self) -> String {
        let mut tag = self.language.clone();
        if let Some(ref script) = self.script {
            tag.push('-');
            tag.push_str(script);
        }
        if let Some(ref region) = self.region {
            tag.push('-');
            tag.push_str(region);
        }
        tag
    }

    /// Get language-only locale (strips region).
    pub fn language_only(&self) -> Self {
        Self {
            language: self.language.clone(),
            script: self.script.clone(),
            region: None,
        }
    }

    /// Check if this locale matches another (with fallback).
    ///
    /// - Exact match: "en-US" matches "en-US"
    /// - Language match: "en-US" matches "en"
    /// - No match: "en-US" doesn't match "fr"
    pub fn matches(&self, other: &Locale) -> bool {
        if self.language != other.language {
            return false;
        }

        // If other has no region, it matches any region of same language
        if other.region.is_none() {
            return true;
        }

        // Otherwise require exact region match
        self.region == other.region
    }

    /// Calculate match score (higher is better).
    ///
    /// - 100: Exact match (language + region + script)
    /// - 50: Language + region match
    /// - 25: Language + script match
    /// - 10: Language only match
    /// - 0: No match
    pub fn match_score(&self, other: &Locale) -> u32 {
        if self.language != other.language {
            return 0;
        }

        let mut score = 10; // Base language match

        if self.region == other.region && self.region.is_some() {
            score += 40;
        }

        if self.script == other.script && self.script.is_some() {
            score += 15;
        }

        // Bonus for exact match
        if self == other {
            score = 100;
        }

        score
    }

    // Common locales

    /// English (no region)
    pub fn en() -> Self {
        Self::new("en", None::<&str>)
    }

    /// English (US)
    pub fn en_us() -> Self {
        Self::new("en", Some("US"))
    }

    /// English (GB)
    pub fn en_gb() -> Self {
        Self::new("en", Some("GB"))
    }

    /// French (no region)
    pub fn fr() -> Self {
        Self::new("fr", None::<&str>)
    }

    /// French (France)
    pub fn fr_fr() -> Self {
        Self::new("fr", Some("FR"))
    }

    /// German (no region)
    pub fn de() -> Self {
        Self::new("de", None::<&str>)
    }

    /// German (Germany)
    pub fn de_de() -> Self {
        Self::new("de", Some("DE"))
    }

    /// Spanish (no region)
    pub fn es() -> Self {
        Self::new("es", None::<&str>)
    }

    /// Spanish (Spain)
    pub fn es_es() -> Self {
        Self::new("es", Some("ES"))
    }

    /// Japanese
    pub fn ja() -> Self {
        Self::new("ja", None::<&str>)
    }

    /// Japanese (Japan)
    pub fn ja_jp() -> Self {
        Self::new("ja", Some("JP"))
    }

    /// Chinese (Simplified)
    pub fn zh_hans() -> Self {
        Self::with_script("zh", Some("Hans"), None::<&str>)
    }

    /// Chinese (Traditional)
    pub fn zh_hant() -> Self {
        Self::with_script("zh", Some("Hant"), None::<&str>)
    }

    /// Chinese (China, Simplified)
    pub fn zh_cn() -> Self {
        Self::with_script("zh", Some("Hans"), Some("CN"))
    }

    /// Chinese (Taiwan, Traditional)
    pub fn zh_tw() -> Self {
        Self::with_script("zh", Some("Hant"), Some("TW"))
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.tag())
    }
}

impl FromStr for Locale {
    type Err = I18nError;

    fn from_str(s: &str) -> Result<Self> {
        Locale::parse(s)
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::en_us()
    }
}

/// Builder for creating locales.
#[derive(Debug, Default)]
pub struct LocaleBuilder {
    language: Option<String>,
    region: Option<String>,
    script: Option<String>,
}

impl LocaleBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the language.
    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }

    /// Set the region.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set the script.
    pub fn script(mut self, script: impl Into<String>) -> Self {
        self.script = Some(script.into());
        self
    }

    /// Build the locale.
    pub fn build(self) -> Result<Locale> {
        let language = self
            .language
            .ok_or_else(|| I18nError::InvalidLocale("Missing language".to_string()))?;
        Ok(Locale::with_script(language, self.script, self.region))
    }
}

// ============================================================================
// Accept-Language Parsing
// ============================================================================

/// Parsed Accept-Language entry with quality value.
#[derive(Debug, Clone)]
struct AcceptLanguageEntry {
    locale: Locale,
    quality: f32,
}

impl PartialEq for AcceptLanguageEntry {
    fn eq(&self, other: &Self) -> bool {
        // Defined in terms of `cmp` so the two agree. `Eq` promises that
        // `a == b` exactly when `a.cmp(b)` is `Equal`, and the order below
        // ranks on quality alone: an equality that also compared the locale,
        // or that compared quality through an epsilon window, would report
        // pairs as unequal that the order calls equal.
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for AcceptLanguageEntry {}

impl PartialOrd for AcceptLanguageEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AcceptLanguageEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher quality first.
        //
        // `total_cmp`, not `partial_cmp(..).unwrap_or(Equal)`: the latter
        // reports every pair involving a NaN as equal, which is not transitive
        // (`NaN == a` and `NaN == b` while `a < b`). `sort` is entitled to
        // produce arbitrary output from a comparator that is not a total
        // order. Quality is validated at parse time so a NaN should not reach
        // here, but the ordering no longer depends on that holding.
        other.quality.total_cmp(&self.quality)
    }
}

/// Parse an Accept-Language header into a list of locales.
///
/// Returns locales sorted by quality (highest first).
///
/// The wildcard `*` is excluded, as is any entry with `q=0` — RFC 9110
/// section 12.5.4 defines that as "not acceptable", so such an entry names a
/// locale the client is refusing rather than one it wants. A quality outside
/// `0..=1` is not a qvalue and is clamped into it: above `1` becomes `1`, and
/// anything negative — including `-inf` — becomes `0` and is therefore refused
/// like an explicit `q=0`. A quality that is not a number at all, or is `NaN`,
/// is treated as though no `q` had been sent.
///
/// # Example
///
/// ```
/// use armature_i18n::parse_accept_language;
///
/// let locales = parse_accept_language("en-US,en;q=0.9,fr;q=0.8,*;q=0.1");
/// assert_eq!(locales.len(), 3); // Excludes wildcard
/// assert_eq!(locales[0].tag(), "en-US");
/// assert_eq!(locales[1].tag(), "en");
/// assert_eq!(locales[2].tag(), "fr");
/// ```
///
/// A refused locale is dropped rather than ranked last:
///
/// ```
/// use armature_i18n::parse_accept_language;
///
/// let locales = parse_accept_language("de;q=0,fr");
/// assert_eq!(locales.len(), 1);
/// assert_eq!(locales[0].tag(), "fr");
/// ```
pub fn parse_accept_language(header: &str) -> Vec<Locale> {
    let mut entries: Vec<AcceptLanguageEntry> = header
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() || part == "*" {
                return None;
            }

            let mut split = part.splitn(2, ';');
            let tag = split.next()?.trim();

            // Skip wildcard
            if tag == "*" {
                return None;
            }

            // Parse quality.
            //
            // Clamped. `str::parse::<f32>` accepts `NaN`, `inf` and any
            // magnitude, none of which is a qvalue: RFC 9110 section 12.4.2
            // defines one as `0[.0*3]` or `1[.0*3]`. An unbounded value would
            // let a client outrank every other entry, and a NaN would reach
            // the comparator, where no ordering of it is meaningful. Anything
            // unparseable is treated as absent, which is the same as sending
            // no `q` at all.
            //
            // Only NaN is rejected before the clamp, because it is the one
            // value with no sign to honour. An infinity is clamped like any
            // other out-of-range magnitude, so `-inf` lands on `0` and is
            // refused exactly as `-1` is; rejecting it instead would drop it
            // through to the `unwrap_or` below and hand the most negative
            // quality expressible the *highest* rank.
            let quality = split
                .next()
                .and_then(|q| {
                    let q = q.trim();
                    q.strip_prefix("q=")
                        .and_then(|v| v.trim().parse::<f32>().ok())
                        .filter(|q| !q.is_nan())
                        .map(|q| q.clamp(0.0, 1.0))
                })
                .unwrap_or(1.0);

            // `q=0` means "not acceptable" (RFC 9110 section 12.5.4), so the
            // client is naming a locale to *exclude*. Returning it anyway
            // inverts the request: `negotiate_locale` walks this list in order
            // and would happily select something the client explicitly
            // refused, whenever nothing it does accept is available.
            if quality <= 0.0 {
                return None;
            }

            let locale = Locale::parse(tag).ok()?;
            Some(AcceptLanguageEntry { locale, quality })
        })
        .collect();

    // Sort by quality (descending)
    entries.sort();

    entries.into_iter().map(|e| e.locale).collect()
}

/// Negotiate the best locale from available locales.
///
/// Uses the Accept-Language preferences to find the best match
/// from the available locales.
///
/// # Example
///
/// ```
/// use armature_i18n::{negotiate_locale, Locale, parse_accept_language};
///
/// let available = vec![Locale::en_us(), Locale::fr_fr(), Locale::de_de()];
/// let requested = parse_accept_language("fr-CA,fr;q=0.9,en;q=0.8");
/// let default = Locale::en_us();
///
/// let best = negotiate_locale(&requested, &available, &default);
/// assert_eq!(best.tag(), "fr-FR"); // fr-CA falls back to fr-FR
/// ```
pub fn negotiate_locale<'a>(
    requested: &[Locale],
    available: &'a [Locale],
    default: &'a Locale,
) -> &'a Locale {
    // Walk the requested locales in preference order. For each, pick the
    // *highest-scoring* available locale rather than the first language match,
    // so a regional/script-specific candidate wins over a coarser one
    // regardless of its position in `available`.
    for req in requested {
        if let Some(best) = best_match(req, available) {
            return best;
        }
    }

    default
}

/// Find the best matching locale using scores.
pub fn best_match<'a>(requested: &Locale, available: &'a [Locale]) -> Option<&'a Locale> {
    let mut best: Option<(&Locale, u32)> = None;

    for locale in available {
        let score = locale.match_score(requested);
        if score > 0 {
            match best {
                Some((_, best_score)) if score > best_score => {
                    best = Some((locale, score));
                }
                None => {
                    best = Some((locale, score));
                }
                _ => {}
            }
        }
    }

    best.map(|(l, _)| l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locale_parse() {
        let en = Locale::parse("en").unwrap();
        assert_eq!(en.language, "en");
        assert!(en.region.is_none());

        let en_us = Locale::parse("en-US").unwrap();
        assert_eq!(en_us.language, "en");
        assert_eq!(en_us.region, Some("US".to_string()));

        let zh_hans_cn = Locale::parse("zh-Hans-CN").unwrap();
        assert_eq!(zh_hans_cn.language, "zh");
        assert_eq!(zh_hans_cn.script, Some("Hans".to_string()));
        assert_eq!(zh_hans_cn.region, Some("CN".to_string()));
    }

    #[test]
    fn test_locale_tag() {
        let locale = Locale::with_script("zh", Some("Hans"), Some("CN"));
        assert_eq!(locale.tag(), "zh-Hans-CN");
    }

    #[test]
    fn test_parse_accept_language() {
        let locales = parse_accept_language("en-US,en;q=0.9,fr;q=0.8");
        assert_eq!(locales.len(), 3);
        assert_eq!(locales[0].tag(), "en-US");
        assert_eq!(locales[1].tag(), "en");
        assert_eq!(locales[2].tag(), "fr");
    }

    #[test]
    fn test_parse_accept_language_with_wildcard() {
        let locales = parse_accept_language("fr-FR,*;q=0.1");
        assert_eq!(locales.len(), 1);
        assert_eq!(locales[0].tag(), "fr-FR");
    }

    /// `q=0` is "not acceptable" (RFC 9110 section 12.5.4). Ranking such an
    /// entry last instead of dropping it inverts the client's meaning, because
    /// `negotiate_locale` walks the list in order and will select a refused
    /// locale whenever nothing the client accepts is on offer.
    #[test]
    fn q_zero_is_a_refusal_not_a_low_preference() {
        let locales = parse_accept_language("de;q=0,fr");
        assert_eq!(
            locales.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            vec!["fr"]
        );

        // The refusal must survive negotiation: with only the refused locale
        // available, the default answers instead.
        let available = vec![Locale::de_de()];
        let default = Locale::en_us();
        let best = negotiate_locale(&locales, &available, &default);
        assert_eq!(
            best.tag(),
            "en-US",
            "a locale the client refused was selected anyway"
        );
    }

    /// `str::parse::<f32>` accepts `NaN`, `inf` and any magnitude; none is a
    /// qvalue. An unbounded value would outrank every honest entry, and a NaN
    /// would reach the comparator, where it is not orderable.
    #[test]
    fn out_of_range_and_non_numeric_qualities_are_treated_as_absent() {
        // An out-of-range quality behaves exactly as though no `q` had been
        // sent. That it then outranks an explicit 0.9 is not a privilege: the
        // client could have sent `q=1`. What matters is that `q=7` cannot buy
        // a rank *above* an honest 1.0, which clamping guarantees.
        let clamped = parse_accept_language("en;q=0.9,de;q=7");
        let absent = parse_accept_language("en;q=0.9,de");
        assert_eq!(
            clamped.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            absent.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            "an out-of-range quality should be indistinguishable from no quality"
        );
        let ceiling = parse_accept_language("en;q=1,de;q=99");
        assert_eq!(
            ceiling.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            vec!["en", "de"],
            "a quality above 1 outranked an honest 1.0 instead of tying with it"
        );

        // NaN is not orderable; it must not reach the sort.
        let locales = parse_accept_language("en;q=NaN,fr;q=0.5");
        assert_eq!(
            locales.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            vec!["en", "fr"]
        );

        // A negative quality clamps to zero, which is a refusal.
        let locales = parse_accept_language("de;q=-1,fr");
        assert_eq!(
            locales.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            vec!["fr"]
        );

        // An infinity is a magnitude like any other, so it clamps by sign
        // rather than being discarded: `-inf` must agree with `-1` and be
        // refused, not fall back to "absent" and thereby outrank everything.
        let locales = parse_accept_language("de;q=-inf,fr");
        assert_eq!(
            locales.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            vec!["fr"],
            "the most negative quality expressible was honoured as a preference"
        );
        let locales = parse_accept_language("en;q=1,de;q=inf");
        assert_eq!(
            locales.iter().map(|l| l.tag()).collect::<Vec<_>>(),
            vec!["en", "de"],
            "an infinite quality outranked an honest 1.0 instead of tying with it"
        );
    }

    #[test]
    fn test_negotiate_locale() {
        let available = vec![Locale::en_us(), Locale::fr_fr(), Locale::de_de()];
        let requested = parse_accept_language("es,fr;q=0.9,en;q=0.8");
        let default = Locale::en_us();

        let best = negotiate_locale(&requested, &available, &default);
        assert_eq!(best.tag(), "fr-FR");
    }

    #[test]
    fn test_negotiate_locale_fallback() {
        let available = vec![Locale::en_us(), Locale::fr_fr()];
        let requested = parse_accept_language("de,ja");
        let default = Locale::en_us();

        let best = negotiate_locale(&requested, &available, &default);
        assert_eq!(best.tag(), "en-US");
    }

    #[test]
    fn test_negotiate_locale_prefers_highest_score() {
        // Regression: negotiation must choose the highest-scoring available
        // locale, not merely the first with any language match. Here the
        // script-specific `zh-Hant` is listed first but `zh-Hans` scores
        // higher against a `zh-Hans-CN` request.
        let available = vec![Locale::zh_hant(), Locale::zh_hans()];
        let requested = vec![Locale::zh_cn()]; // zh-Hans-CN
        let default = Locale::en_us();

        let best = negotiate_locale(&requested, &available, &default);
        assert_eq!(best.tag(), "zh-Hans");
    }

    #[test]
    fn test_match_score() {
        let en_us = Locale::en_us();
        let en = Locale::en();
        let fr = Locale::fr();

        assert_eq!(en_us.match_score(&en_us), 100); // Exact
        assert!(en_us.match_score(&en) > 0); // Language match
        assert_eq!(en_us.match_score(&fr), 0); // No match
    }

    #[test]
    fn test_locale_builder() {
        let locale = LocaleBuilder::new()
            .language("en")
            .region("US")
            .build()
            .unwrap();

        assert_eq!(locale.tag(), "en-US");
    }
}
