// Configuration validation

use crate::{ConfigError, Result};

/// Trait for validating configuration
pub trait Validate {
    fn validate(&self) -> Result<()>;
}

/// Configuration validator with rules
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate that a value is not empty
    pub fn not_empty(value: &str, field: &str) -> Result<()> {
        if value.is_empty() {
            return Err(ConfigError::ValidationError(format!(
                "{} cannot be empty",
                field
            )));
        }
        Ok(())
    }

    /// Validate that a number is within range.
    ///
    /// `T: Display` so the message names the actual bounds and the offending
    /// value; without it the error read literally "must be between min and
    /// max", which tells the operator nothing about what to change.
    pub fn in_range<T: PartialOrd + std::fmt::Display>(
        value: T,
        min: T,
        max: T,
        field: &str,
    ) -> Result<()> {
        if value < min || value > max {
            return Err(ConfigError::ValidationError(format!(
                "{field} must be between {min} and {max} (got {value})"
            )));
        }
        Ok(())
    }

    /// Validate that a value is in a list of allowed values.
    ///
    /// `T: Display` so the message lists the allowed values rather than merely
    /// asserting that some exist.
    pub fn one_of<T: PartialEq + std::fmt::Display>(
        value: &T,
        allowed: &[T],
        field: &str,
    ) -> Result<()> {
        if !allowed.contains(value) {
            let allowed = allowed
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ConfigError::ValidationError(format!(
                "{field} must be one of [{allowed}] (got {value})"
            )));
        }
        Ok(())
    }

    /// Validate URL format
    pub fn is_url(value: &str, field: &str) -> Result<()> {
        if !value.starts_with("http://") && !value.starts_with("https://") {
            return Err(ConfigError::ValidationError(format!(
                "{} must be a valid URL",
                field
            )));
        }
        Ok(())
    }

    /// Validate email format (basic)
    pub fn is_email(value: &str, field: &str) -> Result<()> {
        if !value.contains('@') || !value.contains('.') {
            return Err(ConfigError::ValidationError(format!(
                "{} must be a valid email",
                field
            )));
        }
        Ok(())
    }

    /// Validate port number
    pub fn is_port(value: u16, field: &str) -> Result<()> {
        if value == 0 {
            return Err(ConfigError::ValidationError(format!(
                "{} must be a valid port number",
                field
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_empty_validation() {
        assert!(ConfigValidator::not_empty("value", "field").is_ok());
        assert!(ConfigValidator::not_empty("", "field").is_err());
    }

    #[test]
    fn test_range_validation() {
        assert!(ConfigValidator::in_range(5, 1, 10, "field").is_ok());
        assert!(ConfigValidator::in_range(0, 1, 10, "field").is_err());
        assert!(ConfigValidator::in_range(11, 1, 10, "field").is_err());
    }

    #[test]
    fn test_one_of_validation() {
        let allowed = vec!["a", "b", "c"];
        assert!(ConfigValidator::one_of(&"a", &allowed, "field").is_ok());
        assert!(ConfigValidator::one_of(&"d", &allowed, "field").is_err());
    }

    #[test]
    fn test_url_validation() {
        assert!(ConfigValidator::is_url("https://example.com", "field").is_ok());
        assert!(ConfigValidator::is_url("http://example.com", "field").is_ok());
        assert!(ConfigValidator::is_url("example.com", "field").is_err());
    }

    #[test]
    fn test_port_validation() {
        assert!(ConfigValidator::is_port(8080, "field").is_ok());
        assert!(ConfigValidator::is_port(0, "field").is_err());
    }
}
