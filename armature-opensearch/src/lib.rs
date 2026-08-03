//! OpenSearch integration for the Armature framework.
//!
//! This crate provides a high-level client for OpenSearch with support for:
//! - Document indexing, searching, and management
//! - Index lifecycle management
//! - Mixed bulk operations (index/create/update/delete) via [`OpenSearchClient::bulk_execute`]
//! - Query DSL builder
//! - AWS OpenSearch Service authentication
//!
//! # Example
//!
//! ```rust,no_run
//! use armature_opensearch::{OpenSearchClient, OpenSearchConfig, Document};
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct Article {
//!     title: String,
//!     body: String,
//!     tags: Vec<String>,
//! }
//!
//! impl Document for Article {
//!     fn index_name() -> &'static str {
//!         "articles"
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create client
//!     let config = OpenSearchConfig::new("http://localhost:9200");
//!     let client = OpenSearchClient::new(config)?;
//!
//!     // Index a document
//!     let article = Article {
//!         title: "Hello OpenSearch".to_string(),
//!         body: "Getting started with full-text search.".to_string(),
//!         tags: vec!["tutorial".to_string(), "search".to_string()],
//!     };
//!
//!     client.index("article-1", &article).await?;
//!
//!     // Search
//!     let results: Vec<Article> = client
//!         .search()
//!         .index("articles")
//!         .query_string("hello")
//!         .execute()
//!         .await?;
//!
//!     Ok(())
//! }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]

mod bulk;
mod client;
mod config;
mod document;
mod error;
mod index;
mod query;
mod search;

pub use bulk::{
    BULK_MAX_BYTES_PER_REQUEST, BULK_MAX_DOCS_PER_REQUEST, BulkItem, BulkItemError, BulkItemResult,
    BulkItemStatus, BulkOperation, BulkResponse,
};
pub use client::OpenSearchClient;
pub use config::{OpenSearchConfig, TlsConfig};
pub use document::Document;
pub use error::{OpenSearchError, Result};
pub use index::{FieldType, IndexManager, IndexSettings, Mapping, MappingField};
pub use query::{BoolQuery, MatchQuery, Query, QueryBuilder, RangeQuery, TermQuery};
pub use search::{Aggregation, AggregationResult, Hit, SearchBuilder, SearchResult};

/// Prelude for common imports.
pub mod prelude {
    pub use crate::{
        Document, OpenSearchClient, OpenSearchConfig, OpenSearchError, Query, QueryBuilder, Result,
        SearchBuilder,
    };
}
