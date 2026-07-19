//! Search builder and results.

use crate::{
    document::{Document, DocumentMeta, DocumentWithMeta},
    error::{OpenSearchError, Result},
    query::Query,
};
use armature_log::debug;
use opensearch::OpenSearch;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Search builder for constructing and executing searches.
#[derive(Clone)]
pub struct SearchBuilder {
    client: Arc<OpenSearch>,
    indices: Vec<String>,
    query: Option<Value>,
    from: Option<i64>,
    size: Option<i64>,
    sort: Vec<Value>,
    search_after: Option<Vec<Value>>,
    max_size: i64,
    source_includes: Option<Vec<String>>,
    source_excludes: Option<Vec<String>>,
    highlight: Option<Value>,
    aggregations: Option<Value>,
    track_total_hits: Option<bool>,
}

/// Default upper bound applied to `size` unless overridden via
/// [`SearchBuilder::max_size`]. Prevents a caller from requesting an
/// unbounded page.
pub const DEFAULT_MAX_SIZE: i64 = 1000;

impl SearchBuilder {
    /// Create a new search builder.
    pub(crate) fn new(client: Arc<OpenSearch>) -> Self {
        Self {
            client,
            indices: Vec::new(),
            query: None,
            from: None,
            size: None,
            sort: Vec::new(),
            search_after: None,
            max_size: DEFAULT_MAX_SIZE,
            source_includes: None,
            source_excludes: None,
            highlight: None,
            aggregations: None,
            track_total_hits: None,
        }
    }

    /// Set the index to search.
    pub fn index(mut self, index: impl Into<String>) -> Self {
        self.indices.push(index.into());
        self
    }

    /// Set multiple indices to search.
    pub fn indices(mut self, indices: Vec<String>) -> Self {
        self.indices = indices;
        self
    }

    /// Set the query.
    pub fn query(mut self, query: Query) -> Self {
        self.query = Some(query.to_json());
        self
    }

    /// Set a raw JSON query.
    pub fn query_json(mut self, query: Value) -> Self {
        self.query = Some(query);
        self
    }

    /// Simple query string search.
    pub fn query_string(self, query: impl Into<String>) -> Self {
        self.query_json(json!({
            "query_string": {
                "query": query.into()
            }
        }))
    }

    /// Match query on a field.
    pub fn match_field(self, field: impl Into<String>, value: impl Into<String>) -> Self {
        self.query_json(json!({
            "match": {
                field.into(): value.into()
            }
        }))
    }

    /// Set pagination offset.
    pub fn from(mut self, from: i64) -> Self {
        self.from = Some(from);
        self
    }

    /// Set result size limit.
    ///
    /// The effective size sent to OpenSearch is clamped to [`SearchBuilder::max_size`]
    /// (default [`DEFAULT_MAX_SIZE`]) so a caller cannot request an unbounded page.
    pub fn size(mut self, size: i64) -> Self {
        self.size = Some(size);
        self
    }

    /// Override the maximum allowed page size used to clamp [`SearchBuilder::size`].
    ///
    /// Defaults to [`DEFAULT_MAX_SIZE`]. Values below `1` are treated as `1`.
    pub fn max_size(mut self, max_size: i64) -> Self {
        self.max_size = max_size.max(1);
        self
    }

    /// Enable `search_after` deep pagination.
    ///
    /// Emits a `search_after` clause seeded with the sort values of the last hit
    /// from a previous page. This scrolls past the 10,000-document `from`/`size`
    /// window without the cost of deep offsets. A stable, tie-broken [`sort`] must
    /// be configured (e.g. sort by a timestamp plus a unique id); `from` is ignored
    /// by OpenSearch when `search_after` is present.
    ///
    /// [`sort`]: SearchBuilder::sort_by
    pub fn search_after(mut self, sort_values: Vec<Value>) -> Self {
        self.search_after = Some(sort_values);
        self
    }

    /// Add sort field.
    pub fn sort_by(mut self, field: impl Into<String>, order: SortOrder) -> Self {
        self.sort.push(json!({
            field.into(): {
                "order": match order {
                    SortOrder::Asc => "asc",
                    SortOrder::Desc => "desc",
                }
            }
        }));
        self
    }

    /// Sort by score (relevance).
    pub fn sort_by_score(mut self, order: SortOrder) -> Self {
        self.sort.push(json!({
            "_score": {
                "order": match order {
                    SortOrder::Asc => "asc",
                    SortOrder::Desc => "desc",
                }
            }
        }));
        self
    }

    /// Include only specific fields in the response.
    pub fn source_includes(mut self, fields: Vec<String>) -> Self {
        self.source_includes = Some(fields);
        self
    }

    /// Exclude specific fields from the response.
    pub fn source_excludes(mut self, fields: Vec<String>) -> Self {
        self.source_excludes = Some(fields);
        self
    }

    /// Add highlighting.
    pub fn highlight(mut self, fields: Vec<String>) -> Self {
        let mut highlight_fields = serde_json::Map::new();
        for field in fields {
            highlight_fields.insert(field, json!({}));
        }
        self.highlight = Some(json!({ "fields": highlight_fields }));
        self
    }

    /// Add aggregation.
    pub fn aggregation(mut self, name: impl Into<String>, agg: Aggregation) -> Self {
        let aggs = self.aggregations.get_or_insert(json!({}));
        if let Value::Object(map) = aggs {
            map.insert(name.into(), agg.to_json());
        }
        self
    }

    /// Track total hits accurately (for counts > 10000).
    pub fn track_total_hits(mut self, track: bool) -> Self {
        self.track_total_hits = Some(track);
        self
    }

    /// Build the search body.
    fn build_body(&self) -> Value {
        let mut body = serde_json::Map::new();

        if let Some(query) = &self.query {
            body.insert("query".to_string(), query.clone());
        }

        if let Some(from) = self.from {
            body.insert("from".to_string(), json!(from));
        }

        if let Some(size) = self.size {
            // Clamp so a caller cannot request an unbounded page.
            let clamped = size.clamp(0, self.max_size);
            body.insert("size".to_string(), json!(clamped));
        }

        if !self.sort.is_empty() {
            body.insert("sort".to_string(), Value::Array(self.sort.clone()));
        }

        if let Some(search_after) = &self.search_after {
            body.insert(
                "search_after".to_string(),
                Value::Array(search_after.clone()),
            );
        }

        // Source filtering
        let mut source = serde_json::Map::new();
        if let Some(includes) = &self.source_includes {
            source.insert("includes".to_string(), json!(includes));
        }
        if let Some(excludes) = &self.source_excludes {
            source.insert("excludes".to_string(), json!(excludes));
        }
        if !source.is_empty() {
            body.insert("_source".to_string(), Value::Object(source));
        }

        if let Some(highlight) = &self.highlight {
            body.insert("highlight".to_string(), highlight.clone());
        }

        if let Some(aggs) = &self.aggregations {
            body.insert("aggs".to_string(), aggs.clone());
        }

        if let Some(track) = self.track_total_hits {
            body.insert("track_total_hits".to_string(), json!(track));
        }

        Value::Object(body)
    }

    /// Execute the search and return documents.
    pub async fn execute<T: Document>(self) -> Result<Vec<T>> {
        let results = self.execute_with_meta::<T>().await?;
        Ok(results.hits.into_iter().map(|h| h.doc).collect())
    }

    /// Execute the search and return documents with metadata.
    pub async fn execute_with_meta<T: Document>(self) -> Result<SearchResult<T>> {
        let indices = if self.indices.is_empty() {
            vec![T::index_name().to_string()]
        } else {
            self.indices.clone()
        };

        debug!("Searching indices: {:?}", indices);

        let index_refs: Vec<&str> = indices.iter().map(|s| s.as_str()).collect();
        let body = self.build_body();

        let response = self
            .client
            .search(opensearch::SearchParts::Index(&index_refs))
            .body(body)
            .send()
            .await?;

        let status = response.status_code();
        let result: Value = response.json().await?;

        if !status.is_success() {
            return Err(OpenSearchError::Query(
                result
                    .get("error")
                    .and_then(|e| e.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("Search failed")
                    .to_string(),
            ));
        }

        // Parse hits
        let hits_array = result["hits"]["hits"].as_array();
        let mut hits = Vec::new();

        if let Some(hits_arr) = hits_array {
            for hit in hits_arr {
                let id = hit["_id"].as_str().unwrap_or("").to_string();
                let index = hit["_index"].as_str().unwrap_or("").to_string();
                let score = hit["_score"].as_f64();
                let version = hit["_version"].as_i64();
                let seq_no = hit["_seq_no"].as_i64();
                let primary_term = hit["_primary_term"].as_i64();
                let routing = hit["_routing"].as_str().map(|s| s.to_string());

                // Parse highlight
                let highlight = hit["highlight"].as_object().map(|h| {
                    h.iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                v.as_array()
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                                            .collect()
                                    })
                                    .unwrap_or_default(),
                            )
                        })
                        .collect()
                });

                let source = hit
                    .get("_source")
                    .ok_or_else(|| OpenSearchError::Internal("Missing _source".to_string()))?;

                let doc: T = serde_json::from_value(source.clone())?;

                hits.push(Hit {
                    doc,
                    meta: DocumentMeta {
                        id,
                        index,
                        score,
                        version,
                        seq_no,
                        primary_term,
                        routing,
                        highlight,
                    },
                });
            }
        }

        // Parse total
        let total = result["hits"]["total"]["value"].as_u64().unwrap_or(0);
        let total_relation = result["hits"]["total"]["relation"]
            .as_str()
            .unwrap_or("eq")
            .to_string();

        // Parse aggregations. OpenSearch responses put them under the
        // top-level "aggregations" key; only the *request* body uses "aggs".
        let aggregations = result.get("aggregations").cloned();

        Ok(SearchResult {
            total,
            total_relation,
            max_score: result["hits"]["max_score"].as_f64(),
            hits,
            aggregations,
            took_ms: result["took"].as_u64().unwrap_or(0),
        })
    }

    /// Count matching documents.
    pub async fn count(self) -> Result<u64> {
        let indices: Vec<&str> = self.indices.iter().map(|s| s.as_str()).collect();

        let body = if let Some(query) = &self.query {
            json!({ "query": query })
        } else {
            json!({})
        };

        let response = self
            .client
            .count(opensearch::CountParts::Index(&indices))
            .body(body)
            .send()
            .await?;

        let result: Value = response.json().await?;
        Ok(result["count"].as_u64().unwrap_or(0))
    }
}

/// Sort order.
#[derive(Debug, Clone, Copy)]
pub enum SortOrder {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

/// Search result.
#[derive(Debug, Clone)]
pub struct SearchResult<T> {
    /// Total matching documents.
    pub total: u64,
    /// Total relation ("eq" or "gte").
    pub total_relation: String,
    /// Maximum score.
    pub max_score: Option<f64>,
    /// Matching documents with metadata.
    pub hits: Vec<Hit<T>>,
    /// Aggregation results.
    pub aggregations: Option<Value>,
    /// Time taken in milliseconds.
    pub took_ms: u64,
}

/// A search hit.
pub type Hit<T> = DocumentWithMeta<T>;

/// Aggregation types.
#[derive(Debug, Clone)]
pub enum Aggregation {
    /// Terms aggregation.
    Terms {
        /// Field to aggregate on.
        field: String,
        /// Maximum number of buckets.
        size: Option<i64>,
    },
    /// Date histogram aggregation.
    DateHistogram {
        /// Field containing dates.
        field: String,
        /// Calendar interval (day, week, month, etc.).
        calendar_interval: Option<String>,
        /// Fixed interval (1d, 1h, etc.).
        fixed_interval: Option<String>,
    },
    /// Histogram aggregation.
    Histogram {
        /// Field to aggregate on.
        field: String,
        /// Bucket interval.
        interval: f64,
    },
    /// Range aggregation.
    Range {
        /// Field to aggregate on.
        field: String,
        /// Range definitions.
        ranges: Vec<RangeBucket>,
    },
    /// Avg aggregation.
    Avg {
        /// Field to average.
        field: String,
    },
    /// Sum aggregation.
    Sum {
        /// Field to sum.
        field: String,
    },
    /// Min aggregation.
    Min {
        /// Field to find minimum.
        field: String,
    },
    /// Max aggregation.
    Max {
        /// Field to find maximum.
        field: String,
    },
    /// Cardinality (unique count) aggregation.
    Cardinality {
        /// Field to count unique values.
        field: String,
    },
}

impl Aggregation {
    /// Create a terms aggregation.
    pub fn terms(field: impl Into<String>) -> Self {
        Aggregation::Terms {
            field: field.into(),
            size: None,
        }
    }

    /// Create an average aggregation.
    pub fn avg(field: impl Into<String>) -> Self {
        Aggregation::Avg {
            field: field.into(),
        }
    }

    /// Convert to JSON.
    pub fn to_json(&self) -> Value {
        match self {
            Aggregation::Terms { field, size } => {
                let mut terms = json!({ "field": field });
                if let Some(s) = size {
                    terms["size"] = json!(s);
                }
                json!({ "terms": terms })
            }
            Aggregation::DateHistogram {
                field,
                calendar_interval,
                fixed_interval,
            } => {
                let mut dh = json!({ "field": field });
                if let Some(ci) = calendar_interval {
                    dh["calendar_interval"] = json!(ci);
                }
                if let Some(fi) = fixed_interval {
                    dh["fixed_interval"] = json!(fi);
                }
                json!({ "date_histogram": dh })
            }
            Aggregation::Histogram { field, interval } => {
                json!({
                    "histogram": {
                        "field": field,
                        "interval": interval
                    }
                })
            }
            Aggregation::Range { field, ranges } => {
                let range_arr: Vec<Value> = ranges.iter().map(|r| r.to_json()).collect();
                json!({
                    "range": {
                        "field": field,
                        "ranges": range_arr
                    }
                })
            }
            Aggregation::Avg { field } => json!({ "avg": { "field": field } }),
            Aggregation::Sum { field } => json!({ "sum": { "field": field } }),
            Aggregation::Min { field } => json!({ "min": { "field": field } }),
            Aggregation::Max { field } => json!({ "max": { "field": field } }),
            Aggregation::Cardinality { field } => json!({ "cardinality": { "field": field } }),
        }
    }
}

/// Range bucket definition.
#[derive(Debug, Clone)]
pub struct RangeBucket {
    /// Optional key.
    pub key: Option<String>,
    /// From value (inclusive).
    pub from: Option<f64>,
    /// To value (exclusive).
    pub to: Option<f64>,
}

impl RangeBucket {
    /// Create a range bucket.
    pub fn new() -> Self {
        Self {
            key: None,
            from: None,
            to: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut bucket = serde_json::Map::new();
        if let Some(k) = &self.key {
            bucket.insert("key".to_string(), json!(k));
        }
        if let Some(f) = self.from {
            bucket.insert("from".to_string(), json!(f));
        }
        if let Some(t) = self.to {
            bucket.insert("to".to_string(), json!(t));
        }
        Value::Object(bucket)
    }
}

impl Default for RangeBucket {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregation result wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationResult {
    /// Raw aggregation result.
    pub value: Value,
}

impl AggregationResult {
    /// Get a metric value (avg, sum, min, max).
    pub fn metric_value(&self) -> Option<f64> {
        self.value["value"].as_f64()
    }

    /// Get buckets from bucket aggregation.
    pub fn buckets(&self) -> Option<Vec<Value>> {
        self.value["buckets"].as_array().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder() -> SearchBuilder {
        SearchBuilder::new(Arc::new(OpenSearch::default()))
    }

    #[test]
    fn search_after_is_emitted_with_sort_values() {
        let body = builder()
            .sort_by("created_at", SortOrder::Desc)
            .sort_by("id", SortOrder::Asc)
            .search_after(vec![json!(1_700_000_000), json!("abc")])
            .build_body();

        assert_eq!(
            body["search_after"],
            json!([1_700_000_000, "abc"]),
            "search_after clause should be emitted verbatim"
        );
        assert!(
            body.get("sort").is_some(),
            "search_after requires a stable sort"
        );
    }

    #[test]
    fn size_is_clamped_to_default_max() {
        let body = builder().size(50_000).build_body();
        assert_eq!(body["size"], json!(DEFAULT_MAX_SIZE));
    }

    #[test]
    fn size_below_max_is_untouched() {
        let body = builder().size(25).build_body();
        assert_eq!(body["size"], json!(25));
    }

    #[test]
    fn custom_max_size_clamps() {
        let body = builder().max_size(100).size(5000).build_body();
        assert_eq!(body["size"], json!(100));
    }

    #[test]
    fn no_search_after_when_not_set() {
        let body = builder().size(10).build_body();
        assert!(body.get("search_after").is_none());
    }
}
