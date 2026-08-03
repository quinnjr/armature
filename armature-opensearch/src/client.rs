//! OpenSearch client implementation.

use crate::{
    bulk::{BulkOperation, BulkResponse, bulk_chunk_ranges, bulk_item_error},
    config::OpenSearchConfig,
    document::Document,
    error::{OpenSearchError, Result, error_reason, json_or_error},
    index::IndexManager,
    search::SearchBuilder,
};
use armature_log::{debug, info, warn};
use opensearch::{
    OpenSearch,
    http::transport::{SingleNodeConnectionPool, TransportBuilder},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

/// The NDJSON lines of a single `_bulk` request body, in the shape the
/// `opensearch` crate's `bulk()` builder accepts.
type BulkBody = Vec<opensearch::http::request::JsonBody<Value>>;

/// OpenSearch client for document operations.
#[derive(Clone)]
pub struct OpenSearchClient {
    client: Arc<OpenSearch>,
    config: Arc<OpenSearchConfig>,
}

impl OpenSearchClient {
    /// Create a new OpenSearch client.
    pub fn new(config: OpenSearchConfig) -> Result<Self> {
        info!("Initializing OpenSearch client for: {:?}", config.urls);

        let url = config
            .urls
            .first()
            .ok_or_else(|| OpenSearchError::Validation("No URLs provided".to_string()))?;

        let url = opensearch::http::Url::parse(url)
            .map_err(|e| OpenSearchError::Validation(format!("Invalid URL: {}", e)))?;

        // Refuse to send mTLS client certificates or AWS SigV4 credentials over a
        // non-TLS connection: both are secrets/identity material, and sending them
        // in cleartext over the network defeats the point of authenticating at all.
        // Loopback is exempted so local stub/integration tests (which never speak
        // TLS) keep working.
        #[cfg(feature = "aws-auth")]
        let aws_auth_configured = config.aws_region.is_some();
        #[cfg(not(feature = "aws-auth"))]
        let aws_auth_configured = false;

        if (config.tls.is_some() || aws_auth_configured) && url.scheme() != "https" {
            let host = url.host_str().unwrap_or("");
            let is_loopback =
                host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]";
            if !is_loopback {
                return Err(OpenSearchError::Validation(format!(
                    "config.url '{}' uses scheme '{}', but TLS client certificates and/or AWS \
                     SigV4 credentials must not be sent over a non-https connection; use an \
                     https:// URL (loopback hosts are exempt for local testing)",
                    url,
                    url.scheme()
                )));
            }
        }

        // Basic auth and AWS SigV4 auth are mutually exclusive: the opensearch
        // crate's TransportBuilder can only hold a single Credentials value, so
        // configuring both would silently discard basic auth in favor of SigV4.
        #[cfg(feature = "aws-auth")]
        if config.aws_region.is_some() && config.username.is_some() && config.password.is_some() {
            return Err(OpenSearchError::Validation(
                "Basic auth (username/password) cannot be combined with AWS SigV4 auth \
                 (aws_region): the opensearch crate's TransportBuilder can only hold a single \
                 Credentials value at a time; configure one or the other"
                    .to_string(),
            ));
        }

        let conn_pool = SingleNodeConnectionPool::new(url);
        let mut builder = TransportBuilder::new(conn_pool);

        // Configure timeouts
        builder = builder.timeout(config.request_timeout).disable_proxy();

        // `TransportBuilder` only exposes a single overall request timeout, covering
        // everything from connect until the response body finishes; it has no separate
        // connect-phase timeout. `connect_timeout` therefore can't be applied independently.
        if config.connect_timeout != Duration::from_secs(10) {
            warn!(
                "OpenSearchConfig::connect_timeout ({:?}) is not applied: the opensearch crate's \
                 TransportBuilder has no separate connect-phase timeout, only the combined \
                 `request_timeout` ({:?}) is honored",
                config.connect_timeout, config.request_timeout
            );
        }

        // Response gzip decompression is always-on (the `opensearch` crate is built with
        // reqwest's `gzip` feature) and `TransportBuilder` exposes no way to disable it.
        if !config.compression {
            warn!(
                "OpenSearchConfig::compression = false is not applied: the opensearch crate's \
                 TransportBuilder cannot disable gzip response decompression"
            );
        }

        // Configure basic auth
        if let (Some(user), Some(pass)) = (&config.username, &config.password) {
            builder = builder.auth(opensearch::auth::Credentials::Basic(
                user.clone(),
                pass.clone(),
            ));
        }

        // Configure AWS SigV4 auth for AWS OpenSearch Service / Serverless.
        // Every outgoing request is signed by the `opensearch` crate's own
        // `aws-auth` support (opensearch::http::transport signs via
        // aws-sigv4 immediately before send, using the service name "es").
        #[cfg(feature = "aws-auth")]
        if let Some(region) = &config.aws_region {
            let provider = config.aws_credentials_provider.clone().ok_or_else(|| {
                OpenSearchError::Validation(
                    "aws_region is set but no aws_credentials_provider was configured; \
                     call OpenSearchConfig::with_aws_credentials_provider to sign requests, \
                     or unset aws_region to use unauthenticated/basic auth"
                        .to_string(),
                )
            })?;

            builder = builder.auth(opensearch::auth::Credentials::AwsSigV4(
                provider,
                aws_types::region::Region::new(region.clone()),
            ));
        }

        // Configure TLS: CA trust and mutual-TLS client certificates.
        #[cfg(any(feature = "rustls", feature = "native-tls"))]
        if let Some(tls) = &config.tls {
            use opensearch::{
                auth::{ClientCertificate, Credentials as OsCredentials},
                cert::{Certificate as OsCertificate, CertificateValidation},
            };

            if tls.danger_accept_invalid_certs {
                if tls.ca_cert.is_some() {
                    warn!(
                        "TlsConfig::ca_cert is ignored because danger_accept_invalid_certs is \
                         set; certificate validation is fully disabled"
                    );
                }
                builder = builder.cert_validation(CertificateValidation::None);
            } else if let Some(ca_cert_path) = &tls.ca_cert {
                let pem = std::fs::read(ca_cert_path).map_err(|e| {
                    OpenSearchError::Validation(format!(
                        "Failed to read TlsConfig::ca_cert '{}': {}",
                        ca_cert_path, e
                    ))
                })?;
                let cert = OsCertificate::from_pem(&pem).map_err(|e| {
                    OpenSearchError::Validation(format!(
                        "Invalid TlsConfig::ca_cert PEM at '{}': {}",
                        ca_cert_path, e
                    ))
                })?;
                builder = builder.cert_validation(CertificateValidation::Full(cert));
            }

            if let (Some(cert_path), Some(key_path)) = (&tls.client_cert, &tls.client_key) {
                if config.username.is_some() && config.password.is_some() {
                    return Err(OpenSearchError::Validation(
                        "TlsConfig client_cert/client_key (mTLS) cannot be combined with basic \
                         auth: the opensearch crate's TransportBuilder can only hold a single \
                         Credentials value at a time; configure one or the other"
                            .to_string(),
                    ));
                }

                #[cfg(feature = "aws-auth")]
                if config.aws_region.is_some() {
                    return Err(OpenSearchError::Validation(
                        "TlsConfig client_cert/client_key (mTLS) cannot be combined with AWS \
                         SigV4 auth: the opensearch crate's TransportBuilder can only hold a \
                         single Credentials value at a time; configure one or the other"
                            .to_string(),
                    ));
                }

                let mut pem = std::fs::read(cert_path).map_err(|e| {
                    OpenSearchError::Validation(format!(
                        "Failed to read TlsConfig::client_cert '{}': {}",
                        cert_path, e
                    ))
                })?;
                let mut key_pem = std::fs::read(key_path).map_err(|e| {
                    OpenSearchError::Validation(format!(
                        "Failed to read TlsConfig::client_key '{}': {}",
                        key_path, e
                    ))
                })?;
                pem.push(b'\n');
                pem.append(&mut key_pem);

                builder = builder.auth(OsCredentials::Certificate(ClientCertificate::Pem(pem)));
            }
        }

        #[cfg(not(any(feature = "rustls", feature = "native-tls")))]
        if config.tls.is_some() {
            warn!(
                "OpenSearchConfig::tls is set but neither the `rustls` nor `native-tls` feature \
                 is enabled on armature-opensearch; TLS options are not applied"
            );
        }

        let transport = builder
            .build()
            .map_err(|e| OpenSearchError::Connection(e.to_string()))?;

        let client = OpenSearch::new(transport);

        debug!("OpenSearch client initialized");

        Ok(Self {
            client: Arc::new(client),
            config: Arc::new(config),
        })
    }

    /// Get the underlying OpenSearch client.
    pub fn inner(&self) -> &OpenSearch {
        &self.client
    }

    /// Get the configuration.
    pub fn config(&self) -> &OpenSearchConfig {
        &self.config
    }

    /// Get an index manager for index operations.
    pub fn indices(&self) -> IndexManager {
        IndexManager::new(self.client.clone())
    }

    /// Create a search builder.
    pub fn search(&self) -> SearchBuilder {
        SearchBuilder::new(self.client.clone())
    }

    // =========================================================================
    // Document Operations
    // =========================================================================

    /// Index a document with an explicit ID.
    pub async fn index<T: Document>(&self, id: &str, doc: &T) -> Result<String> {
        let index = T::index_name();
        debug!("Indexing document {} in index {}", id, index);

        let response = self
            .client
            .index(opensearch::IndexParts::IndexId(index, id))
            .body(doc)
            .send()
            .await?;

        let body = json_or_error(response).await?;

        Ok(body["_id"].as_str().unwrap_or(id).to_string())
    }

    /// Index a document with auto-generated ID.
    pub async fn index_auto_id<T: Document>(&self, doc: &T) -> Result<String> {
        let index = T::index_name();
        debug!(
            "Indexing document with auto-generated ID in index {}",
            index
        );

        let response = self
            .client
            .index(opensearch::IndexParts::Index(index))
            .body(doc)
            .send()
            .await?;

        let body = json_or_error(response).await?;

        Ok(body["_id"].as_str().unwrap_or("").to_string())
    }

    /// Get a document by ID.
    pub async fn get<T: Document>(&self, id: &str) -> Result<Option<T>> {
        let index = T::index_name();
        debug!("Getting document {} from index {}", id, index);

        let response = self
            .client
            .get(opensearch::GetParts::IndexId(index, id))
            .send()
            .await?;

        let status = response.status_code();

        if status == opensearch::http::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let body: Value = response.json().await?;

        if !body["found"].as_bool().unwrap_or(false) {
            return Ok(None);
        }

        let source = body
            .get("_source")
            .ok_or_else(|| OpenSearchError::Internal("No _source in response".to_string()))?;

        let doc: T = serde_json::from_value(source.clone())?;
        Ok(Some(doc))
    }

    /// Check if a document exists.
    pub async fn exists<T: Document>(&self, id: &str) -> Result<bool> {
        let index = T::index_name();
        debug!("Checking if document {} exists in index {}", id, index);

        let response = self
            .client
            .exists(opensearch::ExistsParts::IndexId(index, id))
            .send()
            .await?;

        Ok(response.status_code().is_success())
    }

    /// Update a document by ID.
    pub async fn update<T: Document>(&self, id: &str, doc: &T) -> Result<()> {
        let index = T::index_name();
        debug!("Updating document {} in index {}", id, index);

        let response = self
            .client
            .update(opensearch::UpdateParts::IndexId(index, id))
            .body(json!({ "doc": doc }))
            .send()
            .await?;

        if response.status_code() == opensearch::http::StatusCode::NOT_FOUND {
            return Err(OpenSearchError::DocumentNotFound {
                index: index.to_string(),
                id: id.to_string(),
            });
        }

        json_or_error(response).await?;

        Ok(())
    }

    /// Partially update a document using a script or partial doc.
    pub async fn partial_update<T: Document>(
        &self,
        id: &str,
        partial: impl Serialize,
    ) -> Result<()> {
        let index = T::index_name();
        debug!("Partial update of document {} in index {}", id, index);

        let response = self
            .client
            .update(opensearch::UpdateParts::IndexId(index, id))
            .body(json!({ "doc": partial }))
            .send()
            .await?;

        if response.status_code() == opensearch::http::StatusCode::NOT_FOUND {
            return Err(OpenSearchError::DocumentNotFound {
                index: index.to_string(),
                id: id.to_string(),
            });
        }

        json_or_error(response).await?;

        Ok(())
    }

    /// Delete a document by ID.
    pub async fn delete<T: Document>(&self, id: &str) -> Result<bool> {
        let index = T::index_name();
        debug!("Deleting document {} from index {}", id, index);

        let response = self
            .client
            .delete(opensearch::DeleteParts::IndexId(index, id))
            .send()
            .await?;

        if response.status_code() == opensearch::http::StatusCode::NOT_FOUND {
            return Ok(false);
        }

        json_or_error(response).await?;

        Ok(true)
    }

    /// Delete documents by query.
    pub async fn delete_by_query(&self, index: &str, query: Value) -> Result<u64> {
        debug!("Deleting documents by query in index {}", index);

        let response = self
            .client
            .delete_by_query(opensearch::DeleteByQueryParts::Index(&[index]))
            .body(json!({ "query": query }))
            .send()
            .await?;

        let body = json_or_error(response).await?;

        let deleted = body["deleted"].as_u64().unwrap_or(0);

        Ok(deleted)
    }

    // =========================================================================
    // Bulk Operations
    // =========================================================================

    /// Bulk index documents.
    ///
    /// The batch is split into as many `_bulk` requests as it takes to keep
    /// each one under both [`BULK_MAX_DOCS_PER_REQUEST`](crate::BULK_MAX_DOCS_PER_REQUEST)
    /// and [`BULK_MAX_BYTES_PER_REQUEST`](crate::BULK_MAX_BYTES_PER_REQUEST);
    /// see those constants for why the client bounds the request rather than
    /// trusting the caller's batch size.
    ///
    /// Consequently the call is **not atomic**: when it fails, the chunks that
    /// were already accepted stay applied. The returned
    /// [`OpenSearchError::BulkError`] reports the aggregate counts so the
    /// caller can tell how much landed.
    pub async fn bulk_index<T: Document>(&self, docs: Vec<(String, T)>) -> Result<usize> {
        if docs.is_empty() {
            return Ok(0);
        }

        let index = T::index_name();
        debug!("Bulk indexing {} documents in index {}", docs.len(), index);

        // Render every document up front so each chunk's body size is known
        // before it is sent. The action line's size is folded in with it.
        let mut lines: Vec<(Value, Value)> = Vec::with_capacity(docs.len());
        let mut sizes: Vec<usize> = Vec::with_capacity(docs.len());
        for (id, doc) in &docs {
            let action = json!({ "index": { "_index": index, "_id": id } });
            let source = serde_json::to_value(doc)?;
            sizes.push(action.to_string().len() + source.to_string().len());
            lines.push((action, source));
        }

        self.send_bulk_chunks(docs.len(), &sizes, |range| {
            let mut body: BulkBody = Vec::with_capacity(range.len() * 2);
            for (action, source) in &lines[range] {
                body.push(action.clone().into());
                body.push(source.clone().into());
            }
            body
        })
        .await
    }

    /// Bulk delete documents.
    ///
    /// Chunked exactly like [`Self::bulk_index`], with the same non-atomic
    /// failure semantics.
    pub async fn bulk_delete<T: Document>(&self, ids: Vec<String>) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }

        let index = T::index_name();
        debug!("Bulk deleting {} documents from index {}", ids.len(), index);

        let mut lines: Vec<Value> = Vec::with_capacity(ids.len());
        let mut sizes: Vec<usize> = Vec::with_capacity(ids.len());
        for id in &ids {
            let action = json!({ "delete": { "_index": index, "_id": id } });
            sizes.push(action.to_string().len());
            lines.push(action);
        }

        self.send_bulk_chunks(ids.len(), &sizes, |range| {
            lines[range].iter().cloned().map(Into::into).collect()
        })
        .await
    }

    /// Issue one `_bulk` request per chunk of `sizes` and aggregate the
    /// outcomes into a single success count or [`OpenSearchError::BulkError`].
    ///
    /// `build_body` renders the NDJSON lines for one chunk, identified by its
    /// half-open range over the original document order.
    async fn send_bulk_chunks(
        &self,
        total: usize,
        sizes: &[usize],
        build_body: impl Fn(std::ops::Range<usize>) -> BulkBody,
    ) -> Result<usize> {
        let chunks = bulk_chunk_ranges(sizes);
        if chunks.len() > 1 {
            debug!(
                "Splitting {} bulk documents across {} _bulk requests",
                total,
                chunks.len()
            );
        }

        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut errors: Vec<String> = Vec::new();

        for chunk in chunks {
            let chunk_len = chunk.len();

            let response = self
                .client
                .bulk(opensearch::BulkParts::None)
                .body(build_body(chunk))
                .send()
                .await?;

            let status = response.status_code();
            let result: Value = response.json().await?;

            if !status.is_success() {
                // A rejected request (413, 429, 503, ...) takes the whole chunk
                // with it, so every document in it failed. Stop here rather
                // than pushing the remaining chunks at a cluster that just
                // refused one, and report what earlier chunks already applied.
                failed += chunk_len;
                errors.push(error_reason(&result));
                return Err(OpenSearchError::BulkError {
                    succeeded,
                    failed,
                    errors,
                });
            }

            if result["errors"].as_bool().unwrap_or(false) {
                let mut chunk_failed = 0usize;
                for item in result["items"].as_array().into_iter().flatten() {
                    if let Some(reason) = bulk_item_error(item) {
                        chunk_failed += 1;
                        errors.push(reason);
                    }
                }
                failed += chunk_failed;
                succeeded += chunk_len.saturating_sub(chunk_failed);
            } else {
                succeeded += chunk_len;
            }
        }

        if failed > 0 {
            return Err(OpenSearchError::BulkError {
                succeeded,
                failed,
                errors,
            });
        }

        Ok(succeeded)
    }

    /// Execute a batch of mixed bulk operations (index/create/update/delete) in a
    /// single request, in the order given, using [`BulkOperation::to_bulk_lines`]
    /// to render each operation's NDJSON action/data lines.
    ///
    /// Unlike [`Self::bulk_index`] and [`Self::bulk_delete`], this returns the raw
    /// [`BulkResponse`] (including per-item results) rather than failing the whole
    /// call when some items error; inspect `BulkResponse::errors` and
    /// `BulkResponse::items` to see individual outcomes.
    pub async fn bulk_execute<T: Document>(
        &self,
        operations: Vec<BulkOperation<T>>,
    ) -> Result<BulkResponse> {
        if operations.is_empty() {
            return Ok(BulkResponse {
                took: 0,
                errors: false,
                items: Vec::new(),
            });
        }

        debug!("Executing {} bulk operations", operations.len());

        let mut body: Vec<opensearch::http::request::JsonBody<Value>> = Vec::new();
        for op in &operations {
            for line in op.to_bulk_lines()? {
                body.push(line.into());
            }
        }

        let response = self
            .client
            .bulk(opensearch::BulkParts::None)
            .body(body)
            .send()
            .await?;

        let result = json_or_error(response).await?;

        Ok(serde_json::from_value(result)?)
    }

    // =========================================================================
    // Utility Methods
    // =========================================================================

    /// Refresh an index to make recent changes searchable.
    pub async fn refresh(&self, index: &str) -> Result<()> {
        debug!("Refreshing index {}", index);

        let response = self
            .client
            .indices()
            .refresh(opensearch::indices::IndicesRefreshParts::Index(&[index]))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Get cluster health.
    pub async fn health(&self) -> Result<Value> {
        let response = self
            .client
            .cluster()
            .health(opensearch::cluster::ClusterHealthParts::None)
            .send()
            .await?;

        let body = json_or_error(response).await?;

        Ok(body)
    }

    /// Ping the cluster.
    pub async fn ping(&self) -> Result<bool> {
        let response = self.client.ping().send().await?;
        Ok(response.status_code().is_success())
    }
}

impl std::fmt::Debug for OpenSearchClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSearchClient")
            .field("urls", &self.config.urls)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TlsConfig;

    #[cfg(any(feature = "rustls", feature = "native-tls"))]
    #[test]
    fn mtls_conflicts_with_basic_auth() {
        let config = OpenSearchConfig::new("https://localhost:9200")
            .with_basic_auth("user", "pass")
            .with_tls(TlsConfig::default().with_client_cert("cert.pem", "key.pem"));

        let err = OpenSearchClient::new(config)
            .expect_err("mTLS client cert combined with basic auth must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("basic auth") || msg.contains("mTLS"),
            "unexpected error: {msg}"
        );
    }

    #[cfg(all(feature = "aws-auth", any(feature = "rustls", feature = "native-tls")))]
    #[test]
    fn mtls_conflicts_with_sigv4_auth() {
        let provider = aws_credential_types::provider::SharedCredentialsProvider::new(
            aws_credential_types::Credentials::for_tests(),
        );

        let config = OpenSearchConfig::new("https://localhost:9200")
            .with_aws_region("us-east-1")
            .with_aws_credentials_provider(provider)
            .with_tls(TlsConfig::default().with_client_cert("cert.pem", "key.pem"));

        let err = OpenSearchClient::new(config)
            .expect_err("mTLS client cert combined with AWS SigV4 auth must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("SigV4"), "unexpected error: {msg}");
    }

    #[cfg(any(feature = "rustls", feature = "native-tls"))]
    #[test]
    fn ca_cert_missing_file_produces_read_error() {
        let config = OpenSearchConfig::new("https://localhost:9200")
            .with_tls(TlsConfig::with_ca_cert("/no/such/path/ca.pem"));

        let err = OpenSearchClient::new(config)
            .expect_err("a missing ca_cert file must fail client construction");
        let msg = format!("{err}");
        assert!(
            msg.contains("Failed to read"),
            "expected a read-failure message, got: {msg}"
        );
    }

    #[cfg(any(feature = "rustls", feature = "native-tls"))]
    #[test]
    fn tls_over_non_loopback_http_requires_https() {
        let config = OpenSearchConfig::new("http://example.com:9200")
            .with_tls(TlsConfig::with_ca_cert("/no/such/path/ca.pem"));

        let err = OpenSearchClient::new(config)
            .expect_err("TLS config over a non-https, non-loopback URL must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("https"), "unexpected error: {msg}");
    }

    #[cfg(feature = "aws-auth")]
    #[test]
    fn sigv4_over_non_loopback_http_requires_https() {
        let config = OpenSearchConfig::new("http://example.com:9200").with_aws_region("us-east-1");

        let err = OpenSearchClient::new(config)
            .expect_err("AWS SigV4 over a non-https, non-loopback URL must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("https"), "unexpected error: {msg}");
    }

    #[cfg(feature = "aws-auth")]
    #[test]
    fn basic_auth_conflicts_with_sigv4_auth() {
        let provider = aws_credential_types::provider::SharedCredentialsProvider::new(
            aws_credential_types::Credentials::for_tests(),
        );

        let config = OpenSearchConfig::new("https://localhost:9200")
            .with_basic_auth("user", "pass")
            .with_aws_region("us-east-1")
            .with_aws_credentials_provider(provider);

        let err = OpenSearchClient::new(config)
            .expect_err("basic auth combined with AWS SigV4 auth must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("SigV4"), "unexpected error: {msg}");
    }
}
