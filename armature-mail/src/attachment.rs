//! Email attachments.

use crate::{MailError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Content disposition for attachments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ContentDisposition {
    /// Attachment (for downloads).
    #[default]
    Attachment,
    /// Inline (for embedding in HTML).
    Inline,
}

/// Email attachment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// File name.
    pub filename: String,
    /// MIME type.
    pub content_type: String,
    /// File content.
    pub data: Vec<u8>,
    /// Content disposition.
    pub disposition: ContentDisposition,
    /// Content ID (for inline attachments).
    pub content_id: Option<String>,
}

impl Attachment {
    /// Create a new attachment from bytes.
    pub fn new(
        filename: impl Into<String>,
        content_type: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            filename: filename.into(),
            content_type: content_type.into(),
            data: data.into(),
            disposition: ContentDisposition::Attachment,
            content_id: None,
        }
    }

    /// Create an attachment from a file path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| MailError::Attachment("Invalid file name".to_string()))?
            .to_string();

        let content_type = mime_guess::from_path(path)
            .first()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let data = std::fs::read(path)?;

        Ok(Self::new(filename, content_type, data))
    }

    /// Create an attachment from bytes with automatic MIME type detection.
    pub fn from_bytes(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        let filename = filename.into();
        let content_type = mime_guess::from_path(&filename)
            .first()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        Self::new(filename, content_type, data)
    }

    /// Set the content disposition.
    pub fn disposition(mut self, disposition: ContentDisposition) -> Self {
        self.disposition = disposition;
        self
    }

    /// Make this an inline attachment (for embedding in HTML).
    pub fn inline(mut self) -> Self {
        self.disposition = ContentDisposition::Inline;
        self
    }

    /// Set the content ID (for inline references like <img src="cid:xxx">).
    pub fn content_id(mut self, id: impl Into<String>) -> Self {
        self.content_id = Some(id.into());
        self.disposition = ContentDisposition::Inline;
        self
    }

    /// Generate a unique content ID.
    pub fn with_generated_content_id(mut self) -> Self {
        self.content_id = Some(format!("{}@armature", uuid::Uuid::new_v4()));
        self.disposition = ContentDisposition::Inline;
        self
    }

    /// Get the size in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Whether this attachment should be rendered inline in the message body.
    pub fn is_inline(&self) -> bool {
        self.disposition == ContentDisposition::Inline
    }
}

/// Common attachment builders.
impl Attachment {
    /// Create a PDF attachment.
    pub fn pdf(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(filename, "application/pdf", data)
    }

    /// Create a PNG image attachment.
    pub fn png(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(filename, "image/png", data)
    }

    /// Create a JPEG image attachment.
    pub fn jpeg(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(filename, "image/jpeg", data)
    }

    /// Create a GIF image attachment.
    pub fn gif(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(filename, "image/gif", data)
    }

    /// Create a plain text attachment.
    pub fn text(filename: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(
            filename,
            "text/plain; charset=utf-8",
            content.into().into_bytes(),
        )
    }

    /// Create a CSV attachment.
    pub fn csv(filename: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(
            filename,
            "text/csv; charset=utf-8",
            content.into().into_bytes(),
        )
    }

    /// Create a JSON attachment.
    pub fn json(filename: impl Into<String>, content: impl Into<String>) -> Self {
        Self::new(filename, "application/json", content.into().into_bytes())
    }

    /// Create an Excel attachment.
    pub fn xlsx(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(
            filename,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            data,
        )
    }

    /// Create a Word document attachment.
    pub fn docx(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(
            filename,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            data,
        )
    }

    /// Create a ZIP archive attachment.
    pub fn zip(filename: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        Self::new(filename, "application/zip", data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("armature-mail-att-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn from_file_reads_content_and_guesses_the_type() {
        let path = scratch("report.pdf");
        std::fs::write(&path, b"%PDF-1.4 fake").unwrap();

        let attachment = Attachment::from_file(&path).unwrap();

        assert_eq!(attachment.filename, "report.pdf");
        assert_eq!(attachment.content_type, "application/pdf");
        assert_eq!(attachment.data, b"%PDF-1.4 fake");
        assert_eq!(attachment.size(), 13);
        assert_eq!(attachment.disposition, ContentDisposition::Attachment);
        assert!(attachment.content_id.is_none());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn from_file_guesses_png_and_falls_back_to_octet_stream() {
        let png = scratch("logo.png");
        std::fs::write(&png, [0x89, 0x50, 0x4E, 0x47]).unwrap();
        assert_eq!(
            Attachment::from_file(&png).unwrap().content_type,
            "image/png"
        );
        std::fs::remove_dir_all(png.parent().unwrap()).ok();

        let unknown = scratch("blob.zzzunknown");
        std::fs::write(&unknown, b"x").unwrap();
        assert_eq!(
            Attachment::from_file(&unknown).unwrap().content_type,
            "application/octet-stream"
        );
        std::fs::remove_dir_all(unknown.parent().unwrap()).ok();
    }

    #[test]
    fn from_file_errors_on_a_missing_path() {
        let missing = std::env::temp_dir().join("armature-mail-does-not-exist-9d1f2/x.txt");
        assert!(Attachment::from_file(&missing).is_err());
    }

    #[test]
    fn with_generated_content_id_is_unique_and_marks_inline() {
        let a = Attachment::png("a.png", vec![1]).with_generated_content_id();
        let b = Attachment::png("b.png", vec![1]).with_generated_content_id();

        let (Some(cid_a), Some(cid_b)) = (&a.content_id, &b.content_id) else {
            panic!("no content id generated");
        };
        assert_ne!(cid_a, cid_b, "content ids must be unique");
        assert!(cid_a.ends_with("@armature"), "unexpected cid: {cid_a}");
        // The uuid must actually parse; a `cid:` reference to a malformed id
        // resolves to nothing.
        let uuid_part = cid_a.trim_end_matches("@armature");
        assert!(uuid::Uuid::parse_str(uuid_part).is_ok(), "{cid_a}");

        assert!(a.is_inline());
        assert_eq!(a.disposition, ContentDisposition::Inline);
    }

    /// `content_id` implies inline, but an explicit disposition set afterwards
    /// must win — see the `build_part` regression in `email.rs`.
    #[test]
    fn explicit_disposition_overrides_the_content_id_default() {
        let a = Attachment::png("a.png", vec![1])
            .content_id("x")
            .disposition(ContentDisposition::Attachment);

        assert_eq!(a.content_id.as_deref(), Some("x"));
        assert!(!a.is_inline());
    }
}
