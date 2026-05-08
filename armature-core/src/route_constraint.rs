//! Route constraints for parameter validation
//!
//! Route constraints allow you to validate path parameters at the routing level,
//! before the handler is called. This provides early validation and better error messages.
//!
//! # Features
//!
//! - **Built-in Constraints**: Int, UInt, Alpha, AlphaNum, UUID, Email, Regex
//! - **Custom Constraints**: Implement `RouteConstraint` trait
//! - **Composable**: Combine multiple constraints
//! - **Type-safe**: Validate parameters match expected types
//!
//! # Examples
//!
//! ```no_run
//! use armature_core::*;
//!
//! // Only match if :id is a valid integer
//! let constraint = RouteConstraints::new()
//!     .add("id", Box::new(IntConstraint));
//!
//! // Only match if :uuid is a valid UUID
//! let constraint = RouteConstraints::new()
//!     .add("uuid", Box::new(UuidConstraint));
//! ```

use crate::Error;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;

/// Trait for validating route parameters
pub trait RouteConstraint: Send + Sync {
    /// Validate a parameter value
    ///
    /// Returns Ok(()) if valid, Err with a descriptive message if invalid
    fn validate(&self, value: &str) -> Result<(), String>;

    /// Get a description of this constraint (for error messages)
    fn description(&self) -> &str;
}

/// Integer constraint - validates that a parameter is a valid integer
#[derive(Debug, Clone)]
pub struct IntConstraint;

impl RouteConstraint for IntConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        value
            .parse::<i64>()
            .map(|_| ())
            .map_err(|_| format!("'{}' is not a valid integer", value))
    }

    fn description(&self) -> &str {
        "integer"
    }
}

/// Unsigned integer constraint - validates that a parameter is a valid unsigned integer
#[derive(Debug, Clone)]
pub struct UIntConstraint;

impl RouteConstraint for UIntConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        value
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| format!("'{}' is not a valid unsigned integer", value))
    }

    fn description(&self) -> &str {
        "unsigned integer"
    }
}

/// Float constraint - validates that a parameter is a valid floating point number
#[derive(Debug, Clone)]
pub struct FloatConstraint;

impl RouteConstraint for FloatConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        value
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("'{}' is not a valid float", value))
    }

    fn description(&self) -> &str {
        "float"
    }
}

/// Alphabetic constraint - validates that a parameter contains only letters
#[derive(Debug, Clone)]
pub struct AlphaConstraint;

impl RouteConstraint for AlphaConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if value.chars().all(|c| c.is_alphabetic()) {
            Ok(())
        } else {
            Err(format!("'{}' must contain only letters", value))
        }
    }

    fn description(&self) -> &str {
        "alphabetic"
    }
}

/// Alphanumeric constraint - validates that a parameter contains only letters and numbers
#[derive(Debug, Clone)]
pub struct AlphaNumConstraint;

impl RouteConstraint for AlphaNumConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if value.chars().all(|c| c.is_alphanumeric()) {
            Ok(())
        } else {
            Err(format!("'{}' must contain only letters and numbers", value))
        }
    }

    fn description(&self) -> &str {
        "alphanumeric"
    }
}

/// UUID constraint - validates that a parameter is a valid UUID
#[derive(Debug, Clone)]
pub struct UuidConstraint;

impl RouteConstraint for UuidConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        // Simple UUID validation (8-4-4-4-12 format)
        let uuid_regex = Regex::new(
            r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
        )
        .unwrap();

        if uuid_regex.is_match(value) {
            Ok(())
        } else {
            Err(format!("'{}' is not a valid UUID", value))
        }
    }

    fn description(&self) -> &str {
        "UUID"
    }
}

/// Email constraint - validates that a parameter is a valid email address
#[derive(Debug, Clone)]
pub struct EmailConstraint;

impl RouteConstraint for EmailConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        // Basic email validation
        let email_regex = Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();

        if email_regex.is_match(value) {
            Ok(())
        } else {
            Err(format!("'{}' is not a valid email address", value))
        }
    }

    fn description(&self) -> &str {
        "email address"
    }
}

/// Regex constraint - validates that a parameter matches a regex pattern
pub struct RegexConstraint {
    regex: Regex,
    description: String,
}

impl RegexConstraint {
    /// Create a new regex constraint
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::RegexConstraint;
    ///
    /// // Only allow lowercase letters
    /// let constraint = RegexConstraint::new(r"^[a-z]+$", "lowercase letters");
    /// ```
    pub fn new(pattern: &str, description: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: Regex::new(pattern)?,
            description: description.to_string(),
        })
    }
}

impl RouteConstraint for RegexConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if self.regex.is_match(value) {
            Ok(())
        } else {
            Err(format!(
                "'{}' must match pattern: {}",
                value, self.description
            ))
        }
    }

    fn description(&self) -> &str {
        &self.description
    }
}

/// Length constraint - validates that a parameter has a specific length range
#[derive(Debug, Clone)]
pub struct LengthConstraint {
    min: Option<usize>,
    max: Option<usize>,
}

impl LengthConstraint {
    /// Create a new length constraint
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::LengthConstraint;
    ///
    /// // Between 3 and 20 characters
    /// let constraint = LengthConstraint::new(Some(3), Some(20));
    ///
    /// // At least 5 characters
    /// let constraint = LengthConstraint::min(5);
    ///
    /// // At most 100 characters
    /// let constraint = LengthConstraint::max(100);
    /// ```
    pub fn new(min: Option<usize>, max: Option<usize>) -> Self {
        Self { min, max }
    }

    /// Create a length constraint with only a minimum
    pub fn min(min: usize) -> Self {
        Self {
            min: Some(min),
            max: None,
        }
    }

    /// Create a length constraint with only a maximum
    pub fn max(max: usize) -> Self {
        Self {
            min: None,
            max: Some(max),
        }
    }

    /// Create a length constraint with an exact length
    pub fn exact(length: usize) -> Self {
        Self {
            min: Some(length),
            max: Some(length),
        }
    }
}

impl RouteConstraint for LengthConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        let len = value.len();

        if let Some(min) = self.min
            && len < min
        {
            return Err(format!("'{}' must be at least {} characters", value, min));
        }

        if let Some(max) = self.max
            && len > max
        {
            return Err(format!("'{}' must be at most {} characters", value, max));
        }

        Ok(())
    }

    fn description(&self) -> &str {
        match (self.min, self.max) {
            (Some(min), Some(max)) if min == max => "exact length",
            (Some(_), Some(_)) => "length range",
            (Some(_), None) => "minimum length",
            (None, Some(_)) => "maximum length",
            (None, None) => "any length",
        }
    }
}

/// Range constraint - validates that a number is within a range
#[derive(Debug, Clone)]
pub struct RangeConstraint {
    min: Option<i64>,
    max: Option<i64>,
}

impl RangeConstraint {
    /// Create a new range constraint
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::RangeConstraint;
    ///
    /// // Between 1 and 100
    /// let constraint = RangeConstraint::new(Some(1), Some(100));
    ///
    /// // At least 0
    /// let constraint = RangeConstraint::min(0);
    ///
    /// // At most 1000
    /// let constraint = RangeConstraint::max(1000);
    /// ```
    pub fn new(min: Option<i64>, max: Option<i64>) -> Self {
        Self { min, max }
    }

    /// Create a range constraint with only a minimum
    pub fn min(min: i64) -> Self {
        Self {
            min: Some(min),
            max: None,
        }
    }

    /// Create a range constraint with only a maximum
    pub fn max(max: i64) -> Self {
        Self {
            min: None,
            max: Some(max),
        }
    }
}

impl RouteConstraint for RangeConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        let num = value
            .parse::<i64>()
            .map_err(|_| format!("'{}' is not a valid number", value))?;

        if let Some(min) = self.min
            && num < min
        {
            return Err(format!("'{}' must be at least {}", value, min));
        }

        if let Some(max) = self.max
            && num > max
        {
            return Err(format!("'{}' must be at most {}", value, max));
        }

        Ok(())
    }

    fn description(&self) -> &str {
        match (self.min, self.max) {
            (Some(_), Some(_)) => "number in range",
            (Some(_), None) => "minimum value",
            (None, Some(_)) => "maximum value",
            (None, None) => "any number",
        }
    }
}

/// Enum constraint - validates that a parameter is one of a set of values
#[derive(Debug, Clone)]
pub struct EnumConstraint {
    values: Vec<String>,
}

impl EnumConstraint {
    /// Create a new enum constraint
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::EnumConstraint;
    ///
    /// let constraint = EnumConstraint::new(vec![
    ///     "active".to_string(),
    ///     "inactive".to_string(),
    ///     "pending".to_string(),
    /// ]);
    /// ```
    pub fn new(values: Vec<String>) -> Self {
        Self { values }
    }
}

impl RouteConstraint for EnumConstraint {
    fn validate(&self, value: &str) -> Result<(), String> {
        if self.values.contains(&value.to_string()) {
            Ok(())
        } else {
            Err(format!(
                "'{}' must be one of: {}",
                value,
                self.values.join(", ")
            ))
        }
    }

    fn description(&self) -> &str {
        "enum value"
    }
}

/// Collection of route constraints for a route
///
/// Maps parameter names to their constraints.
#[derive(Default)]
pub struct RouteConstraints {
    constraints: HashMap<String, Arc<dyn RouteConstraint>>,
}

impl RouteConstraints {
    /// Create a new empty constraint collection
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a constraint for a parameter
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::*;
    ///
    /// let constraints = RouteConstraints::new()
    ///     .add("id", Box::new(IntConstraint))
    ///     .add("uuid", Box::new(UuidConstraint));
    /// ```
    pub fn add(mut self, param: impl Into<String>, constraint: Box<dyn RouteConstraint>) -> Self {
        self.constraints.insert(param.into(), Arc::from(constraint));
        self
    }

    /// Add a constraint for a parameter (mutable version)
    pub fn add_mut(&mut self, param: impl Into<String>, constraint: Box<dyn RouteConstraint>) {
        self.constraints.insert(param.into(), Arc::from(constraint));
    }

    /// Validate all parameters against their constraints
    ///
    /// Returns Ok(()) if all constraints pass, or an Error if any fail.
    pub fn validate(&self, params: &HashMap<String, String>) -> Result<(), Error> {
        for (param_name, constraint) in &self.constraints {
            if let Some(param_value) = params.get(param_name) {
                constraint.validate(param_value).map_err(|msg| {
                    Error::BadRequest(format!("Invalid route parameter '{}': {}", param_name, msg))
                })?;
            }
        }
        Ok(())
    }

    /// Check if there are any constraints
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }

    /// Get the number of constraints
    pub fn len(&self) -> usize {
        self.constraints.len()
    }
}

impl Clone for RouteConstraints {
    fn clone(&self) -> Self {
        Self {
            constraints: self.constraints.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_constraint() {
        let constraint = IntConstraint;
        assert!(constraint.validate("123").is_ok());
        assert!(constraint.validate("-456").is_ok());
        assert!(constraint.validate("abc").is_err());
        assert!(constraint.validate("12.5").is_err());
    }

    #[test]
    fn test_uint_constraint() {
        let constraint = UIntConstraint;
        assert!(constraint.validate("123").is_ok());
        assert!(constraint.validate("0").is_ok());
        assert!(constraint.validate("-456").is_err());
        assert!(constraint.validate("abc").is_err());
    }

    #[test]
    fn test_alpha_constraint() {
        let constraint = AlphaConstraint;
        assert!(constraint.validate("abc").is_ok());
        assert!(constraint.validate("ABC").is_ok());
        assert!(constraint.validate("abc123").is_err());
        assert!(constraint.validate("abc-def").is_err());
    }

    #[test]
    fn test_alphanum_constraint() {
        let constraint = AlphaNumConstraint;
        assert!(constraint.validate("abc123").is_ok());
        assert!(constraint.validate("ABC").is_ok());
        assert!(constraint.validate("123").is_ok());
        assert!(constraint.validate("abc-def").is_err());
        assert!(constraint.validate("abc 123").is_err());
    }

    #[test]
    fn test_uuid_constraint() {
        let constraint = UuidConstraint;
        assert!(
            constraint
                .validate("550e8400-e29b-41d4-a716-446655440000")
                .is_ok()
        );
        assert!(constraint.validate("not-a-uuid").is_err());
        assert!(constraint.validate("12345").is_err());
    }

    #[test]
    fn test_email_constraint() {
        let constraint = EmailConstraint;
        assert!(constraint.validate("user@example.com").is_ok());
        assert!(constraint.validate("test.user@domain.co.uk").is_ok());
        assert!(constraint.validate("invalid-email").is_err());
        assert!(constraint.validate("@example.com").is_err());
    }

    #[test]
    fn test_length_constraint() {
        let constraint = LengthConstraint::new(Some(3), Some(10));
        assert!(constraint.validate("hello").is_ok());
        assert!(constraint.validate("hi").is_err());
        assert!(constraint.validate("verylongstring").is_err());
    }

    #[test]
    fn test_length_constraint_min() {
        let constraint = LengthConstraint::min(5);
        assert!(constraint.validate("hello").is_ok());
        assert!(constraint.validate("verylongstring").is_ok());
        assert!(constraint.validate("hi").is_err());
    }

    #[test]
    fn test_length_constraint_max() {
        let constraint = LengthConstraint::max(10);
        assert!(constraint.validate("hello").is_ok());
        assert!(constraint.validate("hi").is_ok());
        assert!(constraint.validate("verylongstring").is_err());
    }

    #[test]
    fn test_length_constraint_exact() {
        let constraint = LengthConstraint::exact(5);
        assert!(constraint.validate("hello").is_ok());
        assert!(constraint.validate("hi").is_err());
        assert!(constraint.validate("toolong").is_err());
    }

    #[test]
    fn test_range_constraint() {
        let constraint = RangeConstraint::new(Some(1), Some(100));
        assert!(constraint.validate("50").is_ok());
        assert!(constraint.validate("1").is_ok());
        assert!(constraint.validate("100").is_ok());
        assert!(constraint.validate("0").is_err());
        assert!(constraint.validate("101").is_err());
        assert!(constraint.validate("abc").is_err());
    }

    #[test]
    fn test_enum_constraint() {
        let constraint = EnumConstraint::new(vec![
            "active".to_string(),
            "inactive".to_string(),
            "pending".to_string(),
        ]);
        assert!(constraint.validate("active").is_ok());
        assert!(constraint.validate("pending").is_ok());
        assert!(constraint.validate("unknown").is_err());
    }

    #[test]
    fn test_route_constraints() {
        let constraints = RouteConstraints::new()
            .add("id", Box::new(IntConstraint))
            .add("name", Box::new(AlphaConstraint));

        let mut params = HashMap::new();
        params.insert("id".to_string(), "123".to_string());
        params.insert("name".to_string(), "john".to_string());

        assert!(constraints.validate(&params).is_ok());

        let mut bad_params = HashMap::new();
        bad_params.insert("id".to_string(), "abc".to_string());
        bad_params.insert("name".to_string(), "john".to_string());

        assert!(constraints.validate(&bad_params).is_err());
    }
}
