//! File validation utilities.

#![allow(dead_code)]

use std::collections::HashSet;
use thiserror::Error;

use crate::UploadedFile;

/// Type alias for custom file validator function.
pub type FileValidatorFn = Box<dyn Fn(&UploadedFile) -> Result<(), String> + Send + Sync>;

/// Validation error.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// File is too large.
    #[error("File too large: {size} bytes exceeds maximum of {max} bytes")]
    TooLarge {
        /// Actual size.
        size: u64,
        /// Maximum size.
        max: u64,
    },

    /// File is too small.
    #[error("File too small: {size} bytes is below minimum of {min} bytes")]
    TooSmall {
        /// Actual size.
        size: u64,
        /// Minimum size.
        min: u64,
    },

    /// File type not allowed.
    #[error("File type not allowed: {mime_type}")]
    TypeNotAllowed {
        /// The disallowed MIME type.
        mime_type: String,
    },

    /// File extension not allowed.
    #[error("File extension not allowed: {extension}")]
    ExtensionNotAllowed {
        /// The disallowed extension.
        extension: String,
    },

    /// File name is required but missing.
    #[error("File name is required")]
    NameRequired,

    /// File is empty.
    #[error("File is empty")]
    Empty,

    /// Custom validation failed.
    #[error("Validation failed: {0}")]
    Custom(String),

    /// Multiple validation errors.
    #[error("Multiple validation errors")]
    Multiple(Vec<ValidationError>),
}

impl ValidationError {
    /// Create a custom validation error.
    pub fn custom(message: impl Into<String>) -> Self {
        Self::Custom(message.into())
    }
}

/// A validation rule for files.
pub trait ValidationRule: Send + Sync {
    /// Validate a file.
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError>;

    /// Rule description for error messages.
    fn description(&self) -> &str;
}

/// File validator with configurable rules.
#[derive(Default)]
pub struct FileValidator {
    rules: Vec<Box<dyn ValidationRule>>,
}

impl FileValidator {
    /// Create a new validator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a validation rule.
    pub fn rule(mut self, rule: impl ValidationRule + 'static) -> Self {
        self.rules.push(Box::new(rule));
        self
    }

    /// Set maximum file size.
    pub fn max_size(self, bytes: u64) -> Self {
        self.rule(MaxSizeRule(bytes))
    }

    /// Set minimum file size.
    pub fn min_size(self, bytes: u64) -> Self {
        self.rule(MinSizeRule(bytes))
    }

    /// Set allowed MIME types.
    pub fn allowed_types(self, types: &[&str]) -> Self {
        self.rule(AllowedTypesRule(
            types.iter().map(|s| s.to_string()).collect(),
        ))
    }

    /// Set allowed file extensions.
    pub fn allowed_extensions(self, extensions: &[&str]) -> Self {
        self.rule(AllowedExtensionsRule(
            extensions.iter().map(|s| s.to_lowercase()).collect(),
        ))
    }

    /// Require a file name.
    pub fn require_name(self) -> Self {
        self.rule(RequireNameRule)
    }

    /// Disallow empty files.
    pub fn not_empty(self) -> Self {
        self.rule(NotEmptyRule)
    }

    /// Only allow images.
    pub fn images_only(self) -> Self {
        self.allowed_types(&[
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/svg+xml",
        ])
    }

    /// Only allow documents.
    pub fn documents_only(self) -> Self {
        self.allowed_types(&[
            "application/pdf",
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "text/plain",
            "text/csv",
        ])
    }

    /// Add a custom validation function.
    pub fn custom<F>(self, name: &str, validator: F) -> Self
    where
        F: Fn(&UploadedFile) -> Result<(), String> + Send + Sync + 'static,
    {
        self.rule(CustomRule {
            name: name.to_string(),
            validator: Box::new(validator),
        })
    }

    /// Validate a file.
    pub fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        let mut errors = Vec::new();

        for rule in &self.rules {
            if let Err(e) = rule.validate(file) {
                errors.push(e);
            }
        }

        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.remove(0)),
            _ => Err(ValidationError::Multiple(errors)),
        }
    }

    /// Validate and return the file if valid.
    pub fn validate_file(&self, file: UploadedFile) -> Result<UploadedFile, ValidationError> {
        self.validate(&file)?;
        Ok(file)
    }
}

// Built-in validation rules

struct MaxSizeRule(u64);

impl ValidationRule for MaxSizeRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        if file.size() > self.0 {
            Err(ValidationError::TooLarge {
                size: file.size(),
                max: self.0,
            })
        } else {
            Ok(())
        }
    }

    fn description(&self) -> &str {
        "Maximum file size"
    }
}

struct MinSizeRule(u64);

impl ValidationRule for MinSizeRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        if file.size() < self.0 {
            Err(ValidationError::TooSmall {
                size: file.size(),
                min: self.0,
            })
        } else {
            Ok(())
        }
    }

    fn description(&self) -> &str {
        "Minimum file size"
    }
}

struct AllowedTypesRule(HashSet<String>);

impl ValidationRule for AllowedTypesRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        match file.content_type() {
            Some(mime) => {
                let mime_str = mime.to_string();
                if !self.0.contains(&mime_str) && !self.0.contains(&format!("{}/*", mime.type_())) {
                    return Err(ValidationError::TypeNotAllowed {
                        mime_type: mime_str,
                    });
                }
                Ok(())
            }
            // No detected content type: fail closed when an allowlist is configured.
            None => Err(ValidationError::TypeNotAllowed {
                mime_type: "unknown".to_string(),
            }),
        }
    }

    fn description(&self) -> &str {
        "Allowed MIME types"
    }
}

struct AllowedExtensionsRule(HashSet<String>);

impl ValidationRule for AllowedExtensionsRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        match file.extension() {
            Some(ext) => {
                let ext_lower = ext.to_lowercase();
                if !self.0.contains(&ext_lower) {
                    return Err(ValidationError::ExtensionNotAllowed {
                        extension: ext.to_string(),
                    });
                }
                Ok(())
            }
            // No extension: fail closed when an allowlist is configured.
            None => Err(ValidationError::ExtensionNotAllowed {
                extension: "(none)".to_string(),
            }),
        }
    }

    fn description(&self) -> &str {
        "Allowed file extensions"
    }
}

struct RequireNameRule;

impl ValidationRule for RequireNameRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        if file.name().is_none() {
            Err(ValidationError::NameRequired)
        } else {
            Ok(())
        }
    }

    fn description(&self) -> &str {
        "Require file name"
    }
}

struct NotEmptyRule;

impl ValidationRule for NotEmptyRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        if file.is_empty() {
            Err(ValidationError::Empty)
        } else {
            Ok(())
        }
    }

    fn description(&self) -> &str {
        "File must not be empty"
    }
}

struct CustomRule {
    name: String,
    validator: FileValidatorFn,
}

impl ValidationRule for CustomRule {
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        (self.validator)(file).map_err(ValidationError::Custom)
    }

    fn description(&self) -> &str {
        &self.name
    }
}

/// Common file size constants.
pub mod size {
    /// 1 KB
    pub const KB: u64 = 1024;
    /// 1 MB
    pub const MB: u64 = 1024 * KB;
    /// 1 GB
    pub const GB: u64 = 1024 * MB;

    /// 1 KB
    pub const fn kb(n: u64) -> u64 {
        n * KB
    }

    /// 1 MB
    pub const fn mb(n: u64) -> u64 {
        n * MB
    }

    /// 1 GB
    pub const fn gb(n: u64) -> u64 {
        n * GB
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_max_size_validation() {
        let validator = FileValidator::new().max_size(1024);

        let small_file = UploadedFile::new(Bytes::from(vec![0u8; 512]));
        assert!(validator.validate(&small_file).is_ok());

        let large_file = UploadedFile::new(Bytes::from(vec![0u8; 2048]));
        assert!(validator.validate(&large_file).is_err());
    }

    #[test]
    fn test_allowed_types_validation() {
        let validator = FileValidator::new().allowed_types(&["image/jpeg", "image/png"]);

        let jpeg_file = UploadedFile::from_bytes(Bytes::from("data"), "test.jpg");
        assert!(validator.validate(&jpeg_file).is_ok());
    }

    /// An allowlist that only checks *present* MIME types let untyped
    /// uploads through unconditionally. It must fail closed instead: no
    /// detected content type + a configured allowlist => reject.
    #[test]
    fn allowed_types_rejects_untyped_files() {
        let validator = FileValidator::new().allowed_types(&["image/jpeg", "image/png"]);

        // No file name at all => no MIME type can be inferred.
        let untyped_file = UploadedFile::new(Bytes::from("data"));
        assert!(untyped_file.content_type().is_none());

        let err = validator
            .validate(&untyped_file)
            .expect_err("untyped file must be rejected when an allowlist is configured");
        assert!(matches!(err, ValidationError::TypeNotAllowed { .. }));
    }

    /// Same fail-closed requirement for extensions: a file with no extension
    /// must be rejected when an extension allowlist is configured.
    #[test]
    fn allowed_extensions_rejects_extensionless_files() {
        let validator = FileValidator::new().allowed_extensions(&["jpg", "png"]);

        let mut extensionless_file = UploadedFile::new(Bytes::from("data"));
        extensionless_file.info.name = Some("no-extension-here".to_string());
        assert!(extensionless_file.extension().is_none());

        let err = validator
            .validate(&extensionless_file)
            .expect_err("extensionless file must be rejected when an allowlist is configured");
        assert!(matches!(err, ValidationError::ExtensionNotAllowed { .. }));
    }

    #[test]
    fn allowed_extensions_still_accepts_matching_extension() {
        let validator = FileValidator::new().allowed_extensions(&["jpg", "png"]);
        let file = UploadedFile::from_bytes(Bytes::from("data"), "photo.jpg");
        assert!(validator.validate(&file).is_ok());
    }
}
