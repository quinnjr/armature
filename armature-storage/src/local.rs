//! Local filesystem storage backend.

use async_trait::async_trait;
use bytes::Bytes;
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use tracing::{debug, info};

use crate::{
    Result, Storage, StorageConfig, StorageError, StorageMetadata, UploadedFile,
    calculate_checksum, generate_unique_key, sanitize_filename,
};

/// Local filesystem storage configuration.
#[derive(Debug, Clone)]
pub struct LocalStorageConfig {
    /// Base directory for file storage.
    pub base_path: PathBuf,
    /// Create directories if they don't exist.
    pub create_directories: bool,
    /// Common storage configuration.
    pub storage: StorageConfig,
    /// Base URL for generating file URLs.
    pub base_url: Option<String>,
}

impl Default for LocalStorageConfig {
    fn default() -> Self {
        Self {
            base_path: PathBuf::from("./uploads"),
            create_directories: true,
            storage: StorageConfig::default(),
            base_url: None,
        }
    }
}

impl LocalStorageConfig {
    /// Create configuration with a base path.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
            ..Default::default()
        }
    }

    /// Set the base URL for file URLs.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Set the path prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.storage.path_prefix = Some(prefix.into());
        self
    }
}

/// Local filesystem storage backend.
#[derive(Clone)]
pub struct LocalStorage {
    config: LocalStorageConfig,
    /// Canonicalized `config.base_path`; every resolved key path must stay
    /// inside this directory.
    root: PathBuf,
}

impl LocalStorage {
    /// Create a new local storage backend.
    pub async fn new(config: LocalStorageConfig) -> Result<Self> {
        if config.create_directories {
            fs::create_dir_all(&config.base_path).await.map_err(|e| {
                StorageError::Storage(format!(
                    "Failed to create storage directory {:?}: {}",
                    config.base_path, e
                ))
            })?;
        }

        // Resolve the root once so key resolution never has to canonicalize on
        // the hot path. If the directory doesn't exist (create_directories =
        // false) fall back to the configured path as-is.
        let root = fs::canonicalize(&config.base_path)
            .await
            .unwrap_or_else(|_| config.base_path.clone());

        info!(path = ?root, "Initialized local storage");

        Ok(Self { config, root })
    }

    /// Create with just a base path (convenience method).
    pub async fn with_path(path: impl Into<PathBuf>) -> Result<Self> {
        Self::new(LocalStorageConfig::new(path)).await
    }

    /// The resolved storage root. Every key resolves to a path underneath it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lexically join `key` onto the storage root.
    ///
    /// Keys are untrusted input. A key is rejected with
    /// [`StorageError::InvalidFileName`] if it is absolute, carries a path
    /// prefix (Windows drive/UNC), or contains any `..` component -- all of
    /// which would otherwise escape (or entirely replace) the storage root
    /// when pushed onto it.
    ///
    /// **This is only half the check.** Component validation makes escape
    /// *lexically* impossible, but says nothing about the filesystem: a
    /// symlink already sitting inside the root (`root/reports -> /etc`) is
    /// spelled entirely with `Normal` components, so the key `reports/shadow`
    /// passes here and still reads `/etc/shadow`. Every [`Storage`] method
    /// therefore goes through [`Self::resolve`], which additionally resolves
    /// the path physically. Do not call `full_path` directly.
    fn full_path(&self, key: &str) -> Result<PathBuf> {
        let mut path = self.root.clone();
        if let Some(prefix) = &self.config.storage.path_prefix {
            path.push(Self::validate_relative(prefix)?);
        }
        path.push(Self::validate_relative(key)?);
        Ok(path)
    }

    /// Resolve `key` to a filesystem path that, *at the moment of the check*,
    /// is physically inside the storage root -- rejecting it otherwise.
    ///
    /// Runs [`Self::full_path`]'s lexical validation and then three physical
    /// checks, because lexical validation alone cannot see the filesystem:
    ///
    /// 1. Every component from the root down is `lstat`ed, and any component
    ///    that is itself a symlink is rejected. This catches links planted
    ///    inside the root (by an earlier upload, another tenant, or an
    ///    operator) that point anywhere at all.
    /// 2. On unix, a target that is a regular file with more than one link is
    ///    rejected. A hardlink is indistinguishable from the original by path
    ///    -- `lstat` reports a plain file and `canonicalize` returns the path
    ///    unchanged -- so without this the same actor who could plant a symlink
    ///    could plant a hardlink and defeat both other checks.
    /// 3. The deepest *existing* ancestor of the resolved path is
    ///    canonicalized -- resolving links, `..`, and mount indirection the
    ///    kernel would follow -- and asserted to still start with the
    ///    canonicalized root.
    ///
    /// The path itself need not exist; for a write the deepest existing
    /// ancestor is the parent directory, which is exactly what must be checked
    /// before creating a file inside it.
    ///
    /// # This is a check, not a guarantee
    ///
    /// The result describes the filesystem as it was when the check ran. It is
    /// **not** a capability: nothing binds the verified path to the file a
    /// later `open` reaches, and every path-based syscall re-traverses the name
    /// from the root. An attacker who can write inside the storage root can
    /// therefore swap a component between the check and the open.
    ///
    /// The leaf race -- the practical one, since it needs no directory
    /// scheduling luck -- is closed separately: every final open goes through
    /// [`open_read`]/[`open_write`], which pass `O_NOFOLLOW` on unix, so a
    /// symlink substituted at the last component fails the open outright
    /// rather than being followed. `delete` uses `unlink`, which never follows
    /// a final symlink either. And whatever the open does return is checked
    /// again on the descriptor itself -- not the pre-open path -- by
    /// `assert_opened_file_is_safe`, which rejects it unless it is a regular
    /// file with, on unix, exactly one link. That closes the leaf race for
    /// hardlinks and non-regular files (a FIFO or device node substituted at
    /// the leaf between the check and the open) the same way `O_NOFOLLOW`
    /// closes it for symlinks: the property is verified on the file the
    /// operation actually reads or writes, not on a path that may no longer
    /// describe it.
    ///
    /// What remains on unix is substitution of an *intermediate directory*
    /// component between the check and the open (`root/a/` swapped for a
    /// symlink while `root/a/b` is being written). Closing that needs
    /// `openat2(RESOLVE_BENEATH)` against a long-lived root descriptor, which
    /// this backend does not yet do. On non-unix targets `O_NOFOLLOW` is
    /// unavailable, so a symlink substituted at the leaf is still followed;
    /// `assert_opened_file_is_safe`'s regular-file check still runs there
    /// (`Metadata::is_file` needs no unix-specific API) and catches a FIFO or
    /// other special file, but not a hardlink, since link count has no
    /// portable equivalent in this code. `LocalStorage` is a
    /// dev/test/single-tenant backend; a root that hostile code can write to
    /// out of band is outside what it defends.
    async fn resolve(&self, key: &str) -> Result<PathBuf> {
        let path = self.full_path(key)?;
        self.assert_inside_root(key, &path).await?;
        Ok(path)
    }

    /// The physical half of [`Self::resolve`], split out so writes can re-run
    /// it after `create_dir_all` has materialized the parent directories.
    async fn assert_inside_root(&self, key: &str, path: &Path) -> Result<()> {
        let escaped = || {
            StorageError::InvalidFileName(format!("key {key:?} resolves outside the storage root"))
        };

        // `path` is built by pushing validated components onto `self.root`, so
        // this strip cannot fail -- but treat a failure as an escape rather
        // than unwrapping, so a future refactor cannot turn it into a bypass.
        let relative = path.strip_prefix(&self.root).map_err(|_| escaped())?;

        // (1) No component may be a symlink. `self.root` is already
        // canonicalized, so the walk starts from a link-free base.
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component);
            match fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(StorageError::InvalidFileName(format!(
                        "key {key:?} traverses the symbolic link {current:?}"
                    )));
                }
                Ok(_) => {}
                // Nothing exists from here down, so there is nothing left to
                // check -- and a `put` creating it is fine.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(StorageError::Io(e)),
            }
        }

        // (2) No hardlink. A second name for an inode outside the root is
        // spelled with `Normal` components, `lstat`s as a plain regular file
        // (so check 1 sees nothing), and canonicalizes to itself under the root
        // (so check 3 sees nothing) -- yet reading or writing it reaches the
        // same inode as the original. Planting one needs exactly the capability
        // the symlink defense already assumes an attacker has, so it is
        // rejected on the same grounds.
        #[cfg(unix)]
        if let Ok(metadata) = fs::symlink_metadata(path).await {
            use std::os::unix::fs::MetadataExt;
            if metadata.is_file() && metadata.nlink() > 1 {
                return Err(StorageError::InvalidFileName(format!(
                    "key {key:?} names a hard link ({} links), which may alias a file \
                     outside the storage root",
                    metadata.nlink()
                )));
            }
        }

        // (3) Canonicalize the deepest existing ancestor and re-check it
        // against the root. This is the belt-and-braces assertion the comment
        // here used to claim without doing: unlike the lexical `starts_with`
        // it replaced, it operates on a path the kernel actually resolved.
        let mut probe = path;
        loop {
            match fs::canonicalize(probe).await {
                Ok(resolved) => {
                    if !resolved.starts_with(&self.root) {
                        return Err(escaped());
                    }
                    return Ok(());
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => match probe.parent() {
                    Some(parent) => probe = parent,
                    // Ran out of ancestors: the root itself does not exist
                    // (`create_directories = false`), so there is nothing to
                    // escape into.
                    None => return Ok(()),
                },
                Err(e) => return Err(StorageError::Io(e)),
            }
        }
    }

    /// The directory that keys are resolved relative to: the storage root plus
    /// any configured [`StorageConfig::path_prefix`].
    fn key_root(&self) -> Result<PathBuf> {
        match &self.config.storage.path_prefix {
            Some(prefix) => Ok(self.root.join(Self::validate_relative(prefix)?)),
            None => Ok(self.root.clone()),
        }
    }

    /// Validate that `key` is a safe relative path and return it.
    fn validate_relative(key: &str) -> Result<PathBuf> {
        match Self::normalize_relative(key)? {
            Some(normalized) => Ok(normalized),
            None => Err(StorageError::InvalidFileName(format!(
                "key {key:?} is empty after normalization"
            ))),
        }
    }

    /// Normalize `key` to a safe relative path, or `None` if it normalizes to
    /// nothing at all (`""`, `"."`, `"./"`).
    ///
    /// Listing treats that as "no prefix", matching S3/GCS/Azure, where an
    /// empty prefix means "everything"; every other operation rejects it,
    /// since there is no object with an empty key.
    fn normalize_relative(key: &str) -> Result<Option<PathBuf>> {
        let candidate = Path::new(key);

        if candidate.is_absolute() {
            return Err(StorageError::InvalidFileName(format!(
                "key {key:?} must be relative, not absolute"
            )));
        }

        let mut normalized = PathBuf::new();
        for component in candidate.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                // `./a` is harmless; drop it.
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(StorageError::InvalidFileName(format!(
                        "key {key:?} must not contain path traversal components"
                    )));
                }
            }
        }

        if normalized.as_os_str().is_empty() {
            return Ok(None);
        }

        Ok(Some(normalized))
    }

    /// Generate a key for a file.
    fn generate_key(&self, original_name: Option<&str>) -> String {
        if self.config.storage.generate_unique_names {
            generate_unique_key(original_name, self.config.storage.preserve_extension)
        } else {
            original_name
                .map(sanitize_filename)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        }
    }

    /// The public URL for `key`, or `None` when no base URL is configured.
    ///
    /// `key` is validated (so it cannot climb out of the base path with `..`)
    /// and each of its segments is percent-encoded (so it cannot inject a
    /// query, a fragment, or a header break into the URL). Every URL this
    /// backend produces -- [`Storage::url`], and the `url` on the metadata
    /// returned by `put`/`head`/`list` -- goes through here, so they cannot
    /// disagree.
    fn object_url(&self, key: &str) -> Result<Option<String>> {
        let Some(base_url) = &self.config.base_url else {
            return Ok(None);
        };

        let relative = Self::validate_relative(key)?;

        let mut url = base_url.trim_end_matches('/').to_string();
        for component in relative.components() {
            url.push('/');
            encode_url_segment(&component.as_os_str().to_string_lossy(), &mut url);
        }

        Ok(Some(url))
    }

    /// Build [`StorageMetadata`] from filesystem metadata already in hand.
    fn build_metadata(
        &self,
        key: &str,
        path: &Path,
        metadata: &std::fs::Metadata,
    ) -> StorageMetadata {
        let mut storage_metadata = StorageMetadata::new(key, metadata.len());

        if let Ok(modified) = metadata.modified() {
            storage_metadata.uploaded_at = modified;
        }

        if let Some(mime) = mime_guess::from_path(path).first() {
            storage_metadata = storage_metadata.with_content_type(mime.to_string());
        }

        if let Ok(Some(url)) = self.object_url(key) {
            storage_metadata = storage_metadata.with_url(url);
        }

        storage_metadata
    }
}

/// Map a `NotFound` I/O error to [`StorageError::NotFound`], passing anything
/// else through unchanged.
fn map_not_found(err: std::io::Error, key: &str) -> StorageError {
    if err.kind() == std::io::ErrorKind::NotFound {
        StorageError::NotFound(key.to_string())
    } else {
        StorageError::Io(err)
    }
}

/// Map a failure from one of the `O_NOFOLLOW` opens below.
///
/// The kernel reports `ELOOP` when the final component turned out to be a
/// symbolic link. That is a containment failure -- something replaced the leaf
/// between [`LocalStorage::assert_inside_root`] and the open -- not an I/O
/// error, so it is reported the same way a link caught by the check would be.
fn map_open_error(err: std::io::Error, key: &str) -> StorageError {
    #[cfg(unix)]
    if err.raw_os_error() == Some(libc::ELOOP) {
        return StorageError::InvalidFileName(format!(
            "key {key:?} names a symbolic link at its final component"
        ));
    }
    map_not_found(err, key)
}

/// `OpenOptions` that refuse to follow a symlink at the final component.
///
/// `fs::read`/`fs::write`/`fs::copy` re-traverse the path by name and follow
/// links at every component, so the containment check
/// ([`LocalStorage::assert_inside_root`]) and the open see potentially
/// different filesystems. `O_NOFOLLOW` binds the leaf: whatever else changed,
/// the opened file is not something reached through a link planted where the
/// object should be. Intermediate directory components remain racy; see
/// [`LocalStorage::resolve`].
fn nofollow_options() -> fs::OpenOptions {
    #[allow(unused_mut)]
    let mut options = fs::OpenOptions::new();
    #[cfg(unix)]
    {
        // `tokio::fs::OpenOptions::custom_flags` is inherent on unix.
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
}

#[cfg(test)]
static OPEN_READ_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static OPEN_WRITE_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Open `path` for reading without following a final symlink.
async fn open_read(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(test)]
    OPEN_READ_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    nofollow_options().read(true).open(path).await
}

/// Open (or create) `path` for writing without following a final symlink.
async fn open_write(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(test)]
    OPEN_WRITE_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    nofollow_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await
}

/// Reject an opened file unless it is a regular file with, on unix, exactly
/// one link.
///
/// [`assert_inside_root`](LocalStorage::assert_inside_root) checks the
/// pre-open *path*; this checks the post-open file *descriptor*
/// (`tokio::fs::File::metadata` is `fstat`, not `lstat`/`stat` on the path),
/// so it catches what the path-based check structurally cannot: a FIFO or
/// device node at the leaf (which the pre-open check never inspected, since
/// it only ever rejected symlinks and multiply-linked regular files) and a
/// hardlink or non-regular file substituted at the leaf *after* the check but
/// *before* the open. A FIFO with no peer or a device node like `/dev/zero`
/// would otherwise hang or unboundedly grow `read_to_end`/the write call.
async fn assert_opened_file_is_safe(file: &fs::File, key: &str) -> Result<()> {
    let metadata = file.metadata().await?;

    if !metadata.is_file() {
        return Err(StorageError::InvalidFileName(format!(
            "key {key:?} names a non-regular file"
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(StorageError::InvalidFileName(format!(
                "key {key:?} names a hard link ({} links), which may alias a file \
                 outside the storage root",
                metadata.nlink()
            )));
        }
    }

    Ok(())
}

/// Percent-encode one path segment for inclusion in a URL.
///
/// Everything outside the RFC 3986 unreserved set is escaped, so a key
/// containing `?`, `#`, `%`, or a CR/LF cannot terminate the path and inject a
/// query, a fragment, or a header. The separator between segments is added by
/// the caller, so `/` inside a segment is escaped like anything else.
fn encode_url_segment(segment: &str, out: &mut String) {
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
}

#[async_trait]
impl Storage for LocalStorage {
    async fn put(&self, key: &str, data: Bytes) -> Result<StorageMetadata> {
        self.put_with_content_type(key, data, "application/octet-stream")
            .await
    }

    async fn put_with_content_type(
        &self,
        key: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<StorageMetadata> {
        // Check size limit
        if let Some(max_size) = self.config.storage.max_file_size
            && data.len() as u64 > max_size
        {
            return Err(StorageError::TooLarge {
                size: data.len() as u64,
                limit: max_size,
            });
        }

        let path = self.resolve(key).await?;

        // Create parent directories if needed, then re-run the physical check:
        // `create_dir_all` is the one step that changes what the path resolves
        // to, so the guard has to see the post-creation filesystem too.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
            self.assert_inside_root(key, &path).await?;
        }

        // Calculate checksum if enabled
        let checksum = if self.config.storage.calculate_checksum {
            Some(calculate_checksum(&data))
        } else {
            None
        };

        // Write through a handle opened with `O_NOFOLLOW` rather than
        // `fs::write`, which re-traverses the path by name and would follow a
        // symlink dropped at the leaf after the check above.
        {
            use tokio::io::AsyncWriteExt;
            let mut file = open_write(&path)
                .await
                .map_err(|e| map_open_error(e, key))?;
            assert_opened_file_is_safe(&file, key).await?;
            file.write_all(&data).await?;
            file.flush().await?;
        }

        debug!(key = %key, path = ?path, size = data.len(), "Stored file");

        // Build metadata
        let mut metadata =
            StorageMetadata::new(key, data.len() as u64).with_content_type(content_type);

        if let Some(checksum) = checksum {
            metadata = metadata.with_checksum(checksum);
        }

        if let Some(url) = self.object_url(key)? {
            metadata = metadata.with_url(url);
        }

        Ok(metadata)
    }

    async fn put_file(&self, file: &UploadedFile) -> Result<StorageMetadata> {
        let key = self.generate_key(file.name());
        let content_type = file
            .content_type_str()
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let mut metadata = self
            .put_with_content_type(&key, file.data.clone(), &content_type)
            .await?;

        if let Some(name) = file.name() {
            metadata = metadata.with_original_name(name);
        }

        Ok(metadata)
    }

    async fn get(&self, key: &str) -> Result<Bytes> {
        let path = self.resolve(key).await?;

        // No `exists()` pre-check: it is a blocking `stat` on the reactor
        // thread and leaves a TOCTOU gap. The open already reports NotFound.
        //
        // Read through a handle opened with `O_NOFOLLOW` rather than
        // `fs::read`, which re-traverses the path by name and would follow a
        // symlink dropped at the leaf after `resolve` ran.
        use tokio::io::AsyncReadExt;
        let mut file = open_read(&path).await.map_err(|e| map_open_error(e, key))?;
        assert_opened_file_is_safe(&file, key).await?;

        let capacity = file.metadata().await.map(|m| m.len() as usize).unwrap_or(0);
        let mut data = Vec::with_capacity(capacity);
        file.read_to_end(&mut data).await?;

        Ok(Bytes::from(data))
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let path = self.resolve(key).await?;

        // `symlink_metadata`, not `metadata`: a link substituted at the leaf
        // after `resolve` must not have its target's size and mtime reported as
        // the object's.
        let metadata = fs::symlink_metadata(&path)
            .await
            .map_err(|e| map_not_found(e, key))?;

        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidFileName(format!(
                "key {key:?} names a symbolic link at its final component"
            )));
        }

        Ok(self.build_metadata(key, &path, &metadata))
    }

    /// Delete a file, idempotently.
    ///
    /// Per the [`Storage::delete`] contract a missing key is `Ok(())`, matching
    /// S3, GCS and Azure.
    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.resolve(key).await?;

        // `unlink` never follows a final symlink, so the leaf race that
        // `O_NOFOLLOW` closes for `get`/`put` cannot turn this into a delete of
        // something outside the root: at worst it removes the planted link.
        match fs::remove_file(&path).await {
            Ok(()) => {
                debug!(key = %key, "Deleted file");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Io(e)),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let path = self.resolve(key).await?;

        // `unwrap_or(false)` here reported EACCES/ELOOP/EIO as "absent", so a
        // caller using `exists` as an overwrite guard would happily clobber a
        // file it could not even stat. Only a genuine absence is `false`.
        match fs::try_exists(&path).await {
            Ok(exists) => Ok(exists),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(StorageError::Io(e)),
        }
    }

    /// List one page of objects.
    ///
    /// # Prefix semantics
    ///
    /// `prefix` is a **byte prefix over the whole key**, exactly as it is on
    /// S3, GCS and Azure -- not a subdirectory to descend into. `Some("rep")`
    /// therefore matches `reports/a.txt`, and it does so on every backend, so
    /// code exercised against `LocalStorage` in development behaves the same
    /// way in production. A prefix that normalizes to nothing (`""`, `"."`)
    /// means "everything".
    ///
    /// # Cursor
    ///
    /// There is no server-side cursor here, so the continuation token is the
    /// **last key emitted**, and the next call resumes by skipping everything
    /// at or before it in the walk's (deterministic) order. It used to be a
    /// positional offset into a tree that is re-walked from scratch on every
    /// call, so a `put` or `delete` of a lexically earlier key between two
    /// pages shifted every later position and the pagination silently dropped
    /// or repeated objects.
    ///
    /// # Ordering
    ///
    /// Entries are emitted in the walk's pre-order DFS (component-wise)
    /// order, not byte-lexicographic order over the full key string -- the
    /// two disagree whenever a directory and a file share a lexical prefix
    /// (`a.txt` sorts before `a/b.txt` as bytes, but the walk emits
    /// `a/b.txt` first, since the directory `a` sorts before the file
    /// `a.txt`). `Storage::list_page` documents ordering as backend-specific
    /// for exactly this reason: this does not match the byte-lexicographic
    /// order S3, GCS and Azure return, and callers must not rely on the two
    /// agreeing.
    async fn list_page(
        &self,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<StorageMetadata>, Option<String>)> {
        let limit = if limit == 0 {
            crate::DEFAULT_LIST_PAGE_SIZE
        } else {
            limit
        };

        // Keys are relative to the root *plus* any configured path prefix, so
        // listing must start there and strip the same amount back off --
        // otherwise the returned keys don't round-trip through `get`.
        let key_root = self.key_root()?;

        // A prefix that normalizes to nothing (`""`, `"."`) means "everything",
        // as it does on S3/GCS/Azure -- not `InvalidFileName`. Anything else is
        // matched against the key as a string, so the walk always starts at the
        // key root; joining the prefix onto it and walking *that* directory was
        // the divergence from the cloud backends.
        let key_prefix = match prefix {
            Some(p) => Self::normalize_relative(p)?.map(|_| p),
            None => None,
        };

        // The cursor is the last key of the previous page. Compared as a
        // `Path`, i.e. component-wise, which is the order the walk emits in --
        // a plain string comparison would disagree (`a.txt` sorts before
        // `a/b.txt` as bytes, but the walk emits `a/b.txt` first, since the
        // directory `a` sorts before the file `a.txt`).
        let resume_after = cursor.map(PathBuf::from);

        let mut results: Vec<StorageMetadata> = Vec::new();

        // A pre-order DFS in ascending component order: children are pushed in
        // descending order so popping yields ascending. `put` happily creates
        // nested directories from slash-bearing keys, so listing has to recurse
        // or those objects are silently invisible.
        enum Pending {
            Dir(PathBuf),
            File(PathBuf),
        }
        let mut stack = vec![Pending::Dir(key_root.clone())];

        while let Some(pending) = stack.pop() {
            let path = match pending {
                Pending::Dir(dir) => {
                    let mut entries = match fs::read_dir(&dir).await {
                        Ok(entries) => entries,
                        // A missing listing root is an empty listing, not an
                        // error.
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(e) => return Err(e.into()),
                    };

                    let mut children = Vec::new();
                    while let Some(entry) = entries.next_entry().await? {
                        // `entry.file_type()` does NOT follow symlinks, unlike
                        // `entry.metadata()`. With `metadata()` a symlink to a
                        // directory reported `is_dir()`, so `root/a/loop ->
                        // ../a` made this walk run forever; and a symlink to a
                        // file outside the root was listed with its absolute
                        // path as the key.
                        let file_type = entry.file_type().await?;
                        if file_type.is_symlink() {
                            // Never traverse or report a link: it can point
                            // anywhere, including outside the root and back
                            // into this walk.
                            debug!(path = ?entry.path(), "Skipping symbolic link during list");
                            continue;
                        }
                        if file_type.is_dir() {
                            children.push(Pending::Dir(entry.path()));
                        } else if file_type.is_file() {
                            children.push(Pending::File(entry.path()));
                        }
                    }

                    children.sort_by(|a, b| {
                        let (Pending::Dir(a) | Pending::File(a)) = a;
                        let (Pending::Dir(b) | Pending::File(b)) = b;
                        b.cmp(a)
                    });
                    stack.extend(children);
                    continue;
                }
                Pending::File(path) => path,
            };

            // A key that will not strip back to a root-relative path is not
            // ours to report; emitting the absolute path (the old
            // `unwrap_or(&path)`) leaked the server's filesystem layout into
            // `StorageMetadata::key` and, via `build_metadata`, into the public
            // `url`.
            let Ok(relative) = path.strip_prefix(&key_root) else {
                debug!(path = ?path, "Skipping list entry outside the key root");
                continue;
            };

            // Resume point: everything at or before the previous page's last
            // key has already been reported.
            if let Some(resume_after) = &resume_after
                && relative <= resume_after.as_path()
            {
                continue;
            }

            let key = relative
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");

            if let Some(key_prefix) = key_prefix
                && !key.starts_with(key_prefix)
            {
                continue;
            }

            // The page is full *and* another entry exists, so a continuation is
            // genuinely warranted. Checking here rather than on `push` is what
            // stops a listing whose size is an exact multiple of `limit` from
            // handing back a cursor that only ever yields an empty final page.
            if results.len() == limit {
                let next = results
                    .last()
                    .expect("limit is non-zero, so a full page has a last entry")
                    .key
                    .clone();
                return Ok((results, Some(next)));
            }

            let metadata = fs::symlink_metadata(&path).await?;
            results.push(self.build_metadata(&key, &path, &metadata));
        }

        Ok((results, None))
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_path = self.resolve(from).await?;
        let to_path = self.resolve(to).await?;

        // Create parent directories if needed, then re-run the physical check:
        // `create_dir_all` is the one step that changes what the path resolves
        // to.
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent).await?;
            self.assert_inside_root(to, &to_path).await?;
        }

        // Both ends go through `O_NOFOLLOW` handles rather than `fs::copy`,
        // which re-traverses both paths by name and follows links at every
        // component.
        let mut source = open_read(&from_path)
            .await
            .map_err(|e| map_open_error(e, from))?;
        assert_opened_file_is_safe(&source, from).await?;
        let mut destination = open_write(&to_path)
            .await
            .map_err(|e| map_open_error(e, to))?;
        assert_opened_file_is_safe(&destination, to).await?;

        tokio::io::copy(&mut source, &mut destination).await?;
        {
            use tokio::io::AsyncWriteExt;
            destination.flush().await?;
        }

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        // The only `Storage` method that used to interpolate the key raw: a
        // key of `../../admin` walked out of the base path, and one containing
        // `?`, `#` or `%0d%0a` injected a query, a fragment or a header break.
        self.object_url(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_storage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();

        // Put
        let data = Bytes::from("Hello, World!");
        let metadata = storage.put("test.txt", data.clone()).await.unwrap();
        assert_eq!(metadata.key, "test.txt");
        assert_eq!(metadata.size, 13);

        // Get
        let retrieved = storage.get("test.txt").await.unwrap();
        assert_eq!(retrieved, data);

        // Exists
        assert!(storage.exists("test.txt").await.unwrap());
        assert!(!storage.exists("nonexistent.txt").await.unwrap());

        // Delete
        storage.delete("test.txt").await.unwrap();
        assert!(!storage.exists("test.txt").await.unwrap());
    }

    /// Keys that escape the storage root must be rejected outright. Before the
    /// fix `PathBuf::push` happily walked out of the base on `..` and
    /// *replaced* it entirely on an absolute key, so `put` was an arbitrary
    /// file write anywhere the process could reach.
    const TRAVERSAL_KEYS: &[&str] = &[
        "../escaped.txt",
        "../../etc/shadow",
        "/etc/shadow",
        "a/../../b.txt",
        "./../../b.txt",
    ];

    async fn traversal_storage() -> (tempfile::TempDir, LocalStorage) {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().join("root");
        let storage = LocalStorage::with_path(&root).await.unwrap();
        (temp_dir, storage)
    }

    #[tokio::test]
    async fn put_rejects_traversal_keys() {
        let (temp_dir, storage) = traversal_storage().await;
        let outside = temp_dir.path().join("escaped.txt");

        for key in TRAVERSAL_KEYS {
            let err = storage
                .put(key, Bytes::from("pwned"))
                .await
                .expect_err(&format!("put must reject traversal key {key:?}"));
            assert!(
                matches!(err, StorageError::InvalidFileName(_)),
                "expected InvalidFileName for {key:?}, got {err:?}"
            );
        }

        assert!(
            !outside.exists(),
            "traversal key wrote a file outside the storage root"
        );
    }

    #[tokio::test]
    async fn get_delete_head_exists_copy_reject_traversal_keys() {
        let (_temp_dir, storage) = traversal_storage().await;

        for key in TRAVERSAL_KEYS {
            assert!(
                matches!(
                    storage.get(key).await,
                    Err(StorageError::InvalidFileName(_))
                ),
                "get must reject {key:?}"
            );
            assert!(
                matches!(
                    storage.delete(key).await,
                    Err(StorageError::InvalidFileName(_))
                ),
                "delete must reject {key:?}"
            );
            assert!(
                matches!(
                    storage.head(key).await,
                    Err(StorageError::InvalidFileName(_))
                ),
                "head must reject {key:?}"
            );
            assert!(
                matches!(
                    storage.exists(key).await,
                    Err(StorageError::InvalidFileName(_))
                ),
                "exists must reject {key:?}"
            );
            assert!(
                matches!(
                    storage.copy("ok.txt", key).await,
                    Err(StorageError::InvalidFileName(_))
                ),
                "copy destination must reject {key:?}"
            );
            assert!(
                matches!(
                    storage.copy(key, "ok.txt").await,
                    Err(StorageError::InvalidFileName(_))
                ),
                "copy source must reject {key:?}"
            );
        }
    }

    #[tokio::test]
    async fn traversal_rejection_does_not_reject_legitimate_nested_keys() {
        let (_temp_dir, storage) = traversal_storage().await;

        storage
            .put("a/b/c.txt", Bytes::from("nested"))
            .await
            .expect("nested keys are legitimate");
        assert_eq!(storage.get("a/b/c.txt").await.unwrap(), "nested");

        // A leading `./` is harmless and normalizes to the same object.
        assert_eq!(storage.get("./a/b/c.txt").await.unwrap(), "nested");
    }

    /// `list` used a single non-recursive `read_dir`, so objects stored under
    /// slash-bearing keys -- which `put` creates as nested directories -- were
    /// silently invisible.
    #[tokio::test]
    async fn list_includes_nested_keys() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();

        storage.put("top.txt", Bytes::from("1")).await.unwrap();
        storage
            .put("nested/deep/file.txt", Bytes::from("22"))
            .await
            .unwrap();

        let mut keys: Vec<String> = storage
            .list(None)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.key)
            .collect();
        keys.sort();

        assert_eq!(keys, vec!["nested/deep/file.txt", "top.txt"]);
    }

    /// With a `path_prefix` configured, keys are relative to root+prefix. The
    /// keys `list` returns must round-trip back through `get`.
    #[tokio::test]
    async fn list_keys_round_trip_through_get_with_a_path_prefix() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage =
            LocalStorage::new(LocalStorageConfig::new(temp_dir.path()).with_prefix("tenant-a"))
                .await
                .unwrap();

        storage
            .put("nested/file.txt", Bytes::from("v"))
            .await
            .unwrap();

        let listed = storage.list(None).await.unwrap();
        let keys: Vec<&str> = listed.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["nested/file.txt"]);

        assert_eq!(storage.get(&listed[0].key).await.unwrap(), "v");
    }

    #[tokio::test]
    async fn list_of_missing_prefix_is_empty_not_an_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();

        assert!(storage.list(Some("no-such-dir")).await.unwrap().is_empty());
    }

    /// The pre-`exists()` checks were removed in favour of mapping the real
    /// operation's `NotFound`; the observable contract must be unchanged.
    #[tokio::test]
    async fn missing_keys_still_report_not_found() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();

        assert!(matches!(
            storage.get("nope.txt").await,
            Err(StorageError::NotFound(_))
        ));
        assert!(matches!(
            storage.head("nope.txt").await,
            Err(StorageError::NotFound(_))
        ));
        assert!(matches!(
            storage.copy("nope.txt", "dst.txt").await,
            Err(StorageError::NotFound(_))
        ));
        assert!(!storage.exists("nope.txt").await.unwrap());
    }

    /// `delete` used to be the only backend of the four that reported
    /// `NotFound` for an absent key; S3 returns `Ok(())`, so `is_not_found()`
    /// callers were correct on one backend and wrong on three. The trait now
    /// specifies idempotent deletion.
    #[tokio::test]
    async fn delete_is_idempotent_for_a_missing_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();

        storage
            .delete("never-existed.txt")
            .await
            .expect("deleting a missing key must be Ok(()), not NotFound");

        storage.put("gone.txt", Bytes::from("x")).await.unwrap();
        storage.delete("gone.txt").await.unwrap();
        storage
            .delete("gone.txt")
            .await
            .expect("a second delete must also be Ok(())");
    }

    // --- Symlink containment -------------------------------------------------
    //
    // `full_path` only ever validated key *components*, and the `starts_with`
    // check it called a "belt-and-braces assertion" compared a path built from
    // `self.root` against `self.root` -- unconditionally true, i.e. dead code.
    // `canonicalize` ran once, at construction, on the base path, and never on
    // a resolved key. So a symlink already inside the root escaped completely:
    // its key has only `Normal` components and `fs::read`/`fs::write` follow
    // the link.

    #[cfg(unix)]
    fn symlink(original: &Path, link: &Path) {
        std::os::unix::fs::symlink(original, link).expect("creating a test symlink should succeed");
    }

    /// Storage root containing `escape -> <outside dir>`, plus the path of a
    /// real file sitting in that outside directory.
    #[cfg(unix)]
    async fn symlinked_storage() -> (tempfile::TempDir, LocalStorage, PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, b"TOP SECRET").unwrap();

        let root = temp_dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        symlink(&outside, &root.join("escape"));

        let storage = LocalStorage::with_path(&root).await.unwrap();
        (temp_dir, storage, secret)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn get_through_a_symlink_inside_the_root_is_rejected() {
        let (_temp, storage, _secret) = symlinked_storage().await;

        let err = storage
            .get("escape/secret.txt")
            .await
            .expect_err("reading through a symlink out of the root is an arbitrary file read");
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName, got {err:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn put_through_a_symlink_inside_the_root_is_rejected() {
        let (_temp, storage, secret) = symlinked_storage().await;

        let err = storage
            .put("escape/secret.txt", Bytes::from("PWNED"))
            .await
            .expect_err("writing through a symlink out of the root is an arbitrary file write");
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName, got {err:?}"
        );
        assert_eq!(
            std::fs::read(&secret).unwrap(),
            b"TOP SECRET",
            "the file outside the root must be untouched"
        );

        // ...including a key that does not exist yet, which is the case that
        // has to survive `create_dir_all`.
        let err = storage
            .put("escape/new/dir/file.txt", Bytes::from("PWNED"))
            .await
            .expect_err("creating directories through a symlink must be rejected too");
        assert!(matches!(err, StorageError::InvalidFileName(_)));
        assert!(
            !secret.parent().unwrap().join("new").exists(),
            "no directory may be created outside the root"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn delete_through_a_symlink_inside_the_root_is_rejected() {
        let (_temp, storage, secret) = symlinked_storage().await;

        let err = storage
            .delete("escape/secret.txt")
            .await
            .expect_err("deleting through a symlink out of the root is an arbitrary unlink");
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName, got {err:?}"
        );
        assert!(secret.exists(), "the outside file must still be there");
    }

    /// The link itself is rejected even when its target is a plain file rather
    /// than a directory, and for `head`/`exists`/`copy` as well as the big three.
    #[cfg(unix)]
    #[tokio::test]
    async fn every_storage_method_rejects_a_symlinked_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let secret = temp_dir.path().join("secret.txt");
        std::fs::write(&secret, b"TOP SECRET").unwrap();

        let root = temp_dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        symlink(&secret, &root.join("link.txt"));

        let storage = LocalStorage::with_path(&root).await.unwrap();
        storage.put("real.txt", Bytes::from("ok")).await.unwrap();

        assert!(matches!(
            storage.head("link.txt").await,
            Err(StorageError::InvalidFileName(_))
        ));
        assert!(matches!(
            storage.exists("link.txt").await,
            Err(StorageError::InvalidFileName(_))
        ));
        assert!(matches!(
            storage.copy("link.txt", "dst.txt").await,
            Err(StorageError::InvalidFileName(_))
        ));
        assert!(matches!(
            storage.copy("real.txt", "link.txt").await,
            Err(StorageError::InvalidFileName(_))
        ));
        assert_eq!(std::fs::read(&secret).unwrap(), b"TOP SECRET");
    }

    /// `list` used `entry.metadata()`, which *follows* symlinks, so a link to
    /// an ancestor directory reported `is_dir()` and the stack -- which has no
    /// visited set -- recursed through it forever, growing `results` until the
    /// process died. Symlinks are now skipped outright, so the cycle is
    /// unreachable.
    #[cfg(unix)]
    #[tokio::test]
    async fn list_does_not_loop_on_a_directory_symlink_cycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/file.txt"), b"x").unwrap();
        symlink(Path::new("../a"), &root.join("a/loop"));

        let storage = LocalStorage::with_path(root).await.unwrap();

        let listed = tokio::time::timeout(std::time::Duration::from_secs(10), storage.list(None))
            .await
            .expect("list must terminate on a symlink cycle")
            .expect("list should succeed");

        let keys: Vec<&str> = listed.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["a/file.txt"]);
    }

    /// The walk must not descend into a symlink that leaves the root: a file
    /// reached that way is not an object of this store, and reporting one used
    /// to publish its absolute filesystem path as the key (`strip_prefix`
    /// failed and `unwrap_or(&path)` fell back to the full path), which
    /// `build_metadata` concatenates into the public `url`.
    ///
    /// The former name promised more than the body could check. Links are
    /// skipped at `read_dir` time, so nothing outside the key root ever reaches
    /// the `strip_prefix` -- the "absolute key" assertions this test used to
    /// carry were unreachable by construction, and only the key-set equality
    /// below was ever load-bearing.
    #[cfg(unix)]
    #[tokio::test]
    async fn list_does_not_descend_into_a_symlink_out_of_the_root() {
        let (_temp, storage, _secret) = symlinked_storage().await;
        storage.put("inside.txt", Bytes::from("x")).await.unwrap();

        let listed = storage.list(None).await.unwrap();

        let keys: Vec<&str> = listed.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["inside.txt"],
            "the symlinked tree must not be listed"
        );
    }

    // --- Hardlink containment ------------------------------------------------
    //
    // Both halves of the symlink guard are symlink-specific: `symlink_metadata`
    // reports a hardlink as a plain regular file, and `canonicalize("root/x")`
    // returns `root/x`, which starts with the root. So a second name for an
    // inode outside the root passed every check while `fs::read`/`fs::write`
    // reached the original file. Planting one needs exactly the capability the
    // symlink defense already assumes an attacker has.

    /// Storage root containing `alias.txt`, a hard link to a file outside it.
    #[cfg(unix)]
    async fn hardlinked_storage() -> (tempfile::TempDir, LocalStorage, PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, b"TOP SECRET").unwrap();

        let root = temp_dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::hard_link(&secret, root.join("alias.txt"))
            .expect("creating a test hard link should succeed");

        let storage = LocalStorage::with_path(&root).await.unwrap();
        (temp_dir, storage, secret)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn every_storage_method_rejects_a_hardlinked_key() {
        let (_temp, storage, secret) = hardlinked_storage().await;
        storage.put("real.txt", Bytes::from("ok")).await.unwrap();

        for result in [
            storage.get("alias.txt").await.err(),
            storage.put("alias.txt", Bytes::from("PWNED")).await.err(),
            storage.delete("alias.txt").await.err(),
            storage.head("alias.txt").await.err(),
            storage.exists("alias.txt").await.err(),
            storage.copy("alias.txt", "dst.txt").await.err(),
            storage.copy("real.txt", "alias.txt").await.err(),
        ] {
            let err = result.expect("a hard link out of the root must be rejected");
            assert!(
                matches!(err, StorageError::InvalidFileName(_)),
                "expected InvalidFileName, got {err:?}"
            );
        }

        assert_eq!(
            std::fs::read(&secret).unwrap(),
            b"TOP SECRET",
            "the aliased file outside the root must be untouched"
        );
    }

    /// The check is on *link count*, not on where the link points, so it must
    /// not fire for an ordinary single-linked object.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_singly_linked_file_is_still_readable() {
        let (_temp, storage, _secret) = hardlinked_storage().await;

        storage.put("plain.txt", Bytes::from("v")).await.unwrap();
        assert_eq!(storage.get("plain.txt").await.unwrap(), "v");
        assert!(storage.exists("plain.txt").await.unwrap());
        storage.head("plain.txt").await.unwrap();
    }

    // --- Leaf-race hardening -------------------------------------------------
    //
    // The containment check and the open are two separate path traversals;
    // nothing binds one to the other. `O_NOFOLLOW` on the final open closes the
    // leaf case: a symlink substituted where the object should be fails the
    // open outright instead of being followed. These drive the openers directly
    // because the race itself cannot be scheduled deterministically in a test.

    #[cfg(unix)]
    #[tokio::test]
    async fn the_final_open_refuses_to_follow_a_symlink() {
        let temp_dir = tempfile::tempdir().unwrap();
        let secret = temp_dir.path().join("secret.txt");
        std::fs::write(&secret, b"TOP SECRET").unwrap();
        let link = temp_dir.path().join("link.txt");
        symlink(&secret, &link);

        let read_err = open_read(&link)
            .await
            .expect_err("open_read must not follow a symlink at the leaf");
        assert_eq!(read_err.raw_os_error(), Some(libc::ELOOP));
        assert!(matches!(
            map_open_error(read_err, "link.txt"),
            StorageError::InvalidFileName(_)
        ));

        let write_err = open_write(&link)
            .await
            .expect_err("open_write must not follow a symlink at the leaf");
        assert_eq!(write_err.raw_os_error(), Some(libc::ELOOP));

        assert_eq!(
            std::fs::read(&secret).unwrap(),
            b"TOP SECRET",
            "the link target must not have been truncated or written"
        );
    }

    /// ...while a real file still opens both ways.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_final_open_still_works_on_a_real_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("real.txt");
        std::fs::write(&path, b"hello").unwrap();

        open_read(&path).await.expect("a real file must open");
        open_write(&path)
            .await
            .expect("a real file must open for writing");
    }

    // --- Post-open fd-level rejection (FIFO/device/hardlink at the leaf) -----
    //
    // The pre-open hardlink check in `assert_inside_root` is gated on
    // `is_file()`, so a FIFO or device node at the leaf skips it entirely, and
    // it runs on the path before the open rather than on the opened
    // descriptor, so a hardlink substituted at the leaf *after* the check but
    // *before* the open is not caught either. `assert_opened_file_is_safe`
    // closes both gaps by `fstat`ing the already-open file. These drive it
    // directly, as the leaf-race tests above drive `open_read`/`open_write`
    // directly, since the substitution race itself cannot be scheduled
    // deterministically in a test.

    #[cfg(unix)]
    #[tokio::test]
    async fn assert_opened_file_is_safe_rejects_a_hardlink() {
        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("target.txt");
        std::fs::write(&target, b"hello").unwrap();
        let link = temp_dir.path().join("link.txt");
        std::fs::hard_link(&target, &link).expect("creating a test hard link should succeed");

        let file = open_read(&link)
            .await
            .expect("opening a hard link itself must succeed; only its alias is the problem");
        let err = assert_opened_file_is_safe(&file, "link.txt")
            .await
            .expect_err("the fd-level check must reject a multiply-linked file");
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName, got {err:?}"
        );
    }

    /// The check is on link count, not on where the link points, so an
    /// ordinary singly-linked file must still be accepted.
    #[cfg(unix)]
    #[tokio::test]
    async fn assert_opened_file_is_safe_accepts_a_singly_linked_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("real.txt");
        std::fs::write(&path, b"hello").unwrap();

        let file = open_read(&path).await.unwrap();
        assert_opened_file_is_safe(&file, "real.txt")
            .await
            .expect("a singly-linked regular file must be accepted");
    }

    /// A FIFO at the leaf must be rejected rather than let a later
    /// `read_to_end`/write call hang. Opened `O_RDWR` rather than through
    /// `open_read` (`O_RDONLY`): a FIFO opened read-only blocks the open
    /// itself until a writer connects, which would hang this test rather than
    /// exercise the check -- `O_RDWR` is the standard way to open a FIFO
    /// without blocking, and this test only needs an open descriptor to drive
    /// `assert_opened_file_is_safe` against.
    #[cfg(unix)]
    #[tokio::test]
    async fn assert_opened_file_is_safe_rejects_a_fifo() {
        let temp_dir = tempfile::tempdir().unwrap();
        let fifo_path = temp_dir.path().join("fifo");
        let c_path = std::ffi::CString::new(fifo_path.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) },
            0,
            "creating a test FIFO should succeed"
        );

        let file = nofollow_options()
            .read(true)
            .write(true)
            .open(&fifo_path)
            .await
            .expect("opening a FIFO O_RDWR must not block");

        let err = assert_opened_file_is_safe(&file, "fifo")
            .await
            .expect_err("the fd-level check must reject a non-regular file");
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName, got {err:?}"
        );
    }

    // --- `Storage` methods actually route through the `O_NOFOLLOW` openers ---
    //
    // Every symlink test above either drives `open_read`/`open_write` directly
    // or plants the symlink before calling a `Storage` method, so the earlier
    // lexical check in `assert_inside_root` catches it first -- the open-time
    // `O_NOFOLLOW` defense is never actually exercised end-to-end through the
    // trait methods. A regression that swapped `open_write(&path)` back to
    // `fs::write(&path, data)` inside `put` (or the analogous swap in
    // `get`/`copy`) would pass every test above. These count calls to prove
    // the real methods actually reach the openers, independent of winning any
    // race.

    #[tokio::test]
    async fn put_and_get_route_through_the_nofollow_openers() {
        // The counters are process-global and every test in this file runs in
        // the same process, so a concurrently-running test's own put/get can
        // bump them between the before/after reads here too. That only ever
        // adds to the count, never removes from it, so "increased by at least
        // one" is the race-proof assertion; "increased by exactly one" is not.
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();

        let writes_before = OPEN_WRITE_CALLS.load(std::sync::atomic::Ordering::SeqCst);
        storage.put("k.txt", Bytes::from("x")).await.unwrap();
        assert!(
            OPEN_WRITE_CALLS.load(std::sync::atomic::Ordering::SeqCst) > writes_before,
            "Storage::put must go through open_write, not fs::write"
        );

        let reads_before = OPEN_READ_CALLS.load(std::sync::atomic::Ordering::SeqCst);
        storage.get("k.txt").await.unwrap();
        assert!(
            OPEN_READ_CALLS.load(std::sync::atomic::Ordering::SeqCst) > reads_before,
            "Storage::get must go through open_read, not fs::read"
        );
    }

    // --- URL generation ------------------------------------------------------

    async fn url_storage() -> (tempfile::TempDir, LocalStorage) {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(
            LocalStorageConfig::new(temp_dir.path())
                .with_base_url("https://cdn.example.com/files/"),
        )
        .await
        .unwrap();
        (temp_dir, storage)
    }

    /// `url` was the tenth `Storage` method and the only one that did not
    /// validate its key: it interpolated the key raw, so `../../admin` walked
    /// out of the configured base path.
    #[tokio::test]
    async fn url_rejects_traversal_keys() {
        let (_temp, storage) = url_storage().await;

        for key in TRAVERSAL_KEYS {
            let err = storage
                .url(key)
                .await
                .expect_err(&format!("url must reject traversal key {key:?}"));
            assert!(
                matches!(err, StorageError::InvalidFileName(_)),
                "expected InvalidFileName for {key:?}, got {err:?}"
            );
        }
    }

    /// ...and it percent-encodes, so a key cannot inject a query, a fragment,
    /// or a header break into the URL it is spliced into.
    #[tokio::test]
    async fn url_percent_encodes_each_segment() {
        let (_temp, storage) = url_storage().await;

        assert_eq!(
            storage.url("a/b.txt").await.unwrap().as_deref(),
            Some("https://cdn.example.com/files/a/b.txt"),
            "an ordinary key must keep its shape, separators included"
        );

        assert_eq!(
            storage.url("report?admin=1").await.unwrap().as_deref(),
            Some("https://cdn.example.com/files/report%3Fadmin%3D1")
        );
        assert_eq!(
            storage.url("report#frag").await.unwrap().as_deref(),
            Some("https://cdn.example.com/files/report%23frag")
        );
        assert_eq!(
            storage.url("a\r\nX-Injected: 1").await.unwrap().as_deref(),
            Some("https://cdn.example.com/files/a%0D%0AX-Injected%3A%201")
        );
        assert_eq!(
            storage.url("100%.txt").await.unwrap().as_deref(),
            Some("https://cdn.example.com/files/100%25.txt")
        );
    }

    /// Every URL this backend hands out comes from the same encoder, so
    /// `Storage::url` and the `url` carried on metadata cannot disagree.
    #[tokio::test]
    async fn metadata_urls_are_encoded_the_same_way_as_storage_url() {
        let (_temp, storage) = url_storage().await;

        let key = "odd name?.txt";
        let put = storage.put(key, Bytes::from("x")).await.unwrap();
        let head = storage.head(key).await.unwrap();
        let direct = storage.url(key).await.unwrap();

        assert_eq!(put.url, direct);
        assert_eq!(head.url, direct);
        assert_eq!(
            direct.as_deref(),
            Some("https://cdn.example.com/files/odd%20name%3F.txt")
        );
    }

    #[tokio::test]
    async fn url_is_none_without_a_configured_base_url() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        assert_eq!(storage.url("a.txt").await.unwrap(), None);
    }

    /// S3, GCS and Azure all treat an empty prefix as "everything"; local
    /// storage rejected it with `InvalidFileName`.
    #[tokio::test]
    async fn an_empty_prefix_lists_everything_like_the_cloud_backends() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        storage.put("a.txt", Bytes::from("1")).await.unwrap();
        storage.put("d/b.txt", Bytes::from("2")).await.unwrap();

        for prefix in ["", ".", "./"] {
            let mut keys: Vec<String> = storage
                .list(Some(prefix))
                .await
                .unwrap_or_else(|e| panic!("prefix {prefix:?} must list everything, got {e:?}"))
                .into_iter()
                .map(|m| m.key)
                .collect();
            keys.sort();
            assert_eq!(keys, vec!["a.txt", "d/b.txt"], "for prefix {prefix:?}");
        }
    }

    /// The behaviour the three cloud backends now match: a client-supplied
    /// multipart filename never becomes a path.
    ///
    /// Note this pins *unchanged* local behaviour -- `LocalStorage::generate_key`
    /// has always delegated to `sanitize_filename`. It is the reference the
    /// matching S3/GCS/Azure tests were written against, not a regression test
    /// for a local fix.
    #[tokio::test]
    async fn generate_key_sanitizes_a_path_bearing_filename() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config = LocalStorageConfig::new(temp_dir.path());
        config.storage.generate_unique_names = false;
        let storage = LocalStorage::new(config).await.unwrap();

        assert_eq!(storage.generate_key(Some("../../secrets/key")), "key");
        assert_eq!(storage.generate_key(Some("/etc/shadow")), "shadow");
        assert_eq!(storage.generate_key(Some("ok.txt")), "ok.txt");
    }

    /// `list_page` bounds the result set and hands back a cursor, so a caller
    /// with a request-derived prefix over a huge tree is not forced into one
    /// unbounded allocation.
    #[tokio::test]
    async fn list_page_pages_through_results_with_a_cursor() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        for i in 0..5 {
            storage
                .put(&format!("f{i}.txt"), Bytes::from("x"))
                .await
                .unwrap();
        }

        let mut keys = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let (page, next) = storage.list_page(None, cursor.as_deref(), 2).await.unwrap();
            pages += 1;
            assert!(page.len() <= 2, "a page must respect the limit");
            keys.extend(page.into_iter().map(|m| m.key));
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
            assert!(pages < 10, "pagination must terminate");
        }

        keys.sort();
        assert_eq!(keys, ["f0.txt", "f1.txt", "f2.txt", "f3.txt", "f4.txt"]);
        assert!(pages >= 3, "5 items at 2 per page must take several pages");
    }

    /// The cursor used to be a positional offset into a tree that is re-walked
    /// from scratch on every call. Storing an object with a lexically earlier
    /// key between two pages shifted every later position by one, so the second
    /// page re-reported an object the first page had already returned (and, for
    /// a delete, skipped one entirely). The cursor is now the last key emitted,
    /// which no concurrent mutation can shift.
    #[tokio::test]
    async fn a_write_between_pages_does_not_repeat_or_drop_objects() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        for key in ["f2.txt", "f4.txt", "f6.txt"] {
            storage.put(key, Bytes::from("x")).await.unwrap();
        }

        let (first, cursor) = storage.list_page(None, None, 2).await.unwrap();
        assert_eq!(
            first.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["f2.txt", "f4.txt"]
        );
        let cursor = cursor.expect("a third object is left, so a cursor is due");

        // A key that sorts before everything already returned.
        storage.put("f0.txt", Bytes::from("x")).await.unwrap();

        let (second, next) = storage.list_page(None, Some(&cursor), 2).await.unwrap();
        assert_eq!(
            second.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["f6.txt"],
            "the insert must not push an already-reported key onto the next page"
        );
        assert_eq!(next, None);
    }

    /// The same shift in the other direction: deleting a lexically earlier key
    /// between pages used to make the offset skip an object outright.
    #[tokio::test]
    async fn a_delete_between_pages_does_not_drop_objects() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        for key in ["f0.txt", "f2.txt", "f4.txt", "f6.txt"] {
            storage.put(key, Bytes::from("x")).await.unwrap();
        }

        let (first, cursor) = storage.list_page(None, None, 2).await.unwrap();
        assert_eq!(
            first.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["f0.txt", "f2.txt"]
        );
        let cursor = cursor.unwrap();

        storage.delete("f0.txt").await.unwrap();

        let (second, _) = storage.list_page(None, Some(&cursor), 2).await.unwrap();
        assert_eq!(
            second.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["f4.txt", "f6.txt"],
            "the delete must not cause an unreported key to be skipped"
        );
    }

    /// A continuation was returned whenever the page merely filled, so a
    /// listing whose size is an exact multiple of the limit always cost one
    /// extra full-tree walk that returned nothing. The cursor is now only
    /// handed back when another entry actually exists.
    #[tokio::test]
    async fn an_exactly_full_page_does_not_promise_a_nonexistent_next_one() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        for key in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            storage.put(key, Bytes::from("x")).await.unwrap();
        }

        let (first, cursor) = storage.list_page(None, None, 2).await.unwrap();
        assert_eq!(first.len(), 2);
        let (second, next) = storage.list_page(None, cursor.as_deref(), 2).await.unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(
            next, None,
            "the listing is exhausted; promising another page costs a whole extra walk"
        );
    }

    /// `prefix` is a byte prefix over the whole key on S3/GCS/Azure. Local
    /// storage joined it onto the key root and walked *that directory*, so
    /// `list_page(Some("rep"), ..)` returned `["reports/a.txt"]` in production
    /// and `[]` against the documented dev/test backend.
    #[tokio::test]
    async fn a_partial_prefix_matches_keys_the_way_the_cloud_backends_do() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        storage
            .put("reports/a.txt", Bytes::from("1"))
            .await
            .unwrap();
        storage.put("readme.txt", Bytes::from("2")).await.unwrap();
        storage.put("other.txt", Bytes::from("3")).await.unwrap();

        let (page, _) = storage.list_page(Some("rep"), None, 100).await.unwrap();
        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["reports/a.txt"],
            "a partial prefix must match by key, not by directory"
        );

        // A prefix spanning a separator works too, which a directory walk
        // cannot express at all.
        let (page, _) = storage
            .list_page(Some("reports/a"), None, 100)
            .await
            .unwrap();
        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["reports/a.txt"]
        );

        // And a directory-shaped prefix still behaves as before.
        let (page, _) = storage.list_page(Some("reports"), None, 100).await.unwrap();
        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["reports/a.txt"]
        );
    }

    /// The walk emits in component order, and the cursor is compared the same
    /// way. A plain string comparison would disagree: `a.txt` sorts before
    /// `a/b.txt` as bytes (`.` < `/`) but is emitted *after* it, because the
    /// directory `a` sorts before the file `a.txt`.
    #[tokio::test]
    async fn pagination_is_consistent_where_string_and_path_order_disagree() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::with_path(temp_dir.path()).await.unwrap();
        storage.put("a/b.txt", Bytes::from("1")).await.unwrap();
        storage.put("a.txt", Bytes::from("2")).await.unwrap();

        let mut seen = Vec::new();
        let mut cursor = None;
        for _ in 0..5 {
            let (page, next) = storage.list_page(None, cursor.as_deref(), 1).await.unwrap();
            seen.extend(page.into_iter().map(|m| m.key));
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }

        assert_eq!(
            seen,
            ["a/b.txt", "a.txt"],
            "every object must be reported exactly once"
        );
    }
}
