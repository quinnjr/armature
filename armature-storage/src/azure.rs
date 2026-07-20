//! Azure Blob Storage backend.
//!
//! Migrated to the new Azure SDK (`azure_storage_blob` 1.0 on `azure_core` 1.0 +
//! `azure_identity` 1.0). The new SDK is endpoint-URL + AAD (Entra ID) token
//! credential based; connection-string and shared-key (account key) auth modes
//! are no longer supported by the SDK.

use async_trait::async_trait;
use azure_core::credentials::TokenCredential;
use azure_core::http::{NoFormat, RequestContent, Url};
use azure_storage_blob::models::{
    BlobClientGetPropertiesResultHeaders, BlobClientUploadOptions,
    BlobContainerClientListBlobsOptions,
};
use azure_storage_blob::{BlobClient, BlobContainerClient, BlobServiceClient};
use base64::Engine;
use bytes::Bytes;
use futures::TryStreamExt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

use crate::{
    Result, Storage, StorageConfig, StorageError, StorageMetadata, UploadedFile,
    calculate_checksum, generate_unique_key,
};

/// Azure Blob Storage configuration.
#[derive(Debug, Clone)]
pub struct AzureBlobConfig {
    /// Storage account name.
    pub account: String,
    /// Container name.
    pub container: String,
    /// Access key (if not using default credentials).
    pub access_key: Option<String>,
    /// Connection string (alternative to account/key).
    pub connection_string: Option<String>,
    /// Custom endpoint (for Azurite emulator).
    pub endpoint: Option<String>,
    /// Use Azurite emulator.
    pub use_emulator: bool,
    /// Common storage configuration.
    pub storage: StorageConfig,
    /// SAS token duration.
    pub sas_duration: Duration,
}

impl Default for AzureBlobConfig {
    fn default() -> Self {
        Self {
            account: String::new(),
            container: String::new(),
            access_key: None,
            connection_string: None,
            endpoint: None,
            use_emulator: false,
            storage: StorageConfig::default(),
            sas_duration: Duration::from_secs(3600), // 1 hour
        }
    }
}

impl AzureBlobConfig {
    /// Create configuration for a container.
    pub fn new(account: impl Into<String>, container: impl Into<String>) -> Self {
        Self {
            account: account.into(),
            container: container.into(),
            ..Default::default()
        }
    }

    /// Set the access key.
    pub fn access_key(mut self, key: impl Into<String>) -> Self {
        self.access_key = Some(key.into());
        self
    }

    /// Set the connection string.
    pub fn connection_string(mut self, conn_str: impl Into<String>) -> Self {
        self.connection_string = Some(conn_str.into());
        self
    }

    /// Set a custom endpoint.
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Use Azurite emulator.
    pub fn emulator(mut self) -> Self {
        self.use_emulator = true;
        self
    }

    /// Set the path prefix.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.storage.path_prefix = Some(prefix.into());
        self
    }

    /// Set SAS token duration.
    pub fn sas_duration(mut self, duration: Duration) -> Self {
        self.sas_duration = duration;
        self
    }
}

/// Azure Blob Storage backend.
pub struct AzureBlobStorage {
    container_client: BlobContainerClient,
    config: AzureBlobConfig,
}

impl AzureBlobStorage {
    /// Create a new Azure Blob storage backend.
    ///
    /// The new Azure SDK authenticates with AAD (Entra ID) token credentials.
    /// Connection-string and shared-key (account key) authentication are no
    /// longer supported by the SDK and will return a configuration error.
    pub async fn new(config: AzureBlobConfig) -> Result<Self> {
        let blob_service = if config.use_emulator {
            // Azurite emulator: HTTP endpoint, unauthenticated pipeline.
            let endpoint = config
                .endpoint
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:10000/devstoreaccount1".to_string());
            let service_url =
                Url::parse(&endpoint).map_err(|e| StorageError::Config(e.to_string()))?;
            BlobServiceClient::new(service_url, None, None)
                .map_err(|e| StorageError::Config(e.to_string()))?
        } else if config.connection_string.is_some() {
            return Err(StorageError::Config(
                "azure_storage_blob 1.0 requires AAD (Entra ID) token credentials; \
                 connection-string authentication is no longer supported by the new \
                 Azure SDK. Use default Azure credentials instead."
                    .to_string(),
            ));
        } else if config.access_key.is_some() {
            return Err(StorageError::Config(
                "azure_storage_blob 1.0 requires AAD (Entra ID) token credentials; \
                 shared-key (storage account key) authentication is no longer supported \
                 by the new Azure SDK. Use default Azure credentials instead."
                    .to_string(),
            ));
        } else {
            let endpoint = config
                .endpoint
                .clone()
                .unwrap_or_else(|| format!("https://{}.blob.core.windows.net/", config.account));
            let service_url =
                Url::parse(&endpoint).map_err(|e| StorageError::Config(e.to_string()))?;

            // Default credential chain (Azure CLI, environment, managed identity, ...).
            let credential: Arc<dyn TokenCredential> =
                azure_identity::DeveloperToolsCredential::new(None)
                    .map_err(|e| StorageError::Config(e.to_string()))?;

            BlobServiceClient::new(service_url, Some(credential), None)
                .map_err(|e| StorageError::Config(e.to_string()))?
        };

        let container_client = blob_service.blob_container_client(&config.container);

        info!(
            account = %config.account,
            container = %config.container,
            "Initialized Azure Blob storage"
        );

        Ok(Self {
            container_client,
            config,
        })
    }

    /// Create from armature-azure services.
    pub fn from_azure_services(
        services: &Arc<armature_azure::AzureServices>,
        config: AzureBlobConfig,
    ) -> Result<Self> {
        let blob_service = services
            .blob_service()
            .map_err(|e| StorageError::Config(e.to_string()))?;
        let container_client = blob_service.blob_container_client(&config.container);

        Ok(Self {
            container_client,
            config,
        })
    }

    /// Get the full blob name for a path.
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
        if self.config.use_emulator {
            format!(
                "http://127.0.0.1:10000/devstoreaccount1/{}/{}",
                self.config.container, full_key
            )
        } else {
            format!(
                "https://{}.blob.core.windows.net/{}/{}",
                self.config.account, self.config.container, full_key
            )
        }
    }

    /// Get a blob client for a key.
    fn blob_client(&self, key: &str) -> BlobClient {
        let full_key = self.full_key(key);
        self.container_client.blob_client(&full_key)
    }
}

/// Translate an Azure error into a [`StorageError`], mapping not-found responses.
fn map_blob_error(err: azure_core::Error, key: &str) -> StorageError {
    let err_str = err.to_string();
    if err_str.contains("BlobNotFound") || err_str.contains("404") {
        StorageError::NotFound(key.to_string())
    } else {
        StorageError::Storage(err_str)
    }
}

#[async_trait]
impl Storage for AzureBlobStorage {
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

        let size = data.len() as u64;

        // Calculate checksum if enabled
        let checksum = if self.config.storage.calculate_checksum {
            Some(calculate_checksum(&data))
        } else {
            None
        };

        // Upload blob (overwrites by default).
        let blob_client = self.blob_client(key);
        let options = BlobClientUploadOptions {
            blob_content_type: Some(content_type.to_string()),
            ..Default::default()
        };

        blob_client
            .upload(
                <RequestContent<Bytes, NoFormat> as From<Bytes>>::from(data),
                Some(options),
            )
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        debug!(
            key = %key,
            container = %self.config.container,
            size = size,
            "Uploaded to Azure Blob"
        );

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
        let blob_client = self.blob_client(key);

        let response = blob_client
            .download(None)
            .await
            .map_err(|e| map_blob_error(e, key))?;

        let data = response
            .body
            .collect()
            .await
            .map_err(|e| map_blob_error(e, key))?;

        Ok(data)
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let blob_client = self.blob_client(key);

        let properties = blob_client
            .get_properties(None)
            .await
            .map_err(|e| map_blob_error(e, key))?;

        let size = properties
            .content_length()
            .map_err(|e| StorageError::Storage(e.to_string()))?
            .unwrap_or(0);
        let mut metadata = StorageMetadata::new(key, size).with_url(self.public_url(key));

        if let Some(ct) = properties
            .content_type()
            .map_err(|e| StorageError::Storage(e.to_string()))?
        {
            metadata = metadata.with_content_type(&ct);
        }

        if let Some(md5) = properties
            .content_md5()
            .map_err(|e| StorageError::Storage(e.to_string()))?
        {
            metadata = metadata
                .with_checksum(base64::engine::general_purpose::STANDARD.encode(md5.as_slice()));
        }

        Ok(metadata)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let blob_client = self.blob_client(key);

        blob_client
            .delete(None)
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        debug!(
            key = %key,
            container = %self.config.container,
            "Deleted from Azure Blob"
        );
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
        let mut full_prefix = String::new();
        if let Some(p) = &self.config.storage.path_prefix {
            full_prefix.push_str(p);
            full_prefix.push('/');
        }
        if let Some(p) = prefix {
            full_prefix.push_str(p);
        }

        let options = BlobContainerClientListBlobsOptions {
            prefix: Some(full_prefix),
            ..Default::default()
        };

        let mut results = Vec::new();
        let mut blobs = self
            .container_client
            .list_blobs(Some(options))
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        while let Some(blob) = blobs
            .try_next()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?
        {
            let name = blob.name.unwrap_or_default();

            // Remove prefix to get the relative key
            let relative_key = if let Some(p) = &self.config.storage.path_prefix {
                name.strip_prefix(&format!("{}/", p))
                    .unwrap_or(&name)
                    .to_string()
            } else {
                name.clone()
            };

            let properties = blob.properties.unwrap_or_default();
            let size = properties.content_length.unwrap_or(0);
            let mut metadata =
                StorageMetadata::new(&relative_key, size).with_url(self.public_url(&relative_key));

            if let Some(ct) = &properties.content_type {
                metadata = metadata.with_content_type(ct);
            }

            if let Some(md5) = &properties.content_md5 {
                metadata = metadata.with_checksum(
                    base64::engine::general_purpose::STANDARD.encode(md5.as_slice()),
                );
            }

            results.push(metadata);
        }

        Ok(results)
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_client = self.blob_client(from);
        let to_client = self.blob_client(to);

        let source_url = from_client.url().to_string();

        // The new SDK performs a server-side copy via "upload from URL" on the
        // destination block blob (sets the `x-ms-copy-source` header).
        to_client
            .block_blob_client()
            .upload_blob_from_url(source_url, None)
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(self.public_url(key)))
    }

    async fn temporary_url(&self, _key: &str, _expires_in: Duration) -> Result<Option<String>> {
        // Azure SAS token generation requires an account key or a user-delegation
        // key. The new azure_storage_blob 1.0 SDK does not yet expose SAS
        // generation. Returning the plain public URL here would be misleading
        // (it grants no access to a private container and never expires), so
        // report this as unsupported per the `Storage::temporary_url` contract
        // until real SAS signing is implemented.
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `temporary_url` used to return `Some(public_url)` regardless of
    /// `expires_in`, which is a permanent, unsigned URL masquerading as a
    /// temporary/signed one -- misleading for a private container and never
    /// expiring. It must report "unsupported" (`None`) per the
    /// `Storage::temporary_url` contract until real SAS signing exists.
    #[tokio::test]
    async fn temporary_url_reports_unsupported() {
        let storage =
            AzureBlobStorage::new(AzureBlobConfig::new("account", "container").emulator())
                .await
                .expect("emulator config should build without network access");

        let result = storage
            .temporary_url("key.txt", Duration::from_secs(60))
            .await
            .expect("temporary_url should not error");

        assert!(
            result.is_none(),
            "expected None (unsupported) for Azure temporary_url, got {result:?}"
        );
    }
}
