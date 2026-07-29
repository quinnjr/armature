//! Validation framework for Armature
//!
//! Provides comprehensive data validation with built-in validators,
//! custom validation rules, and automatic request validation.
//!
//! # Examples
//!
//! ## Basic Validation
//!
//! ```
//! use armature_validation::{Validate, ValidationError, NotEmpty, MinLength, IsEmail};
//!
//! struct UserInput {
//!     name: String,
//!     email: String,
//! }
//!
//! impl Validate for UserInput {
//!     fn validate(&self) -> Result<(), Vec<ValidationError>> {
//!         let mut errors = Vec::new();
//!
//!         if let Err(e) = NotEmpty::validate(&self.name, "name") {
//!             errors.push(e);
//!         }
//!         if let Err(e) = MinLength(3).validate(&self.name, "name") {
//!             errors.push(e);
//!         }
//!         if let Err(e) = IsEmail::validate(&self.email, "email") {
//!             errors.push(e);
//!         }
//!
//!         if errors.is_empty() {
//!             Ok(())
//!         } else {
//!             Err(errors)
//!         }
//!     }
//! }
//!
//! let input = UserInput {
//!     name: "John".to_string(),
//!     email: "john@example.com".to_string(),
//! };
//! assert!(input.validate().is_ok());
//! ```
//!
//! ## Validation Rules Builder
//!
//! ```
//! use armature_validation::{ValidationRules, NotEmpty, MinLength};
//!
//! // Create rules for a field
//! let rules = ValidationRules::for_field("username")
//!     .add(|value, field| NotEmpty::validate(value, field))
//!     .add(|value, field| MinLength(3).validate(value, field));
//!
//! // Validate a value
//! let result = rules.validate("john");
//! assert!(result.is_ok());
//! ```
//!
//! ## Number Validation
//!
//! ```
//! use armature_validation::{Min, Max, InRange, IsPositive};
//!
//! // Min/Max validation
//! assert!(Min(18).validate(25, "age").is_ok());
//! assert!(Max(100).validate(50, "age").is_ok());
//!
//! // Range validation
//! let range = InRange { min: 1, max: 10 };
//! assert!(range.validate(5, "score").is_ok());
//!
//! // Positive number
//! assert!(IsPositive::validate_i32(42, "count").is_ok());
//! ```
//!
//! ## String Validators
//!
//! ```
//! use armature_validation::{IsEmail, IsUrl, MaxLength, Matches};
//!
//! // Email validation
//! assert!(IsEmail::validate("user@example.com", "email").is_ok());
//! assert!(IsEmail::validate("invalid-email", "email").is_err());
//!
//! // URL validation
//! assert!(IsUrl::validate("https://example.com", "website").is_ok());
//! assert!(IsUrl::validate("not-a-url", "website").is_err());
//!
//! // Length constraints
//! assert!(MaxLength(100).validate("short text", "description").is_ok());
//! assert!(MaxLength(5).validate("this is too long", "description").is_err());
//!
//! // Pattern matching with regex
//! let starts_with_capital = Matches::new("^[A-Z]").unwrap();
//! assert!(starts_with_capital.validate("Hello", "name").is_ok());
//! assert!(starts_with_capital.validate("hello", "name").is_err());
//! ```
//!
//! ## Custom Validators
//!
//! ```
//! use armature_validation::{Validator, ValidationError};
//! use std::any::Any;
//!
//! // Create a custom validator
//! struct IsStrongPassword;
//!
//! impl Validator for IsStrongPassword {
//!     fn validate(&self, value: &dyn Any, field: &str) -> Result<(), ValidationError> {
//!         let value = value.downcast_ref::<String>()
//!             .ok_or_else(|| ValidationError::new(field, "Expected string"))?;
//!
//!         let has_uppercase = value.chars().any(|c| c.is_uppercase());
//!         let has_lowercase = value.chars().any(|c| c.is_lowercase());
//!         let has_digit = value.chars().any(|c| c.is_numeric());
//!         let long_enough = value.len() >= 8;
//!
//!         if has_uppercase && has_lowercase && has_digit && long_enough {
//!             Ok(())
//!         } else {
//!             Err(ValidationError::new(
//!                 field,
//!                 "Password must be at least 8 characters with uppercase, lowercase, and digits"
//!             ))
//!         }
//!     }
//!
//!     fn name(&self) -> &'static str {
//!         "IsStrongPassword"
//!     }
//! }
//!
//! // Use the custom validator
//! let validator = IsStrongPassword;
//! let strong = "MyP@ssw0rd".to_string();
//! let weak = "password".to_string();
//!
//! assert!(validator.validate(&strong, "password").is_ok());
//! assert!(validator.validate(&weak, "password").is_err());
//! assert_eq!(validator.name(), "IsStrongPassword");
//! ```

mod errors;
mod pipe;
mod rules;
mod traits;
mod validators;

pub use errors::*;
pub use pipe::*;
pub use rules::*;
pub use traits::*;
pub use validators::*;

/// Derive macro for the [`Validate`] trait.
///
/// See the crate-level documentation and README for the `#[validate(...)]`
/// field-attribute syntax.
pub use armature_proc_macro::Validate;

/// Prelude for common imports.
///
/// ```
/// use armature_validation::prelude::*;
/// ```
pub mod prelude {
    pub use crate::errors::ValidationError;
    pub use crate::pipe::ValidationPipe;
    pub use crate::rules::ValidationRules;
    pub use crate::traits::{Validate, Validator};
    // Derive macro (macro namespace) alongside the `Validate` trait (type namespace).
    pub use crate::validators::{
        InRange, IsAlpha, IsAlphanumeric, IsEmail, IsPositive, IsUrl, Matches, Max, MaxLength, Min,
        MinLength, NotEmpty,
    };
    pub use armature_proc_macro::Validate;
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
