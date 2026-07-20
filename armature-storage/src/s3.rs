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
    calculate_checksum, generate_unique_key, sanitize_filename,
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
    ///
    /// `UploadedFile::name` comes straight off the client-controlled multipart
    /// `filename` header, so it is sanitized before it becomes an object key --
    /// exactly as the local backend does. Taking it verbatim let a filename of
    /// `../../secrets/key` become the object key as written.
    fn generate_key(&self, original_name: Option<&str>) -> String {
        if self.config.storage.generate_unique_names {
            generate_unique_key(original_name, self.config.storage.preserve_extension)
        } else {
            original_name
                .map(sanitize_filename)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        }
    }

    /// Get the public URL for a key (if bucket is public).
    ///
    /// `key` is validated (rejecting `..` traversal segments) and
    /// percent-encoded before being interpolated into the URL, so it cannot
    /// escape the intended path or inject a query, a fragment, or a header
    /// break.
    pub fn public_url(&self, key: &str) -> Result<String> {
        let full_key = crate::storage::encode_key_for_url(&self.full_key(key))?;
        Ok(if let Some(endpoint) = &self.config.endpoint {
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
        })
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

        // Set ACL if configured. `From<&str>` silently yields an `Unknown`
        // variant for typos and forwards them to S3 verbatim, so parse
        // strictly and surface a configuration error instead.
        if let Some(acl) = &self.config.default_acl {
            request = request.acl(
                ObjectCannedAcl::try_parse(acl.as_str())
                    .map_err(|_| StorageError::Config(format!("unknown S3 ACL: {acl:?}")))?,
            );
        }

        // Set encryption if configured
        if let Some(encryption) = &self.config.server_side_encryption {
            request = request.server_side_encryption(
                ServerSideEncryption::try_parse(encryption.as_str()).map_err(|_| {
                    StorageError::Config(format!(
                        "unknown S3 server-side encryption: {encryption:?}"
                    ))
                })?,
            );
        }

        // Set storage class if configured
        if let Some(storage_class) = &self.config.storage_class {
            request = request.storage_class(
                StorageClass::try_parse(storage_class.as_str()).map_err(|_| {
                    StorageError::Config(format!("unknown S3 storage class: {storage_class:?}"))
                })?,
            );
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
        let mut metadata = StorageMetadata::new(key, size).with_url(self.public_url(key)?);

        if let Some(ct) = response.content_type() {
            metadata = metadata.with_content_type(ct);
        }

        if let Some(etag) = response.e_tag() {
            metadata = metadata.with_checksum(etag.trim_matches('"'));
        }

        Ok(metadata)
    }

    /// Delete an object.
    ///
    /// `DeleteObject` succeeds for a key that is not there, which is the
    /// idempotent semantic the [`Storage::delete`] contract adopts for every
    /// backend.
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
            .client
            .list_objects_v2()
            .bucket(&self.config.bucket)
            .max_keys(limit.min(i32::MAX as usize) as i32)
            .set_continuation_token(cursor.map(String::from));

        if !full_prefix.is_empty() {
            request = request.prefix(&full_prefix);
        }

        let page = request
            .send()
            .await
            .map_err(|e| StorageError::Storage(e.to_string()))?;

        // Hoisted out of the per-object loop: this was reallocated for every
        // key in the bucket.
        let strip = self
            .config
            .storage
            .path_prefix
            .as_ref()
            .map(|p| format!("{p}/"));

        let mut results = Vec::with_capacity(page.contents().len());

        for object in page.contents() {
            let Some(key) = object.key() else { continue };

            // Remove prefix to get the relative key
            let relative_key = match &strip {
                Some(strip) => key.strip_prefix(strip.as_str()).unwrap_or(key).to_string(),
                None => key.to_string(),
            };

            let size = object.size().unwrap_or(0) as u64;
            let mut metadata =
                StorageMetadata::new(&relative_key, size).with_url(self.public_url(&relative_key)?);

            if let Some(etag) = object.e_tag() {
                metadata = metadata.with_checksum(etag.trim_matches('"'));
            }

            results.push(metadata);
        }

        // S3 caps a single `list_objects_v2` response at 1000 keys and signals
        // more via `IsTruncated`/`NextContinuationToken`.
        let next = page
            .next_continuation_token()
            .filter(|t| !t.is_empty())
            .map(String::from);

        Ok((results, next))
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
            .map_err(|e| {
                // A missing source is `NotFound` on every backend, so
                // `is_not_found()` means the same thing everywhere. (Only
                // `delete` is idempotent.)
                let err_str = e.to_string();
                if err_str.contains("NoSuchKey") || err_str.contains("NotFound") {
                    StorageError::NotFound(from.to_string())
                } else {
                    StorageError::Storage(err_str)
                }
            })?;

        self.head(to).await
    }

    async fn url(&self, key: &str) -> Result<Option<String>> {
        Ok(Some(self.public_url(key)?))
    }

    fn default_url_duration(&self) -> Duration {
        self.config.presigned_url_duration
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
    use std::sync::{Arc, Mutex};

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

    /// `StorageClass::from(&str)` maps anything at all to an `Unknown` variant
    /// which is then forwarded to S3 verbatim; a typo silently became a
    /// server-side error (or worse, was accepted). Configuration is now parsed
    /// strictly.
    #[tokio::test]
    async fn unknown_storage_class_is_rejected_as_a_config_error() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage =
            stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;
        let mut config = storage.config.clone();
        config.storage_class = Some("NOT_A_CLASS".to_string());
        let storage = S3Storage::from_client(storage.client, config);

        let err = storage
            .put_with_content_type("key.txt", Bytes::from("hi"), "text/plain")
            .await
            .expect_err("an unknown storage class must be rejected");

        assert!(
            matches!(&err, StorageError::Config(m) if m.contains("NOT_A_CLASS")),
            "expected a Config error naming the bad value, got: {err:?}"
        );
        assert!(
            server.requests().is_empty(),
            "the request must not be sent at all"
        );
    }

    #[tokio::test]
    async fn unknown_acl_and_encryption_are_rejected_as_config_errors() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let base = stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;

        let mut config = base.config.clone();
        config.default_acl = Some("not-an-acl".to_string());
        let storage = S3Storage::from_client(base.client.clone(), config);
        assert!(matches!(
            storage
                .put_with_content_type("k", Bytes::from("hi"), "text/plain")
                .await,
            Err(StorageError::Config(_))
        ));

        let mut config = base.config.clone();
        config.server_side_encryption = Some("ROT13".to_string());
        let storage = S3Storage::from_client(base.client, config);
        assert!(matches!(
            storage
                .put_with_content_type("k", Bytes::from("hi"), "text/plain")
                .await,
            Err(StorageError::Config(_))
        ));
    }

    /// A minimal raw-TCP S3 stub that can script *different* responses for
    /// successive `ListObjectsV2` calls. The shared `StubServer` routes only on
    /// (method, path), and both pagination requests are `GET /test-bucket`
    /// differing solely in the `continuation-token` query parameter.
    struct ListStub {
        base_url: String,
        request_queries: Arc<Mutex<Vec<String>>>,
        handle: tokio::task::JoinHandle<()>,
    }

    impl Drop for ListStub {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    fn list_page(keys: &[&str], next_token: Option<&str>) -> String {
        let contents: String = keys
            .iter()
            .map(|k| {
                format!(
                    "<Contents><Key>{k}</Key><Size>3</Size><ETag>&quot;{k}-etag&quot;</ETag>\
                     <LastModified>2024-01-01T00:00:00.000Z</LastModified></Contents>"
                )
            })
            .collect();
        let truncation = match next_token {
            Some(t) => format!(
                "<IsTruncated>true</IsTruncated><NextContinuationToken>{t}</NextContinuationToken>"
            ),
            None => "<IsTruncated>false</IsTruncated>".to_string(),
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
<Name>test-bucket</Name><Prefix></Prefix><MaxKeys>1</MaxKeys>
<KeyCount>{}</KeyCount>{truncation}{contents}
</ListBucketResult>"#,
            keys.len()
        )
    }

    /// Serve a truncated first page, then the final page once the client sends
    /// the continuation token back.
    async fn start_list_stub() -> ListStub {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let request_queries = Arc::new(Mutex::new(Vec::new()));
        let queries = request_queries.clone();

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    continue;
                };
                let queries = queries.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    // Read just the request head; these are bodiless GETs.
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }

                    let head = String::from_utf8_lossy(&buf).into_owned();
                    let target = head
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or_default()
                        .to_string();
                    queries.lock().unwrap().push(target.clone());

                    let body = if target.contains("continuation-token") {
                        list_page(&["page2.txt"], None)
                    } else {
                        list_page(&["page1.txt"], Some("TOKEN-2"))
                    };

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/xml\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.flush().await;
                });
            }
        });

        ListStub {
            base_url,
            request_queries,
            handle,
        }
    }

    /// `list` issued a single `list_objects_v2` call and never read
    /// `IsTruncated`, so any bucket with more than 1000 objects was silently
    /// truncated -- while the Azure and GCS backends behind the same `Storage`
    /// trait returned everything.
    #[tokio::test]
    async fn list_follows_the_continuation_token_across_pages() {
        let stub = start_list_stub().await;

        let ambient = aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .credentials_provider(SharedCredentialsProvider::new(Credentials::for_tests()))
            .build();
        let config = S3Config::new("test-bucket")
            .region("us-east-1")
            .endpoint(stub.base_url.clone());
        let client = Client::from_conf(S3Storage::build_client_config(&ambient, &config));
        let storage = S3Storage::from_client(client, config);

        let keys: Vec<String> = storage
            .list(None)
            .await
            .expect("list should succeed against the paginating stub")
            .into_iter()
            .map(|m| m.key)
            .collect();

        assert_eq!(
            keys,
            vec!["page1.txt".to_string(), "page2.txt".to_string()],
            "both pages must be returned"
        );

        let queries = stub.request_queries.lock().unwrap().clone();
        assert_eq!(
            queries.len(),
            2,
            "expected two list calls, got: {queries:?}"
        );
        assert!(
            queries[1].contains("continuation-token"),
            "the second call must carry the continuation token, got: {}",
            queries[1]
        );
    }

    /// `put_file` sanitized the client-supplied filename on the local backend
    /// only; S3/GCS/Azure used it verbatim, so a multipart `filename` of
    /// `../../secrets/key` became the object key as written -- while the
    /// README advertised traversal rejection as a trait-level guarantee.
    #[tokio::test]
    async fn put_file_sanitizes_a_path_bearing_filename() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let base = stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;
        let mut config = base.config.clone();
        config.storage.generate_unique_names = false;
        let storage = S3Storage::from_client(base.client, config);

        assert_eq!(storage.generate_key(Some("../../secrets/key")), "key");
        assert_eq!(storage.generate_key(Some("/etc/shadow")), "shadow");
        assert_eq!(storage.generate_key(Some("ok.txt")), "ok.txt");

        // ...and that is what actually goes on the wire.
        let file = UploadedFile::from_bytes(Bytes::from_static(b"x"), "../../secrets/key");
        let metadata = storage
            .put_file(&file)
            .await
            .expect("put_file should succeed");

        assert_eq!(metadata.key, "key");
        server.assert_received("PUT", "/test-bucket/key");
        assert!(
            !server.requests().iter().any(|r| r.path.contains("secrets")),
            "the traversal path must never reach the bucket: {:?}",
            server.requests()
        );
    }

    /// `DeleteObject` is idempotent server-side, which is the semantic the
    /// `Storage::delete` contract now standardizes on for all four backends.
    #[tokio::test]
    async fn delete_of_a_missing_key_is_ok() {
        let server = StubServer::start_single(StubResponse::new(204, "")).await;
        let storage =
            stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;

        storage
            .delete("never-existed.txt")
            .await
            .expect("deleting a missing key must be Ok(()), not NotFound");
        server.assert_received("DELETE", "/test-bucket/never-existed.txt");
    }

    /// `list` now pages through `list_page`, which asks for one page at a time
    /// and hands back the continuation token, so a caller can stop early
    /// instead of draining a 5M-object bucket into a single `Vec`.
    #[tokio::test]
    async fn list_page_returns_one_page_and_its_continuation_token() {
        let stub = start_list_stub().await;

        let ambient = aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .credentials_provider(SharedCredentialsProvider::new(Credentials::for_tests()))
            .build();
        let config = S3Config::new("test-bucket")
            .region("us-east-1")
            .endpoint(stub.base_url.clone());
        let client = Client::from_conf(S3Storage::build_client_config(&ambient, &config));
        let storage = S3Storage::from_client(client, config);

        let (page, next) = storage
            .list_page(None, None, 1)
            .await
            .expect("list_page should succeed");

        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["page1.txt"],
            "only the first page must be fetched"
        );
        assert_eq!(next.as_deref(), Some("TOKEN-2"));
        assert_eq!(
            stub.request_queries.lock().unwrap().len(),
            1,
            "list_page must issue exactly one request"
        );

        // Feeding the cursor back yields the final page and no continuation.
        let (page, next) = storage
            .list_page(None, next.as_deref(), 1)
            .await
            .expect("the second page should succeed");
        assert_eq!(
            page.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(),
            ["page2.txt"]
        );
        assert_eq!(next, None);
    }

    /// `url`/`public_url` used to interpolate the raw key into the URL
    /// string: a key of `../../admin` walked out of the intended path, and one
    /// containing `?`, `#` or CRLF injected a query, a fragment, or a header
    /// break.
    #[tokio::test]
    async fn url_rejects_traversal_and_percent_encodes_special_characters() {
        let server = StubServer::start_single(StubResponse::new(200, "")).await;
        let storage =
            stub_s3_storage(&server, S3Config::new("test-bucket").region("us-east-1")).await;

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
