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

    /// Resolve `key` to a filesystem path that is *physically* inside the
    /// storage root, rejecting it otherwise.
    ///
    /// Runs [`Self::full_path`]'s lexical validation and then two physical
    /// checks, because lexical validation alone cannot see symlinks:
    ///
    /// 1. Every component from the root down is `lstat`ed, and any component
    ///    that is itself a symlink is rejected. This catches links planted
    ///    inside the root (by an earlier upload, another tenant, or an
    ///    operator) that point anywhere at all.
    /// 2. The deepest *existing* ancestor of the resolved path is
    ///    canonicalized -- resolving links, `..`, and mount indirection the
    ///    kernel would follow -- and asserted to still start with the
    ///    canonicalized root.
    ///
    /// The path itself need not exist; for a write the deepest existing
    /// ancestor is the parent directory, which is exactly what must be checked
    /// before creating a file inside it.
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

        // (2) Canonicalize the deepest existing ancestor and re-check it
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

        if let Some(base_url) = &self.config.base_url {
            storage_metadata =
                storage_metadata.with_url(format!("{}/{}", base_url.trim_end_matches('/'), key));
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

        // Write file
        fs::write(&path, &data).await?;

        debug!(key = %key, path = ?path, size = data.len(), "Stored file");

        // Build metadata
        let mut metadata =
            StorageMetadata::new(key, data.len() as u64).with_content_type(content_type);

        if let Some(checksum) = checksum {
            metadata = metadata.with_checksum(checksum);
        }

        if let Some(base_url) = &self.config.base_url {
            metadata = metadata.with_url(format!("{}/{}", base_url.trim_end_matches('/'), key));
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
        // thread and leaves a TOCTOU gap. `read` already reports NotFound.
        let data = fs::read(&path).await.map_err(|e| map_not_found(e, key))?;
        Ok(Bytes::from(data))
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let path = self.resolve(key).await?;

        let metadata = fs::metadata(&path)
            .await
            .map_err(|e| map_not_found(e, key))?;

        Ok(self.build_metadata(key, &path, &metadata))
    }

    /// Delete a file, idempotently.
    ///
    /// Per the [`Storage::delete`] contract a missing key is `Ok(())`, matching
    /// S3, GCS and Azure. This backend used to be the odd one out, returning
    /// [`StorageError::NotFound`].
    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.resolve(key).await?;

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
        // as it does on S3/GCS/Azure -- not `InvalidFileName`.
        let normalized_prefix = match prefix {
            Some(p) => Self::normalize_relative(p)?,
            None => None,
        };
        let base = match &normalized_prefix {
            Some(p) => {
                let path = key_root.join(p);
                self.assert_inside_root(prefix.unwrap_or_default(), &path)
                    .await?;
                path
            }
            None => key_root.clone(),
        };

        // The local backend has no server-side cursor, so the cursor is an
        // offset into the walk, which is ordered deterministically by sorting
        // each directory's entries.
        let offset: usize = match cursor {
            Some(c) => c.parse().map_err(|_| {
                StorageError::Storage(format!("invalid list cursor {c:?} for local storage"))
            })?,
            None => 0,
        };

        let mut seen = 0usize;
        let mut results = Vec::new();
        // `put` happily creates nested directories from slash-bearing keys, so
        // listing has to recurse or those objects are silently invisible.
        let mut stack = vec![base];

        while let Some(dir) = stack.pop() {
            let mut entries = match fs::read_dir(&dir).await {
                Ok(entries) => entries,
                // A missing listing root is an empty listing, not an error.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };

            let mut children = Vec::new();
            while let Some(entry) = entries.next_entry().await? {
                // `entry.file_type()` does NOT follow symlinks, unlike
                // `entry.metadata()`. With `metadata()` a symlink to a
                // directory reported `is_dir()`, so `root/a/loop -> ../a` made
                // this walk run forever; and a symlink to a file outside the
                // root was listed with its absolute path as the key.
                let file_type = entry.file_type().await?;
                children.push((entry.path(), file_type));
            }
            // Deterministic order, so the offset cursor is stable across calls.
            children.sort_by(|(a, _), (b, _)| b.cmp(a));

            for (path, file_type) in children {
                if file_type.is_symlink() {
                    // Never traverse or report a link: it can point anywhere,
                    // including outside the root and back into this walk.
                    debug!(path = ?path, "Skipping symbolic link during list");
                    continue;
                }

                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }

                // A key that will not strip back to a root-relative path is not
                // ours to report; emitting the absolute path (the old
                // `unwrap_or(&path)`) leaked the server's filesystem layout
                // into `StorageMetadata::key` and, via `build_metadata`, into
                // the public `url`.
                let Ok(relative) = path.strip_prefix(&key_root) else {
                    debug!(path = ?path, "Skipping list entry outside the key root");
                    continue;
                };
                let key = relative
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");

                seen += 1;
                if seen <= offset {
                    continue;
                }

                let metadata = fs::metadata(&path).await?;
                results.push(self.build_metadata(&key, &path, &metadata));

                if results.len() == limit {
                    // Only report a continuation if something is actually left.
                    let next = offset + results.len();
                    return Ok((results, Some(next.to_string())));
                }
            }
        }

        Ok((results, None))
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_path = self.resolve(from).await?;
        let to_path = self.resolve(to).await?;

        // Create parent directories if needed
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent).await?;
            self.assert_inside_root(to, &to_path).await?;
        }

        fs::copy(&from_path, &to_path)
            .await
            .map_err(|e| map_not_found(e, from))?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        if let Some(base_url) = &self.config.base_url {
            Ok(Some(format!("{}/{}", base_url.trim_end_matches('/'), key)))
        } else {
            Ok(None)
        }
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

    /// A file reached through a symlink pointing *outside* the root failed
    /// `strip_prefix`, and `unwrap_or(&path)` then published the absolute
    /// filesystem path as the object key -- which `build_metadata` concatenates
    /// into the public `url`, leaking the server's layout to every `list`
    /// caller.
    #[cfg(unix)]
    #[tokio::test]
    async fn list_never_leaks_absolute_paths_from_outside_the_root() {
        let (_temp, storage, secret) = symlinked_storage().await;
        storage.put("inside.txt", Bytes::from("x")).await.unwrap();

        let listed = storage.list(None).await.unwrap();

        let keys: Vec<&str> = listed.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            ["inside.txt"],
            "the symlinked tree must not be listed"
        );
        for meta in &listed {
            assert!(
                !meta.key.starts_with('/'),
                "absolute path leaked as a key: {:?}",
                meta.key
            );
            assert!(
                !meta
                    .key
                    .contains(secret.parent().unwrap().to_str().unwrap()),
                "server layout leaked as a key: {:?}",
                meta.key
            );
        }
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
}
