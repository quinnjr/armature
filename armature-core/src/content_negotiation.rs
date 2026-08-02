//! Content negotiation for HTTP requests.
//!
//! This module provides support for HTTP content negotiation, allowing servers
//! to serve different representations of a resource based on client preferences.
//!
//! # Supported Headers
//!
//! - `Accept` - Media type negotiation (e.g., `application/json`, `text/html`)
//! - `Accept-Language` - Language negotiation (e.g., `en-US`, `fr`)
//! - `Accept-Charset` - Character set negotiation (e.g., `utf-8`, `iso-8859-1`)
//! - `Accept-Encoding` - Encoding negotiation (e.g., `gzip`, `br`)
//!
//! # Examples
//!
//! ```
//! use armature_core::content_negotiation::{Accept, MediaType, negotiate_media_type};
//!
//! // Parse Accept header
//! let accept = Accept::parse("application/json, text/html;q=0.9, */*;q=0.1");
//!
//! // Negotiate best media type from available options
//! let available = vec![
//!     MediaType::json(),
//!     MediaType::html(),
//!     MediaType::xml(),
//! ];
//! let best = negotiate_media_type(&accept, &available);
//! assert_eq!(best, Some(&MediaType::json()));
//! ```

use crate::{Error, HttpRequest, HttpResponse};
use bytes::Bytes;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

// ============================================================================
// Media Types
// ============================================================================

/// Represents a media type (MIME type) with optional parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaType {
    /// The type (e.g., "application", "text", "image")
    pub type_: String,
    /// The subtype (e.g., "json", "html", "png")
    pub subtype: String,
    /// Optional parameters (e.g., charset=utf-8)
    pub params: HashMap<String, String>,
}

impl MediaType {
    /// Create a new media type.
    pub fn new(type_: impl Into<String>, subtype: impl Into<String>) -> Self {
        Self {
            type_: type_.into(),
            subtype: subtype.into(),
            params: HashMap::new(),
        }
    }

    /// Create a media type with a parameter.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// Create `application/json` media type.
    pub fn json() -> Self {
        Self::new("application", "json")
    }

    /// Create `text/html` media type.
    pub fn html() -> Self {
        Self::new("text", "html")
    }

    /// Create `text/plain` media type.
    pub fn plain_text() -> Self {
        Self::new("text", "plain")
    }

    /// Create `application/xml` media type.
    pub fn xml() -> Self {
        Self::new("application", "xml")
    }

    /// Create `text/xml` media type.
    pub fn text_xml() -> Self {
        Self::new("text", "xml")
    }

    /// Create `application/x-www-form-urlencoded` media type.
    pub fn form_urlencoded() -> Self {
        Self::new("application", "x-www-form-urlencoded")
    }

    /// Create `multipart/form-data` media type.
    pub fn multipart_form_data() -> Self {
        Self::new("multipart", "form-data")
    }

    /// Create `application/octet-stream` media type.
    pub fn octet_stream() -> Self {
        Self::new("application", "octet-stream")
    }

    /// Create `*/*` wildcard media type.
    pub fn any() -> Self {
        Self::new("*", "*")
    }

    /// Parse a media type from a string (without quality value).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let mut parts = s.split(';');

        let type_subtype = parts.next()?.trim();
        let mut type_parts = type_subtype.splitn(2, '/');

        let type_ = type_parts.next()?.trim().to_lowercase();
        let subtype = type_parts.next()?.trim().to_lowercase();

        let mut params = HashMap::new();
        for param in parts {
            let param = param.trim();
            if let Some((key, value)) = param.split_once('=') {
                let key = key.trim().to_lowercase();
                let value = value.trim().trim_matches('"').to_string();
                // Skip quality parameter
                if key != "q" {
                    params.insert(key, value);
                }
            }
        }

        Some(Self {
            type_,
            subtype,
            params,
        })
    }

    /// Check if this media type matches another (considering wildcards).
    pub fn matches(&self, other: &MediaType) -> bool {
        let type_matches = self.type_ == "*" || other.type_ == "*" || self.type_ == other.type_;
        let subtype_matches =
            self.subtype == "*" || other.subtype == "*" || self.subtype == other.subtype;
        type_matches && subtype_matches
    }

    /// Check if this is a wildcard type (`*/*`).
    pub fn is_any(&self) -> bool {
        self.type_ == "*" && self.subtype == "*"
    }

    /// Check if the type is a wildcard (`*/something`).
    pub fn is_type_wildcard(&self) -> bool {
        self.type_ == "*"
    }

    /// Check if the subtype is a wildcard (`something/*`).
    pub fn is_subtype_wildcard(&self) -> bool {
        self.subtype == "*"
    }

    /// Get the full MIME type string.
    pub fn mime_type(&self) -> String {
        format!("{}/{}", self.type_, self.subtype)
    }

    /// Get the full MIME type string with parameters.
    pub fn to_header_value(&self) -> String {
        let mut result = self.mime_type();
        for (key, value) in &self.params {
            result.push_str(&format!("; {}={}", key, value));
        }
        result
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_header_value())
    }
}

/// Find the byte offset of a case-insensitive `;q=` in `s` without allocating.
///
/// Searching a lowercased copy would be incorrect: lowercasing can change the
/// byte length of non-ASCII text, so offsets found in the copy may not be
/// valid (or even char-aligned) in the original string.
fn find_quality_param(s: &str) -> Option<usize> {
    s.as_bytes()
        .windows(3)
        .position(|w| w[0] == b';' && w[1].eq_ignore_ascii_case(&b'q') && w[2] == b'=')
}

// ============================================================================
// Accept Header
// ============================================================================

/// Represents a parsed `Accept` header with quality values.
#[derive(Debug, Clone)]
pub struct Accept {
    /// Media types with their quality values, sorted by preference.
    pub media_types: Vec<(MediaType, f32)>,
}

impl Default for Accept {
    /// A missing Accept header accepts anything, per the HTTP spec.
    fn default() -> Self {
        Self::new()
    }
}

impl Accept {
    /// Create an empty Accept header (accepts anything).
    pub fn new() -> Self {
        Self {
            media_types: vec![(MediaType::any(), 1.0)],
        }
    }

    /// Parse an Accept header string.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::content_negotiation::Accept;
    ///
    /// let accept = Accept::parse("application/json, text/html;q=0.9, */*;q=0.1");
    /// assert_eq!(accept.media_types.len(), 3);
    /// ```
    pub fn parse(header: &str) -> Self {
        let mut media_types: Vec<(MediaType, f32)> = header
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return None;
                }

                // Extract quality value
                let (media_part, quality) = Self::extract_quality(part);

                MediaType::parse(media_part).map(|mt| (mt, quality))
            })
            .collect();

        // Sort by quality (highest first), then by specificity
        media_types.sort_by(|a, b| {
            // First compare by quality
            match b.1.partial_cmp(&a.1) {
                Some(Ordering::Equal) | None => {}
                Some(ord) => return ord,
            }

            // Then by specificity (more specific = higher priority)
            let a_specificity = Self::specificity(&a.0);
            let b_specificity = Self::specificity(&b.0);
            b_specificity.cmp(&a_specificity)
        });

        Self { media_types }
    }

    /// Extract quality value from a media type string.
    fn extract_quality(s: &str) -> (&str, f32) {
        // Find q= parameter
        if let Some(q_pos) = find_quality_param(s) {
            let media_part = &s[..q_pos];
            let q_part = &s[q_pos + 3..];

            // Parse quality value
            let quality = q_part
                .split(';')
                .next()
                .and_then(|q| q.trim().parse::<f32>().ok())
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);

            (media_part, quality)
        } else {
            (s, 1.0)
        }
    }

    /// Calculate specificity of a media type.
    fn specificity(mt: &MediaType) -> u8 {
        let mut score = 0u8;
        if mt.type_ != "*" {
            score += 2;
        }
        if mt.subtype != "*" {
            score += 1;
        }
        score
    }

    /// Check if a media type is acceptable.
    pub fn accepts(&self, media_type: &MediaType) -> bool {
        self.quality_for(media_type) > 0.0
    }

    /// Get the quality value for a specific media type.
    pub fn quality_for(&self, media_type: &MediaType) -> f32 {
        for (mt, quality) in &self.media_types {
            if mt.matches(media_type) {
                return *quality;
            }
        }
        0.0
    }

    /// Get the preferred media type from this Accept header.
    pub fn preferred(&self) -> Option<&MediaType> {
        self.media_types.first().map(|(mt, _)| mt)
    }

    /// Check if JSON is preferred over HTML.
    pub fn prefers_json(&self) -> bool {
        self.quality_for(&MediaType::json()) > self.quality_for(&MediaType::html())
    }

    /// Check if HTML is preferred over JSON.
    pub fn prefers_html(&self) -> bool {
        self.quality_for(&MediaType::html()) > self.quality_for(&MediaType::json())
    }
}

/// Negotiate the best media type from available options.
///
/// Returns the media type from `available` that best matches the client's
/// preferences in `accept`.
pub fn negotiate_media_type<'a>(
    accept: &Accept,
    available: &'a [MediaType],
) -> Option<&'a MediaType> {
    let mut best: Option<(&'a MediaType, f32, u8)> = None;

    for available_mt in available {
        let quality = accept.quality_for(available_mt);
        if quality > 0.0 {
            let specificity = Accept::specificity(available_mt);
            match &best {
                None => best = Some((available_mt, quality, specificity)),
                Some((_, best_q, best_s)) => {
                    if quality > *best_q || (quality == *best_q && specificity > *best_s) {
                        best = Some((available_mt, quality, specificity));
                    }
                }
            }
        }
    }

    best.map(|(mt, _, _)| mt)
}

// ============================================================================
// Accept-Language Header
// ============================================================================

/// Represents a language tag with quality value.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageTag {
    /// The primary language (e.g., "en", "fr", "de")
    pub primary: String,
    /// Optional subtag (e.g., "US", "GB" for en-US, en-GB)
    pub subtag: Option<String>,
}

impl LanguageTag {
    /// Create a new language tag.
    pub fn new(primary: impl Into<String>) -> Self {
        Self {
            primary: primary.into().to_lowercase(),
            subtag: None,
        }
    }

    /// Create a language tag with subtag.
    pub fn with_subtag(primary: impl Into<String>, subtag: impl Into<String>) -> Self {
        Self {
            primary: primary.into().to_lowercase(),
            subtag: Some(subtag.into().to_uppercase()),
        }
    }

    /// Parse a language tag from a string.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() || s == "*" {
            return Some(Self::new("*"));
        }

        let mut parts = s.splitn(2, '-');
        let primary = parts.next()?.trim().to_lowercase();
        let subtag = parts.next().map(|s| s.trim().to_uppercase());

        Some(Self { primary, subtag })
    }

    /// Check if this tag matches another (considering wildcards).
    pub fn matches(&self, other: &LanguageTag) -> bool {
        if self.primary == "*" || other.primary == "*" {
            return true;
        }
        if self.primary != other.primary {
            return false;
        }
        // If we have a subtag, it must match
        match (&self.subtag, &other.subtag) {
            (Some(a), Some(b)) => a == b,
            (None, _) => true,        // "en" matches "en-US"
            (Some(_), None) => false, // "en-US" doesn't match just "en"
        }
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.subtag {
            Some(sub) => write!(f, "{}-{}", self.primary, sub),
            None => write!(f, "{}", self.primary),
        }
    }
}

/// Represents a parsed `Accept-Language` header.
#[derive(Debug, Clone, Default)]
pub struct AcceptLanguage {
    /// Language tags with their quality values, sorted by preference.
    pub languages: Vec<(LanguageTag, f32)>,
}

impl AcceptLanguage {
    /// Parse an Accept-Language header string.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::content_negotiation::AcceptLanguage;
    ///
    /// let accept = AcceptLanguage::parse("en-US, en;q=0.9, fr;q=0.8, *;q=0.1");
    /// assert_eq!(accept.languages.len(), 4);
    /// ```
    pub fn parse(header: &str) -> Self {
        let mut languages: Vec<(LanguageTag, f32)> = header
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return None;
                }

                let (lang_part, quality) = Self::extract_quality(part);
                LanguageTag::parse(lang_part).map(|lt| (lt, quality))
            })
            .collect();

        // Sort by quality (highest first)
        languages.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

        Self { languages }
    }

    fn extract_quality(s: &str) -> (&str, f32) {
        if let Some(q_pos) = find_quality_param(s) {
            let lang_part = &s[..q_pos];
            let q_part = &s[q_pos + 3..];

            let quality = q_part.trim().parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);

            (lang_part, quality)
        } else {
            (s, 1.0)
        }
    }

    /// Get the quality value for a specific language.
    pub fn quality_for(&self, language: &LanguageTag) -> f32 {
        for (lt, quality) in &self.languages {
            if lt.matches(language) {
                return *quality;
            }
        }
        0.0
    }

    /// Get the preferred language.
    pub fn preferred(&self) -> Option<&LanguageTag> {
        self.languages.first().map(|(lt, _)| lt)
    }
}

/// Negotiate the best language from available options.
pub fn negotiate_language<'a>(
    accept: &AcceptLanguage,
    available: &'a [LanguageTag],
) -> Option<&'a LanguageTag> {
    let mut best: Option<(&'a LanguageTag, f32)> = None;

    for available_lt in available {
        let quality = accept.quality_for(available_lt);
        if quality > 0.0 {
            match &best {
                None => best = Some((available_lt, quality)),
                Some((_, best_q)) if quality > *best_q => {
                    best = Some((available_lt, quality));
                }
                _ => {}
            }
        }
    }

    best.map(|(lt, _)| lt)
}

// ============================================================================
// Accept-Encoding Header
// ============================================================================

/// Supported content encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// Gzip compression
    Gzip,
    /// Deflate compression
    Deflate,
    /// Brotli compression
    Brotli,
    /// Zstandard compression
    Zstd,
    /// No encoding (identity)
    Identity,
}

impl Encoding {
    /// Parse an encoding from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "gzip" | "x-gzip" => Some(Self::Gzip),
            "deflate" => Some(Self::Deflate),
            "br" => Some(Self::Brotli),
            "zstd" => Some(Self::Zstd),
            "identity" => Some(Self::Identity),
            _ => None,
        }
    }

    /// Get the header value for this encoding.
    pub fn to_header_value(&self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Brotli => "br",
            Self::Zstd => "zstd",
            Self::Identity => "identity",
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_header_value())
    }
}

/// Represents a parsed `Accept-Encoding` header.
#[derive(Debug, Clone, Default)]
pub struct AcceptEncoding {
    /// Encodings with their quality values, sorted by preference.
    pub encodings: Vec<(Encoding, f32)>,
}

impl AcceptEncoding {
    /// Parse an Accept-Encoding header string.
    ///
    /// # Example
    ///
    /// ```
    /// use armature_core::content_negotiation::AcceptEncoding;
    ///
    /// let accept = AcceptEncoding::parse("gzip, deflate, br;q=0.9");
    /// assert_eq!(accept.encodings.len(), 3);
    /// ```
    pub fn parse(header: &str) -> Self {
        let mut encodings: Vec<(Encoding, f32)> = header
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return None;
                }

                let (enc_part, quality) = Self::extract_quality(part);
                Encoding::parse(enc_part).map(|enc| (enc, quality))
            })
            .collect();

        // Sort by quality (highest first)
        encodings.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

        Self { encodings }
    }

    fn extract_quality(s: &str) -> (&str, f32) {
        if let Some(q_pos) = find_quality_param(s) {
            let enc_part = &s[..q_pos];
            let q_part = &s[q_pos + 3..];

            let quality = q_part.trim().parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);

            (enc_part, quality)
        } else {
            (s, 1.0)
        }
    }

    /// Get the quality value for a specific encoding.
    pub fn quality_for(&self, encoding: Encoding) -> f32 {
        for (enc, quality) in &self.encodings {
            if *enc == encoding {
                return *quality;
            }
        }
        0.0
    }

    /// Get the preferred encoding.
    pub fn preferred(&self) -> Option<Encoding> {
        self.encodings.first().map(|(enc, _)| *enc)
    }

    /// Check if an encoding is acceptable.
    pub fn accepts(&self, encoding: Encoding) -> bool {
        self.quality_for(encoding) > 0.0
    }
}

/// Negotiate the best encoding from available options.
pub fn negotiate_encoding(accept: &AcceptEncoding, available: &[Encoding]) -> Option<Encoding> {
    let mut best: Option<(Encoding, f32)> = None;

    for &enc in available {
        let quality = accept.quality_for(enc);
        if quality > 0.0 {
            match &best {
                None => best = Some((enc, quality)),
                Some((_, best_q)) if quality > *best_q => {
                    best = Some((enc, quality));
                }
                _ => {}
            }
        }
    }

    best.map(|(enc, _)| enc)
}

// ============================================================================
// Accept-Charset Header
// ============================================================================

/// Represents a parsed `Accept-Charset` header.
#[derive(Debug, Clone, Default)]
pub struct AcceptCharset {
    /// Charsets with their quality values, sorted by preference.
    pub charsets: Vec<(String, f32)>,
}

impl AcceptCharset {
    /// Parse an Accept-Charset header string.
    pub fn parse(header: &str) -> Self {
        let mut charsets: Vec<(String, f32)> = header
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                if part.is_empty() {
                    return None;
                }

                let (charset_part, quality) = Self::extract_quality(part);
                Some((charset_part.trim().to_lowercase(), quality))
            })
            .collect();

        // Sort by quality (highest first)
        charsets.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

        Self { charsets }
    }

    fn extract_quality(s: &str) -> (&str, f32) {
        if let Some(q_pos) = find_quality_param(s) {
            let charset_part = &s[..q_pos];
            let q_part = &s[q_pos + 3..];

            let quality = q_part.trim().parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);

            (charset_part, quality)
        } else {
            (s, 1.0)
        }
    }

    /// Get the quality value for a specific charset.
    pub fn quality_for(&self, charset: &str) -> f32 {
        let charset = charset.to_lowercase();
        for (cs, quality) in &self.charsets {
            if cs == &charset || cs == "*" {
                return *quality;
            }
        }
        // UTF-8 is acceptable by default per HTTP spec
        if charset == "utf-8" {
            return 1.0;
        }
        0.0
    }

    /// Get the preferred charset.
    pub fn preferred(&self) -> Option<&str> {
        self.charsets.first().map(|(cs, _)| cs.as_str())
    }
}

// ============================================================================
// HttpRequest Extensions
// ============================================================================

/// Extension methods for HttpRequest to support content negotiation.
impl HttpRequest {
    /// Get the Accept header parsed into media types.
    pub fn accept(&self) -> Accept {
        self.headers
            .get("Accept")
            .or_else(|| self.headers.get("accept"))
            .map(Accept::parse)
            .unwrap_or_default()
    }

    /// Get the Accept-Language header parsed into language tags.
    pub fn accept_language(&self) -> AcceptLanguage {
        self.headers
            .get("Accept-Language")
            .or_else(|| self.headers.get("accept-language"))
            .map(AcceptLanguage::parse)
            .unwrap_or_default()
    }

    /// Get the Accept-Encoding header parsed into encodings.
    pub fn accept_encoding(&self) -> AcceptEncoding {
        self.headers
            .get("Accept-Encoding")
            .or_else(|| self.headers.get("accept-encoding"))
            .map(AcceptEncoding::parse)
            .unwrap_or_default()
    }

    /// Get the Accept-Charset header parsed into charsets.
    pub fn accept_charset(&self) -> AcceptCharset {
        self.headers
            .get("Accept-Charset")
            .or_else(|| self.headers.get("accept-charset"))
            .map(AcceptCharset::parse)
            .unwrap_or_default()
    }

    /// Check if the client accepts a specific media type.
    pub fn accepts(&self, media_type: &MediaType) -> bool {
        self.accept().accepts(media_type)
    }

    /// Check if the client prefers JSON over HTML.
    pub fn prefers_json(&self) -> bool {
        self.accept().prefers_json()
    }

    /// Check if the client prefers HTML over JSON.
    pub fn prefers_html(&self) -> bool {
        self.accept().prefers_html()
    }

    /// Negotiate the best media type from available options.
    pub fn negotiate_media_type<'a>(&self, available: &'a [MediaType]) -> Option<&'a MediaType> {
        negotiate_media_type(&self.accept(), available)
    }

    /// Negotiate the best language from available options.
    pub fn negotiate_language<'a>(&self, available: &'a [LanguageTag]) -> Option<&'a LanguageTag> {
        negotiate_language(&self.accept_language(), available)
    }

    /// Negotiate the best encoding from available options.
    pub fn negotiate_encoding(&self, available: &[Encoding]) -> Option<Encoding> {
        negotiate_encoding(&self.accept_encoding(), available)
    }
}

// ============================================================================
// Content Negotiation Response Helper
// ============================================================================

/// Helper for building responses with content negotiation.
///
/// # Example
///
/// ```ignore
/// use armature_core::content_negotiation::{ContentNegotiator, MediaType};
///
/// let negotiator = ContentNegotiator::new()
///     .json(|| serde_json::json!({"message": "Hello"}))
///     .html(|| "<h1>Hello</h1>".to_string())
///     .plain_text(|| "Hello".to_string());
///
/// let response = negotiator.negotiate(&request)?;
/// ```
pub struct ContentNegotiator<J, H, T, X>
where
    J: FnOnce() -> serde_json::Value,
    H: FnOnce() -> String,
    T: FnOnce() -> String,
    X: FnOnce() -> String,
{
    json_fn: Option<J>,
    html_fn: Option<H>,
    text_fn: Option<T>,
    xml_fn: Option<X>,
    default_media_type: MediaType,
}

impl ContentNegotiator<fn() -> serde_json::Value, fn() -> String, fn() -> String, fn() -> String> {
    /// Create a new content negotiator with JSON as the default.
    pub fn new() -> Self {
        Self {
            json_fn: None,
            html_fn: None,
            text_fn: None,
            xml_fn: None,
            default_media_type: MediaType::json(),
        }
    }
}

impl<J, H, T, X> ContentNegotiator<J, H, T, X>
where
    J: FnOnce() -> serde_json::Value,
    H: FnOnce() -> String,
    T: FnOnce() -> String,
    X: FnOnce() -> String,
{
    /// Set the JSON response generator.
    pub fn json<NJ: FnOnce() -> serde_json::Value>(self, f: NJ) -> ContentNegotiator<NJ, H, T, X> {
        ContentNegotiator {
            json_fn: Some(f),
            html_fn: self.html_fn,
            text_fn: self.text_fn,
            xml_fn: self.xml_fn,
            default_media_type: self.default_media_type,
        }
    }

    /// Set the HTML response generator.
    pub fn html<NH: FnOnce() -> String>(self, f: NH) -> ContentNegotiator<J, NH, T, X> {
        ContentNegotiator {
            json_fn: self.json_fn,
            html_fn: Some(f),
            text_fn: self.text_fn,
            xml_fn: self.xml_fn,
            default_media_type: self.default_media_type,
        }
    }

    /// Set the plain text response generator.
    pub fn plain_text<NT: FnOnce() -> String>(self, f: NT) -> ContentNegotiator<J, H, NT, X> {
        ContentNegotiator {
            json_fn: self.json_fn,
            html_fn: self.html_fn,
            text_fn: Some(f),
            xml_fn: self.xml_fn,
            default_media_type: self.default_media_type,
        }
    }

    /// Set the XML response generator.
    pub fn xml<NX: FnOnce() -> String>(self, f: NX) -> ContentNegotiator<J, H, T, NX> {
        ContentNegotiator {
            json_fn: self.json_fn,
            html_fn: self.html_fn,
            text_fn: self.text_fn,
            xml_fn: Some(f),
            default_media_type: self.default_media_type,
        }
    }

    /// Set the default media type when no Accept header is present.
    pub fn default_to(mut self, media_type: MediaType) -> Self {
        self.default_media_type = media_type;
        self
    }

    /// Negotiate and build the response based on the request's Accept header.
    pub fn negotiate(self, request: &HttpRequest) -> Result<HttpResponse, Error> {
        let accept = request.accept();

        // Build list of available media types
        let mut available = Vec::new();
        if self.json_fn.is_some() {
            available.push(MediaType::json());
        }
        if self.html_fn.is_some() {
            available.push(MediaType::html());
        }
        if self.text_fn.is_some() {
            available.push(MediaType::plain_text());
        }
        if self.xml_fn.is_some() {
            available.push(MediaType::xml());
        }

        // If no formats available, return error
        if available.is_empty() {
            return Err(Error::Internal(
                "No response formats configured".to_string(),
            ));
        }

        // Negotiate best media type
        let best = negotiate_media_type(&accept, &available)
            .cloned()
            .unwrap_or_else(|| self.default_media_type.clone());

        // Build response based on negotiated type
        let mut response = HttpResponse::ok();

        if best.matches(&MediaType::json()) {
            if let Some(f) = self.json_fn {
                let value = f();
                let body =
                    serde_json::to_vec(&value).map_err(|e| Error::Serialization(e.to_string()))?;
                response.body = Bytes::from(body);
                response
                    .headers
                    .insert("Content-Type".to_string(), "application/json".to_string());
            }
        } else if best.matches(&MediaType::html()) {
            if let Some(f) = self.html_fn {
                let html = f();
                response.body = Bytes::from(html.into_bytes());
                response.headers.insert(
                    "Content-Type".to_string(),
                    "text/html; charset=utf-8".to_string(),
                );
            }
        } else if best.matches(&MediaType::plain_text()) {
            if let Some(f) = self.text_fn {
                let text = f();
                response.body = Bytes::from(text.into_bytes());
                response.headers.insert(
                    "Content-Type".to_string(),
                    "text/plain; charset=utf-8".to_string(),
                );
            }
        } else if best.matches(&MediaType::xml()) {
            if let Some(f) = self.xml_fn {
                let xml = f();
                response.body = Bytes::from(xml.into_bytes());
                response.headers.insert(
                    "Content-Type".to_string(),
                    "application/xml; charset=utf-8".to_string(),
                );
            }
        } else {
            return Err(Error::NotAcceptable(format!(
                "Cannot produce response in requested format: {}",
                best
            )));
        }

        // Add Vary header
        response
            .headers
            .insert("Vary".to_string(), "Accept".to_string());

        Ok(response)
    }
}

impl Default
    for ContentNegotiator<fn() -> serde_json::Value, fn() -> String, fn() -> String, fn() -> String>
{
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Simple Response Helpers
// ============================================================================

/// Create a response that adapts to the client's Accept header.
///
/// This is a simpler alternative to `ContentNegotiator` for common cases.
///
/// # Example
///
/// ```ignore
/// use armature_core::content_negotiation::respond_with;
///
/// let data = MyData { name: "John" };
/// let response = respond_with(&request, &data)?;
/// ```
pub fn respond_with<T: Serialize>(request: &HttpRequest, data: &T) -> Result<HttpResponse, Error> {
    let accept = request.accept();

    let mut response = HttpResponse::ok();

    if accept.prefers_html() {
        // For HTML, serialize as JSON in a pre tag (basic fallback)
        let json =
            serde_json::to_string_pretty(data).map_err(|e| Error::Serialization(e.to_string()))?;
        let html = format!(
            "<!DOCTYPE html><html><body><pre>{}</pre></body></html>",
            html_escape(&json)
        );
        response.body = Bytes::from(html.into_bytes());
        response.headers.insert(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        );
    } else {
        // Default to JSON
        response.body =
            Bytes::from(serde_json::to_vec(data).map_err(|e| Error::Serialization(e.to_string()))?);
        response
            .headers
            .insert("Content-Type".to_string(), "application/json".to_string());
    }

    response
        .headers
        .insert("Vary".to_string(), "Accept".to_string());

    Ok(response)
}

/// Simple HTML escaping for content.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_parse() {
        let mt = MediaType::parse("application/json").unwrap();
        assert_eq!(mt.type_, "application");
        assert_eq!(mt.subtype, "json");
    }

    #[test]
    fn test_media_type_with_params() {
        let mt = MediaType::parse("text/html; charset=utf-8").unwrap();
        assert_eq!(mt.type_, "text");
        assert_eq!(mt.subtype, "html");
        assert_eq!(mt.params.get("charset"), Some(&"utf-8".to_string()));
    }

    #[test]
    fn test_media_type_matches() {
        let json = MediaType::json();
        let any = MediaType::any();
        let html = MediaType::html();

        assert!(any.matches(&json));
        assert!(json.matches(&any));
        assert!(!json.matches(&html));
    }

    #[test]
    fn test_accept_parse() {
        let accept = Accept::parse("application/json, text/html;q=0.9, */*;q=0.1");
        assert_eq!(accept.media_types.len(), 3);

        // JSON should be first (q=1.0)
        assert_eq!(accept.media_types[0].0.subtype, "json");
        assert_eq!(accept.media_types[0].1, 1.0);

        // HTML should be second (q=0.9)
        assert_eq!(accept.media_types[1].0.subtype, "html");
        assert_eq!(accept.media_types[1].1, 0.9);
    }

    #[test]
    fn test_accept_quality_for() {
        let accept = Accept::parse("application/json, text/html;q=0.9");

        assert_eq!(accept.quality_for(&MediaType::json()), 1.0);
        assert_eq!(accept.quality_for(&MediaType::html()), 0.9);
        assert_eq!(accept.quality_for(&MediaType::xml()), 0.0);
    }

    #[test]
    fn test_extract_quality_case_insensitive() {
        // ";Q=" must be recognized just like ";q="
        let accept = Accept::parse("text/html;Q=0.8");
        assert_eq!(accept.quality_for(&MediaType::html()), 0.8);
    }

    #[test]
    fn test_extract_quality_non_ascii_no_panic() {
        // U+212A (KELVIN SIGN) is 3 bytes but lowercases to 1-byte 'k'.
        // Searching a lowercased copy used to yield an offset that sliced
        // the original string mid-character and panicked.
        let accept = Accept::parse("application/json\u{212A}\u{212A};q=0.5");
        assert_eq!(accept.media_types.len(), 1);
        assert_eq!(accept.media_types[0].1, 0.5);

        let accept_lang = AcceptLanguage::parse("en\u{212A}\u{212A};q=0.5");
        assert_eq!(accept_lang.languages[0].1, 0.5);

        let accept_charset = AcceptCharset::parse("utf\u{212A}\u{212A};q=0.5");
        assert_eq!(accept_charset.charsets[0].1, 0.5);

        // Unknown encoding name, but extract_quality must not panic on it.
        let _ = AcceptEncoding::parse("gzip\u{212A}\u{212A};q=0.5");
    }

    #[test]
    fn test_accept_prefers_json() {
        let accept = Accept::parse("application/json, text/html;q=0.9");
        assert!(accept.prefers_json());
        assert!(!accept.prefers_html());
    }

    #[test]
    fn test_accept_prefers_html() {
        let accept = Accept::parse("text/html, application/json;q=0.9");
        assert!(accept.prefers_html());
        assert!(!accept.prefers_json());
    }

    #[test]
    fn test_negotiate_media_type() {
        let accept = Accept::parse("application/json, text/html;q=0.9");
        let available = vec![MediaType::html(), MediaType::json()];

        let best = negotiate_media_type(&accept, &available);
        assert_eq!(best, Some(&MediaType::json()));
    }

    #[test]
    fn test_language_tag_parse() {
        let tag = LanguageTag::parse("en-US").unwrap();
        assert_eq!(tag.primary, "en");
        assert_eq!(tag.subtag, Some("US".to_string()));
    }

    #[test]
    fn test_language_tag_matches() {
        let en = LanguageTag::new("en");
        let en_us = LanguageTag::with_subtag("en", "US");
        let fr = LanguageTag::new("fr");

        assert!(en.matches(&en_us)); // "en" matches "en-US"
        assert!(!en_us.matches(&en)); // "en-US" doesn't match just "en"
        assert!(!en.matches(&fr));
    }

    #[test]
    fn test_accept_language_parse() {
        let accept = AcceptLanguage::parse("en-US, en;q=0.9, fr;q=0.8");
        assert_eq!(accept.languages.len(), 3);
        assert_eq!(accept.languages[0].0.primary, "en");
    }

    #[test]
    fn test_encoding_parse() {
        assert_eq!(Encoding::parse("gzip"), Some(Encoding::Gzip));
        assert_eq!(Encoding::parse("br"), Some(Encoding::Brotli));
        assert_eq!(Encoding::parse("deflate"), Some(Encoding::Deflate));
    }

    #[test]
    fn test_accept_encoding_parse() {
        let accept = AcceptEncoding::parse("gzip, deflate, br;q=0.9");
        assert_eq!(accept.encodings.len(), 3);
    }

    #[test]
    fn test_accept_charset_parse() {
        let accept = AcceptCharset::parse("utf-8, iso-8859-1;q=0.8");
        assert_eq!(accept.charsets.len(), 2);
        assert_eq!(accept.quality_for("utf-8"), 1.0);
    }

    #[test]
    fn test_http_request_accept() {
        let mut request = HttpRequest::new("GET", "/".to_string());
        request
            .headers
            .insert("Accept", "application/json".to_string());

        let accept = request.accept();
        assert!(accept.accepts(&MediaType::json()));
    }

    #[test]
    fn test_http_request_prefers_json() {
        let mut request = HttpRequest::new("GET", "/".to_string());
        request
            .headers
            .insert("Accept", "application/json, text/html;q=0.9".to_string());

        assert!(request.prefers_json());
        assert!(!request.prefers_html());
    }
}
