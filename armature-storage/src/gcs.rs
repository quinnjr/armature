//! Google Cloud Storage backend.
//!
//! Migrated to the new official `google-cloud-storage` 1.15 SDK. The data-plane
//! [`Storage`] client handles object reads/writes; the control-plane
//! [`StorageControl`] client handles metadata, listing, deletion and rewrite
//! (server-side copy). The control client is built lazily on first use so the
//! synchronous constructors (`from_client`, `from_gcp_services`) keep working.

use async_trait::async_trait;
use bytes::Bytes;
use google_cloud_storage::client::{Storage, StorageControl};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::{debug, info};

use crate::{
    Result, Storage as StorageTrait, StorageConfig, StorageError, StorageMetadata, UploadedFile,
    calculate_checksum, generate_unique_key,
};

/// Google Cloud Storage configuration.
#[derive(Debug, Clone)]
pub struct GcsConfig {
    /// GCS bucket name.
    pub bucket: String,
    /// GCP project ID (optional, uses default).
    pub project_id: Option<String>,
    /// Custom endpoint (for emulators).
    pub endpoint: Option<String>,
    /// Make uploaded objects publicly readable.
    pub public_access: bool,
    /// Common storage configuration.
    pub storage: StorageConfig,
    /// Signed URL duration.
    pub signed_url_duration: Duration,
}

impl Default for GcsConfig {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            project_id: None,
            endpoint: None,
            public_access: false,
            storage: StorageConfig::default(),
            signed_url_duration: Duration::from_secs(3600), // 1 hour
        }
    }
}

impl GcsConfig {
    /// Create configuration for a bucket.
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            ..Default::default()
        }
    }

    /// Set the project ID.
    pub fn project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    /// Set a custom endpoint (for emulators).
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Enable public access for uploaded objects.
    pub fn public_access(mut self, public: bool) -> Self {
        self.public_access = public;
        self
    }

    /// Set the path prefix.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.storage.path_prefix = Some(prefix.into());
        self
    }

    /// Set signed URL duration.
    pub fn signed_url_duration(mut self, duration: Duration) -> Self {
        self.signed_url_duration = duration;
        self
    }
}

/// Google Cloud Storage backend.
pub struct GcsStorage {
    /// Data-plane client (object uploads/downloads).
    client: Storage,
    /// Control-plane client (metadata/list/delete/rewrite), built lazily.
    control: OnceCell<StorageControl>,
    config: GcsConfig,
}

impl GcsStorage {
    /// Create a new GCS storage backend.
    ///
    /// Builds the data-plane [`Storage`] client using Application Default
    /// Credentials. The control-plane client is initialized lazily on first use.
    pub async fn new(config: GcsConfig) -> Result<Self> {
        let client = Storage::builder()
            .build()
            .await
            .map_err(|e| StorageError::Config(e.to_string()))?;

        info!(bucket = %config.bucket, "Initialized GCS storage");

        Ok(Self {
            client,
            control: OnceCell::new(),
            config,
        })
    }

    /// Create from an existing GCS data-plane client.
    pub fn from_client(client: Storage, config: GcsConfig) -> Self {
        Self {
            client,
            control: OnceCell::new(),
            config,
        }
    }

    /// Create from armature-gcp services.
    pub fn from_gcp_services(
        services: &Arc<armature_gcp::GcpServices>,
        config: GcsConfig,
    ) -> Result<Self> {
        let client = services
            .storage()
            .map_err(|e| StorageError::Config(e.to_string()))?;
        Ok(Self {
            client,
            control: OnceCell::new(),
            config,
        })
    }

    /// Get (lazily initializing) the control-plane client.
    async fn control(&self) -> Result<&StorageControl> {
        self.control
            .get_or_try_init(|| async {
                StorageControl::builder()
                    .build()
                    .await
                    .map_err(|e| StorageError::Config(e.to_string()))
            })
            .await
    }

    /// Bucket resource name in `projects/_/buckets/{bucket}` form, as required
    /// by the GCS v2 API surface.
    fn bucket_resource(&self) -> String {
        format!("projects/_/buckets/{}", self.config.bucket)
    }

    /// Get the full GCS object name for a path.
    fn full_key(&self, key: &str) -> String {
        if let Some(prefix) = &self.config.storage.path_prefix {
            format!("{}/{}", prefix.trim_end_matches('/'), key)
        } else {
            key.to_string()
        }
    }

    /// Generate a key for a file.
    fn generate_key(&self, original_name: Option<&str>) -> String {
        if self.config.storage.generate_unique_names {
            generate_unique_key(original_name, self.config.storage.preserve_extension)
        } else {
            original_name
                .map(String::from)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        }
    }

    /// Get the public URL for a key.
    pub fn public_url(&self, key: &str) -> String {
        let full_key = self.full_key(key);
        format!(
            "https://storage.googleapis.com/{}/{}",
            self.config.bucket, full_key
        )
    }
}

#[async_trait]
impl StorageTrait for GcsStorage {
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
        if let Some(max_size) = self.config.storage.max_file_size {
            if data.len() as u64 > max_size {
                return Err(StorageError::TooLarge {
                    size: data.len() as u64,
                    limit: max_size,
                });
            }
        }

        let full_key = self.full_key(key);
        let size = data.len() as u64;

        // Calculate checksum if enabled
        let checksum = if self.config.storage.calculate_checksum {
            Some(calculate_checksum(&data))
        } else {
            None
        };

        // Upload object (in-memory Bytes are seekable, so an unbuffered upload works).
        self.client
            .write_object(self.bucket_resource(), full_key.clone(), data)
            .set_content_type(content_type)
            .send_unbuffered()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        debug!(key = %key, bucket = %self.config.bucket, size = size, "Uploaded to GCS");

        // Build metadata
        let mut metadata = StorageMetadata::new(key, size)
            .with_content_type(content_type)
            .with_url(self.public_url(key));

        if let Some(checksum) = checksum {
            metadata = metadata.with_checksum(checksum);
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
        let full_key = self.full_key(key);

        let mut response = self
            .client
            .read_object(self.bucket_resource(), full_key.clone())
            .send()
            .await
            .map_err(|e| map_object_error(e, key))?;

        let mut contents = Vec::new();
        while let Some(chunk) = response.next().await.transpose().map_err(|e| {
            let err_str = e.to_string();
            if err_str.contains("404") || err_str.contains("not found") {
                StorageError::NotFound(key.to_string())
            } else {
                StorageError::Storage(err_str)
            }
        })? {
            contents.extend_from_slice(&chunk);
        }

        Ok(Bytes::from(contents))
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let full_key = self.full_key(key);

        let object = self
            .control()
            .await?
            .get_object()
            .set_bucket(self.bucket_resource())
            .set_object(full_key.clone())
            .send()
            .await
            .map_err(|e| map_object_error(e, key))?;

        let size = object.size as u64;
        let mut metadata = StorageMetadata::new(key, size).with_url(self.public_url(key));

        if !object.content_type.is_empty() {
            metadata = metadata.with_content_type(&object.content_type);
        }

        if let Some(checksums) = &object.checksums {
            if !checksums.md5_hash.is_empty() {
                metadata = metadata.with_checksum(hex::encode(&checksums.md5_hash));
            }
        }

        Ok(metadata)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let full_key = self.full_key(key);

        self.control()
            .await?
            .delete_object()
            .set_bucket(self.bucket_resource())
            .set_object(full_key.clone())
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        debug!(key = %key, bucket = %self.config.bucket, "Deleted from GCS");
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        match self.head(key).await {
            Ok(_) => Ok(true),
            Err(StorageError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn list(&self, prefix: Option<&str>) -> Result<Vec<StorageMetadata>> {
        use google_cloud_gax::paginator::ItemPaginator;

        let mut full_prefix = String::new();
        if let Some(p) = &self.config.storage.path_prefix {
            full_prefix.push_str(p);
            full_prefix.push('/');
        }
        if let Some(p) = prefix {
            full_prefix.push_str(p);
        }

        let mut items = self
            .control()
            .await?
            .list_objects()
            .set_parent(self.bucket_resource())
            .set_prefix(full_prefix)
            .by_item();

        let mut results = Vec::new();

        while let Some(object) = items
            .next()
            .await
            .transpose()
            .map_err(|e| StorageError::Storage(e.to_string()))?
        {
            // Remove prefix to get the relative key
            let relative_key = if let Some(p) = &self.config.storage.path_prefix {
                object
                    .name
                    .strip_prefix(&format!("{}/", p))
                    .unwrap_or(&object.name)
                    .to_string()
            } else {
                object.name.clone()
            };

            let size = object.size as u64;
            let mut metadata =
                StorageMetadata::new(&relative_key, size).with_url(self.public_url(&relative_key));

            if !object.content_type.is_empty() {
                metadata = metadata.with_content_type(&object.content_type);
            }

            if let Some(checksums) = &object.checksums {
                if !checksums.md5_hash.is_empty() {
                    metadata = metadata.with_checksum(hex::encode(&checksums.md5_hash));
                }
            }

            results.push(metadata);
        }

        Ok(results)
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_key = self.full_key(from);
        let to_key = self.full_key(to);

        // Server-side copy is performed via the rewrite operation in the new SDK.
        self.control()
            .await?
            .rewrite_object()
            .set_source_bucket(self.bucket_resource())
            .set_source_object(from_key)
            .set_destination_bucket(self.bucket_resource())
            .set_destination_name(to_key)
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(self.public_url(key)))
    }

    async fn temporary_url(&self, key: &str, expires_in: Duration) -> Result<Option<String>> {
        use google_cloud_auth::credentials::Builder as CredentialsBuilder;
        use google_cloud_storage::builder::storage::SignedUrlBuilder;

        let full_key = self.full_key(key);

        // The new SDK signs V4 URLs locally using a `Signer` derived from the
        // ambient credentials (Application Default Credentials).
        let signer = CredentialsBuilder::default()
            .build_signer()
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        let url = SignedUrlBuilder::for_object(self.bucket_resource(), full_key)
            .with_method(http::Method::GET)
            .with_expiration(expires_in)
            .sign_with(&signer)
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        Ok(Some(url))
    }
}

/// Map a GCS error to a [`StorageError`], translating not-found responses.
fn map_object_error(err: google_cloud_storage::Error, key: &str) -> StorageError {
    let err_str = err.to_string();
    if err_str.contains("404") || err_str.contains("not found") || err_str.contains("NOT_FOUND") {
        StorageError::NotFound(key.to_string())
    } else {
        StorageError::Storage(err_str)
    }
}
