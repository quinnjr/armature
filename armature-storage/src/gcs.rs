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
    calculate_checksum, generate_unique_key, sanitize_filename,
};

/// Google Cloud Storage configuration.
#[derive(Debug, Clone)]
pub struct GcsConfig {
    /// GCS bucket name.
    pub bucket: String,
    /// GCP project ID used as the billing/quota project.
    ///
    /// Object resource names use the `projects/_/buckets/{bucket}` placeholder
    /// form regardless of project, so this is applied per request as the quota
    /// project (the `x-goog-user-project` header), which is what bills
    /// requester-pays buckets and attributes quota. Leave unset to use the
    /// project associated with the ambient credentials.
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

    /// Set the billing/quota project ID.
    ///
    /// Sent as `x-goog-user-project` on every request; required for
    /// requester-pays buckets.
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
        let mut builder = Storage::builder();

        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint.clone());
        }

        let client = builder
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
                let mut builder = StorageControl::builder();
                if let Some(endpoint) = &self.config.endpoint {
                    builder = builder.with_endpoint(endpoint.clone());
                }
                builder
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

    /// Apply [`GcsConfig::project_id`] to a control-plane request builder as
    /// the quota/billing project (`x-goog-user-project`).
    fn with_quota<B>(&self, builder: B) -> B
    where
        B: google_cloud_gax::options::RequestOptionsBuilder,
    {
        match &self.config.project_id {
            Some(project) => builder.with_quota_project(project.clone()),
            None => builder,
        }
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
    ///
    /// `UploadedFile::name` comes straight off the client-controlled multipart
    /// `filename` header, so it is sanitized before it becomes an object name --
    /// exactly as the local backend does. Taking it verbatim let a filename of
    /// `../../secrets/key` become the object name as written.
    fn generate_key(&self, original_name: Option<&str>) -> String {
        if self.config.storage.generate_unique_names {
            generate_unique_key(original_name, self.config.storage.preserve_extension)
        } else {
            original_name
                .map(sanitize_filename)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        }
    }

    /// Sign a V4 URL for `key` with an explicit signer.
    ///
    /// Split out of [`StorageTrait::temporary_url`] so tests can sign with a
    /// deterministic service-account key instead of Application Default
    /// Credentials, which are not available in CI.
    async fn sign_url_with(
        &self,
        key: &str,
        expires_in: Duration,
        signer: &google_cloud_auth::signer::Signer,
    ) -> Result<Option<String>> {
        use google_cloud_storage::builder::storage::SignedUrlBuilder;

        let full_key = self.full_key(key);

        let url = SignedUrlBuilder::for_object(self.bucket_resource(), full_key)
            .with_method(http::Method::GET)
            .with_expiration(expires_in)
            .sign_with(signer)
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        Ok(Some(url))
    }

    /// Turn one `ListObjects` response into the [`StorageTrait::list_page`]
    /// return shape.
    ///
    /// Split out of `list_page` so it is reachable from a test: the
    /// control-plane client is gRPC, so unlike the S3 and Azure backends there
    /// is no way to drive listing end-to-end against an HTTP stub.
    ///
    /// Two things it must get right. The configured
    /// [`StorageConfig::path_prefix`] is stripped back off every key, or the
    /// keys do not round-trip through `get`. And GCS signals "no more pages"
    /// with an *empty* `next_page_token`, not an absent one -- since
    /// [`StorageTrait::list`] is a draining loop keyed on `Some`/`None`,
    /// reporting `Some("")` would make it re-request the same final page
    /// forever, never growing its result set and so never tripping the
    /// `LIST_MAX_ITEMS` escape either.
    fn map_list_page(
        &self,
        page: &google_cloud_storage::model::ListObjectsResponse,
    ) -> (Vec<StorageMetadata>, Option<String>) {
        // Hoisted out of the per-object loop: this was reallocated for every
        // object in the bucket.
        let strip = self
            .config
            .storage
            .path_prefix
            .as_ref()
            .map(|p| format!("{p}/"));

        let mut results = Vec::with_capacity(page.objects.len());

        for object in &page.objects {
            // Remove prefix to get the relative key
            let relative_key = match &strip {
                Some(strip) => object
                    .name
                    .strip_prefix(strip.as_str())
                    .unwrap_or(&object.name)
                    .to_string(),
                None => object.name.clone(),
            };

            let size = object.size as u64;
            let mut metadata =
                StorageMetadata::new(&relative_key, size).with_url(self.public_url(&relative_key));

            if !object.content_type.is_empty() {
                metadata = metadata.with_content_type(&object.content_type);
            }

            if let Some(checksums) = &object.checksums
                && !checksums.md5_hash.is_empty()
            {
                metadata = metadata.with_checksum(hex::encode(&checksums.md5_hash));
            }

            results.push(metadata);
        }

        let next = Some(page.next_page_token.clone()).filter(|t| !t.is_empty());

        (results, next)
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
        if let Some(max_size) = self.config.storage.max_file_size
            && data.len() as u64 > max_size
        {
            return Err(StorageError::TooLarge {
                size: data.len() as u64,
                limit: max_size,
            });
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
        let mut request = self
            .client
            .write_object(self.bucket_resource(), full_key.clone(), data)
            .set_content_type(content_type);

        if let Some(project) = &self.config.project_id {
            request = request.with_quota_project(project.clone());
        }

        if self.config.public_access {
            request = request.set_predefined_acl("publicRead");
        }

        request
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

        let mut request = self
            .client
            .read_object(self.bucket_resource(), full_key.clone());

        if let Some(project) = &self.config.project_id {
            request = request.with_quota_project(project.clone());
        }

        let mut response = request.send().await.map_err(|e| map_object_error(e, key))?;

        let mut contents = Vec::new();
        // Use the shared mapper rather than re-inlining a weaker copy of it:
        // the inlined version was missing the `NOT_FOUND` arm, so a not-found
        // surfacing mid-stream came back as an opaque `Storage` error.
        while let Some(chunk) = response
            .next()
            .await
            .transpose()
            .map_err(|e| map_object_error(e, key))?
        {
            contents.extend_from_slice(&chunk);
        }

        Ok(Bytes::from(contents))
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let full_key = self.full_key(key);

        let object = self
            .with_quota(self.control().await?.get_object())
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

        if let Some(checksums) = &object.checksums
            && !checksums.md5_hash.is_empty()
        {
            metadata = metadata.with_checksum(hex::encode(&checksums.md5_hash));
        }

        Ok(metadata)
    }

    /// Delete an object, idempotently.
    ///
    /// Per the [`StorageTrait::delete`] contract a missing key is `Ok(())`;
    /// every other failure is classified by [`map_object_error`] and
    /// propagated.
    async fn delete(&self, key: &str) -> Result<()> {
        let full_key = self.full_key(key);

        let result = self
            .with_quota(self.control().await?.delete_object())
            .set_bucket(self.bucket_resource())
            .set_object(full_key.clone())
            .send()
            .await;

        if let Err(e) = result {
            return delete_outcome(e, key);
        }

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

        let mut request = self
            .with_quota(self.control().await?.list_objects())
            .set_parent(self.bucket_resource())
            .set_prefix(full_prefix)
            .set_page_size(limit.min(i32::MAX as usize) as i32);

        if let Some(cursor) = cursor {
            request = request.set_page_token(cursor);
        }

        let page = request
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        Ok(self.map_list_page(&page))
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_key = self.full_key(from);
        let to_key = self.full_key(to);

        // Server-side copy is performed via the rewrite operation in the new SDK.
        // Routed through `map_object_error` so a missing source is `NotFound`
        // here as it is on every other backend, rather than opaque `Storage`.
        self.with_quota(self.control().await?.rewrite_object())
            .set_source_bucket(self.bucket_resource())
            .set_source_object(from_key)
            .set_destination_bucket(self.bucket_resource())
            .set_destination_name(to_key)
            .send()
            .await
            .map_err(|e| map_object_error(e, from))?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(self.public_url(key)))
    }

    fn default_url_duration(&self) -> Duration {
        self.config.signed_url_duration
    }

    async fn temporary_url(&self, key: &str, expires_in: Duration) -> Result<Option<String>> {
        use google_cloud_auth::credentials::Builder as CredentialsBuilder;

        // The new SDK signs V4 URLs locally using a `Signer` derived from the
        // ambient credentials (Application Default Credentials).
        let signer = CredentialsBuilder::default()
            .build_signer()
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        self.sign_url_with(key, expires_in, &signer).await
    }
}

/// Map a GCS error to a [`StorageError`], translating not-found responses.
///
/// Generic over the error type because the data-plane read stream, the
/// data-plane request and the control-plane client each surface a different
/// one; every call site must classify not-found the same way.
fn map_object_error<E: std::fmt::Display>(err: E, key: &str) -> StorageError {
    let err_str = err.to_string();
    if err_str.contains("404") || err_str.contains("not found") || err_str.contains("NOT_FOUND") {
        StorageError::NotFound(key.to_string())
    } else {
        StorageError::Storage(err_str)
    }
}

/// Collapse a `delete_object` failure into the idempotent [`StorageTrait::delete`]
/// contract: a not-found object is a successful delete, anything else is an error.
fn delete_outcome<E: std::fmt::Display>(err: E, key: &str) -> Result<()> {
    match map_object_error(err, key) {
        StorageError::NotFound(_) => Ok(()),
        other => Err(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_testkit::http_stub::{StubResponse, StubServer};
    use google_cloud_auth::credentials::anonymous::Builder as Anonymous;

    /// Build a [`GcsStorage`] whose data-plane client is pointed at `server`
    /// via [`GcsConfig::endpoint`], exercising the same `with_endpoint(...)`
    /// wiring that `GcsStorage::new` applies. `endpoint` was documented for
    /// emulator use but previously ignored, so without the fix this client
    /// would instead target real GCS.
    async fn stub_gcs_storage(server: &StubServer, config: GcsConfig) -> GcsStorage {
        let config = config.endpoint(server.url());
        let client = Storage::builder()
            .with_endpoint(config.endpoint.clone().unwrap())
            .with_credentials(Anonymous::new().build())
            .build()
            .await
            .expect("building a stub-backed GCS client should succeed");
        GcsStorage::from_client(client, config)
    }

    #[tokio::test]
    async fn public_access_sets_a_public_read_predefined_acl() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage =
            stub_gcs_storage(&server, GcsConfig::new("test-bucket").public_access(true)).await;

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("write should succeed against the stub server");

        let requests = server.requests();
        let upload = requests
            .iter()
            .find(|r| {
                r.query
                    .as_deref()
                    .is_some_and(|q| q.contains("name=key.txt"))
            })
            .unwrap_or_else(|| panic!("no upload request recorded: {requests:?}"));
        let query = upload.query.as_deref().unwrap_or_default();
        assert!(
            query.contains("predefinedAcl=publicRead"),
            "expected predefinedAcl=publicRead in query, got: {query}"
        );
    }

    /// A throwaway RSA key used purely to build a deterministic signer, so the
    /// signed-URL assertions below do not depend on Application Default
    /// Credentials being present. It signs nothing real.
    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDiKQN8zuLbaZN5\nMTPXXDvl/GsAknmPwNtgr6F1EzH/HntrdtZ+lvYzjFtvZ/+tjmjr2w/lcptxZKKk\n21VEZm8pN9YPCQqDuZVEzlhy8JnQ9C3eyNrTOCue1qWz31xCkwn4rDHtVasEIajm\noTThz9xMB7Gf2IgPgDMbhCkFBd35EBbzlGUhXQG/Ke1C0juKV6JT32AwAxVyr3rL\nRJ55zMBLaoJAKbQBmnfWJqiHWsMb+WYffwDTDs4xR2niMJrwaNhDutRTts9k6GnY\nlDl6ra9ZV0spHfnLXvWpskTFSnCFGMJ7fBI+sM4jPO/WvayzW6eaC7j+foJpKE4M\n4xQrTrkFAgMBAAECggEAT82kHubL8xtyf+nGPsCbnEBxK38EKR8m6hufT/YJhtnl\nOBrzfjDbyH3HB+09MatWR5+BoPfLdPxLTfvdPykcIYHD5YNNtASI8QIVAN34kNyQ\n0ROz76Na9Q4N44Y2AoHrG1X7uiEoGumbtWH+DI5x0FxIp7xa6olUv2lnpg+Xb5o9\ngBQrcnMDc8cyC4ObV3dz/KOQ7z1Hr0KT3GVzQmH+IGubhI1Q5R6NC6esEDvlG8l5\n8bgK5MXRi5peIsNodT2ZZAI1VzjAd+AiMme+lGz9xMrhm0F2V0NvF8EpSo4uas5M\nUUJrFa9BMBSK60MPPgEPpli70doxOm9pNOc+zHugwwKBgQD0+wJdbZTn0bUtHoOv\nYdpClGLbSolKA1l7a9CU/BpMCGTfS8yPXM4ohzRDNixdvJlLlF4FIOztKLiIWJ3D\nYFrPwfN1I6qea3EsIf/mK9qmB+Z8KpPZBUEzw//YphJvjq/LbTFz5aoEkJgiLpGV\n9F8aW98myVZPkbVmmL1DW6i22wKBgQDsVUkci7sP1uvfH+CL00OLcL04/zM8MZyw\nfl+7oFJEPEWxp146hNKphAq21n4XRBTQWCxzU39x6GUmXpDoakntGhWwPNld34Q5\ntJZidf1FnnVv3Mq0N/rC13awHgHENdmYgYzO2CxSXfV8viE+5uErFf8o9sK61HmZ\nk/2+1h6lnwKBgQDLVMc6umg8HM+umkQcPjCE0FpYvr3Cg5MyoGLoNXKyJslqmKQ5\nXYLzGn0jSAR87Lujgoqi4RglI4Y+DKcs8X2OMOGcGTVU9cJiKfoWldGNusLvzfsW\nxoi+qXBh5j0pAJoiUwgXtMhvr3/F5zcI6mJBI33M2JFdy4dvl1iHXr1ivwKBgDLh\n0dHhi67HWRU66b9xBtPYvASvfTpyfAfLzZS52bxzNZYgMLtsqWZx1VS0LYWY1Npe\ngYN68K93l3+BULWZXL09pnnBQBNj8jXyWYZtXNBGY4ZoBQR0IPseJKGadEroRSb+\njXBjPnelXxsyXDoMv2HlZIBPUHGlGWElabZSp1qFAoGBANdzGUXT1ozW48YcP4Iq\nCXpr8pOpxGOup3Ts8OcmLFgbXcT3SkmiwlD02TmrDYQIWtT7ZQoAx38Xk6f29cUn\niGeE9ycRZfgR8BJneYielQ/y5d/A9KmpORr1BuQCf0JMnJB8veC/NDIoNDFpVPIq\nr1cMH1jgia8+TqcycOHM1e43\n-----END PRIVATE KEY-----\n";

    fn test_signer() -> google_cloud_auth::signer::Signer {
        use google_cloud_auth::credentials::service_account::Builder as ServiceAccountBuilder;

        let key = serde_json::json!({
            "type": "service_account",
            "project_id": "test-project",
            "private_key_id": "test-key-id",
            "private_key": TEST_PRIVATE_KEY,
            "client_email": "test@test-project.iam.gserviceaccount.com",
            "client_id": "1234567890",
            "universe_domain": "googleapis.com",
        });

        ServiceAccountBuilder::new(key)
            .build_signer()
            .expect("building a signer from the test service-account key should succeed")
    }

    /// `GcsConfig::signed_url_duration` must actually reach the signed URL.
    ///
    /// The previous version of this test put its only assertion inside
    /// `if let Ok(Some(url)) = result`, and in CI (no ADC) `result` is `Err`,
    /// so it passed unconditionally -- including against code that ignored
    /// `signed_url_duration` entirely. Signing with a fixed test key makes the
    /// expiry observable without live credentials.
    #[tokio::test]
    async fn signed_url_carries_the_requested_expiration() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;

        let url = storage
            .sign_url_with("key.txt", Duration::from_secs(120), &test_signer())
            .await
            .expect("signing with the test key should succeed")
            .expect("a signed url should be produced");

        assert!(
            url.contains("X-Goog-Expires=120"),
            "expected X-Goog-Expires=120 in the signed URL, got: {url}"
        );
    }

    /// `default_url_duration` exposes the configured duration on the `Storage`
    /// trait, and the provided `temporary_url_default` forwards it.
    #[tokio::test]
    async fn default_url_duration_reports_the_configured_duration() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(
            &server,
            GcsConfig::new("test-bucket").signed_url_duration(Duration::from_secs(120)),
        )
        .await;

        assert_eq!(
            StorageTrait::default_url_duration(&storage),
            Duration::from_secs(120)
        );

        // And that duration is what `temporary_url_default` would sign with.
        let url = storage
            .sign_url_with(
                "key.txt",
                StorageTrait::default_url_duration(&storage),
                &test_signer(),
            )
            .await
            .unwrap()
            .unwrap();
        assert!(url.contains("X-Goog-Expires=120"), "got: {url}");
    }

    /// The default is one hour when nothing is configured -- a distinct value
    /// from the 120s above, so this cannot pass by accident.
    #[tokio::test]
    async fn default_url_duration_defaults_to_one_hour() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;

        assert_eq!(
            StorageTrait::default_url_duration(&storage),
            Duration::from_secs(3600)
        );
    }

    /// `GcsConfig::project_id` was documented as "currently only
    /// informational" -- a deferral. It is now applied per request as the
    /// quota/billing project, which the SDK sends as `x-goog-user-project`.
    #[tokio::test]
    async fn project_id_is_sent_as_the_quota_project() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(
            &server,
            GcsConfig::new("test-bucket").project_id("my-billing-project"),
        )
        .await;

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("write should succeed against the stub server");

        let requests = server.requests();
        let upload = requests
            .iter()
            .find(|r| {
                r.query
                    .as_deref()
                    .is_some_and(|q| q.contains("name=key.txt"))
            })
            .unwrap_or_else(|| panic!("no upload request recorded: {requests:?}"));

        assert_eq!(
            upload.header("x-goog-user-project"),
            Some("my-billing-project"),
            "expected the configured project as the quota project"
        );
    }

    #[tokio::test]
    async fn no_project_id_sends_no_quota_project_header() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("write should succeed against the stub server");

        let requests = server.requests();
        let upload = requests
            .iter()
            .find(|r| {
                r.query
                    .as_deref()
                    .is_some_and(|q| q.contains("name=key.txt"))
            })
            .unwrap_or_else(|| panic!("no upload request recorded: {requests:?}"));

        assert_eq!(upload.header("x-goog-user-project"), None);
    }

    /// `put_file` sanitized the client-supplied filename on the local backend
    /// only; S3/GCS/Azure used it verbatim, so a multipart `filename` of
    /// `../../secrets/key` became the object name as written -- while the
    /// README advertised traversal rejection as a trait-level guarantee.
    #[tokio::test]
    async fn generate_key_sanitizes_a_path_bearing_filename() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let mut storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;
        storage.config.storage.generate_unique_names = false;

        assert_eq!(
            storage.generate_key(Some("../../secrets/key")),
            "key",
            "a path-bearing filename must not survive into the object name"
        );
        assert_eq!(storage.generate_key(Some("/etc/shadow")), "shadow");
        assert_eq!(storage.generate_key(Some("ok.txt")), "ok.txt");
    }

    /// `delete` bypassed `map_object_error` -- defined in this same file and
    /// used by `get`/`head` -- so a missing object came back as an opaque
    /// `Storage` error while S3 returned `Ok(())`. The control plane is gRPC
    /// and cannot be pointed at an HTTP stub, so this exercises the exact
    /// classifier `delete` runs on the error it gets back.
    #[test]
    fn delete_treats_a_missing_object_as_success() {
        assert!(
            delete_outcome(
                "NOT_FOUND: No such object: test-bucket/gone.txt",
                "gone.txt"
            )
            .is_ok(),
            "deleting a missing object must be Ok(()) per the Storage::delete contract"
        );
        assert!(delete_outcome("404 Not Found", "gone.txt").is_ok());

        // Everything else still propagates.
        match delete_outcome(
            "PERMISSION_DENIED: caller lacks storage.objects.delete",
            "k",
        ) {
            Err(StorageError::Storage(msg)) => assert!(msg.contains("PERMISSION_DENIED")),
            other => panic!("a permission failure must not be swallowed, got {other:?}"),
        }
    }

    /// `copy` and the body-streaming path in `get` both re-implemented or
    /// skipped the shared mapper; every not-found path must classify alike.
    #[test]
    fn map_object_error_classifies_every_not_found_spelling() {
        for err in [
            "NOT_FOUND: No such object",
            "404 Not Found",
            "object not found",
        ] {
            assert!(
                matches!(map_object_error(err, "k"), StorageError::NotFound(_)),
                "{err:?} must classify as NotFound"
            );
        }
        assert!(matches!(
            map_object_error("UNAVAILABLE: backend overloaded", "k"),
            StorageError::Storage(_)
        ));
    }

    fn object(name: &str, size: i64, content_type: &str) -> google_cloud_storage::model::Object {
        google_cloud_storage::model::Object::new()
            .set_name(name)
            .set_size(size)
            .set_content_type(content_type)
    }

    /// `list_page` is the bounded listing primitive on every backend; S3 and
    /// Azure each had a stub-server test for it and GCS had none. The
    /// control-plane client is gRPC, so it cannot be pointed at the HTTP stub
    /// the rest of this module uses -- this drives the response-mapping half
    /// (`map_list_page`) directly instead, which is where every behaviour the
    /// S3 and Azure tests assert on actually lives.
    #[tokio::test]
    async fn list_page_maps_a_page_and_returns_the_next_page_token() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;

        let page = google_cloud_storage::model::ListObjectsResponse::new()
            .set_objects([
                object("a.txt", 3, "text/plain"),
                object("dir/b.bin", 5, "application/octet-stream"),
            ])
            .set_next_page_token("TOKEN-2");

        let (results, next) = storage.map_list_page(&page);

        assert_eq!(
            results.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["a.txt", "dir/b.bin"]
        );
        assert_eq!(results[0].size, 3);
        assert_eq!(results[0].content_type.as_deref(), Some("text/plain"));
        assert_eq!(results[1].size, 5);
        assert_eq!(
            results[0].url.as_deref(),
            Some("https://storage.googleapis.com/test-bucket/a.txt")
        );
        assert_eq!(next.as_deref(), Some("TOKEN-2"));
    }

    /// The configured `path_prefix` is stripped back off the returned keys, so
    /// they round-trip through `get`.
    #[tokio::test]
    async fn list_page_strips_the_configured_path_prefix_from_returned_keys() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage =
            stub_gcs_storage(&server, GcsConfig::new("test-bucket").prefix("tenant-a")).await;

        let page = google_cloud_storage::model::ListObjectsResponse::new().set_objects([object(
            "tenant-a/reports/x.txt",
            1,
            "",
        )]);

        let (results, next) = storage.map_list_page(&page);

        assert_eq!(
            results.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["reports/x.txt"]
        );
        assert_eq!(next, None);
    }

    /// GCS signals "no more pages" with an *empty* `next_page_token`, not an
    /// absent one. `StorageTrait::list` is a draining loop keyed on
    /// `Some`/`None`, so an empty token reported as a continuation makes it
    /// re-request the same final page forever -- never growing `results`, so
    /// the `LIST_MAX_ITEMS` escape never fires either. The
    /// `.filter(|t| !t.is_empty())` is the only thing between `list()` and a
    /// hung task.
    #[tokio::test]
    async fn an_empty_page_token_terminates_the_listing() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;

        // Explicitly empty, which is what the service sends on the last page.
        let page = google_cloud_storage::model::ListObjectsResponse::new()
            .set_objects([object("only.txt", 3, "")])
            .set_next_page_token("");

        let (results, next) = storage.map_list_page(&page);

        assert_eq!(
            results.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["only.txt"]
        );
        assert_eq!(
            next, None,
            "an empty nextPageToken means the listing is complete, not 'ask again'"
        );

        // ...and the worst case: an empty final page with an empty token. If
        // this ever came back as `Some("")`, `list` would never terminate and
        // never accumulate, so it would hang rather than fail.
        let empty = google_cloud_storage::model::ListObjectsResponse::new().set_next_page_token("");
        assert_eq!(storage.map_list_page(&empty).1, None);
    }

    /// The `list()` draining loop itself must terminate when `list_page`
    /// reports no continuation, even when it also reports no objects. Driven
    /// through a `Storage` whose `list_page` replays exactly that shape, under
    /// a timeout so a regression fails fast instead of hanging CI.
    #[tokio::test]
    async fn list_terminates_on_an_empty_final_page() {
        struct EmptyFinalPage {
            calls: std::sync::atomic::AtomicUsize,
        }

        #[async_trait]
        impl StorageTrait for EmptyFinalPage {
            async fn put(&self, _: &str, _: Bytes) -> Result<StorageMetadata> {
                unimplemented!()
            }
            async fn put_with_content_type(
                &self,
                _: &str,
                _: Bytes,
                _: &str,
            ) -> Result<StorageMetadata> {
                unimplemented!()
            }
            async fn put_file(&self, _: &UploadedFile) -> Result<StorageMetadata> {
                unimplemented!()
            }
            async fn get(&self, _: &str) -> Result<Bytes> {
                unimplemented!()
            }
            async fn head(&self, _: &str) -> Result<StorageMetadata> {
                unimplemented!()
            }
            async fn delete(&self, _: &str) -> Result<()> {
                unimplemented!()
            }
            async fn exists(&self, _: &str) -> Result<bool> {
                unimplemented!()
            }
            async fn copy(&self, _: &str, _: &str) -> Result<StorageMetadata> {
                unimplemented!()
            }
            async fn list_page(
                &self,
                _: Option<&str>,
                _: Option<&str>,
                _: usize,
            ) -> Result<(Vec<StorageMetadata>, Option<String>)> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // What `map_list_page` produces for a final, empty page.
                Ok((Vec::new(), None))
            }
        }

        let backend = EmptyFinalPage {
            calls: std::sync::atomic::AtomicUsize::new(0),
        };

        let listed = tokio::time::timeout(Duration::from_secs(10), backend.list(None))
            .await
            .expect("list must terminate on an empty final page, not spin forever")
            .expect("list should succeed");

        assert!(listed.is_empty());
        assert_eq!(
            backend.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one page, one call -- a terminated listing must not be re-requested"
        );
    }

    #[tokio::test]
    async fn default_config_omits_predefined_acl() {
        let server = StubServer::start_single(StubResponse::json(200, "{}")).await;
        let storage = stub_gcs_storage(&server, GcsConfig::new("test-bucket")).await;

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("write should succeed against the stub server");

        let requests = server.requests();
        let upload = requests
            .iter()
            .find(|r| {
                r.query
                    .as_deref()
                    .is_some_and(|q| q.contains("name=key.txt"))
            })
            .unwrap_or_else(|| panic!("no upload request recorded: {requests:?}"));
        let query = upload.query.as_deref().unwrap_or_default();
        assert!(
            !query.contains("predefinedAcl"),
            "expected no predefinedAcl in query, got: {query}"
        );
    }
}
