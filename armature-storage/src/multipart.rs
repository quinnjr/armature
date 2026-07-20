//! Multipart form data parsing.

use bytes::Bytes;
use futures_core::Stream;

use crate::{Result, StorageError, UploadedFile};

/// Re-export multer's Field type.
pub type MultipartField<'a> = multer::Field<'a>;

/// Multipart form data parser.
///
/// ## Example
///
/// ```rust,no_run
/// use armature_storage::{Multipart, Result, UploadedFile};
///
/// async fn handle_upload(multipart: Multipart) -> Result<Vec<UploadedFile>> {
///     let mut files = Vec::new();
///     let mut stream = multipart.into_stream();
///
///     while let Some(field) = stream.next_field().await? {
///         if field.file_name().is_some() {
///             files.push(UploadedFile::from_field(field).await?);
///         }
///     }
///
///     Ok(files)
/// }
/// ```
///
/// `Multipart::collect_files` does exactly this, if you do not need to inspect
/// each field as it arrives.
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
    /// Create a new multipart parser enforcing [`MultipartConstraints::default`].
    ///
    /// The defaults cap total stream size, per-field size, field count and file
    /// count, so an unauthenticated request cannot drive unbounded heap growth
    /// through [`Self::collect_files`] / [`Self::collect_all`]. Use
    /// [`Self::with_constraints`] to tune them, or [`Self::unconstrained`] to
    /// opt out entirely.
    pub fn new<S>(stream: S, boundary: &str) -> Self
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self::with_constraints(stream, boundary, MultipartConstraints::default())
    }

    /// Create a multipart parser with **no** limits of any kind.
    ///
    /// This is an explicit escape hatch: every field, file and byte count is
    /// unbounded, so only use it for trusted input. Prefer [`Self::new`].
    pub fn unconstrained<S>(stream: S, boundary: &str) -> Self
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self::with_constraints(stream, boundary, MultipartConstraints::unlimited())
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

    /// Create from HTTP headers and body, enforcing
    /// [`MultipartConstraints::default`].
    pub fn from_request<S>(content_type: &str, body: S) -> Result<Self>
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self::from_request_with_constraints(content_type, body, MultipartConstraints::default())
    }

    /// Create from HTTP headers and body with **no** limits of any kind.
    ///
    /// Explicit escape hatch; see [`Self::unconstrained`].
    pub fn from_request_unconstrained<S>(content_type: &str, body: S) -> Result<Self>
    where
        S: Stream<Item = std::result::Result<Bytes, std::io::Error>> + Send + 'static,
    {
        Self::from_request_with_constraints(content_type, body, MultipartConstraints::unlimited())
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
        MultipartStream(self)
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
///
/// A thin newtype over [`Multipart`] so constraint enforcement has exactly one
/// implementation rather than two copies that can drift apart.
pub struct MultipartStream(Multipart);

impl MultipartStream {
    /// Get the next field, enforcing the same constraints as [`Multipart`].
    pub async fn next_field(&mut self) -> Result<Option<multer::Field<'static>>> {
        self.0.next_field().await
    }

    /// Convert back into the underlying [`Multipart`], preserving the counters.
    pub fn into_multipart(self) -> Multipart {
        self.0
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

    /// Build `n` plain form fields.
    fn text_parts(n: usize) -> Vec<(String, Option<String>, String)> {
        (0..n)
            .map(|i| (format!("field{i}"), None, i.to_string()))
            .collect()
    }

    /// Build `n` file fields.
    fn file_parts(n: usize) -> Vec<(String, Option<String>, String)> {
        (0..n)
            .map(|i| (format!("file{i}"), Some(format!("f{i}.txt")), i.to_string()))
            .collect()
    }

    fn as_refs(parts: &[(String, Option<String>, String)]) -> Vec<(&str, Option<&str>, &str)> {
        parts
            .iter()
            .map(|(n, f, c)| (n.as_str(), f.as_deref(), c.as_str()))
            .collect()
    }

    /// The default `max_fields: Some(100)` must actually reject the 101st
    /// field. The previous version of this test fed 2 fields against a limit
    /// of 100 and asserted they were drained -- which passes identically under
    /// `unlimited()` or with the defaults deleted outright.
    #[tokio::test]
    async fn default_constraints_reject_the_101st_field() {
        let parts = text_parts(101);
        let mp = multipart_with(&as_refs(&parts), MultipartConstraints::default());

        let err = drain(mp).await.expect_err("101st field must be rejected");
        let msg = err.to_string();
        assert!(
            matches!(err, StorageError::Multipart(_)) && msg.contains("100"),
            "expected a multipart error naming the limit of 100, got: {msg}"
        );
    }

    #[tokio::test]
    async fn default_constraints_accept_exactly_100_fields() {
        let parts = text_parts(100);
        let mp = multipart_with(&as_refs(&parts), MultipartConstraints::default());
        assert_eq!(drain(mp).await.unwrap(), 100);
    }

    /// Same for the default `max_files: Some(10)`.
    #[tokio::test]
    async fn default_constraints_reject_the_11th_file() {
        let parts = file_parts(11);
        let mp = multipart_with(&as_refs(&parts), MultipartConstraints::default());

        let err = drain(mp).await.expect_err("11th file must be rejected");
        let msg = err.to_string();
        assert!(
            matches!(err, StorageError::Multipart(_)) && msg.contains("10"),
            "expected a multipart error naming the limit of 10, got: {msg}"
        );
    }

    #[tokio::test]
    async fn default_constraints_accept_exactly_10_files() {
        let parts = file_parts(10);
        let mp = multipart_with(&as_refs(&parts), MultipartConstraints::default());
        assert_eq!(drain(mp).await.unwrap(), 10);
    }

    /// `Multipart::new` installed `ConstraintCounters::default()` (every limit
    /// `None`) and a bare `multer::Multipart::new` with no constraints, so the
    /// ergonomic entry point enforced nothing at all.
    #[tokio::test]
    async fn new_applies_default_constraints() {
        let parts = file_parts(11);
        let body = build_body(&as_refs(&parts));
        let stream = stream::once(async move { Ok::<_, std::io::Error>(body) });

        let err = drain(Multipart::new(stream, BOUNDARY))
            .await
            .expect_err("Multipart::new must enforce the default file cap");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    #[tokio::test]
    async fn from_request_applies_default_constraints() {
        let parts = file_parts(11);
        let body = build_body(&as_refs(&parts));
        let stream = stream::once(async move { Ok::<_, std::io::Error>(body) });

        let mp =
            Multipart::from_request(&format!("multipart/form-data; boundary={BOUNDARY}"), stream)
                .unwrap();

        let err = drain(mp)
            .await
            .expect_err("from_request must enforce the default file cap");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    #[tokio::test]
    async fn parse_multipart_applies_default_constraints() {
        let parts = file_parts(11);
        let body = build_body(&as_refs(&parts));
        let stream = stream::once(async move { Ok::<_, std::io::Error>(body) });

        let header =
            http::HeaderValue::from_str(&format!("multipart/form-data; boundary={BOUNDARY}"))
                .unwrap();
        let mp = parse_multipart(&header, stream).unwrap();

        let err = drain(mp)
            .await
            .expect_err("parse_multipart must enforce the default file cap");
        assert!(matches!(err, StorageError::Multipart(_)));
    }

    /// The explicit escape hatches must genuinely opt out.
    #[tokio::test]
    async fn unconstrained_entry_points_opt_out_of_the_defaults() {
        let parts = file_parts(11);

        let body = build_body(&as_refs(&parts));
        let stream = stream::once(async move { Ok::<_, std::io::Error>(body) });
        assert_eq!(
            drain(Multipart::unconstrained(stream, BOUNDARY))
                .await
                .unwrap(),
            11
        );

        let body = build_body(&as_refs(&parts));
        let stream = stream::once(async move { Ok::<_, std::io::Error>(body) });
        let mp = Multipart::from_request_unconstrained(
            &format!("multipart/form-data; boundary={BOUNDARY}"),
            stream,
        )
        .unwrap();
        assert_eq!(drain(mp).await.unwrap(), 11);
    }

    /// `into_stream()` must carry the constraints across, not reset them.
    #[tokio::test]
    async fn into_stream_preserves_constraints() {
        let constraints = MultipartConstraints::unlimited().max_files(1);
        let mp = multipart_with(
            &[("f1", Some("a.txt"), "1"), ("f2", Some("b.txt"), "2")],
            constraints,
        );

        let mut stream = mp.into_stream();
        stream.next_field().await.unwrap().expect("first file");
        let err = stream
            .next_field()
            .await
            .expect_err("second file must be rejected by the carried-over limit");
        assert!(matches!(err, StorageError::Multipart(_)));
    }
}
