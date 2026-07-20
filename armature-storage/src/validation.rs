//! File validation utilities.

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

    /// The declared MIME type disagrees with the file's actual content.
    #[error("Declared content type {declared} does not match the detected content type {detected}")]
    ContentMismatch {
        /// The client-declared MIME type.
        declared: String,
        /// The MIME type sniffed from the file's magic bytes.
        detected: String,
    },

    /// The file's content type could not be determined from its bytes.
    #[error("Could not determine the content type from the file's contents")]
    ContentUndetectable,

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

    /// A failure attributed to the named rule that produced it.
    ///
    /// [`FileValidator::validate`] wraps every rule failure in this variant so
    /// the rule's [`ValidationRule::description`] -- including the
    /// caller-supplied name given to [`FileValidator::custom`] -- reaches the
    /// caller. Use [`ValidationError::kind`] to match on the underlying cause.
    #[error("{rule}: {source}")]
    Rule {
        /// The failing rule's description.
        rule: String,
        /// The underlying failure.
        #[source]
        source: Box<ValidationError>,
    },
}

impl ValidationError {
    /// Create a custom validation error.
    pub fn custom(message: impl Into<String>) -> Self {
        Self::Custom(message.into())
    }

    /// Attribute this error to the named rule.
    pub fn with_rule(self, rule: impl Into<String>) -> Self {
        Self::Rule {
            rule: rule.into(),
            source: Box::new(self),
        }
    }

    /// The underlying failure, with any [`ValidationError::Rule`] attribution
    /// stripped. Use this to match on the cause:
    ///
    /// ```
    /// use armature_storage::{FileValidator, UploadedFile, ValidationError, Bytes};
    ///
    /// let validator = FileValidator::new().max_size(1);
    /// let err = validator
    ///     .validate(&UploadedFile::new(Bytes::from("too big")))
    ///     .unwrap_err();
    ///
    /// assert!(matches!(err.kind(), ValidationError::TooLarge { .. }));
    /// ```
    pub fn kind(&self) -> &ValidationError {
        match self {
            Self::Rule { source, .. } => source.kind(),
            other => other,
        }
    }

    /// The description of the rule that produced this error, if attributed.
    pub fn rule(&self) -> Option<&str> {
        match self {
            Self::Rule { rule, .. } => Some(rule),
            _ => None,
        }
    }
}

/// Content-derived facts about a file, computed once per
/// [`FileValidator::validate`] call and shared by every rule.
///
/// [`sniff_content_type`] scans the payload and, for the SVG probe, allocates a
/// lossy `String` of the first KiB. Computing it once here and sharing it means
/// an N-rule validator scans the bytes once, not N times.
#[derive(Debug, Clone, Default)]
pub struct ValidationContext {
    /// The MIME type sniffed from the file's magic bytes, if recognizable.
    detected_content_type: Option<String>,
}

impl ValidationContext {
    /// Sniff `file` once, producing the context every rule then reads.
    pub fn for_file(file: &UploadedFile) -> Self {
        Self {
            detected_content_type: sniff_content_type(file.data()),
        }
    }

    /// The MIME type sniffed from the file's content, or `None` when its bytes
    /// match no format this can recognize.
    pub fn detected_content_type(&self) -> Option<&str> {
        self.detected_content_type.as_deref()
    }
}

/// A validation rule for files.
pub trait ValidationRule: Send + Sync {
    /// Validate a file.
    ///
    /// Implement [`Self::validate_with`] instead if the rule needs the sniffed
    /// content type; this method is what that defaults to.
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError>;

    /// Validate a file given content facts already computed for it.
    ///
    /// [`FileValidator::validate`] calls this, passing one [`ValidationContext`]
    /// to every rule so the payload is sniffed once per file rather than once
    /// per rule. The default ignores the context and defers to
    /// [`Self::validate`].
    fn validate_with(
        &self,
        file: &UploadedFile,
        _context: &ValidationContext,
    ) -> Result<(), ValidationError> {
        self.validate(file)
    }

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
    ///
    /// Both the client-declared `Content-Type` and the type sniffed from the
    /// file's magic bytes must be on the allowlist, and the two must agree, so
    /// an upload cannot pass by lying about its type. Content whose type
    /// cannot be sniffed at all (plain text, CSV -- neither has magic bytes)
    /// is accepted on the strength of its declared type; use
    /// [`Self::allowed_types_detected`] to reject it instead.
    pub fn allowed_types(self, types: &[&str]) -> Self {
        self.allowed_types_inner(types, false)
    }

    /// Like [`Self::allowed_types`], but additionally require that the type be
    /// determinable from the file's content, rejecting payloads whose bytes
    /// match no known format.
    pub fn allowed_types_detected(self, types: &[&str]) -> Self {
        self.allowed_types_inner(types, true)
    }

    fn allowed_types_inner(self, types: &[&str], require_detection: bool) -> Self {
        self.rule(AllowedTypesRule::new(types, require_detection))
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
    ///
    /// Every accepted image format is recognizable from its content, so this
    /// requires the payload to actually *be* one of them -- a text file
    /// uploaded as `Content-Type: image/png` is rejected.
    pub fn images_only(self) -> Self {
        self.allowed_types_detected(&[
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/svg+xml",
        ])
    }

    /// Only allow documents.
    ///
    /// The declared type must be on the allowlist and must agree with the
    /// file's content whenever the content is recognizable. `text/plain` and
    /// `text/csv` have no magic bytes, so they are accepted on their declared
    /// type alone.
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
    ///
    /// Each failure is attributed to the rule that produced it via
    /// [`ValidationError::Rule`]; match on [`ValidationError::kind`] to reach
    /// the underlying cause.
    pub fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        // Sniff the payload once and hand the result to every rule, rather
        // than letting each content-inspecting rule re-scan the whole file.
        let context = ValidationContext::for_file(file);
        let mut errors = Vec::new();

        for rule in &self.rules {
            if let Err(e) = rule.validate_with(file, &context) {
                errors.push(e.with_rule(rule.description()));
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

/// Sniff a file's MIME type from its leading magic bytes.
///
/// Returns `None` when the format is not one this can recognize -- notably
/// plain-text formats such as `text/plain` and `text/csv`, which have no magic
/// bytes at all. A `Some` result reflects the *actual* bytes, unlike the
/// client-declared `Content-Type` carried on a multipart part header.
pub fn sniff_content_type(data: &[u8]) -> Option<String> {
    if let Some(kind) = infer::get(data) {
        return Some(kind.mime_type().to_string());
    }

    // `infer` is magic-byte based and so cannot see SVG, which is XML text.
    // Recognize it explicitly since `images_only()` allows it.
    let head = &data[..data.len().min(1024)];
    let head = String::from_utf8_lossy(head);
    let trimmed = head.trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && head.contains("<svg")) {
        return Some("image/svg+xml".to_string());
    }

    None
}

/// Whether a client-declared MIME type is consistent with one sniffed from the
/// file's content.
///
/// Tolerates the well-known aliases and container generalizations that make an
/// exact string comparison too strict (a `.docx` is a ZIP; a legacy `.doc` is a
/// CFB compound file).
fn types_compatible(declared: &str, detected: &str) -> bool {
    fn normalize(t: &str) -> &str {
        match t {
            "image/jpg" => "image/jpeg",
            "image/svg" => "image/svg+xml",
            "text/xml" => "application/xml",
            other => other,
        }
    }

    let declared = normalize(declared);
    let detected = normalize(detected);

    if declared == detected {
        return true;
    }

    // OOXML documents are ZIP containers; older Office documents are CFB
    // compound files. Sniffing may legitimately report the container.
    let zip_backed = declared.starts_with("application/vnd.openxmlformats-officedocument.")
        || declared == "application/vnd.oasis.opendocument.text";
    if zip_backed && matches!(detected, "application/zip" | "application/x-zip-compressed") {
        return true;
    }

    let cfb_backed = declared == "application/msword"
        || declared == "application/vnd.ms-excel"
        || declared.starts_with("application/vnd.ms-");
    if cfb_backed
        && matches!(
            detected,
            "application/x-cfb" | "application/vnd.ms-office" | "application/x-ole-storage"
        )
    {
        return true;
    }

    false
}

/// MIME-type allowlist.
///
/// Checks both the client-declared `Content-Type` **and** the type sniffed
/// from the file's magic bytes, so an upload cannot pass by simply lying about
/// its type. When `require_detection` is set, content whose type cannot be
/// determined from its bytes is rejected outright.
struct AllowedTypesRule {
    /// Exact types, e.g. `image/png`.
    exact: HashSet<String>,
    /// Top-level types from wildcard entries, e.g. `image` for `image/*`.
    wildcard_tops: HashSet<String>,
    require_detection: bool,
}

impl AllowedTypesRule {
    /// Partition the allowlist into exact and wildcard entries up front.
    ///
    /// `permits` used to build a `format!("{top}/*")` `String` on every call --
    /// twice per file, per rule -- purely to probe a set. The split is decided
    /// once, at construction, instead.
    fn new(types: &[&str], require_detection: bool) -> Self {
        let mut exact = HashSet::new();
        let mut wildcard_tops = HashSet::new();

        for entry in types {
            match entry.split_once('/') {
                Some((top, "*")) => {
                    wildcard_tops.insert(top.to_string());
                }
                _ => {
                    exact.insert((*entry).to_string());
                }
            }
        }

        Self {
            exact,
            wildcard_tops,
            require_detection,
        }
    }

    fn permits(&self, mime_type: &str) -> bool {
        if self.exact.contains(mime_type) {
            return true;
        }
        // Wildcard form, e.g. "image/*".
        match mime_type.split_once('/') {
            Some((top, _)) => self.wildcard_tops.contains(top),
            None => false,
        }
    }

    fn check(&self, file: &UploadedFile, detected: Option<&str>) -> Result<(), ValidationError> {
        // 1. The declared type must be present and allowed. Fail closed when
        //    the client declared nothing.
        let declared = match file.content_type() {
            Some(mime) => {
                let declared = mime.to_string();
                if !self.permits(&declared) {
                    return Err(ValidationError::TypeNotAllowed {
                        mime_type: declared,
                    });
                }
                declared
            }
            None => {
                return Err(ValidationError::TypeNotAllowed {
                    mime_type: "unknown".to_string(),
                });
            }
        };

        // 2. The declared type is only a claim. Check the bytes -- sniffed once
        //    by `FileValidator::validate` and handed to us.
        match detected {
            Some(detected) => {
                if !self.permits(detected) {
                    return Err(ValidationError::TypeNotAllowed {
                        mime_type: detected.to_string(),
                    });
                }
                if !types_compatible(&declared, detected) {
                    return Err(ValidationError::ContentMismatch {
                        declared,
                        detected: detected.to_string(),
                    });
                }
                Ok(())
            }
            // Undetectable content: plain text and CSV have no magic bytes, so
            // this is only fatal for allowlists that promised otherwise.
            None if self.require_detection => Err(ValidationError::ContentUndetectable),
            None => Ok(()),
        }
    }
}

impl ValidationRule for AllowedTypesRule {
    /// Standalone entry point: sniffs the payload itself, for callers driving
    /// the rule directly rather than through [`FileValidator::validate`].
    fn validate(&self, file: &UploadedFile) -> Result<(), ValidationError> {
        self.check(file, sniff_content_type(file.data()).as_deref())
    }

    fn validate_with(
        &self,
        file: &UploadedFile,
        context: &ValidationContext,
    ) -> Result<(), ValidationError> {
        self.check(file, context.detected_content_type())
    }

    fn description(&self) -> &str {
        if self.require_detection {
            "Allowed MIME types (declared and detected)"
        } else {
            "Allowed MIME types"
        }
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
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal but genuine magic-byte payloads, so type checks see real
    /// content rather than the string "data".
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00";
    const JPEG: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00";
    const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x00\xff\x00,";
    const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF\n";
    const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg"><rect/></svg>"#;

    fn file(bytes: &'static [u8], name: &str) -> UploadedFile {
        UploadedFile::from_bytes(Bytes::from_static(bytes), name)
    }

    #[test]
    fn test_max_size_validation() {
        let validator = FileValidator::new().max_size(1024);

        let small_file = UploadedFile::new(Bytes::from(vec![0u8; 512]));
        assert!(validator.validate(&small_file).is_ok());

        let large_file = UploadedFile::new(Bytes::from(vec![0u8; 2048]));
        assert!(validator.validate(&large_file).is_err());
    }

    /// This test used to pass `from_bytes(Bytes::from("data"), "test.jpg")` --
    /// i.e. it asserted that a payload which is not remotely a JPEG passes a
    /// JPEG allowlist, documenting the divergence rather than catching it.
    #[test]
    fn test_allowed_types_validation() {
        let validator = FileValidator::new().allowed_types(&["image/jpeg", "image/png"]);

        assert!(validator.validate(&file(JPEG, "test.jpg")).is_ok());
        assert!(validator.validate(&file(PNG, "test.png")).is_ok());
        assert!(validator.validate(&file(GIF, "test.gif")).is_err());
    }

    /// The core of the finding: the MIME compared against the allowlist was
    /// whatever the client declared, so an arbitrary payload announcing
    /// `image/png` sailed through `images_only()`.
    #[test]
    fn images_only_rejects_a_payload_lying_about_its_type() {
        let validator = FileValidator::new().images_only();

        // Declared image/png (inferred from the .png name), actual content a
        // PDF. Must be rejected as a mismatch.
        let liar = UploadedFile::from_bytes(Bytes::from_static(PDF), "totally-an-image.png");
        assert_eq!(
            liar.content_type().map(|m| m.to_string()).as_deref(),
            Some("image/png"),
            "test precondition: the declared type is image/png"
        );

        let err = validator
            .validate(&liar)
            .expect_err("content that is not an image must be rejected");
        assert!(
            matches!(
                err.kind(),
                ValidationError::ContentMismatch { .. } | ValidationError::TypeNotAllowed { .. }
            ),
            "expected a content mismatch, got {err:?}"
        );
    }

    /// A payload with no recognizable format at all must not slip through
    /// `images_only()` either.
    #[test]
    fn images_only_rejects_undetectable_content() {
        let validator = FileValidator::new().images_only();
        let bogus = UploadedFile::from_bytes(Bytes::from_static(b"just some text"), "x.png");

        let err = validator
            .validate(&bogus)
            .expect_err("undetectable content must be rejected by images_only");
        assert!(matches!(err.kind(), ValidationError::ContentUndetectable));
    }

    #[test]
    fn images_only_accepts_real_images() {
        let validator = FileValidator::new().images_only();

        for (bytes, name) in [(PNG, "a.png"), (JPEG, "a.jpg"), (GIF, "a.gif")] {
            validator
                .validate(&file(bytes, name))
                .unwrap_or_else(|e| panic!("{name} should validate, got {e:?}"));
        }

        // SVG has no magic bytes; it is recognized structurally.
        validator
            .validate(&file(SVG, "a.svg"))
            .expect("svg should validate");
    }

    #[test]
    fn documents_only_rejects_a_payload_lying_about_its_type() {
        let validator = FileValidator::new().documents_only();
        let liar = UploadedFile::from_bytes(Bytes::from_static(PNG), "invoice.pdf");

        let err = validator
            .validate(&liar)
            .expect_err("a PNG declared as a PDF must be rejected");
        assert!(matches!(
            err.kind(),
            ValidationError::ContentMismatch { .. } | ValidationError::TypeNotAllowed { .. }
        ));
    }

    /// `text/plain` and `text/csv` have no magic bytes, so `documents_only`
    /// must still accept them on their declared type.
    #[test]
    fn documents_only_accepts_real_documents_and_plain_text() {
        let validator = FileValidator::new().documents_only();

        validator
            .validate(&file(PDF, "doc.pdf"))
            .expect("a real pdf should validate");
        validator
            .validate(&UploadedFile::from_bytes(
                Bytes::from_static(b"hello, world"),
                "notes.txt",
            ))
            .expect("plain text should validate");
        validator
            .validate(&UploadedFile::from_bytes(
                Bytes::from_static(b"a,b\n1,2\n"),
                "rows.csv",
            ))
            .expect("csv should validate");
    }

    #[test]
    fn sniff_content_type_reads_magic_bytes() {
        assert_eq!(sniff_content_type(PNG).as_deref(), Some("image/png"));
        assert_eq!(sniff_content_type(JPEG).as_deref(), Some("image/jpeg"));
        assert_eq!(sniff_content_type(PDF).as_deref(), Some("application/pdf"));
        assert_eq!(sniff_content_type(SVG).as_deref(), Some("image/svg+xml"));
        assert_eq!(sniff_content_type(b"plain text"), None);
    }

    /// `ValidationRule::description` was required, implemented six times, and
    /// never called -- so the name given to `FileValidator::custom` was
    /// unreachable and failures reported as a bare `Custom(msg)`.
    #[test]
    fn custom_rule_name_appears_in_the_failure() {
        let validator = FileValidator::new().custom("no-empty-uploads", |file| {
            if file.is_empty() {
                Err("file body was empty".to_string())
            } else {
                Ok(())
            }
        });

        let err = validator
            .validate(&UploadedFile::new(Bytes::new()))
            .expect_err("the custom rule should fail");

        assert_eq!(err.rule(), Some("no-empty-uploads"));
        let rendered = err.to_string();
        assert!(
            rendered.contains("no-empty-uploads") && rendered.contains("file body was empty"),
            "expected the rule name and message in {rendered:?}"
        );
        assert!(matches!(err.kind(), ValidationError::Custom(_)));
    }

    /// Built-in rules are attributed too, and `kind()` still reaches the cause.
    #[test]
    fn builtin_rule_failures_are_attributed() {
        let validator = FileValidator::new().max_size(1);
        let err = validator
            .validate(&UploadedFile::new(Bytes::from("too big")))
            .unwrap_err();

        assert_eq!(err.rule(), Some("Maximum file size"));
        assert!(matches!(err.kind(), ValidationError::TooLarge { .. }));
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
        assert!(matches!(err.kind(), ValidationError::TypeNotAllowed { .. }));
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
        assert!(matches!(
            err.kind(),
            ValidationError::ExtensionNotAllowed { .. }
        ));
    }

    #[test]
    fn allowed_extensions_still_accepts_matching_extension() {
        let validator = FileValidator::new().allowed_extensions(&["jpg", "png"]);
        let file = UploadedFile::from_bytes(Bytes::from("data"), "photo.jpg");
        assert!(validator.validate(&file).is_ok());
    }

    // --- ValidationContext / validate_with ----------------------------------

    /// A rule that records how many times it was handed a context, and what the
    /// context said the content type was.
    struct SniffCountingRule {
        contexts_seen: Arc<AtomicUsize>,
        detected: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl ValidationRule for SniffCountingRule {
        fn validate(&self, _file: &UploadedFile) -> Result<(), ValidationError> {
            Ok(())
        }

        fn validate_with(
            &self,
            _file: &UploadedFile,
            context: &ValidationContext,
        ) -> Result<(), ValidationError> {
            self.contexts_seen.fetch_add(1, Ordering::SeqCst);
            self.detected
                .lock()
                .unwrap()
                .push(context.detected_content_type().map(str::to_string));
            Ok(())
        }

        fn description(&self) -> &str {
            "Sniff counter"
        }
    }

    /// The point of `ValidationContext`: one sniff per `validate` call, shared
    /// by every rule, rather than one sniff per rule.
    #[test]
    fn every_rule_in_a_validator_sees_the_same_single_sniff() {
        let contexts_seen = Arc::new(AtomicUsize::new(0));
        let detected = Arc::new(Mutex::new(Vec::new()));

        let mut validator = FileValidator::new();
        for _ in 0..4 {
            validator = validator.rule(SniffCountingRule {
                contexts_seen: contexts_seen.clone(),
                detected: detected.clone(),
            });
        }

        validator
            .validate(&file(PNG, "a.png"))
            .expect("the counting rules never fail");

        assert_eq!(
            contexts_seen.load(Ordering::SeqCst),
            4,
            "every rule must be driven through validate_with"
        );
        assert_eq!(
            detected.lock().unwrap().as_slice(),
            [
                Some("image/png".to_string()),
                Some("image/png".to_string()),
                Some("image/png".to_string()),
                Some("image/png".to_string()),
            ],
            "all four rules must see the same, already-computed content type"
        );
    }

    /// `validate_with` is defaulted, so a rule written before it existed --
    /// implementing only `validate` -- must still be run, and still be able to
    /// fail the validation.
    struct LegacyRule {
        ran: Arc<AtomicUsize>,
    }

    impl ValidationRule for LegacyRule {
        fn validate(&self, _file: &UploadedFile) -> Result<(), ValidationError> {
            self.ran.fetch_add(1, Ordering::SeqCst);
            Err(ValidationError::custom("legacy rule says no"))
        }

        fn description(&self) -> &str {
            "Legacy rule"
        }
    }

    #[test]
    fn a_rule_implementing_only_validate_still_runs() {
        let ran = Arc::new(AtomicUsize::new(0));
        let validator = FileValidator::new().rule(LegacyRule { ran: ran.clone() });

        let err = validator
            .validate(&file(PNG, "a.png"))
            .expect_err("the legacy rule fails, and its failure must surface");

        assert_eq!(
            ran.load(Ordering::SeqCst),
            1,
            "the default validate_with must defer to validate"
        );
        assert_eq!(err.rule(), Some("Legacy rule"));
        assert!(matches!(err.kind(), ValidationError::Custom(_)));
    }

    /// The context is exactly `sniff_content_type` over the file's bytes --
    /// nothing more, nothing stale.
    #[test]
    fn context_detected_content_type_agrees_with_sniff_content_type() {
        for (bytes, name) in [
            (PNG, "a.png"),
            (JPEG, "a.jpg"),
            (GIF, "a.gif"),
            (PDF, "a.pdf"),
            (SVG, "a.svg"),
        ] {
            let uploaded = file(bytes, name);
            assert_eq!(
                ValidationContext::for_file(&uploaded)
                    .detected_content_type()
                    .map(str::to_string),
                sniff_content_type(bytes),
                "context and direct sniff must agree for {name}"
            );
        }

        // Undetectable content is `None`, not an empty string.
        let plain = UploadedFile::from_bytes(Bytes::from_static(b"just words"), "a.txt");
        assert_eq!(
            ValidationContext::for_file(&plain).detected_content_type(),
            None
        );
    }

    /// The default context sniffs nothing, so a rule constructed by hand gets
    /// `None` rather than a stale type from some other file.
    #[test]
    fn a_default_context_has_no_detected_type() {
        assert_eq!(ValidationContext::default().detected_content_type(), None);
    }

    // --- AllowedTypesRule exact/wildcard partition ---------------------------

    /// The allowlist is split into exact and wildcard entries once, at
    /// construction, instead of rebuilding a `format!("{top}/*")` probe string
    /// on every `permits` call. The partition must accept and reject exactly
    /// what the string probe did.
    #[test]
    fn allowed_types_wildcard_matches_a_whole_top_level_type() {
        let rule = AllowedTypesRule::new(&["image/*"], false);

        assert!(rule.permits("image/png"));
        assert!(rule.permits("image/jpeg"));
        assert!(rule.permits("image/anything-at-all"));
        assert!(!rule.permits("text/plain"));
        assert!(!rule.permits("application/pdf"));
        // Not a MIME type at all: no slash, no top-level part, no match.
        assert!(!rule.permits("image"));
    }

    #[test]
    fn allowed_types_exact_entries_do_not_match_siblings() {
        let rule = AllowedTypesRule::new(&["image/png", "text/plain"], false);

        assert!(rule.permits("image/png"));
        assert!(rule.permits("text/plain"));
        assert!(
            !rule.permits("image/jpeg"),
            "an exact entry is not a wildcard"
        );
        assert!(!rule.permits("text/csv"));
    }

    /// `*/*` splits as `("*", "*")`, i.e. a wildcard whose top-level type is
    /// `*` -- which matches nothing, exactly as it did before the partition.
    #[test]
    fn a_bare_star_slash_star_entry_behaves_as_before() {
        let rule = AllowedTypesRule::new(&["*/*"], false);

        assert!(!rule.permits("image/png"));
        assert!(!rule.permits("text/plain"));
        // ...but the literal string still matches itself, since the top-level
        // part of `*/*` is `*`.
        assert!(rule.permits("*/anything"));
    }

    /// The mixed case: exact and wildcard entries in one allowlist, driven
    /// through the whole validator rather than the private helper.
    #[test]
    fn a_mixed_allowlist_accepts_both_kinds_of_entry() {
        let validator = FileValidator::new().allowed_types(&["image/*", "application/pdf"]);

        validator
            .validate(&file(PNG, "a.png"))
            .expect("image/png is covered by image/*");
        validator
            .validate(&file(PDF, "a.pdf"))
            .expect("application/pdf is covered exactly");

        let err = validator
            .validate(&UploadedFile::from_bytes(
                Bytes::from_static(b"plain words"),
                "a.txt",
            ))
            .expect_err("text/plain is on neither list");
        assert!(matches!(err.kind(), ValidationError::TypeNotAllowed { .. }));
    }
}
