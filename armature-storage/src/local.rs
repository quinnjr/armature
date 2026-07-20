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

    /// Get the full filesystem path for a key.
    ///
    /// Keys are untrusted input. A key is rejected with
    /// [`StorageError::InvalidFileName`] if it is absolute, carries a path
    /// prefix (Windows drive/UNC), or contains any `..` component -- all of
    /// which would otherwise escape (or entirely replace) the storage root
    /// when pushed onto it.
    fn full_path(&self, key: &str) -> Result<PathBuf> {
        let mut path = self.root.clone();
        if let Some(prefix) = &self.config.storage.path_prefix {
            path.push(Self::validate_relative(prefix)?);
        }
        path.push(Self::validate_relative(key)?);

        // Component validation already makes escape lexically impossible; this
        // is a cheap belt-and-braces assertion on the joined result.
        if !path.starts_with(&self.root) {
            return Err(StorageError::InvalidFileName(format!(
                "key {key:?} resolves outside the storage root"
            )));
        }

        Ok(path)
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
            return Err(StorageError::InvalidFileName(format!(
                "key {key:?} is empty after normalization"
            )));
        }

        Ok(normalized)
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

        let path = self.full_path(key)?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
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
        let path = self.full_path(key)?;

        // No `exists()` pre-check: it is a blocking `stat` on the reactor
        // thread and leaves a TOCTOU gap. `read` already reports NotFound.
        let data = fs::read(&path).await.map_err(|e| map_not_found(e, key))?;
        Ok(Bytes::from(data))
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let path = self.full_path(key)?;

        let metadata = fs::metadata(&path)
            .await
            .map_err(|e| map_not_found(e, key))?;

        Ok(self.build_metadata(key, &path, &metadata))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.full_path(key)?;

        fs::remove_file(&path)
            .await
            .map_err(|e| map_not_found(e, key))?;
        debug!(key = %key, "Deleted file");
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let path = self.full_path(key)?;
        Ok(fs::try_exists(&path).await.unwrap_or(false))
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<StorageMetadata>> {
        // Keys are relative to the root *plus* any configured path prefix, so
        // listing must start there and strip the same amount back off --
        // otherwise the returned keys don't round-trip through `get`.
        let key_root = self.key_root()?;
        let base = match prefix {
            Some(p) => self.full_path(p)?,
            None => key_root.clone(),
        };

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

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                // `entry.metadata()` is already one syscall; re-deriving it via
                // `head()` would cost three more per file.
                let metadata = entry.metadata().await?;

                if metadata.is_dir() {
                    stack.push(path);
                } else if metadata.is_file() {
                    let key = path
                        .strip_prefix(&key_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/");

                    results.push(self.build_metadata(&key, &path, &metadata));
                }
            }
        }

        Ok(results)
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_path = self.full_path(from)?;
        let to_path = self.full_path(to)?;

        // Create parent directories if needed
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent).await?;
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
            storage.delete("nope.txt").await,
            Err(StorageError::NotFound(_))
        ));
        assert!(matches!(
            storage.copy("nope.txt", "dst.txt").await,
            Err(StorageError::NotFound(_))
        ));
        assert!(!storage.exists("nope.txt").await.unwrap());
    }
}
