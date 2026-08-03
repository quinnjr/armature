//! Bulk operation types.
//!
//! [`BulkOperation`] describes a single mixed index/create/update/delete
//! operation and [`BulkOperation::to_bulk_lines`] renders it into the NDJSON
//! action/data line pairs OpenSearch's `_bulk` API expects. Use
//! [`crate::OpenSearchClient::bulk_execute`] to send a batch of them.

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
        /// Full replacement document (sent under the `doc` key).
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

/// Upper bound on the number of documents [`crate::OpenSearchClient::bulk_index`]
/// and [`crate::OpenSearchClient::bulk_delete`] will place in a single `_bulk`
/// request.
///
/// `_bulk` is rejected *wholesale* when the request body exceeds the cluster's
/// `http.max_content_length` -- no document in an oversized request is applied,
/// not just the overflow -- so the client bounds each request rather than
/// letting the caller's batch size decide. This count bound complements
/// [`BULK_MAX_BYTES_PER_REQUEST`]: very small documents would otherwise let one
/// request carry an unbounded number of items, which costs coordinator-node
/// memory even when the body is small.
pub const BULK_MAX_DOCS_PER_REQUEST: usize = 500;

/// Approximate upper bound, in bytes, on the NDJSON body of a single `_bulk`
/// request issued by [`crate::OpenSearchClient::bulk_index`] and
/// [`crate::OpenSearchClient::bulk_delete`].
///
/// OpenSearch's `http.max_content_length` defaults to 100 MB. 8 MiB is
/// deliberately far below that so the batch still fits when the cluster has
/// lowered the setting, and when a reverse proxy or load balancer in front of
/// it imposes its own (usually smaller) body limit. It also sits in the 5-15 MB
/// range that OpenSearch recommends for bulk batches, above which a single
/// request starts to hurt indexing throughput rather than help it.
///
/// The bound is approximate because it is measured against the serialized
/// documents alone, ignoring the NDJSON newlines and HTTP framing; the margin
/// below `http.max_content_length` absorbs the difference.
pub const BULK_MAX_BYTES_PER_REQUEST: usize = 8 * 1024 * 1024;

/// Split a run of per-document sizes into contiguous request-sized chunks
/// bounded by both [`BULK_MAX_DOCS_PER_REQUEST`] and
/// [`BULK_MAX_BYTES_PER_REQUEST`].
///
/// `sizes[i]` is the approximate serialized byte size of document `i`'s
/// contribution to the request body. The returned ranges cover `0..sizes.len()`
/// in order, without gaps or overlap.
///
/// A document larger than the byte bound on its own is placed in a chunk by
/// itself rather than being merged into a neighbour: the client cannot make it
/// fit, and isolating it means the server's rejection takes down one document
/// instead of a whole batch.
pub(crate) fn bulk_chunk_ranges(sizes: &[usize]) -> Vec<std::ops::Range<usize>> {
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0usize;

    for (i, size) in sizes.iter().enumerate() {
        let len = i - start;
        // `len > 0` keeps an over-large single document from producing an empty
        // chunk followed by an infinitely deferred one.
        if len > 0
            && (len >= BULK_MAX_DOCS_PER_REQUEST || bytes + size > BULK_MAX_BYTES_PER_REQUEST)
        {
            chunks.push(start..i);
            start = i;
            bytes = 0;
        }
        bytes += size;
    }

    if start < sizes.len() {
        chunks.push(start..sizes.len());
    }

    chunks
}

/// Extract the failure reason from one `_bulk` response item, if it failed.
///
/// OpenSearch keys each response item by the action that was actually
/// performed -- `index`, `create`, `update` or `delete` -- so the error has to
/// be read from whichever key the item carries. Looking under a single assumed
/// action key reads every item written under a different one as a success.
pub(crate) fn bulk_item_error(item: &Value) -> Option<String> {
    let error = item
        .as_object()?
        .values()
        .find_map(|result| result.get("error"))
        .filter(|error| !error.is_null())?;

    Some(
        error
            .get("reason")
            .and_then(|reason| reason.as_str())
            // Some errors are reported as a bare string rather than an object.
            .or_else(|| error.as_str())
            .unwrap_or("Unknown error")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_is_bounded_by_document_count() {
        let sizes = vec![1usize; BULK_MAX_DOCS_PER_REQUEST * 2 + 1];
        let chunks = bulk_chunk_ranges(&sizes);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], 0..BULK_MAX_DOCS_PER_REQUEST);
        assert_eq!(
            chunks[1],
            BULK_MAX_DOCS_PER_REQUEST..BULK_MAX_DOCS_PER_REQUEST * 2
        );
        assert_eq!(
            chunks[2],
            BULK_MAX_DOCS_PER_REQUEST * 2..BULK_MAX_DOCS_PER_REQUEST * 2 + 1
        );
    }

    /// Exactly the count bound is one request, not two.
    #[test]
    fn chunking_does_not_split_at_exactly_the_count_bound() {
        let sizes = vec![1usize; BULK_MAX_DOCS_PER_REQUEST];
        assert_eq!(
            bulk_chunk_ranges(&sizes),
            vec![0..BULK_MAX_DOCS_PER_REQUEST]
        );
    }

    #[test]
    fn chunking_is_bounded_by_byte_size() {
        // Four documents of a quarter of the byte bound each fit in one
        // request; a fifth must start a new one.
        let quarter = BULK_MAX_BYTES_PER_REQUEST / 4;
        let sizes = vec![quarter; 5];
        let chunks = bulk_chunk_ranges(&sizes);

        assert_eq!(chunks, vec![0..4, 4..5]);
    }

    /// The byte bound is inclusive: a chunk that lands exactly on it is not
    /// split, but one byte more is.
    #[test]
    fn chunking_splits_one_byte_past_the_bound() {
        let half = BULK_MAX_BYTES_PER_REQUEST / 2;
        assert_eq!(bulk_chunk_ranges(&[half, half]), vec![0..2]);
        assert_eq!(bulk_chunk_ranges(&[half, half + 1]), vec![0..1, 1..2]);
    }

    /// A document that cannot fit the byte bound on its own is isolated rather
    /// than dragging a neighbour into an oversized request.
    #[test]
    fn chunking_isolates_an_oversized_document() {
        let sizes = [1, BULK_MAX_BYTES_PER_REQUEST + 1, 1];
        assert_eq!(bulk_chunk_ranges(&sizes), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn chunking_an_empty_batch_yields_no_requests() {
        assert!(bulk_chunk_ranges(&[]).is_empty());
    }

    #[test]
    fn item_error_is_found_under_the_index_action() {
        let item = json!({ "index": { "_id": "1", "status": 400, "error": { "reason": "boom" } } });
        assert_eq!(bulk_item_error(&item).as_deref(), Some("boom"));
    }

    /// The regression this guards: an error reported under `create`, `update`
    /// or `delete` must not be read as a success just because it is not under
    /// `index`.
    #[test]
    fn item_error_is_found_under_non_index_actions() {
        for action in ["create", "update", "delete"] {
            let item =
                json!({ action: { "_id": "1", "status": 409, "error": { "reason": "boom" } } });
            assert_eq!(
                bulk_item_error(&item).as_deref(),
                Some("boom"),
                "an error under the {action:?} action must be detected"
            );
        }
    }

    #[test]
    fn item_without_an_error_is_a_success() {
        for action in ["index", "create", "update", "delete"] {
            let item = json!({ action: { "_id": "1", "status": 200, "result": "created" } });
            assert!(
                bulk_item_error(&item).is_none(),
                "a successful {action:?} item must not be counted as failed"
            );
        }
    }

    #[test]
    fn item_error_falls_back_when_the_reason_is_missing() {
        let item = json!({ "update": { "_id": "1", "status": 500, "error": {} } });
        assert_eq!(bulk_item_error(&item).as_deref(), Some("Unknown error"));

        let bare = json!({ "update": { "_id": "1", "status": 500, "error": "boom" } });
        assert_eq!(bulk_item_error(&bare).as_deref(), Some("boom"));
    }
}
