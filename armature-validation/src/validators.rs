// Built-in validators

use crate::ValidationError;
use once_cell::sync::Lazy;
use regex::Regex;

// Common regex patterns
static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$").unwrap()
});

/// RFC 9562 UUID string form.
///
/// Case-insensitive: §4 defines the hex digits as case-insensitive on input,
/// so an uppercase UUID is valid and must not be rejected. The version nibble
/// is constrained to 1-8 and the variant nibble to the RFC 9562 variant
/// (`8`/`9`/`a`/`b`); the nil and max UUIDs are permitted explicitly, since
/// neither carries a valid version/variant.
static UUID_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(?:[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}|00000000-0000-0000-0000-000000000000|ffffffff-ffff-ffff-ffff-ffffffffffff)$",
    )
    .unwrap()
});

/// Characters permitted in a hostname or IP-literal host.
///
/// `url` accepts some hosts that no DNS resolver or web client will (`https://!!`
/// parses, with `!!` as the host), so the parsed host is checked against this
/// afterwards.
static HOST_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:\[[0-9a-f:.]+\]|[a-z0-9](?:[a-z0-9._-]*[a-z0-9])?)$").unwrap()
});

static ALPHA_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-zA-Z]+$").unwrap());

static ALPHANUMERIC_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-zA-Z0-9]+$").unwrap());

static NUMERIC_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9]+$").unwrap());

// String validators

/// Validates that a string is not empty
pub struct NotEmpty;

impl NotEmpty {
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        if value.trim().is_empty() {
            Err(
                ValidationError::new(field, format!("{} should not be empty", field))
                    .with_constraint("notEmpty"),
            )
        } else {
            Ok(())
        }
    }
}

/// Validates minimum string length
pub struct MinLength(pub usize);

impl MinLength {
    pub fn validate(&self, value: &str, field: &str) -> Result<(), ValidationError> {
        if value.chars().count() < self.0 {
            Err(ValidationError::new(
                field,
                format!("{} must be at least {} characters", field, self.0),
            )
            .with_constraint("minLength")
            .with_value(value.to_string()))
        } else {
            Ok(())
        }
    }
}

/// Validates maximum string length
pub struct MaxLength(pub usize);

impl MaxLength {
    pub fn validate(&self, value: &str, field: &str) -> Result<(), ValidationError> {
        if value.chars().count() > self.0 {
            Err(ValidationError::new(
                field,
                format!("{} must be at most {} characters", field, self.0),
            )
            .with_constraint("maxLength")
            .with_value(value.to_string()))
        } else {
            Ok(())
        }
    }
}

/// Validates email format
pub struct IsEmail;

impl IsEmail {
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        if EMAIL_REGEX.is_match(value) {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must be a valid email", field))
                    .with_constraint("isEmail")
                    .with_value(value.to_string()),
            )
        }
    }
}

/// Validates URL format
pub struct IsUrl;

impl IsUrl {
    /// Accept an absolute `http`/`https` URL with a plausible host.
    ///
    /// The value is *parsed* (via the `url` crate), not pattern-matched, then
    /// two checks are applied on top of a successful parse:
    ///
    /// - the scheme must be `http` or `https` - other schemes parse fine but
    ///   are not what a field annotated `IsUrl` means in a web form;
    /// - the host must look like a hostname or IP literal. `url` will happily
    ///   parse `https://!!` with `!!` as the host.
    ///
    /// Single-label hosts are accepted: `https://a`, `http://localhost` and
    /// `http://intranet` are all real, resolvable URLs.
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        let is_valid = match url::Url::parse(value) {
            Ok(parsed) => {
                matches!(parsed.scheme(), "http" | "https")
                    && parsed.host_str().is_some_and(|h| HOST_REGEX.is_match(h))
            }
            Err(_) => false,
        };

        if is_valid {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must be a valid URL", field))
                    .with_constraint("isUrl")
                    .with_value(value.to_string()),
            )
        }
    }
}

/// Validates UUID format
pub struct IsUuid;

impl IsUuid {
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        if UUID_REGEX.is_match(value) {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must be a valid UUID", field))
                    .with_constraint("isUuid")
                    .with_value(value.to_string()),
            )
        }
    }
}

/// Validates alphabetic characters only
pub struct IsAlpha;

impl IsAlpha {
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        if ALPHA_REGEX.is_match(value) {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must contain only letters", field))
                    .with_constraint("isAlpha")
                    .with_value(value.to_string()),
            )
        }
    }
}

/// Validates alphanumeric characters only
pub struct IsAlphanumeric;

impl IsAlphanumeric {
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        if ALPHANUMERIC_REGEX.is_match(value) {
            Ok(())
        } else {
            Err(ValidationError::new(
                field,
                format!("{} must contain only letters and numbers", field),
            )
            .with_constraint("isAlphanumeric")
            .with_value(value.to_string()))
        }
    }
}

/// Validates numeric characters only
pub struct IsNumeric;

impl IsNumeric {
    pub fn validate(value: &str, field: &str) -> Result<(), ValidationError> {
        if NUMERIC_REGEX.is_match(value) {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must contain only numbers", field))
                    .with_constraint("isNumeric")
                    .with_value(value.to_string()),
            )
        }
    }
}

// Number validators

/// Validates minimum value
pub struct Min<T>(pub T);

impl Min<i32> {
    pub fn validate(&self, value: i32, field: &str) -> Result<(), ValidationError> {
        if value < self.0 {
            Err(
                ValidationError::new(field, format!("{} must be at least {}", field, self.0))
                    .with_constraint("min")
                    .with_value(value.to_string()),
            )
        } else {
            Ok(())
        }
    }
}

impl Min<f64> {
    pub fn validate(&self, value: f64, field: &str) -> Result<(), ValidationError> {
        if value < self.0 {
            Err(
                ValidationError::new(field, format!("{} must be at least {}", field, self.0))
                    .with_constraint("min")
                    .with_value(value.to_string()),
            )
        } else {
            Ok(())
        }
    }
}

/// Validates maximum value
pub struct Max<T>(pub T);

impl Max<i32> {
    pub fn validate(&self, value: i32, field: &str) -> Result<(), ValidationError> {
        if value > self.0 {
            Err(
                ValidationError::new(field, format!("{} must be at most {}", field, self.0))
                    .with_constraint("max")
                    .with_value(value.to_string()),
            )
        } else {
            Ok(())
        }
    }
}

impl Max<f64> {
    pub fn validate(&self, value: f64, field: &str) -> Result<(), ValidationError> {
        if value > self.0 {
            Err(
                ValidationError::new(field, format!("{} must be at most {}", field, self.0))
                    .with_constraint("max")
                    .with_value(value.to_string()),
            )
        } else {
            Ok(())
        }
    }
}

/// Validates value is positive
pub struct IsPositive;

impl IsPositive {
    pub fn validate_i32(value: i32, field: &str) -> Result<(), ValidationError> {
        if value > 0 {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must be a positive number", field))
                    .with_constraint("isPositive")
                    .with_value(value.to_string()),
            )
        }
    }

    pub fn validate_f64(value: f64, field: &str) -> Result<(), ValidationError> {
        if value > 0.0 {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} must be a positive number", field))
                    .with_constraint("isPositive")
                    .with_value(value.to_string()),
            )
        }
    }
}

/// Validates value is in range
pub struct InRange<T> {
    pub min: T,
    pub max: T,
}

impl InRange<i32> {
    pub fn validate(&self, value: i32, field: &str) -> Result<(), ValidationError> {
        if value >= self.min && value <= self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                field,
                format!("{} must be between {} and {}", field, self.min, self.max),
            )
            .with_constraint("inRange")
            .with_value(value.to_string()))
        }
    }
}

/// Custom regex validator
pub struct Matches(pub Regex);

impl Matches {
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self(Regex::new(pattern)?))
    }

    pub fn validate(&self, value: &str, field: &str) -> Result<(), ValidationError> {
        if self.0.is_match(value) {
            Ok(())
        } else {
            Err(
                ValidationError::new(field, format!("{} does not match required pattern", field))
                    .with_constraint("matches")
                    .with_value(value.to_string()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_empty() {
        assert!(NotEmpty::validate("test", "field").is_ok());
        assert!(NotEmpty::validate("", "field").is_err());
        assert!(NotEmpty::validate("   ", "field").is_err());
    }

    #[test]
    fn test_min_length() {
        let validator = MinLength(5);
        assert!(validator.validate("hello", "field").is_ok());
        assert!(validator.validate("hi", "field").is_err());
    }

    #[test]
    fn test_is_email() {
        assert!(IsEmail::validate("test@example.com", "email").is_ok());
        assert!(IsEmail::validate("invalid", "email").is_err());
    }

    #[test]
    fn test_min_value() {
        let validator = Min(10);
        assert!(validator.validate(15, "age").is_ok());
        assert!(validator.validate(5, "age").is_err());
    }

    #[test]
    fn test_max_length() {
        let validator = MaxLength(10);
        assert!(validator.validate("short", "field").is_ok());
        assert!(validator.validate("this is too long", "field").is_err());
    }

    #[test]
    fn test_max_value() {
        let validator = Max(100i32);
        assert!(validator.validate(50i32, "value").is_ok());
        assert!(validator.validate(150i32, "value").is_err());
    }

    #[test]
    fn test_in_range() {
        let validator = InRange {
            min: 10i32,
            max: 20i32,
        };
        assert!(validator.validate(15i32, "value").is_ok());
        assert!(validator.validate(5i32, "value").is_err());
        assert!(validator.validate(25i32, "value").is_err());
    }

    #[test]
    fn test_is_url() {
        assert!(IsUrl::validate("https://example.com", "url").is_ok());
        assert!(IsUrl::validate("http://test.org/path", "url").is_ok());
        assert!(IsUrl::validate("not a url", "url").is_err());

        // A single-label host is a real URL. The old regex required at least
        // two characters after the scheme and rejected this.
        assert!(IsUrl::validate("https://a", "url").is_ok());
        assert!(IsUrl::validate("http://localhost:8080/health", "url").is_ok());
        assert!(IsUrl::validate("https://[::1]:8443/", "url").is_ok());

        // The old regex accepted anything non-whitespace after the scheme.
        assert!(IsUrl::validate("https://!!", "url").is_err());
        assert!(IsUrl::validate("https://", "url").is_err());
        // Only http(s); other schemes parse but are not web URLs.
        assert!(IsUrl::validate("ftp://example.com", "url").is_err());
        assert!(IsUrl::validate("javascript:alert(1)", "url").is_err());
        // Relative references are not absolute URLs.
        assert!(IsUrl::validate("/path/only", "url").is_err());
    }

    /// RFC 9562 §4: the hex digits are case-insensitive on input, so an
    /// uppercase UUID is valid. The old regex was lowercase-only and also
    /// checked neither the version nor the variant nibble.
    #[test]
    fn test_is_uuid_case_and_version_variant() {
        let lower = "f47ac10b-58cc-4372-a567-0e02b2c3d479";
        assert!(IsUuid::validate(lower, "id").is_ok());
        assert!(IsUuid::validate(&lower.to_uppercase(), "id").is_ok());
        assert!(IsUuid::validate("F47AC10B-58cc-4372-A567-0e02b2c3d479", "id").is_ok());

        // Nil and max UUIDs are valid despite carrying no version/variant.
        assert!(IsUuid::validate("00000000-0000-0000-0000-000000000000", "id").is_ok());
        assert!(IsUuid::validate("ffffffff-ffff-ffff-ffff-ffffffffffff", "id").is_ok());

        // Version 0 and version 9 do not exist.
        assert!(IsUuid::validate("f47ac10b-58cc-0372-a567-0e02b2c3d479", "id").is_err());
        assert!(IsUuid::validate("f47ac10b-58cc-9372-a567-0e02b2c3d479", "id").is_err());
        // Variant nibble must be 8/9/a/b.
        assert!(IsUuid::validate("f47ac10b-58cc-4372-c567-0e02b2c3d479", "id").is_err());

        assert!(IsUuid::validate("not-a-uuid", "id").is_err());
        assert!(IsUuid::validate("f47ac10b58cc4372a5670e02b2c3d479", "id").is_err());
    }

    #[test]
    fn test_is_alpha() {
        assert!(IsAlpha::validate("abcXYZ", "field").is_ok());
        assert!(IsAlpha::validate("abc123", "field").is_err());
        assert!(IsAlpha::validate("abc xyz", "field").is_err());
    }

    #[test]
    fn test_is_alphanumeric() {
        assert!(IsAlphanumeric::validate("abc123", "field").is_ok());
        assert!(IsAlphanumeric::validate("abc@123", "field").is_err());
        assert!(IsAlphanumeric::validate("test", "field").is_ok());
    }

    #[test]
    fn test_is_numeric() {
        assert!(IsNumeric::validate("12345", "field").is_ok());
        assert!(IsNumeric::validate("123.45", "field").is_err());
        assert!(IsNumeric::validate("abc", "field").is_err());
    }

    #[test]
    fn test_is_uuid() {
        assert!(IsUuid::validate("550e8400-e29b-41d4-a716-446655440000", "id").is_ok());
        assert!(IsUuid::validate("not-a-uuid", "id").is_err());
        assert!(IsUuid::validate("", "id").is_err());
    }

    #[test]
    fn test_not_empty_with_whitespace_only() {
        assert!(NotEmpty::validate("\t\n  \r", "field").is_err());
    }

    #[test]
    fn test_min_length_exact() {
        let validator = MinLength(5);
        assert!(validator.validate("exact", "field").is_ok());
        assert!(validator.validate("four", "field").is_err());
    }

    #[test]
    fn test_max_length_exact() {
        let validator = MaxLength(5);
        assert!(validator.validate("exact", "field").is_ok());
        assert!(validator.validate("sixsix", "field").is_err());
    }

    #[test]
    fn test_in_range_boundaries() {
        let validator = InRange {
            min: 0i32,
            max: 10i32,
        };
        assert!(validator.validate(0i32, "value").is_ok());
        assert!(validator.validate(10i32, "value").is_ok());
        assert!(validator.validate(-1i32, "value").is_err());
        assert!(validator.validate(11i32, "value").is_err());
    }

    #[test]
    fn test_email_variations() {
        assert!(IsEmail::validate("user+tag@example.com", "email").is_ok());
        assert!(IsEmail::validate("user.name@example.co.uk", "email").is_ok());
        assert!(IsEmail::validate("@example.com", "email").is_err());
        assert!(IsEmail::validate("user@", "email").is_err());
    }

    #[test]
    fn test_url_variations() {
        assert!(IsUrl::validate("https://example.com", "url").is_ok());
        assert!(IsUrl::validate("http://test.com/path", "url").is_ok());
        assert!(IsUrl::validate("//example.com", "url").is_err());
    }

    #[test]
    fn test_uuid_formats() {
        // UUID v4 format
        assert!(IsUuid::validate("123e4567-e89b-12d3-a456-426614174000", "id").is_ok());
        // Without hyphens should fail
        assert!(IsUuid::validate("123e4567e89b12d3a456426614174000", "id").is_err());
    }

    #[test]
    fn test_max_length_counts_characters_not_bytes() {
        // Three emoji = 3 characters but 12 bytes. Under a 5-char cap it must pass.
        let three_emoji = "😀😀😀";
        assert_eq!(three_emoji.chars().count(), 3);
        assert_eq!(three_emoji.len(), 12);
        assert!(MaxLength(5).validate(three_emoji, "field").is_ok());
        // A 2-char cap must still reject the 3 characters.
        assert!(MaxLength(2).validate(three_emoji, "field").is_err());
    }

    #[test]
    fn test_min_length_counts_characters_not_bytes() {
        // "café" is 4 characters but 5 bytes. MinLength(4) must accept it.
        let cafe = "café";
        assert_eq!(cafe.chars().count(), 4);
        assert_eq!(cafe.len(), 5);
        assert!(MinLength(4).validate(cafe, "field").is_ok());
        assert!(MinLength(5).validate(cafe, "field").is_err());
    }

    #[test]
    fn test_empty_string_validators() {
        // Empty strings fail because regex requires at least one character
        assert!(IsAlpha::validate("", "field").is_err());
        assert!(IsNumeric::validate("", "field").is_err());
        assert!(IsAlphanumeric::validate("", "field").is_err());
    }
}
