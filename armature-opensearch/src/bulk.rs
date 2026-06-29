//! Bulk operations with streaming support.

use crate::{document::Document, error::Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Bulk operation type.
#[derive(Debug, Clone)]
pub enum BulkOperation<T> {
    /// Index a document.
    Index {
        /// Document ID.
        id: String,
        /// Document data.
        doc: T,
    },
    /// Create a document (fail if exists).
    Create {
        /// Document ID.
        id: String,
        /// Document data.
        doc: T,
    },
    /// Update a document.
    Update {
        /// Document ID.
        id: String,
        /// Partial document.
        doc: T,
    },
    /// Delete a document.
    Delete {
        /// Document ID.
        id: String,
    },
}

impl<T: Document> BulkOperation<T> {
    /// Convert to bulk request lines.
    pub fn to_bulk_lines(&self) -> Result<Vec<Value>> {
        let index = T::index_name();

        match self {
            BulkOperation::Index { id, doc } => Ok(vec![
                json!({ "index": { "_index": index, "_id": id } }),
                serde_json::to_value(doc)?,
            ]),
            BulkOperation::Create { id, doc } => Ok(vec![
                json!({ "create": { "_index": index, "_id": id } }),
                serde_json::to_value(doc)?,
            ]),
            BulkOperation::Update { id, doc } => Ok(vec![
                json!({ "update": { "_index": index, "_id": id } }),
                json!({ "doc": doc }),
            ]),
            BulkOperation::Delete { id } => {
                Ok(vec![json!({ "delete": { "_index": index, "_id": id } })])
            }
        }
    }
}

/// Bulk operation response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkResponse {
    /// Time taken in milliseconds.
    pub took: u64,
    /// Whether there were errors.
    pub errors: bool,
    /// Individual item results.
    pub items: Vec<BulkItem>,
}

/// Individual bulk item result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkItem {
    /// Operation type.
    #[serde(flatten)]
    pub operation: BulkItemResult,
}

/// Bulk item result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BulkItemResult {
    /// Index result.
    Index(BulkItemStatus),
    /// Create result.
    Create(BulkItemStatus),
    /// Update result.
    Update(BulkItemStatus),
    /// Delete result.
    Delete(BulkItemStatus),
}

/// Status of a bulk item operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkItemStatus {
    /// Index name.
    #[serde(rename = "_index")]
    pub index: String,
    /// Document ID.
    #[serde(rename = "_id")]
    pub id: String,
    /// Document version.
    #[serde(rename = "_version")]
    pub version: Option<i64>,
    /// Result status.
    pub result: Option<String>,
    /// HTTP status code.
    pub status: u16,
    /// Error details.
    pub error: Option<BulkItemError>,
}

/// Bulk item error details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkItemError {
    /// Error type.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Error reason.
    pub reason: String,
}

impl BulkItemStatus {
    /// Check if the operation was successful.
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}
