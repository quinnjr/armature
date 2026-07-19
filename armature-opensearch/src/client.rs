//! OpenSearch client implementation.

use crate::{
    config::OpenSearchConfig,
    document::Document,
    error::{OpenSearchError, Result},
    index::IndexManager,
    search::SearchBuilder,
};
use armature_log::{debug, info};
use opensearch::{
    OpenSearch,
    http::transport::{SingleNodeConnectionPool, TransportBuilder},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Arc;

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

        let conn_pool = SingleNodeConnectionPool::new(url);
        let mut builder = TransportBuilder::new(conn_pool);

        // Configure timeouts
        builder = builder.timeout(config.request_timeout).disable_proxy();

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

        let status = response.status_code();
        let body: Value = response.json().await?;

        if !status.is_success() {
            return Err(OpenSearchError::Internal(
                body.get("error")
                    .and_then(|e| e.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("Unknown error")
                    .to_string(),
            ));
        }

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

        let status = response.status_code();
        let body: Value = response.json().await?;

        if !status.is_success() {
            return Err(OpenSearchError::Internal(
                body.get("error")
                    .and_then(|e| e.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("Unknown error")
                    .to_string(),
            ));
        }

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

        let status = response.status_code();

        if status == opensearch::http::StatusCode::NOT_FOUND {
            return Err(OpenSearchError::DocumentNotFound {
                index: index.to_string(),
                id: id.to_string(),
            });
        }

        if !status.is_success() {
            let body: Value = response.json().await?;
            return Err(OpenSearchError::Internal(
                body.get("error")
                    .and_then(|e| e.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("Unknown error")
                    .to_string(),
            ));
        }

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

        let status = response.status_code();

        if status == opensearch::http::StatusCode::NOT_FOUND {
            return Err(OpenSearchError::DocumentNotFound {
                index: index.to_string(),
                id: id.to_string(),
            });
        }

        if !status.is_success() {
            let body: Value = response.json().await?;
            return Err(OpenSearchError::Internal(
                body.get("error")
                    .and_then(|e| e.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("Unknown error")
                    .to_string(),
            ));
        }

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

        let status = response.status_code();

        if status == opensearch::http::StatusCode::NOT_FOUND {
            return Ok(false);
        }

        if !status.is_success() {
            let body: Value = response.json().await?;
            return Err(OpenSearchError::Internal(
                body.get("error")
                    .and_then(|e| e.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("Unknown error")
                    .to_string(),
            ));
        }

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

        let body: Value = response.json().await?;
        let deleted = body["deleted"].as_u64().unwrap_or(0);

        Ok(deleted)
    }

    // =========================================================================
    // Bulk Operations
    // =========================================================================

    /// Bulk index documents.
    pub async fn bulk_index<T: Document>(&self, docs: Vec<(String, T)>) -> Result<usize> {
        if docs.is_empty() {
            return Ok(0);
        }

        let index = T::index_name();
        debug!("Bulk indexing {} documents in index {}", docs.len(), index);

        // Build body as Vec of BulkOperation bytes
        let mut body: Vec<opensearch::http::request::JsonBody<Value>> =
            Vec::with_capacity(docs.len() * 2);
        for (id, doc) in &docs {
            body.push(json!({ "index": { "_index": index, "_id": id } }).into());
            body.push(serde_json::to_value(doc)?.into());
        }

        let response = self
            .client
            .bulk(opensearch::BulkParts::None)
            .body(body)
            .send()
            .await?;

        let result: Value = response.json().await?;

        if result["errors"].as_bool().unwrap_or(false) {
            let items = result["items"].as_array();
            let mut errors = Vec::new();
            let mut failed = 0;

            if let Some(items) = items {
                for item in items {
                    if let Some(error) = item["index"]["error"].as_object() {
                        failed += 1;
                        errors.push(
                            error
                                .get("reason")
                                .and_then(|r| r.as_str())
                                .unwrap_or("Unknown error")
                                .to_string(),
                        );
                    }
                }
            }

            return Err(OpenSearchError::BulkError {
                succeeded: docs.len() - failed,
                failed,
                errors,
            });
        }

        Ok(docs.len())
    }

    /// Bulk delete documents.
    pub async fn bulk_delete<T: Document>(&self, ids: Vec<String>) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }

        let index = T::index_name();
        debug!("Bulk deleting {} documents from index {}", ids.len(), index);

        // Build body as Vec of BulkOperation bytes
        let mut body: Vec<opensearch::http::request::JsonBody<Value>> =
            Vec::with_capacity(ids.len());
        for id in &ids {
            body.push(json!({ "delete": { "_index": index, "_id": id } }).into());
        }

        let response = self
            .client
            .bulk(opensearch::BulkParts::None)
            .body(body)
            .send()
            .await?;

        let result: Value = response.json().await?;

        if result["errors"].as_bool().unwrap_or(false) {
            let items = result["items"].as_array();
            let mut errors = Vec::new();
            let mut failed = 0;

            if let Some(items) = items {
                for item in items {
                    if let Some(error) = item["delete"]["error"].as_object() {
                        failed += 1;
                        errors.push(
                            error
                                .get("reason")
                                .and_then(|r| r.as_str())
                                .unwrap_or("Unknown error")
                                .to_string(),
                        );
                    }
                }
            }

            return Err(OpenSearchError::BulkError {
                succeeded: ids.len() - failed,
                failed,
                errors,
            });
        }

        Ok(ids.len())
    }

    // =========================================================================
    // Utility Methods
    // =========================================================================

    /// Refresh an index to make recent changes searchable.
    pub async fn refresh(&self, index: &str) -> Result<()> {
        debug!("Refreshing index {}", index);

        self.client
            .indices()
            .refresh(opensearch::indices::IndicesRefreshParts::Index(&[index]))
            .send()
            .await?;

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

        Ok(response.json().await?)
    }

    /// Ping the cluster.
    pub async fn ping(&self) -> Result<bool> {
        let response = self.client.ping().send().await;
        Ok(response.is_ok())
    }
}

impl std::fmt::Debug for OpenSearchClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSearchClient")
            .field("urls", &self.config.urls)
            .finish()
    }
}
