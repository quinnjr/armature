//! Storage trait and common types.

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::{Result, UploadedFile};

/// Metadata about a stored file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageMetadata {
    /// Unique key/path of the file.
    pub key: String,
    /// Original file name.
    pub original_name: Option<String>,
    /// File size in bytes.
    pub size: u64,
    /// MIME type.
    pub content_type: Option<String>,
    /// SHA-256 hash of the file content.
    pub checksum: Option<String>,
    /// When the file was uploaded.
    pub uploaded_at: SystemTime,
    /// Storage-specific URL (if available).
    pub url: Option<String>,
    /// Additional metadata.
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

impl StorageMetadata {
    /// Create new metadata.
    pub fn new(key: impl Into<String>, size: u64) -> Self {
        Self {
            key: key.into(),
            original_name: None,
            size,
            content_type: None,
            checksum: None,
            uploaded_at: SystemTime::now(),
            url: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set the original file name.
    pub fn with_original_name(mut self, name: impl Into<String>) -> Self {
        self.original_name = Some(name.into());
        self
    }

    /// Set the content type.
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Set the checksum.
    pub fn with_checksum(mut self, checksum: impl Into<String>) -> Self {
        self.checksum = Some(checksum.into());
        self
    }

    /// Set the URL.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Add custom metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Common storage configuration.
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Maximum file size in bytes.
    pub max_file_size: Option<u64>,
    /// Generate unique file names.
    pub generate_unique_names: bool,
    /// Preserve file extensions.
    pub preserve_extension: bool,
    /// Calculate checksums.
    pub calculate_checksum: bool,
    /// Custom path prefix.
    pub path_prefix: Option<String>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            max_file_size: Some(100 * 1024 * 1024), // 100 MB
            generate_unique_names: true,
            preserve_extension: true,
            calculate_checksum: true,
            path_prefix: None,
        }
    }
}

/// Storage backend trait.
///
/// # Limitations
///
/// This trait is `Bytes`-in / `Bytes`-out end to end. There is no streaming,
/// no multipart or resumable upload, and no ranged download anywhere in this
/// crate -- neither in the trait nor in any backend that implements it. Note
/// that [`crate::Multipart`] is an HTTP `multipart/form-data` *request body*
/// parser and has nothing to do with S3-style multipart upload.
///
/// Two consequences follow, and they apply to every backend (S3, GCS, Azure,
/// and the local filesystem one):
///
/// * **Objects are fully materialized in memory in both directions.** Writing
///   an object requires the whole payload resident as a [`Bytes`] before the
///   call, and reading one allocates the whole object. Peak usage is
///   proportional to the largest single object, not to a bounded buffer, so
///   concurrent transfers multiply. Size the objects you route through this
///   trait against the process's memory budget, and cap uploads with
///   [`StorageConfig::max_file_size`] or [`crate::FileValidator::max_size`].
/// * **A single object cannot exceed the backend's single-request ceiling.**
///   Because every write is one request, the S3 backend inherits S3's
///   `PutObject` limit of **5 GiB** per object; larger objects are only
///   reachable through multipart upload, which is not implemented. GCS and
///   Azure have their own single-request ceilings that apply the same way.
///
/// If you need objects beyond those bounds, drive the vendor SDK directly
/// rather than routing them through `Storage`.
#[async_trait]
pub trait Storage: Send + Sync {
    /// Store bytes with a given key.
    ///
    /// # Limitations
    ///
    /// `data` is the entire object: it must already be resident in memory, and
    /// it is written in a single backend request. On S3 that request is
    /// `PutObject`, which rejects anything over **5 GiB**. See the
    /// [trait-level limitations](Storage#limitations).
    async fn put(&self, key: &str, data: Bytes) -> Result<StorageMetadata>;

    /// Store bytes with a given key and content type.
    ///
    /// # Limitations
    ///
    /// Same single-request, fully-buffered contract as [`Self::put`], including
    /// the 5 GiB S3 `PutObject` ceiling.
    async fn put_with_content_type(
        &self,
        key: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<StorageMetadata>;

    /// Store an uploaded file.
    ///
    /// # Limitations
    ///
    /// The file's contents are already buffered in [`UploadedFile`] and are
    /// written with the same single-request contract as [`Self::put`].
    async fn put_file(&self, file: &UploadedFile) -> Result<StorageMetadata>;

    /// Retrieve file contents.
    ///
    /// # Limitations
    ///
    /// The whole object is downloaded and allocated before this returns; there
    /// is no ranged or streaming read, so a caller that wants only a slice of a
    /// large object still pays for all of it in bandwidth and memory. See the
    /// [trait-level limitations](Storage#limitations).
    async fn get(&self, key: &str) -> Result<Bytes>;

    /// Get file metadata without downloading.
    async fn head(&self, key: &str) -> Result<StorageMetadata>;

    /// Delete a file.
    ///
    /// # Contract
    ///
    /// Deletion is **idempotent**: deleting a key that does not exist is
    /// `Ok(())`, not [`StorageError::NotFound`]. This is the conventional
    /// object-store semantic (it is what S3's `DeleteObject` does) and every
    /// backend in this crate -- including the local filesystem one -- conforms
    /// to it, so `Arc<dyn Storage>` callers get the same behaviour whichever
    /// backend is underneath.
    ///
    /// Errors are still reported for anything else: permission failures, I/O
    /// errors, and keys rejected by the backend's own validation.
    ///
    /// [`StorageError::NotFound`]: crate::StorageError::NotFound
    async fn delete(&self, key: &str) -> Result<()>;

    /// Check if a file exists.
    async fn exists(&self, key: &str) -> Result<bool>;

    /// List one page of objects.
    ///
    /// This is the bounded primitive; [`Self::list`] is a convenience wrapper
    /// over it. `cursor` is an opaque, backend-specific continuation token --
    /// pass `None` for the first page and then feed back whatever the previous
    /// call returned. A `None` in the returned tuple's second slot means the
    /// listing is complete.
    ///
    /// `limit` is an upper bound on the number of entries in the returned page;
    /// backends may return fewer (S3 and Azure additionally cap it at 1000 and
    /// 5000 respectively). A `limit` of `0` is treated as
    /// [`DEFAULT_LIST_PAGE_SIZE`].
    ///
    /// Prefer this over [`Self::list`] whenever `prefix` is derived from a
    /// request: a caller-controlled prefix over a large bucket is otherwise an
    /// unbounded allocation.
    ///
    /// # Ordering
    ///
    /// Entry order within and across pages is backend-specific: S3, GCS and
    /// Azure each return their native byte-lexicographic-by-key order, while
    /// the local filesystem backend walks the directory tree and can emit a
    /// different order for key shapes where a directory and a file share a
    /// lexical prefix (e.g. `a.txt` versus `a/b.txt`). Do not rely on a
    /// specific cross-backend order -- only on the pagination contract itself
    /// (every object reported exactly once across a full walk of pages),
    /// which every backend upholds.
    async fn list_page(
        &self,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<StorageMetadata>, Option<String>)>;

    /// List files with an optional prefix, draining every page.
    ///
    /// Convenience wrapper over [`Self::list_page`] for listings known to be
    /// small. It accumulates at most [`LIST_MAX_ITEMS`] entries and returns
    /// [`StorageError::Storage`] once that is exceeded rather than growing an
    /// unbounded `Vec`; use [`Self::list_page`] for anything that might be
    /// larger, or whose prefix comes from a request.
    ///
    /// [`StorageError::Storage`]: crate::StorageError::Storage
    async fn list(&self, prefix: Option<&str>) -> Result<Vec<StorageMetadata>> {
        let mut results: Vec<StorageMetadata> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let (page, next) = self
                .list_page(prefix, cursor.as_deref(), DEFAULT_LIST_PAGE_SIZE)
                .await?;
            // Checked *before* extending: the cap is on what this method will
            // allocate, so a page that would push it over must not be appended
            // first. With the default 1000-entry page size, checking afterwards
            // let the `Vec` reach 11_000 before erroring.
            if results.len() + page.len() > LIST_MAX_ITEMS {
                return Err(crate::StorageError::Storage(format!(
                    "list exceeded the {LIST_MAX_ITEMS}-object cap for prefix {prefix:?}; \
                     use Storage::list_page to page through the results instead"
                )));
            }

            results.extend(page);

            match next {
                Some(next) => cursor = Some(next),
                None => return Ok(results),
            }
        }
    }

    /// Copy a file to a new key.
    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata>;

    /// Move a file to a new key.
    async fn rename(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let metadata = self.copy(from, to).await?;
        self.delete(from).await?;
        Ok(metadata)
    }

    /// Get a URL for the file (if supported).
    async fn url(&self, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Get a temporary/signed URL (if supported).
    async fn temporary_url(
        &self,
        _key: &str,
        _expires_in: std::time::Duration,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// The backend's configured default lifetime for [`Self::temporary_url`].
    ///
    /// Backends that expose a configurable signed-URL duration override this
    /// so `Arc<dyn Storage>` holders can reach it; the default is one hour.
    fn default_url_duration(&self) -> std::time::Duration {
        DEFAULT_URL_DURATION
    }

    /// Get a temporary/signed URL using [`Self::default_url_duration`].
    async fn temporary_url_default(&self, key: &str) -> Result<Option<String>> {
        self.temporary_url(key, self.default_url_duration()).await
    }
}

/// Fallback lifetime for [`Storage::default_url_duration`]: one hour.
pub const DEFAULT_URL_DURATION: std::time::Duration = std::time::Duration::from_secs(3600);

/// Page size [`Storage::list`] requests from [`Storage::list_page`], and the
/// size substituted when `list_page` is called with a `limit` of `0`.
pub const DEFAULT_LIST_PAGE_SIZE: usize = 1000;

/// Ceiling on the number of entries [`Storage::list`] will accumulate before
/// giving up with an error.
///
/// `list` exists for small listings; a bucket larger than this must be walked
/// with [`Storage::list_page`] so the caller controls the memory.
pub const LIST_MAX_ITEMS: usize = 10_000;

/// Generate a unique file key.
pub fn generate_unique_key(original_name: Option<&str>, preserve_extension: bool) -> String {
    let id = uuid::Uuid::new_v4();

    if preserve_extension
        && let Some(name) = original_name
        && let Some(ext) = std::path::Path::new(name).extension()
    {
        return format!("{}.{}", id, ext.to_string_lossy());
    }

    id.to_string()
}

/// Calculate SHA-256 checksum of data.
pub fn calculate_checksum(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Sanitize a file name for safe storage.
pub fn sanitize_filename(name: &str) -> String {
    // Remove path components
    let name = std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());

    // Remove potentially dangerous characters
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim_start_matches('.')
        .to_string()
}

/// Percent-encode one path segment of an object key for inclusion in a URL.
///
/// Everything outside the RFC 3986 unreserved set is escaped, so a segment
/// containing `?`, `#`, `%`, or a CR/LF cannot terminate the path and inject a
/// query, a fragment, or a header break.
///
/// Used only by the cloud backends (the local backend has its own copy); dead
/// code without at least one of them enabled.
#[cfg(any(feature = "s3", feature = "gcs", feature = "azure"))]
pub(crate) fn encode_url_segment(segment: &str, out: &mut String) {
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
}

/// Validate and percent-encode a cloud object key for inclusion in a public
/// URL.
///
/// Cloud keys are flat strings, not filesystem paths -- there is no root to
/// walk out of -- but a `..` segment surviving unescaped into a URL can still
/// be collapsed by an intermediary (a reverse proxy or CDN normalizing dot
/// segments per RFC 3986 5.2.4) and end up addressing something outside the
/// intended prefix, and an unescaped `?`/`#`/CR/LF can inject a query, a
/// fragment, or a header break. This rejects the former outright and escapes
/// the rest, shared by the S3/GCS/Azure backends so their `url`s and
/// `public_url`s cannot disagree.
#[cfg(any(feature = "s3", feature = "gcs", feature = "azure"))]
pub(crate) fn encode_key_for_url(key: &str) -> Result<String> {
    if key.split('/').any(|segment| segment == "..") {
        return Err(crate::StorageError::InvalidFileName(format!(
            "key {key:?} must not contain path traversal components"
        )));
    }

    let mut encoded = String::new();
    for (i, segment) in key.split('/').enumerate() {
        if i > 0 {
            encoded.push('/');
        }
        encode_url_segment(segment, &mut encoded);
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A `Storage` that only implements `list_page`, handing back pages of
    /// synthetic metadata drawn from a fixed total. Everything else panics, so
    /// a test that strays off the listing path says so loudly.
    struct PagingStub {
        total: usize,
        page_size: usize,
        /// Every `limit` this stub was asked for, in order.
        limits: Mutex<Vec<usize>>,
    }

    impl PagingStub {
        fn new(total: usize, page_size: usize) -> Self {
            Self {
                total,
                page_size,
                limits: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> usize {
            self.limits.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl Storage for PagingStub {
        async fn put(&self, _key: &str, _data: Bytes) -> Result<StorageMetadata> {
            unimplemented!("the listing tests never write")
        }
        async fn put_with_content_type(
            &self,
            _key: &str,
            _data: Bytes,
            _content_type: &str,
        ) -> Result<StorageMetadata> {
            unimplemented!("the listing tests never write")
        }
        async fn put_file(&self, _file: &UploadedFile) -> Result<StorageMetadata> {
            unimplemented!("the listing tests never write")
        }
        async fn get(&self, _key: &str) -> Result<Bytes> {
            unimplemented!("the listing tests never read")
        }
        async fn head(&self, _key: &str) -> Result<StorageMetadata> {
            unimplemented!("the listing tests never stat")
        }
        async fn delete(&self, _key: &str) -> Result<()> {
            unimplemented!("the listing tests never delete")
        }
        async fn exists(&self, _key: &str) -> Result<bool> {
            unimplemented!("the listing tests never probe")
        }
        async fn copy(&self, _from: &str, _to: &str) -> Result<StorageMetadata> {
            unimplemented!("the listing tests never copy")
        }

        async fn list_page(
            &self,
            _prefix: Option<&str>,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<(Vec<StorageMetadata>, Option<String>)> {
            self.limits.lock().unwrap().push(limit);

            let offset: usize = cursor.map(|c| c.parse().unwrap()).unwrap_or(0);
            let end = (offset + self.page_size).min(self.total);
            let page = (offset..end)
                .map(|i| StorageMetadata::new(format!("k{i}"), 1))
                .collect::<Vec<_>>();

            let next = (end < self.total).then(|| end.to_string());
            Ok((page, next))
        }
    }

    #[tokio::test]
    async fn list_drains_every_page() {
        let stub = PagingStub::new(2500, 1000);

        let all = stub.list(None).await.expect("a small listing must succeed");

        assert_eq!(all.len(), 2500);
        assert_eq!(all[0].key, "k0");
        assert_eq!(all[2499].key, "k2499");
        assert_eq!(stub.calls(), 3);
        assert_eq!(
            stub.limits.lock().unwrap().as_slice(),
            [DEFAULT_LIST_PAGE_SIZE; 3],
            "list must request its documented page size"
        );
    }

    /// The documented contract: `list` accumulates *at most* `LIST_MAX_ITEMS`.
    /// Exactly that many is the last accepted listing.
    #[tokio::test]
    async fn list_accepts_exactly_the_cap() {
        let stub = PagingStub::new(LIST_MAX_ITEMS, 1000);

        let all = stub
            .list(None)
            .await
            .expect("a listing of exactly LIST_MAX_ITEMS must succeed");
        assert_eq!(all.len(), LIST_MAX_ITEMS);
    }

    /// One more than the cap is refused rather than accumulated. The cap is
    /// checked *before* the page is appended, so the `Vec` never grows past
    /// `LIST_MAX_ITEMS`; appending first let it reach 11_000 at the default
    /// 1000-entry page size before the check fired.
    #[tokio::test]
    async fn list_refuses_to_exceed_the_cap() {
        let stub = PagingStub::new(LIST_MAX_ITEMS + 1, 1000);

        match stub.list(None).await {
            Err(crate::StorageError::Storage(msg)) => {
                assert!(
                    msg.contains(&LIST_MAX_ITEMS.to_string()) && msg.contains("list_page"),
                    "the error must name the cap and the way out: {msg}"
                );
            }
            other => panic!("expected the cap to be enforced, got {other:?}"),
        }
    }

    /// A single oversized page is refused without being appended either.
    #[tokio::test]
    async fn list_refuses_an_oversized_first_page() {
        let stub = PagingStub::new(LIST_MAX_ITEMS * 2, LIST_MAX_ITEMS * 2);

        assert!(stub.list(None).await.is_err());
        assert_eq!(
            stub.calls(),
            1,
            "the cap must fire on the first page, not after another round trip"
        );
    }
}
