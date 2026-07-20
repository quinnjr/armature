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
use futures_util::TryStreamExt;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

use crate::{
    Result, Storage, StorageConfig, StorageError, StorageMetadata, UploadedFile,
    calculate_checksum, generate_unique_key, sanitize_filename,
};

/// Azure Blob Storage configuration.
#[derive(Debug, Clone, Default)]
pub struct AzureBlobConfig {
    /// Storage account name.
    pub account: String,
    /// Container name.
    pub container: String,
    /// Custom endpoint (for Azurite emulator).
    pub endpoint: Option<String>,
    /// Use Azurite emulator.
    pub use_emulator: bool,
    /// Common storage configuration.
    pub storage: StorageConfig,
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
}

/// Azure Blob Storage backend.
pub struct AzureBlobStorage {
    container_client: BlobContainerClient,
    config: AzureBlobConfig,
}

impl AzureBlobStorage {
    /// Create a new Azure Blob storage backend.
    ///
    /// The new Azure SDK authenticates with AAD (Entra ID) token credentials
    /// only; connection-string and shared-key (account key) authentication are
    /// not supported by the SDK, so this config exposes no way to request
    /// them. Use the ambient Azure credential chain (Azure CLI, environment,
    /// managed identity) or [`AzureBlobConfig::emulator`].
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
    ///
    /// `UploadedFile::name` comes straight off the client-controlled multipart
    /// `filename` header, so it is sanitized before it becomes a blob name --
    /// exactly as the local backend does. Taking it verbatim let a filename of
    /// `../../secrets/key` become the blob name as written.
    fn generate_key(&self, original_name: Option<&str>) -> String {
        if self.config.storage.generate_unique_names {
            generate_unique_key(original_name, self.config.storage.preserve_extension)
        } else {
            original_name
                .map(sanitize_filename)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        }
    }

    /// Get the public URL for a key.
    ///
    /// `key` is validated (rejecting `..` traversal segments) and
    /// percent-encoded before being interpolated into the URL, so it cannot
    /// escape the intended path or inject a query, a fragment, or a header
    /// break.
    pub fn public_url(&self, key: &str) -> Result<String> {
        let full_key = crate::storage::encode_key_for_url(&self.full_key(key))?;
        Ok(if self.config.use_emulator {
            format!(
                "http://127.0.0.1:10000/devstoreaccount1/{}/{}",
                self.config.container, full_key
            )
        } else {
            format!(
                "https://{}.blob.core.windows.net/{}/{}",
                self.config.account, self.config.container, full_key
            )
        })
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
            .with_url(self.public_url(key)?);

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
        let mut metadata = StorageMetadata::new(key, size).with_url(self.public_url(key)?);

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

    /// Delete a blob, idempotently.
    ///
    /// Per the [`Storage::delete`] contract a missing key is `Ok(())`; every
    /// other failure is classified by [`map_blob_error`] and propagated.
    async fn delete(&self, key: &str) -> Result<()> {
        let blob_client = self.blob_client(key);

        if let Err(e) = blob_client.delete(None).await {
            match map_blob_error(e, key) {
                StorageError::NotFound(_) => return Ok(()),
                other => return Err(other),
            }
        }

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
            marker: cursor.map(String::from),
            maxresults: Some(limit.min(i32::MAX as usize) as i32),
            ..Default::default()
        };

        // `into_pages()` so exactly one request goes out and the server's
        // `NextMarker` is observable; iterating the item pager instead drains
        // the whole container into memory with no way to stop.
        let mut pages = self
            .container_client
            .list_blobs(Some(options))
            .map_err(|e| StorageError::Storage(e.to_string()))?
            .into_pages();

        let Some(page) = pages
            .try_next()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?
        else {
            return Ok((Vec::new(), None));
        };

        let page = page
            .into_model()
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        let next = page.next_marker.clone().filter(|m: &String| !m.is_empty());

        // Hoisted out of the per-blob loop: this was reallocated for every
        // blob in the container.
        let strip = self
            .config
            .storage
            .path_prefix
            .as_ref()
            .map(|p| format!("{p}/"));

        let mut results = Vec::with_capacity(page.blob_items.len());

        for blob in page.blob_items {
            let name = blob.name.unwrap_or_default();

            // Remove prefix to get the relative key
            let relative_key = match &strip {
                Some(strip) => name
                    .strip_prefix(strip.as_str())
                    .unwrap_or(&name)
                    .to_string(),
                None => name.clone(),
            };

            let properties = blob.properties.unwrap_or_default();
            let size = properties.content_length.unwrap_or(0);
            let mut metadata =
                StorageMetadata::new(&relative_key, size).with_url(self.public_url(&relative_key)?);

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

        Ok((results, next))
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_client = self.blob_client(from);
        let to_client = self.blob_client(to);

        let source_url = from_client.url().to_string();

        // The new SDK performs a server-side copy via "upload from URL" on the
        // destination block blob (sets the `x-ms-copy-source` header).
        // Routed through `map_blob_error` so a missing source is `NotFound`
        // here as it is on every other backend, rather than opaque `Storage`.
        to_client
            .block_blob_client()
            .upload_blob_from_url(source_url, None)
            .await
            .map_err(|e| map_blob_error(e, from))?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(self.public_url(key)?))
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
    use armature_testkit::http_stub::{StubResponse, StubServer};

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

    /// `sas_duration` was read nowhere -- its only possible consumer,
    /// `temporary_url`, returns `Ok(None)` and ignores its `expires_in`. The
    /// knob is gone; `default_url_duration` falls back to the trait default,
    /// and `temporary_url_default` is still honestly unsupported.
    #[tokio::test]
    async fn temporary_url_default_is_also_unsupported() {
        let storage =
            AzureBlobStorage::new(AzureBlobConfig::new("account", "container").emulator())
                .await
                .expect("emulator config should build without network access");

        assert_eq!(storage.default_url_duration(), crate::DEFAULT_URL_DURATION);
        assert!(
            storage
                .temporary_url_default("key.txt")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// The `access_key` / `connection_string` builders read as supported auth
    /// modes but guaranteed `new()` would fail -- deferring the failure from
    /// the infallible builder to the constructor. They are gone, so the only
    /// configurations that reach `new()` are the supported ones plus genuinely
    /// malformed input, which must still be rejected.
    #[tokio::test]
    async fn constructor_rejects_a_malformed_endpoint() {
        let config = AzureBlobConfig::new("account", "container").endpoint("not a url");

        match AzureBlobStorage::new(config).await {
            Err(StorageError::Config(_)) => {}
            Err(other) => panic!("expected a Config error, got {other:?}"),
            Ok(_) => panic!("a malformed endpoint must be rejected"),
        }
    }

    /// `put_file` sanitized the client-supplied filename on the local backend
    /// only; S3/GCS/Azure used it verbatim, so a multipart `filename` of
    /// `../../secrets/key` became the blob name as written -- while the README
    /// advertised traversal rejection as a trait-level guarantee.
    #[tokio::test]
    async fn generate_key_sanitizes_a_path_bearing_filename() {
        let mut config = AzureBlobConfig::new("devstoreaccount1", "container").emulator();
        config.storage.generate_unique_names = false;
        let storage = AzureBlobStorage::new(config)
            .await
            .expect("emulator config should build without network access");

        assert_eq!(
            storage.generate_key(Some("../../secrets/key")),
            "key",
            "a path-bearing filename must not survive into the blob name"
        );
        assert_eq!(storage.generate_key(Some("/etc/shadow")), "shadow");
        assert_eq!(storage.generate_key(Some("ok.txt")), "ok.txt");
    }

    /// A stub-backed storage in emulator mode: the emulator pipeline is
    /// unauthenticated, so a plain HTTP stub is enough to drive real requests.
    async fn stub_azure_storage(server: &StubServer) -> AzureBlobStorage {
        AzureBlobStorage::new(
            AzureBlobConfig::new("devstoreaccount1", "container")
                .emulator()
                .endpoint(server.url()),
        )
        .await
        .expect("stub-backed emulator config should build")
    }

    /// `delete` bypassed `map_blob_error` -- defined in this same file and used
    /// by `get`/`head` -- so a missing blob came back as an opaque `Storage`
    /// error while S3 returned `Ok(())`. Deletion is idempotent on every
    /// backend per the `Storage::delete` contract.
    #[tokio::test]
    async fn delete_of_a_missing_blob_is_ok() {
        let server = StubServer::start_single(StubResponse::new(
            404,
            "<?xml version=\"1.0\"?><Error><Code>BlobNotFound</Code></Error>",
        ))
        .await;
        let storage = stub_azure_storage(&server).await;

        storage
            .delete("never-existed.txt")
            .await
            .expect("deleting a missing blob must be Ok(()), not a Storage error");

        server.assert_received("DELETE", "/container/never-existed.txt");
    }

    /// ...but a real failure is still a failure.
    #[tokio::test]
    async fn delete_propagates_a_non_not_found_failure() {
        let server = StubServer::start_single(StubResponse::new(
            403,
            "<?xml version=\"1.0\"?><Error><Code>AuthorizationFailure</Code></Error>",
        ))
        .await;
        let storage = stub_azure_storage(&server).await;

        match storage.delete("k.txt").await {
            Err(StorageError::Storage(_)) => {}
            other => panic!("a 403 must not be swallowed by the idempotent path, got {other:?}"),
        }
    }

    /// `copy` reported a missing source as an opaque `Storage` error; every
    /// backend now agrees that a missing copy source is `NotFound`.
    #[tokio::test]
    async fn copy_of_a_missing_source_is_not_found() {
        let server = StubServer::start_single(StubResponse::new(
            404,
            "<?xml version=\"1.0\"?><Error><Code>BlobNotFound</Code></Error>",
        ))
        .await;
        let storage = stub_azure_storage(&server).await;

        match storage.copy("missing.txt", "dst.txt").await {
            Err(StorageError::NotFound(key)) => assert_eq!(key, "missing.txt"),
            other => panic!("expected NotFound naming the source, got {other:?}"),
        }
    }

    /// `list` drained the whole container into one `Vec`; `list_page` requests
    /// a single page and surfaces the server's `NextMarker`.
    #[tokio::test]
    async fn list_page_requests_one_page_and_returns_the_next_marker() {
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <EnumerationResults ContainerName=\"container\">\
            <Blobs><Blob><Name>a.txt</Name><Properties><Content-Length>3</Content-Length>\
            </Properties></Blob></Blobs><NextMarker>MARKER-2</NextMarker></EnumerationResults>";
        let server = StubServer::start_single(StubResponse::new(200, body)).await;
        let storage = stub_azure_storage(&server).await;

        let (page, next) = storage
            .list_page(None, None, 1)
            .await
            .expect("list_page should succeed");

        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["a.txt"]
        );
        assert_eq!(next.as_deref(), Some("MARKER-2"));
        assert_eq!(
            server.requests().len(),
            1,
            "list_page must issue exactly one request"
        );

        let query = server.requests()[0].query.clone().unwrap_or_default();
        assert!(
            query.contains("maxresults=1"),
            "the limit must reach the service: {query}"
        );

        // The cursor is sent back as the marker. Bind the result rather than
        // discarding it with `let _ =`: if the second call fails, the request
        // may never have been made and `requests()[1]` would panic on an index
        // out of bounds instead of reporting the real failure.
        let (second, _) = storage
            .list_page(None, Some("MARKER-2"), 1)
            .await
            .expect("the second list_page must succeed too");
        assert_eq!(
            second.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["a.txt"],
            "the stub replays the same page, so the same key comes back"
        );

        let requests = server.requests();
        assert_eq!(requests.len(), 2, "a second request must have gone out");
        let query = requests[1].query.clone().unwrap_or_default();
        assert!(
            query.contains("marker=MARKER-2"),
            "the cursor must be sent as the marker: {query}"
        );
    }

    /// Azure emits a self-closing `<NextMarker />` on the *final* page, which
    /// deserializes to `Some("")`. `Storage::list` is a draining loop keyed on
    /// `Some`/`None`, so an empty marker treated as a continuation means it
    /// re-requests the same final page forever, never growing `results` and so
    /// never tripping the `LIST_MAX_ITEMS` escape either. The
    /// `.filter(|m| !m.is_empty())` is the only thing between `list()` and a
    /// hung task.
    #[tokio::test]
    async fn an_empty_next_marker_terminates_the_listing() {
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <EnumerationResults ContainerName=\"container\">\
            <Blobs><Blob><Name>only.txt</Name><Properties><Content-Length>3</Content-Length>\
            </Properties></Blob></Blobs><NextMarker /></EnumerationResults>";
        let server = StubServer::start_single(StubResponse::new(200, body)).await;
        let storage = stub_azure_storage(&server).await;

        let (page, next) = storage
            .list_page(None, None, 10)
            .await
            .expect("list_page should succeed");

        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["only.txt"]
        );
        assert_eq!(
            next, None,
            "an empty NextMarker means the listing is complete, not 'ask again'"
        );
    }

    /// The same at the `list()` level, and against the worst case: a final page
    /// with an empty marker and *no* blobs. A regression here does not fail, it
    /// hangs -- so the timeout is the assertion.
    #[tokio::test]
    async fn list_terminates_on_an_empty_marker_with_no_blobs() {
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
            <EnumerationResults ContainerName=\"container\">\
            <Blobs></Blobs><NextMarker /></EnumerationResults>";
        let server = StubServer::start_single(StubResponse::new(200, body)).await;
        let storage = stub_azure_storage(&server).await;

        let listed = tokio::time::timeout(Duration::from_secs(10), storage.list(None))
            .await
            .expect("list must terminate on an empty NextMarker, not spin forever")
            .expect("list should succeed");

        assert!(listed.is_empty());
        assert_eq!(
            server.requests().len(),
            1,
            "one page, one request -- an empty marker must not be re-sent"
        );
    }

    /// `url`/`public_url` used to interpolate the raw key into the URL
    /// string: a key of `../../admin` walked out of the intended path, and one
    /// containing `?`, `#` or CRLF injected a query, a fragment, or a header
    /// break.
    #[tokio::test]
    async fn url_rejects_traversal_and_percent_encodes_special_characters() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage = stub_azure_storage(&server).await;

        let err = storage
            .url("../../admin")
            .await
            .expect_err("a traversal key must be rejected");
        assert!(
            matches!(err, StorageError::InvalidFileName(_)),
            "expected InvalidFileName, got {err:?}"
        );

        let url = storage
            .url("report?admin=1#frag\r\nX-Injected: 1")
            .await
            .unwrap()
            .unwrap();
        assert!(
            !url.contains(['?', '#', '\r', '\n']),
            "the raw special characters must not survive into the URL: {url}"
        );
        assert!(
            url.ends_with("report%3Fadmin%3D1%23frag%0D%0AX-Injected%3A%201"),
            "got: {url}"
        );
    }

    #[tokio::test]
    async fn emulator_config_uses_the_default_azurite_endpoint() {
        let storage =
            AzureBlobStorage::new(AzureBlobConfig::new("devstoreaccount1", "container").emulator())
                .await
                .expect("emulator config should build");

        assert!(
            storage
                .public_url("k.txt")
                .unwrap()
                .starts_with("http://127.0.0.1:10000/"),
            "got: {}",
            storage.public_url("k.txt").unwrap()
        );
    }
}
