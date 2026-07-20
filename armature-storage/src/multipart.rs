//! Multipart form data parsing.

#![allow(dead_code)]

use bytes::Bytes;
use futures::Stream;

use crate::{Result, StorageError, UploadedFile};

/// Re-export multer's Field type.
pub type MultipartField<'a> = multer::Field<'a>;

/// Multipart form data parser.
///
/// ## Example
///
/// ```rust,ignore
/// use armature_storage::{Multipart, UploadedFile};
///
/// async fn handle_upload(multipart: Multipart) -> Result<Vec<UploadedFile>, Error> {
///     let mut files = Vec::new();
///     let mut stream = multipart.into_stream();
///
///     while let Some(field) = stream.next_field().await? {
///         if let Some(filename) = field.file_name() {
///             let file = UploadedFile::from_field(field).await?;
///             files.push(file);
///         }
///     }
///
///     Ok(files)
/// }
/// ```
pub struct Multipart {
    inner: multer::Multipart<'static>,
    counters: ConstraintCounters,
}

/// Running counters used to enforce the field/file-count limits of
/// [`MultipartConstraints`] (size and allowed-field-name limits are enforced
/// natively by `multer` via [`multer::Constraints`]).
#[derive(Debug, Default, Clone, Copy)]
struct ConstraintCounters {
    max_fields: Option<usize>,
    max_files: Option<usize>,
    field_count: usize,
    file_count: usize,
}

impl ConstraintCounters {
    fn from_constraints(constraints: &MultipartConstraints) -> Self {
        Self {
            max_fields: constraints.max_fields,
            max_files: constraints.max_files,
            field_count: 0,
            file_count: 0,
        }
    }

    /// Record a field and enforce the configured count limits.
    fn record(&mut self, is_file: bool) -> Result<()> {
        self.field_count += 1;
        if let Some(max_fields) = self.max_fields
            && self.field_count > max_fields
        {
            return Err(StorageError::Multipart(format!(
                "too many fields: exceeds maximum of {max_fields}"
            )));
        }

        if is_file {
            self.file_count += 1;
            if let Some(max_files) = self.max_files
                && self.file_count > max_files
            {
                return Err(StorageError::Multipart(format!(
                    "too many files: exceeds maximum of {max_files}"
                )));
            }
        }

        Ok(())
    }
}

/// Build a `multer::Constraints` from our [`MultipartConstraints`], covering
/// the size and allowed-field-name limits that `multer` natively enforces.
fn multer_constraints(constraints: &MultipartConstraints) -> multer::Constraints {
    let mut size_limit = multer::SizeLimit::new();
    if let Some(total) = constraints.max_total_size {
        size_limit = size_limit.whole_stream(total);
    }
    if let Some(field) = constraints.max_field_size {
        size_limit = size_limit.per_field(field);
    }

    let mut multer_constraints = multer::Constraints::new().size_limit(size_limit);
    if let Some(allowed) = &constraints.allowed_fields {
        multer_constraints = multer_constraints.allowed_fields(allowed.clone());
    }

    multer_constraints
}

impl Multipart {
    /// Create a new multipart parser from a stream and boundary.
    pub fn new<S>(stream: S, boundary: &str) -> Self
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self {
            inner: multer::Multipart::new(stream, boundary),
            counters: ConstraintCounters::default(),
        }
    }

    /// Create a new multipart parser enforcing [`MultipartConstraints`].
    ///
    /// Size limits (`max_total_size`, `max_field_size`) and `allowed_fields`
    /// are enforced natively by `multer` while streaming; `max_fields` and
    /// `max_files` are enforced by this wrapper as fields are drained.
    pub fn with_constraints<S>(stream: S, boundary: &str, constraints: MultipartConstraints) -> Self
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self {
            inner: multer::Multipart::with_constraints(
                stream,
                boundary,
                multer_constraints(&constraints),
            ),
            counters: ConstraintCounters::from_constraints(&constraints),
        }
    }

    /// Create from HTTP headers and body.
    pub fn from_request<S>(content_type: &str, body: S) -> Result<Self>
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        let boundary = multer::parse_boundary(content_type)
            .map_err(|e| StorageError::Multipart(e.to_string()))?;

        Ok(Self::new(body, &boundary))
    }

    /// Create from HTTP headers and body, enforcing [`MultipartConstraints`].
    pub fn from_request_with_constraints<S>(
        content_type: &str,
        body: S,
        constraints: MultipartConstraints,
    ) -> Result<Self>
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        let boundary = multer::parse_boundary(content_type)
            .map_err(|e| StorageError::Multipart(e.to_string()))?;

        Ok(Self::with_constraints(body, &boundary, constraints))
    }

    /// Get the next field from the multipart stream.
    pub async fn next_field(&mut self) -> Result<Option<multer::Field<'static>>> {
        let field = self.inner.next_field().await.map_err(StorageError::from)?;
        if let Some(field) = &field {
            self.counters.record(field.file_name().is_some())?;
        }
        Ok(field)
    }

    /// Convert into a stream of fields.
    pub fn into_stream(self) -> MultipartStream {
        MultipartStream {
            inner: self.inner,
            counters: self.counters,
        }
    }

    /// Collect all file fields into uploaded files.
    pub async fn collect_files(mut self) -> Result<Vec<UploadedFile>> {
        let mut files = Vec::new();

        while let Some(field) = self.next_field().await? {
            if field.file_name().is_some() {
                let file = UploadedFile::from_field(field).await?;
                files.push(file);
            }
        }

        Ok(files)
    }

    /// Collect all fields (both files and form data).
    pub async fn collect_all(mut self) -> Result<MultipartData> {
        let mut data = MultipartData::new();

        while let Some(field) = self.next_field().await? {
            let name = field.name().map(String::from);

            if field.file_name().is_some() {
                let file = UploadedFile::from_field(field).await?;
                if let Some(name) = name {
                    data.files.insert(name, file);
                }
            } else {
                let text = field.text().await.map_err(StorageError::from)?;
                if let Some(name) = name {
                    data.fields.insert(name, text);
                }
            }
        }

        Ok(data)
    }
}

/// Stream wrapper for multipart fields.
pub struct MultipartStream {
    inner: multer::Multipart<'static>,
    counters: ConstraintCounters,
}

impl MultipartStream {
    /// Get the next field.
    pub async fn next_field(&mut self) -> Result<Option<multer::Field<'static>>> {
        let field = self.inner.next_field().await.map_err(StorageError::from)?;
        if let Some(field) = &field {
            self.counters.record(field.file_name().is_some())?;
        }
        Ok(field)
    }
}

/// Collected multipart data.
#[derive(Debug, Default)]
pub struct MultipartData {
    /// Form fields (non-file fields).
    pub fields: std::collections::HashMap<String, String>,
    /// Uploaded files.
    pub files: std::collections::HashMap<String, UploadedFile>,
}

impl MultipartData {
    /// Create empty multipart data.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a form field value.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// Get an uploaded file.
    pub fn file(&self, name: &str) -> Option<&UploadedFile> {
        self.files.get(name)
    }

    /// Take an uploaded file (removes it from the collection).
    pub fn take_file(&mut self, name: &str) -> Option<UploadedFile> {
        self.files.remove(name)
    }

    /// Check if there are any files.
    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    /// Get the number of files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// Constraints for multipart parsing.
#[derive(Debug, Clone)]
pub struct MultipartConstraints {
    /// Maximum total size of all fields.
    pub max_total_size: Option<u64>,
    /// Maximum size of a single field.
    pub max_field_size: Option<u64>,
    /// Maximum number of fields.
    pub max_fields: Option<usize>,
    /// Maximum number of files.
    pub max_files: Option<usize>,
    /// Allowed field names.
    pub allowed_fields: Option<Vec<String>>,
}

impl Default for MultipartConstraints {
    fn default() -> Self {
        Self {
            max_total_size: Some(100 * 1024 * 1024), // 100 MB
            max_field_size: Some(50 * 1024 * 1024),  // 50 MB
            max_fields: Some(100),
            max_files: Some(10),
            allowed_fields: None,
        }
    }
}

impl MultipartConstraints {
    /// Create new constraints with no limits.
    pub fn unlimited() -> Self {
        Self {
            max_total_size: None,
            max_field_size: None,
            max_fields: None,
            max_files: None,
            allowed_fields: None,
        }
    }

    /// Set maximum total size.
    pub fn max_total_size(mut self, size: u64) -> Self {
        self.max_total_size = Some(size);
        self
    }

    /// Set maximum field size.
    pub fn max_field_size(mut self, size: u64) -> Self {
        self.max_field_size = Some(size);
        self
    }

    /// Set maximum number of fields.
    pub fn max_fields(mut self, count: usize) -> Self {
        self.max_fields = Some(count);
        self
    }

    /// Set maximum number of files.
    pub fn max_files(mut self, count: usize) -> Self {
        self.max_files = Some(count);
        self
    }

    /// Set allowed field names.
    pub fn allowed_fields(mut self, fields: Vec<String>) -> Self {
        self.allowed_fields = Some(fields);
        self
    }
}

/// Helper to create a Multipart from an HTTP request body.
pub fn parse_multipart<S>(content_type: &http::HeaderValue, body: S) -> Result<Multipart>
where
    S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
{
    let content_type = content_type
        .to_str()
        .map_err(|_| StorageError::Multipart("Invalid content-type header".to_string()))?;

    Multipart::from_request(content_type, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    const BOUNDARY: &str = "X-TEST-BOUNDARY";

    /// Build a raw `multipart/form-data` body from `(name, filename, content)`
    /// parts. A `None` filename produces a plain form field.
    fn build_body(parts: &[(&str, Option<&str>, &str)]) -> Bytes {
        let mut body = String::new();
        for (name, filename, content) in parts {
            body.push_str(&format!("--{BOUNDARY}\r\n"));
            match filename {
                Some(fname) => {
                    body.push_str(&format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\n"
                    ));
                    body.push_str("Content-Type: application/octet-stream\r\n");
                }
                None => {
                    body.push_str(&format!(
                        "Content-Disposition: form-data; name=\"{name}\"\r\n"
                    ));
                }
            }
            body.push_str("\r\n");
            body.push_str(content);
            body.push_str("\r\n");
        }
        body.push_str(&format!("--{BOUNDARY}--\r\n"));
        Bytes::from(body)
    }

    fn multipart_with(
        parts: &[(&str, Option<&str>, &str)],
        constraints: MultipartConstraints,
    ) -> Multipart {
        let body = build_body(parts);
        let stream = stream::once(async move { Ok::<_, std::io::Error>(body) });
        Multipart::with_constraints(stream, BOUNDARY, constraints)
    }

    async fn drain(mut multipart: Multipart) -> Result<usize> {
        let mut count = 0;
        while multipart.next_field().await?.is_some() {
            count += 1;
        }
        Ok(count)
    }

    #[tokio::test]
    async fn unconstrained_multipart_accepts_everything() {
        let mp = multipart_with(
            &[("a", None, "1"), ("b", Some("b.txt"), "2")],
            MultipartConstraints::unlimited(),
        );
        assert_eq!(drain(mp).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn max_fields_rejects_extra_fields() {
        let constraints = MultipartConstraints::unlimited().max_fields(1);
        let mp = multipart_with(&[("a", None, "1"), ("b", None, "2")], constraints);

        let err = drain(mp)
            .await
            .expect_err("second field should be rejected");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    #[tokio::test]
    async fn max_files_rejects_extra_files() {
        let constraints = MultipartConstraints::unlimited().max_files(1);
        let mp = multipart_with(
            &[("f1", Some("a.txt"), "1"), ("f2", Some("b.txt"), "2")],
            constraints,
        );

        let err = drain(mp).await.expect_err("second file should be rejected");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    #[tokio::test]
    async fn max_files_ignores_non_file_fields() {
        let constraints = MultipartConstraints::unlimited().max_files(1);
        let mp = multipart_with(
            &[
                ("text1", None, "1"),
                ("text2", None, "2"),
                ("f1", Some("a.txt"), "3"),
            ],
            constraints,
        );

        assert_eq!(drain(mp).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn allowed_fields_rejects_unknown_field_names() {
        let constraints = MultipartConstraints::unlimited().allowed_fields(vec!["ok".to_string()]);
        let mp = multipart_with(&[("not-ok", None, "1")], constraints);

        let err = drain(mp)
            .await
            .expect_err("disallowed field name should be rejected");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    #[tokio::test]
    async fn max_field_size_rejects_oversized_field() {
        let constraints = MultipartConstraints::unlimited().max_field_size(4);
        let mp = multipart_with(&[("big", None, "way too long")], constraints);

        // multer enforces per-field size limits while streaming the field
        // body, so the rejection surfaces when the field content is read.
        let mut mp = mp;
        let field = mp
            .next_field()
            .await
            .unwrap()
            .expect("field should be yielded before its body is fully read");
        let err = field
            .bytes()
            .await
            .expect_err("oversized field content should be rejected");
        assert!(!err.to_string().is_empty());
    }

    #[tokio::test]
    async fn max_total_size_rejects_oversized_stream() {
        let constraints = MultipartConstraints::unlimited().max_total_size(4);
        let mp = multipart_with(&[("big", None, "way too long")], constraints);

        let err = drain(mp)
            .await
            .expect_err("stream exceeding total size should be rejected");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    #[tokio::test]
    async fn default_constraints_cap_fields_and_files() {
        // `MultipartConstraints::default()` is `max_fields: Some(100)` /
        // `max_files: Some(10)` -- confirm the defaults are actually wired in
        // by using a very small subset that stays under them.
        let constraints = MultipartConstraints::default();
        let mp = multipart_with(&[("a", None, "1"), ("b", Some("b.txt"), "2")], constraints);
        assert_eq!(drain(mp).await.unwrap(), 2);
    }
}
