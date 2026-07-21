//! Data masking for sensitive information

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

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

static SSN_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap());

static CREDIT_CARD_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b\d{4}[- ]?\d{4}[- ]?\d{4}[- ]?\d{4}\b").unwrap());

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
    let mut result = input.to_string();

    // Mask emails
    if config.mask_emails {
        result = EMAIL_REGEX.replace_all(&result, "[EMAIL]").to_string();
    }

    // Mask phone numbers
    if config.mask_phones {
        result = PHONE_REGEX.replace_all(&result, "[PHONE]").to_string();
    }

    // Mask SSNs
    if config.mask_ssn {
        result = SSN_REGEX.replace_all(&result, "[SSN]").to_string();
    }

    // Mask credit cards
    if config.mask_credit_cards {
        result = CREDIT_CARD_REGEX.replace_all(&result, "[CARD]").to_string();
    }

    result
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
    match value {
        Value::Object(map) => {
            let mut masked_map = serde_json::Map::new();

            for (key, val) in map {
                let key_lower = key.to_lowercase();

                // Check if this field should be masked
                let should_mask = config
                    .masked_fields
                    .iter()
                    .any(|field| key_lower.contains(field));

                if should_mask {
                    if let Value::String(s) = val {
                        masked_map.insert(
                            key.clone(),
                            Value::String(mask_value(s, config.mask_char, config.show_last_chars)),
                        );
                    } else {
                        masked_map.insert(key.clone(), Value::String("[REDACTED]".to_string()));
                    }
                } else {
                    // Recursively mask nested objects
                    masked_map.insert(key.clone(), mask_json(val, config));
                }
            }

            Value::Object(masked_map)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| mask_json(v, config)).collect()),
        Value::String(s) => Value::String(mask_string(s, config)),
        _ => value.clone(),
    }
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
    fn test_mask_body_text() {
        let config = MaskingConfig::default();
        let body = "Email: user@example.com, Phone: 123-456-7890";
        let masked = mask_body(body, &config);
        assert!(masked.contains("[EMAIL]"));
        assert!(masked.contains("[PHONE]"));
    }
}
