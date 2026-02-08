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
#[async_trait]
pub trait Storage: Send + Sync {
    /// Store bytes with a given key.
    async fn put(&self, key: &str, data: Bytes) -> Result<StorageMetadata>;

    /// Store bytes with a given key and content type.
    async fn put_with_content_type(
        &self,
        key: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<StorageMetadata>;

    /// Store an uploaded file.
    async fn put_file(&self, file: &UploadedFile) -> Result<StorageMetadata>;

    /// Retrieve file contents.
    async fn get(&self, key: &str) -> Result<Bytes>;

    /// Get file metadata without downloading.
    async fn head(&self, key: &str) -> Result<StorageMetadata>;

    /// Delete a file.
    async fn delete(&self, key: &str) -> Result<()>;

    /// Check if a file exists.
    async fn exists(&self, key: &str) -> Result<bool>;

    /// List files with optional prefix.
    async fn list(&self, prefix: Option<&str>) -> Result<Vec<StorageMetadata>>;

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
}

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
