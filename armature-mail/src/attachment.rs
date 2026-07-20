//! Email attachments.

use crate::{MailError, Result};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Default ceiling on the size of a file read by [`Attachment::from_file`] and
/// [`Attachment::from_file_async`].
///
/// 25 MB is the smallest of the common provider limits (Gmail, SendGrid, SES),
/// so anything above it could not be delivered anyway — and reading it would
/// pull the whole file into memory first, which is exactly what an uncapped
/// read on a path the caller does not fully control gets you.
pub const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// Serialize attachment payloads as base64 rather than a JSON number array.
///
/// `Vec<u8>`/`Bytes` serialize as a sequence, which in JSON is `[104,105,…]` —
/// roughly 3.5–4 bytes of JSON per source byte. The email queue stores the whole
/// job as JSON in Redis and rewrites it on every retry, so a 10 MB attachment
/// cost ~37 MB per `SET`. Base64 costs 1.33x instead.
///
/// Deserialization accepts *both* forms. `EmailJob` is persisted as JSON in
/// Redis and holds an `Arc<Email>`, so a queue populated by 0.1.x contains
/// `"data":[1,2,3]` bodies. Rejecting those would make `EmailQueueWorker::pop`
/// fail every attachment-bearing job, push it onto `lost`, and `discard_lost`
/// would then `DEL` the body — destroying mail that `enqueue` already
/// acknowledged. So a sequence is still read; only writing is base64.
mod base64_bytes {
    use base64::Engine;
    use bytes::Bytes;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(data: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(data))
    }

    /// Either a base64 string (0.2+) or a byte sequence (0.1.x).
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Payload {
        Base64(String),
        Sequence(Vec<u8>),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        match Payload::deserialize(d)? {
            Payload::Base64(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded.as_bytes())
                .map(Bytes::from)
                .map_err(serde::de::Error::custom),
            Payload::Sequence(bytes) => Ok(Bytes::from(bytes)),
        }
    }
}

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
    ///
    /// [`Bytes`] rather than `Vec<u8>`: an `Attachment` is cloned once per send
    /// attempt and once per queue retry, and a refcount bump is not a copy of a
    /// 10 MB payload. Serialized as base64 — see [`base64_bytes`].
    #[serde(with = "base64_bytes")]
    pub data: Bytes,
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
        data: impl Into<Bytes>,
    ) -> Self {
        Self {
            filename: filename.into(),
            content_type: content_type.into(),
            data: data.into(),
            disposition: ContentDisposition::Attachment,
            content_id: None,
        }
    }

    /// Create an attachment from a file path, reading the file synchronously.
    ///
    /// Rejects a file larger than [`DEFAULT_MAX_ATTACHMENT_BYTES`] *before*
    /// reading it, so a wrong path cannot pull an arbitrarily large file into
    /// memory. Use [`Attachment::from_file_with_limit`] for a different ceiling.
    /// This blocks the calling thread; from async code use
    /// [`Attachment::from_file_async`], which does not.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_file_with_limit(path, DEFAULT_MAX_ATTACHMENT_BYTES)
    }

    /// [`Attachment::from_file`] with an explicit size ceiling.
    pub fn from_file_with_limit(path: impl AsRef<Path>, max_bytes: u64) -> Result<Self> {
        let path = path.as_ref();
        let (filename, content_type) = Self::describe(path)?;

        let size = std::fs::metadata(path)?.len();
        Self::check_size(&filename, size, max_bytes)?;

        let data = std::fs::read(path)?;

        Ok(Self::new(filename, content_type, data))
    }

    /// Create an attachment from a file path without blocking the runtime.
    ///
    /// [`Attachment::from_file`] does a blocking `std::fs::read`; called from a
    /// task it stalls the whole executor thread for the duration of the read,
    /// which for a multi-megabyte attachment on a slow disk is exactly the kind
    /// of stall an async crate exists to avoid. The size is checked against
    /// [`DEFAULT_MAX_ATTACHMENT_BYTES`] via `metadata` before any bytes are read;
    /// use [`Attachment::from_file_async_with_limit`] for a different ceiling.
    pub async fn from_file_async(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_file_async_with_limit(path, DEFAULT_MAX_ATTACHMENT_BYTES).await
    }

    /// [`Attachment::from_file_async`] with an explicit size ceiling.
    pub async fn from_file_async_with_limit(
        path: impl AsRef<Path>,
        max_bytes: u64,
    ) -> Result<Self> {
        let path = path.as_ref();
        let (filename, content_type) = Self::describe(path)?;

        let size = tokio::fs::metadata(path).await?.len();
        Self::check_size(&filename, size, max_bytes)?;

        let data = tokio::fs::read(path).await?;

        Ok(Self::new(filename, content_type, data))
    }

    /// Derive the attachment filename and MIME type from a path.
    fn describe(path: &Path) -> Result<(String, String)> {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| MailError::Attachment("Invalid file name".to_string()))?
            .to_string();

        let content_type = mime_guess::from_path(path)
            .first()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        Ok((filename, content_type))
    }

    fn check_size(filename: &str, size: u64, max_bytes: u64) -> Result<()> {
        if size > max_bytes {
            return Err(MailError::Attachment(format!(
                "attachment {filename:?} is {size} bytes, over the {max_bytes}-byte limit"
            )));
        }
        Ok(())
    }

    /// Create an attachment from bytes with automatic MIME type detection.
    pub fn from_bytes(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
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
    pub fn pdf(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self::new(filename, "application/pdf", data)
    }

    /// Create a PNG image attachment.
    pub fn png(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self::new(filename, "image/png", data)
    }

    /// Create a JPEG image attachment.
    pub fn jpeg(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self::new(filename, "image/jpeg", data)
    }

    /// Create a GIF image attachment.
    pub fn gif(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
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
    pub fn xlsx(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self::new(
            filename,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            data,
        )
    }

    /// Create a Word document attachment.
    pub fn docx(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
        Self::new(
            filename,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            data,
        )
    }

    /// Create a ZIP archive attachment.
    pub fn zip(filename: impl Into<String>, data: impl Into<Bytes>) -> Self {
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
        assert_eq!(&attachment.data[..], b"%PDF-1.4 fake");
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

    /// `from_file` did a blocking `std::fs::read` on an async crate's path, so
    /// there was no way to load an attachment from a task without stalling the
    /// executor thread. The async variant must produce an identical attachment.
    #[tokio::test]
    async fn from_file_async_matches_the_blocking_variant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"%PDF-1.4 fake").unwrap();

        let sync = Attachment::from_file(&path).unwrap();
        let asynced = Attachment::from_file_async(&path).await.unwrap();

        assert_eq!(asynced.filename, sync.filename);
        assert_eq!(asynced.content_type, sync.content_type);
        assert_eq!(asynced.data, sync.data);
    }

    #[tokio::test]
    async fn from_file_async_errors_on_a_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Attachment::from_file_async(dir.path().join("nope.txt"))
                .await
                .is_err()
        );
    }

    /// The read was uncapped: a wrong path pulled an arbitrarily large file into
    /// memory in full. The size must be checked via `metadata` *before* reading.
    #[tokio::test]
    async fn an_oversized_file_is_rejected_before_it_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();

        for err in [
            Attachment::from_file_with_limit(&path, 1024).unwrap_err(),
            Attachment::from_file_async_with_limit(&path, 1024)
                .await
                .unwrap_err(),
        ] {
            assert!(
                matches!(err, MailError::Attachment(ref m) if m.contains("limit")),
                "unexpected error: {err}"
            );
        }

        // At or under the limit it still reads.
        assert_eq!(
            Attachment::from_file_with_limit(&path, 4096)
                .unwrap()
                .size(),
            4096
        );
    }

    /// An attachment payload used to serialize as a JSON number array — ~4 bytes
    /// of JSON per source byte, paid again on every queue retry.
    #[test]
    fn payloads_serialize_as_base64_not_a_number_array() {
        let attachment = Attachment::new("x.bin", "application/octet-stream", vec![1u8, 2, 3]);
        let json = serde_json::to_string(&attachment).unwrap();

        assert!(json.contains(r#""data":"AQID""#), "not base64: {json}");
        assert!(!json.contains("[1,2,3]"), "number array emitted: {json}");

        let round_tripped: Attachment = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.data, attachment.data);
    }

    /// 0.1.x wrote `"data":[1,2,3]`. `EmailJob` is persisted as JSON in Redis, so
    /// an in-place upgrade finds those bodies in a live queue: if the new
    /// deserializer rejected them, every attachment-bearing job would fail in
    /// `pop`, land on `lost`, and be `DEL`d by `discard_lost` — mail destroyed
    /// after `enqueue` already returned `Ok(job_id)`. The legacy form must still
    /// load.
    #[test]
    fn a_legacy_number_array_payload_still_deserializes() {
        let legacy = r#"{
            "filename": "x.bin",
            "content_type": "application/octet-stream",
            "data": [1, 2, 3],
            "disposition": "Attachment",
            "content_id": null
        }"#;

        let attachment: Attachment = serde_json::from_str(legacy).expect("0.1.x payload must load");

        assert_eq!(&attachment.data[..], &[1u8, 2, 3]);
        assert_eq!(attachment.filename, "x.bin");

        // And re-serializing upgrades it to base64 in place.
        let json = serde_json::to_string(&attachment).unwrap();
        assert!(json.contains(r#""data":"AQID""#), "{json}");
    }

    /// The current form: a base64 string.
    #[test]
    fn a_base64_payload_deserializes() {
        let current = r#"{
            "filename": "x.bin",
            "content_type": "application/octet-stream",
            "data": "AQID",
            "disposition": "Attachment",
            "content_id": null
        }"#;

        let attachment: Attachment = serde_json::from_str(current).unwrap();
        assert_eq!(&attachment.data[..], &[1u8, 2, 3]);
    }

    /// An empty payload round-trips in both forms — `""` and `[]` both decode to
    /// zero bytes rather than erroring.
    #[test]
    fn an_empty_payload_round_trips_in_both_forms() {
        for data in [r#""""#, "[]"] {
            let json = format!(
                r#"{{"filename":"e","content_type":"text/plain","data":{data},"disposition":"Attachment","content_id":null}}"#
            );
            let attachment: Attachment = serde_json::from_str(&json).unwrap();
            assert!(attachment.data.is_empty(), "{data}");
        }
    }

    /// A genuinely malformed payload is still an error — the sequence fallback
    /// must not turn invalid base64 into silent corruption.
    #[test]
    fn a_malformed_payload_is_still_rejected() {
        let bad = r#"{"filename":"x","content_type":"text/plain","data":"not base64!!","disposition":"Attachment","content_id":null}"#;
        assert!(serde_json::from_str::<Attachment>(bad).is_err());
    }

    /// The base64 encoding must stay compact for a realistic payload: 1.33x, not
    /// the ~3.5-4x a number array costs.
    #[test]
    fn base64_payloads_stay_close_to_the_source_size() {
        let attachment = Attachment::new("x.bin", "application/octet-stream", vec![0xABu8; 10_000]);
        let json = serde_json::to_string(&attachment).unwrap();
        assert!(
            json.len() < 15_000,
            "payload encoding is not compact: {} bytes for 10000",
            json.len()
        );
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
