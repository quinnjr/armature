//! AWS S3 storage backend.

use async_trait::async_trait;
use aws_sdk_s3::{
    Client,
    config::Region,
    primitives::ByteStream,
    types::{ObjectCannedAcl, ServerSideEncryption, StorageClass},
};
use bytes::Bytes;
use std::time::Duration;
use tracing::{debug, info};

use crate::{
    Result, Storage, StorageConfig, StorageError, StorageMetadata, UploadedFile,
    calculate_checksum, generate_unique_key,
};

/// S3 storage configuration.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// S3 bucket name.
    pub bucket: String,
    /// AWS region.
    pub region: Option<String>,
    /// Custom endpoint (for S3-compatible services).
    pub endpoint: Option<String>,
    /// Default ACL for uploaded objects.
    pub default_acl: Option<String>,
    /// Server-side encryption.
    pub server_side_encryption: Option<String>,
    /// Storage class.
    pub storage_class: Option<String>,
    /// Common storage configuration.
    pub storage: StorageConfig,
    /// Generate presigned URLs duration.
    pub presigned_url_duration: Duration,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            region: None,
            endpoint: None,
            default_acl: None,
            server_side_encryption: None,
            storage_class: None,
            storage: StorageConfig::default(),
            presigned_url_duration: Duration::from_secs(3600), // 1 hour
        }
    }
}

impl S3Config {
    /// Create configuration for a bucket.
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            ..Default::default()
        }
    }

    /// Set the region.
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Set a custom endpoint (for S3-compatible services like MinIO).
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Set the default ACL.
    pub fn acl(mut self, acl: impl Into<String>) -> Self {
        self.default_acl = Some(acl.into());
        self
    }

    /// Enable public read access.
    pub fn public_read(self) -> Self {
        self.acl("public-read")
    }

    /// Set server-side encryption.
    pub fn encryption(mut self, encryption: impl Into<String>) -> Self {
        self.server_side_encryption = Some(encryption.into());
        self
    }

    /// Enable AES256 server-side encryption.
    pub fn aes256_encryption(self) -> Self {
        self.encryption("AES256")
    }

    /// Set the path prefix.
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.storage.path_prefix = Some(prefix.into());
        self
    }

    /// Set presigned URL duration.
    pub fn presigned_duration(mut self, duration: Duration) -> Self {
        self.presigned_url_duration = duration;
        self
    }
}

/// AWS S3 storage backend.
pub struct S3Storage {
    client: Client,
    config: S3Config,
}

impl S3Storage {
    /// Create a new S3 storage backend.
    pub async fn new(config: S3Config) -> Result<Self> {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;

        let s3_config = Self::build_client_config(&aws_config, &config);
        let client = Client::from_conf(s3_config);

        info!(bucket = %config.bucket, "Initialized S3 storage");

        Ok(Self { client, config })
    }

    /// Build the `aws_sdk_s3::Config` for `config`, layered on top of an
    /// ambient `SdkConfig` (from the default provider chain, or an
    /// explicitly-constructed one in tests). Applies the configured region
    /// and custom endpoint, both of which are otherwise silently ignored by
    /// the SDK.
    fn build_client_config(
        aws_config: &aws_config::SdkConfig,
        config: &S3Config,
    ) -> aws_sdk_s3::Config {
        let mut s3_config = aws_sdk_s3::config::Builder::from(aws_config);

        if let Some(region) = &config.region {
            s3_config = s3_config.region(Region::new(region.clone()));
        }

        if let Some(endpoint) = &config.endpoint {
            s3_config = s3_config.endpoint_url(endpoint);
            s3_config = s3_config.force_path_style(true);
        }

        s3_config.build()
    }

    /// Create from an existing AWS SDK client.
    pub fn from_client(client: Client, config: S3Config) -> Self {
        Self { client, config }
    }

    /// Get the full S3 key for a path.
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

    /// Get a temporary/presigned URL using the configured
    /// [`S3Config::presigned_url_duration`] as the expiration when the caller
    /// doesn't need to override it.
    pub async fn temporary_url_default(&self, key: &str) -> Result<Option<String>> {
        let duration = self.config.presigned_url_duration;
        Storage::temporary_url(self, key, duration).await
    }

    /// Get the public URL for a key (if bucket is public).
    pub fn public_url(&self, key: &str) -> String {
        let full_key = self.full_key(key);
        if let Some(endpoint) = &self.config.endpoint {
            format!("{}/{}/{}", endpoint, self.config.bucket, full_key)
        } else if let Some(region) = &self.config.region {
            format!(
                "https://{}.s3.{}.amazonaws.com/{}",
                self.config.bucket, region, full_key
            )
        } else {
            format!(
                "https://{}.s3.amazonaws.com/{}",
                self.config.bucket, full_key
            )
        }
    }
}

#[async_trait]
impl Storage for S3Storage {
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

        // Build put request
        let mut request = self
            .client
            .put_object()
            .bucket(&self.config.bucket)
            .key(&full_key)
            .body(ByteStream::from(data))
            .content_type(content_type);

        // Set ACL if configured
        if let Some(acl) = &self.config.default_acl {
            request = request.acl(ObjectCannedAcl::from(acl.as_str()));
        }

        // Set encryption if configured
        if let Some(encryption) = &self.config.server_side_encryption {
            request =
                request.server_side_encryption(ServerSideEncryption::from(encryption.as_str()));
        }

        // Set storage class if configured
        if let Some(storage_class) = &self.config.storage_class {
            request = request.storage_class(StorageClass::from(storage_class.as_str()));
        }

        // Execute upload
        request
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        debug!(key = %key, bucket = %self.config.bucket, size = size, "Uploaded to S3");

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

        let response = self
            .client
            .get_object()
            .bucket(&self.config.bucket)
            .key(&full_key)
            .send()
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("NoSuchKey") {
                    StorageError::NotFound(key.to_string())
                } else {
                    StorageError::Storage(err_str)
                }
            })?;

        let bytes = response
            .body
            .collect()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        Ok(bytes.into_bytes())
    }

    async fn head(&self, key: &str) -> Result<StorageMetadata> {
        let full_key = self.full_key(key);

        let response = self
            .client
            .head_object()
            .bucket(&self.config.bucket)
            .key(&full_key)
            .send()
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("NotFound") {
                    StorageError::NotFound(key.to_string())
                } else {
                    StorageError::Storage(err_str)
                }
            })?;

        let size = response.content_length().unwrap_or(0) as u64;
        let mut metadata = StorageMetadata::new(key, size).with_url(self.public_url(key));

        if let Some(ct) = response.content_type() {
            metadata = metadata.with_content_type(ct);
        }

        if let Some(etag) = response.e_tag() {
            metadata = metadata.with_checksum(etag.trim_matches('"'));
        }

        Ok(metadata)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let full_key = self.full_key(key);

        self.client
            .delete_object()
            .bucket(&self.config.bucket)
            .key(&full_key)
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        debug!(key = %key, bucket = %self.config.bucket, "Deleted from S3");
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

        let mut request = self.client.list_objects_v2().bucket(&self.config.bucket);

        if !full_prefix.is_empty() {
            request = request.prefix(&full_prefix);
        }

        let response = request
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        let mut results = Vec::new();

        for object in response.contents() {
            if let Some(key) = object.key() {
                // Remove prefix to get the relative key
                let relative_key = if let Some(p) = &self.config.storage.path_prefix {
                    key.strip_prefix(&format!("{}/", p))
                        .unwrap_or(key)
                        .to_string()
                } else {
                    key.to_string()
                };

                let size = object.size().unwrap_or(0) as u64;
                let mut metadata = StorageMetadata::new(&relative_key, size)
                    .with_url(self.public_url(&relative_key));

                if let Some(etag) = object.e_tag() {
                    metadata = metadata.with_checksum(etag.trim_matches('"'));
                }

                results.push(metadata);
            }
        }

        Ok(results)
    }

    async fn copy(&self, from: &str, to: &str) -> Result<StorageMetadata> {
        let from_key = self.full_key(from);
        let to_key = self.full_key(to);

        self.client
            .copy_object()
            .bucket(&self.config.bucket)
            .copy_source(format!("{}/{}", self.config.bucket, from_key))
            .key(&to_key)
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(self.public_url(key)))
    }

    async fn temporary_url(&self, key: &str, expires_in: Duration) -> Result<Option<String>> {
        let full_key = self.full_key(key);

        let presigning_config = aws_sdk_s3::presigning::PresigningConfig::builder()
            .expires_in(expires_in)
            .build()
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        let presigned = self
            .client
            .get_object()
            .bucket(&self.config.bucket)
            .key(&full_key)
            .presigned(presigning_config)
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        Ok(Some(presigned.uri().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_testkit::http_stub::{StubResponse, StubServer};
    use aws_sdk_s3::config::{Credentials, SharedCredentialsProvider};

    /// `S3Config::region` was documented but never applied to the client in
    /// `new()` (it was only used for `public_url` string formatting), so the
    /// SDK silently talked to the ambient region while URLs claimed a
    /// different one. Regression test for `S3Storage::build_client_config`,
    /// the helper `new()` now uses.
    #[test]
    fn region_is_applied_to_built_client_config() {
        let ambient = aws_config::SdkConfig::builder().build();
        let config = S3Config::new("test-bucket").region("eu-west-3");

        let built = S3Storage::build_client_config(&ambient, &config);

        assert_eq!(built.region().map(|r| r.as_ref()), Some("eu-west-3"));
    }

    /// A stub S3-compatible server, wired up with a client built the same
    /// way `S3Storage::new` builds one (region + endpoint override applied),
    /// so we can assert on the real signed request that goes out.
    async fn stub_s3_storage(server: &StubServer, config: S3Config) -> S3Storage {
        let ambient = aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .credentials_provider(SharedCredentialsProvider::new(Credentials::for_tests()))
            .build();
        let config = config.endpoint(server.url());
        let s3_config = S3Storage::build_client_config(&ambient, &config);
        let client = Client::from_conf(s3_config);
        S3Storage::from_client(client, config)
    }

    #[tokio::test]
    async fn region_is_reflected_in_the_signed_request() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage =
            stub_s3_storage(&server, S3Config::new("test-bucket").region("sa-east-1")).await;

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("put should succeed against the stub server");

        let rec = server.assert_received("PUT", "/test-bucket/key.txt");
        let auth = rec
            .header("authorization")
            .expect("SigV4 request must carry an Authorization header");
        assert!(
            auth.contains("sa-east-1/s3/aws4_request"),
            "expected the configured region in the SigV4 credential scope, got: {auth}"
        );
    }

    /// `S3Config::storage_class` was documented but never read by
    /// `put_with_content_type`, so objects always used the bucket default.
    #[tokio::test]
    async fn storage_class_is_sent_on_put() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage =
            stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;
        let mut config = storage.config.clone();
        config.storage_class = Some("GLACIER".to_string());
        let storage = S3Storage::from_client(storage.client, config);

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("put should succeed against the stub server");

        let rec = server.assert_received("PUT", "/test-bucket/key.txt");
        assert_eq!(rec.header("x-amz-storage-class"), Some("GLACIER"));
    }

    /// `S3Config::presigned_url_duration` was documented as the default
    /// presigned-URL lifetime but was read nowhere; `temporary_url` only
    /// ever used the caller-supplied `expires_in`. `temporary_url_default`
    /// wires the configured duration in as the default.
    #[tokio::test]
    async fn temporary_url_default_uses_configured_duration() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage = stub_s3_storage(
            &server,
            S3Config::new("test-bucket")
                .region("us-east-1")
                .presigned_duration(Duration::from_secs(120)),
        )
        .await;

        let url = storage
            .temporary_url_default("key.txt")
            .await
            .unwrap()
            .expect("presigned url should be produced");

        assert!(
            url.contains("X-Amz-Expires=120"),
            "expected the configured 120s duration in the presigned URL, got: {url}"
        );
    }

    #[tokio::test]
    async fn put_without_storage_class_omits_the_header() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage =
            stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;

        storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect("put should succeed against the stub server");

        let rec = server.assert_received("PUT", "/test-bucket/key.txt");
        assert_eq!(rec.header("x-amz-storage-class"), None);
    }
}
