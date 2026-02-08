//! Local filesystem storage backend.

use async_trait::async_trait;
use bytes::Bytes;
use std::path::PathBuf;
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

        info!(path = ?config.base_path, "Initialized local storage");

        Ok(Self { config })
    }

    /// Create with just a base path (convenience method).
    pub async fn with_path(path: impl Into<PathBuf>) -> Result<Self> {
        Self::new(LocalStorageConfig::new(path)).await
    }

    /// Get the full filesystem path for a key.
    fn full_path(&self, key: &str) -> PathBuf {
        let mut path = self.config.base_path.clone();
        if let Some(prefix) = &self.config.storage.path_prefix {
            path.push(prefix);
        }
        path.push(key);
        path
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

        let path = self.full_path(key);

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
        let path = self.full_path(key);

        if !path.exists() {
            return Err(StorageError::NotFound(key.to_string()));
        }

        let data = fs::read(&path).await?;
        Ok(Bytes::from(data))
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let path = self.full_path(key);

        if !path.exists() {
            return Err(StorageError::NotFound(key.to_string()));
        }

        let metadata = fs::metadata(&path).await?;
        let mut storage_metadata = StorageMetadata::new(key, metadata.len());

        // Try to get modified time
        if let Ok(modified) = metadata.modified() {
            storage_metadata.uploaded_at = modified;
        }

        // Guess content type from extension
        if let Some(mime) = mime_guess::from_path(&path).first() {
            storage_metadata = storage_metadata.with_content_type(mime.to_string());
        }

        // Add URL if base_url is configured
        if let Some(base_url) = &self.config.base_url {
            storage_metadata =
                storage_metadata.with_url(format!("{}/{}", base_url.trim_end_matches('/'), key));
        }

        Ok(storage_metadata)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.full_path(key);

        if !path.exists() {
            return Err(StorageError::NotFound(key.to_string()));
        }

        fs::remove_file(&path).await?;
        debug!(key = %key, "Deleted file");
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let path = self.full_path(key);
        Ok(path.exists())
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<StorageMetadata>> {
        let base = if let Some(p) = prefix {
            self.full_path(p)
        } else {
            self.config.base_path.clone()
        };

        if !base.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut entries = fs::read_dir(&base).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                let key = path
                    .strip_prefix(&self.config.base_path)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                if let Ok(metadata) = self.head(&key).await {
                    results.push(metadata);
                }
            }
        }

        Ok(results)
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_path = self.full_path(from);
        let to_path = self.full_path(to);

        if !from_path.exists() {
            return Err(StorageError::NotFound(from.to_string()));
        }

        // Create parent directories if needed
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::copy(&from_path, &to_path).await?;

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
}
