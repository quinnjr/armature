//! Data masking for sensitive information

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::borrow::Cow;

/// Field-name fragments that identify high-sensitivity identifiers (SSNs, card
/// numbers, and similar). For these, even the last few characters are abused
/// (e.g. the last four of an SSN), so field-name masking reveals *zero*
/// trailing characters regardless of the configured `show_last_chars`.
const IDENTITY_FIELDS: &[&str] = &[
    "ssn",
    "social_security",
    "credit_card",
    "creditcard",
    "card_number",
    "cvv",
    "pin",
];

/// Fields to mask by default
pub const DEFAULT_MASKED_FIELDS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "access_token",
    "refresh_token",
    "bearer",
    "authorization",
    "auth",
    "credit_card",
    "creditcard",
    "card_number",
    "cvv",
    "ssn",
    "social_security",
    "pin",
    "private_key",
    "privatekey",
];

/// Regular expressions for masking patterns
static EMAIL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").unwrap());

static PHONE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{3}[-.]?\d{3}[-.]?\d{4}\b").unwrap());

// Matches both the hyphenated `123-45-6789` form and a bare 9-digit run
// `123456789` so a dashless SSN is not left in the clear.
static SSN_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(?:\d{3}-\d{2}-\d{4}|\d{9})\b").unwrap());

// Matches 13–19 digit card numbers (Visa/MC 16, Amex 15, Diners 14, and the
// 13/19-digit extremes) with optional single space/hyphen group separators.
static CREDIT_CARD_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d(?:[- ]?\d){12,18}\b").unwrap());

/// Data masking configuration
#[derive(Debug, Clone)]
pub struct MaskingConfig {
    /// Fields to mask
    pub masked_fields: Vec<String>,

    /// Mask email addresses
    pub mask_emails: bool,

    /// Mask phone numbers
    pub mask_phones: bool,

    /// Mask SSNs
    pub mask_ssn: bool,

    /// Mask credit cards
    pub mask_credit_cards: bool,

    /// Mask character (default: '*')
    pub mask_char: char,

    /// Show last N characters
    pub show_last_chars: usize,
}

impl Default for MaskingConfig {
    fn default() -> Self {
        Self {
            masked_fields: DEFAULT_MASKED_FIELDS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            mask_emails: true,
            mask_phones: true,
            mask_ssn: true,
            mask_credit_cards: true,
            mask_char: '*',
            show_last_chars: 4,
        }
    }
}

impl MaskingConfig {
    /// Create a new masking configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a field to mask
    pub fn add_field(mut self, field: impl Into<String>) -> Self {
        self.masked_fields.push(field.into());
        self
    }

    /// Set whether to mask emails
    pub fn mask_emails(mut self, mask: bool) -> Self {
        self.mask_emails = mask;
        self
    }

    /// Set whether to mask phone numbers
    pub fn mask_phones(mut self, mask: bool) -> Self {
        self.mask_phones = mask;
        self
    }

    /// Set mask character
    pub fn mask_char(mut self, c: char) -> Self {
        self.mask_char = c;
        self
    }

    /// Set number of characters to show at end
    pub fn show_last_chars(mut self, n: usize) -> Self {
        self.show_last_chars = n;
        self
    }
}

/// Mask sensitive data in a string
///
/// # Examples
///
/// ```
/// use armature_audit::*;
///
/// let config = MaskingConfig::default();
/// let masked = mask_string("password: secret123", &config);
/// // Result contains masked password
/// ```
pub fn mask_string(input: &str, config: &MaskingConfig) -> String {
    // Thread a `Cow` through the substitution chain so the dominant no-match
    // path never clones the body. `Regex::replace_all` returns `Cow::Borrowed`
    // when it made no replacement; only a real match materializes an owned
    // `String`, and subsequent stages keep operating on that same buffer.
    let mut result: Cow<str> = Cow::Borrowed(input);

    if config.mask_emails {
        result = apply_regex(result, &EMAIL_REGEX, "[EMAIL]");
    }

    if config.mask_phones {
        result = apply_regex(result, &PHONE_REGEX, "[PHONE]");
    }

    if config.mask_ssn {
        result = apply_regex(result, &SSN_REGEX, "[SSN]");
    }

    if config.mask_credit_cards {
        result = apply_regex(result, &CREDIT_CARD_REGEX, "[CARD]");
    }

    result.into_owned()
}

/// Apply one masking regex, preserving the input buffer when nothing matched.
///
/// If `replace_all` returns `Cow::Borrowed` no replacement occurred, so the
/// original `Cow` is returned untouched (no allocation). Only an actual match
/// (`Cow::Owned`) produces a new buffer.
fn apply_regex<'a>(input: Cow<'a, str>, regex: &Regex, replacement: &str) -> Cow<'a, str> {
    match regex.replace_all(&input, replacement) {
        Cow::Borrowed(_) => input,
        Cow::Owned(owned) => Cow::Owned(owned),
    }
}

/// Mask a value (show only last N characters)
///
/// # Examples
///
/// ```
/// use armature_audit::*;
///
/// let masked = mask_value("secret123", '*', 3);
/// assert_eq!(masked, "******123");
/// ```
pub fn mask_value(value: &str, mask_char: char, show_last: usize) -> String {
    // Count/slice by Unicode scalar values (chars), never by bytes, so a
    // multibyte secret (emoji, accented text, CJK, …) can never be sliced at a
    // mid-codepoint offset and panic.
    let total_chars = value.chars().count();

    if total_chars <= show_last {
        return std::iter::repeat_n(mask_char, total_chars).collect();
    }

    let masked_len = total_chars - show_last;
    let mut result: String = std::iter::repeat_n(mask_char, masked_len).collect();
    result.extend(value.chars().skip(masked_len));

    result
}

/// Mask sensitive fields in JSON
///
/// # Examples
///
/// ```
/// use armature_audit::*;
/// use serde_json::json;
///
/// let config = MaskingConfig::default();
/// let data = json!({
///     "username": "alice",
///     "password": "secret123"
/// });
///
/// let masked = mask_json(&data, &config);
/// // password field will be masked
/// ```
pub fn mask_json(value: &Value, config: &MaskingConfig) -> Value {
    // Precompute the lowercased matcher set once for the whole (recursive)
    // traversal instead of lowercasing every configured field for every key.
    let matchers: Vec<String> = config
        .masked_fields
        .iter()
        .map(|f| f.to_lowercase())
        .collect();
    mask_json_inner(value, config, &matchers)
}

fn mask_json_inner(value: &Value, config: &MaskingConfig, matchers: &[String]) -> Value {
    match value {
        Value::Object(map) => {
            let mut masked_map = serde_json::Map::new();

            for (key, val) in map {
                let key_lower = key.to_lowercase();

                // Check if this field should be masked
                let should_mask = matchers
                    .iter()
                    .any(|field| key_lower.contains(field.as_str()));

                if should_mask {
                    if let Value::String(s) = val {
                        // For high-sensitivity identifiers (SSN, card, …) reveal
                        // zero trailing characters — the last four digits are
                        // exactly what an attacker wants — regardless of the
                        // configured `show_last_chars`.
                        let show_last = if is_identity_field(&key_lower) {
                            0
                        } else {
                            config.show_last_chars
                        };
                        masked_map.insert(
                            key.clone(),
                            Value::String(mask_value(s, config.mask_char, show_last)),
                        );
                    } else {
                        masked_map.insert(key.clone(), Value::String("[REDACTED]".to_string()));
                    }
                } else {
                    // Recursively mask nested objects
                    masked_map.insert(key.clone(), mask_json_inner(val, config, matchers));
                }
            }

            Value::Object(masked_map)
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| mask_json_inner(v, config, matchers))
                .collect(),
        ),
        Value::String(s) => Value::String(mask_string(s, config)),
        _ => value.clone(),
    }
}

/// Whether a (lowercased) field name denotes a high-sensitivity identifier
/// whose trailing characters must never be revealed.
fn is_identity_field(key_lower: &str) -> bool {
    IDENTITY_FIELDS
        .iter()
        .any(|field| key_lower.contains(field))
}

/// Mask sensitive data in text body
pub fn mask_body(body: &str, config: &MaskingConfig) -> String {
    // Try to parse as JSON first
    if let Ok(json) = serde_json::from_str::<Value>(body) {
        let masked = mask_json(&json, config);
        serde_json::to_string(&masked).unwrap_or_else(|_| body.to_string())
    } else {
        // Fall back to string masking
        mask_string(body, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_mask_value() {
        assert_eq!(mask_value("secret123", '*', 3), "******123");
        assert_eq!(mask_value("abc", '*', 3), "***");
        assert_eq!(mask_value("ab", '*', 3), "**");
    }

    #[test]
    fn test_mask_value_multibyte_no_panic() {
        // Each 🔒 is 4 bytes; a byte-based slice at masked_len would fall in the
        // middle of a codepoint and panic. Char-based masking must not.
        let masked = mask_value("🔒🔒🔒🔒🔒", '*', 2);
        assert_eq!(masked, "***🔒🔒");
        // 3 masked chars + 2 visible chars = 5 chars total.
        assert_eq!(masked.chars().count(), 5);

        // Mixed multibyte secret masked entirely except last char.
        let masked = mask_value("café☕", '*', 1);
        assert_eq!(masked, "****☕");

        // show_last larger than the (char) length: fully masked, no panic.
        let masked = mask_value("naïve", '*', 10);
        assert_eq!(masked, "*****");
    }

    #[test]
    fn test_mask_email() {
        let config = MaskingConfig::default();
        let masked = mask_string("Email: user@example.com", &config);
        assert!(masked.contains("[EMAIL]"));
        assert!(!masked.contains("user@example.com"));
    }

    #[test]
    fn test_mask_phone() {
        let config = MaskingConfig::default();
        let masked = mask_string("Phone: 123-456-7890", &config);
        assert!(masked.contains("[PHONE]"));
    }

    #[test]
    fn test_mask_json_password() {
        let config = MaskingConfig::default();
        let data = json!({
            "username": "alice",
            "password": "secret123"
        });

        let masked = mask_json(&data, &config);
        assert_eq!(masked["username"], "alice");
        assert_ne!(masked["password"], "secret123");
        assert!(masked["password"].as_str().unwrap().contains("*"));
    }

    #[test]
    fn test_mask_json_nested() {
        let config = MaskingConfig::default();
        let data = json!({
            "user": {
                "name": "alice",
                "password": "secret123"
            }
        });

        let masked = mask_json(&data, &config);
        assert_eq!(masked["user"]["name"], "alice");
        assert_ne!(masked["user"]["password"], "secret123");
    }

    #[test]
    fn test_mask_json_array() {
        let config = MaskingConfig::default();
        let data = json!([
            {"username": "alice", "password": "secret1"},
            {"username": "bob", "password": "secret2"}
        ]);

        let masked = mask_json(&data, &config);
        assert_eq!(masked[0]["username"], "alice");
        assert_ne!(masked[0]["password"], "secret1");
    }

    #[test]
    fn test_mask_body_json() {
        let config = MaskingConfig::default();
        let body = r#"{"username":"alice","password":"secret123"}"#;
        let masked = mask_body(body, &config);
        assert!(masked.contains("alice"));
        assert!(!masked.contains("secret123"));
    }

    #[test]
    fn test_mask_ssn_dashless() {
        // A bare 9-digit SSN must be masked, not just the hyphenated form.
        let config = MaskingConfig::default();
        let masked = mask_string("SSN: 123456789 done", &config);
        assert!(masked.contains("[SSN]"), "dashless SSN masked: {masked}");
        assert!(!masked.contains("123456789"));

        // The hyphenated form keeps working.
        let masked = mask_string("SSN: 123-45-6789", &config);
        assert!(masked.contains("[SSN]"));
        assert!(!masked.contains("123-45-6789"));
    }

    #[test]
    fn test_mask_credit_card_variable_length() {
        let config = MaskingConfig::default();

        // 15-digit Amex (4-6-5 grouping).
        let masked = mask_string("Card 3782 822463 10005", &config);
        assert!(masked.contains("[CARD]"), "amex masked: {masked}");
        assert!(!masked.contains("822463"));

        // 14-digit Diners.
        let masked = mask_string("Card 3056 930902 5904", &config);
        assert!(masked.contains("[CARD]"), "diners masked: {masked}");

        // Classic 16-digit still masked.
        let masked = mask_string("Card 4111 1111 1111 1111", &config);
        assert!(masked.contains("[CARD]"));
        assert!(!masked.contains("4111"));
    }

    #[test]
    fn test_mask_json_identity_field_hides_last_four() {
        // Field-name masking of an SSN must NOT reveal the last four digits,
        // even with the default show_last_chars = 4.
        let config = MaskingConfig::default();
        let data = json!({ "ssn": "123-45-6789", "card_number": "4111111111111111" });
        let masked = mask_json(&data, &config);

        let ssn = masked["ssn"].as_str().unwrap();
        assert!(!ssn.contains("6789"), "SSN last-4 leaked: {ssn}");
        assert!(ssn.chars().all(|c| c == '*'), "SSN fully masked: {ssn}");

        let card = masked["card_number"].as_str().unwrap();
        assert!(!card.contains("1111"), "card tail leaked: {card}");
        assert!(card.chars().all(|c| c == '*'));
    }

    #[test]
    fn test_mask_json_nonidentity_field_keeps_last_chars() {
        // Non-identity secret fields still reveal the configured tail.
        let config = MaskingConfig::default();
        let data = json!({ "password": "secret123" });
        let masked = mask_json(&data, &config);
        let pw = masked["password"].as_str().unwrap();
        assert!(pw.ends_with("123"), "password tail preserved: {pw}");
        assert!(pw.starts_with('*'));
    }

    #[test]
    fn test_mask_body_text() {
        let config = MaskingConfig::default();
        let body = "Email: user@example.com, Phone: 123-456-7890";
        let masked = mask_body(body, &config);
        assert!(masked.contains("[EMAIL]"));
        assert!(masked.contains("[PHONE]"));
    }
}
