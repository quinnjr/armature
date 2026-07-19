//! Index management for OpenSearch.

use crate::error::{OpenSearchError, Result, error_reason, json_or_error};
use armature_log::{debug, info};
use opensearch::OpenSearch;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

/// Index manager for creating and managing indices.
#[derive(Clone)]
pub struct IndexManager {
    client: Arc<OpenSearch>,
}

impl IndexManager {
    /// Create a new index manager.
    pub(crate) fn new(client: Arc<OpenSearch>) -> Self {
        Self { client }
    }

    /// Create a new index.
    pub async fn create(&self, name: &str, settings: IndexSettings) -> Result<()> {
        info!("Creating index: {}", name);

        let body = settings.to_json();

        let response = self
            .client
            .indices()
            .create(opensearch::indices::IndicesCreateParts::Index(name))
            .body(body)
            .send()
            .await?;

        let status = response.status_code();

        if status == opensearch::http::StatusCode::BAD_REQUEST {
            let body: Value = response.json().await?;
            let error_type = body["error"]["type"].as_str().unwrap_or("");

            if error_type == "resource_already_exists_exception" {
                return Err(OpenSearchError::IndexExists(name.to_string()));
            }

            return Err(OpenSearchError::Internal(error_reason(&body)));
        }

        json_or_error(response).await?;

        Ok(())
    }

    /// Delete an index.
    pub async fn delete(&self, name: &str) -> Result<()> {
        info!("Deleting index: {}", name);

        let response = self
            .client
            .indices()
            .delete(opensearch::indices::IndicesDeleteParts::Index(&[name]))
            .send()
            .await?;

        if response.status_code() == opensearch::http::StatusCode::NOT_FOUND {
            return Err(OpenSearchError::IndexNotFound(name.to_string()));
        }

        json_or_error(response).await?;

        Ok(())
    }

    /// Check if an index exists.
    pub async fn exists(&self, name: &str) -> Result<bool> {
        debug!("Checking if index exists: {}", name);

        let response = self
            .client
            .indices()
            .exists(opensearch::indices::IndicesExistsParts::Index(&[name]))
            .send()
            .await?;

        Ok(response.status_code().is_success())
    }

    /// Get index settings and mappings.
    pub async fn get(&self, name: &str) -> Result<Value> {
        debug!("Getting index: {}", name);

        let response = self
            .client
            .indices()
            .get(opensearch::indices::IndicesGetParts::Index(&[name]))
            .send()
            .await?;

        let status = response.status_code();

        if status == opensearch::http::StatusCode::NOT_FOUND {
            return Err(OpenSearchError::IndexNotFound(name.to_string()));
        }

        Ok(response.json().await?)
    }

    /// Update index mappings.
    pub async fn put_mapping(&self, name: &str, mapping: Mapping) -> Result<()> {
        debug!("Updating mapping for index: {}", name);

        let response = self
            .client
            .indices()
            .put_mapping(opensearch::indices::IndicesPutMappingParts::Index(&[name]))
            .body(mapping.to_json())
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Update index settings.
    pub async fn put_settings(&self, name: &str, settings: Value) -> Result<()> {
        debug!("Updating settings for index: {}", name);

        let response = self
            .client
            .indices()
            .put_settings(opensearch::indices::IndicesPutSettingsParts::Index(&[name]))
            .body(settings)
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Open a closed index.
    pub async fn open(&self, name: &str) -> Result<()> {
        info!("Opening index: {}", name);

        let response = self
            .client
            .indices()
            .open(opensearch::indices::IndicesOpenParts::Index(&[name]))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Close an index.
    pub async fn close(&self, name: &str) -> Result<()> {
        info!("Closing index: {}", name);

        let response = self
            .client
            .indices()
            .close(opensearch::indices::IndicesCloseParts::Index(&[name]))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Refresh an index.
    pub async fn refresh(&self, name: &str) -> Result<()> {
        debug!("Refreshing index: {}", name);

        let response = self
            .client
            .indices()
            .refresh(opensearch::indices::IndicesRefreshParts::Index(&[name]))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Flush an index.
    pub async fn flush(&self, name: &str) -> Result<()> {
        debug!("Flushing index: {}", name);

        let response = self
            .client
            .indices()
            .flush(opensearch::indices::IndicesFlushParts::Index(&[name]))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Create an index alias.
    pub async fn create_alias(&self, index: &str, alias: &str) -> Result<()> {
        info!("Creating alias {} for index {}", alias, index);

        let response = self
            .client
            .indices()
            .put_alias(opensearch::indices::IndicesPutAliasParts::IndexName(
                &[index],
                alias,
            ))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// Delete an index alias.
    pub async fn delete_alias(&self, index: &str, alias: &str) -> Result<()> {
        info!("Deleting alias {} from index {}", alias, index);

        let response = self
            .client
            .indices()
            .delete_alias(opensearch::indices::IndicesDeleteAliasParts::IndexName(
                &[index],
                &[alias],
            ))
            .send()
            .await?;

        json_or_error(response).await?;

        Ok(())
    }

    /// List all indices.
    pub async fn list(&self) -> Result<Vec<IndexInfo>> {
        let response = self
            .client
            .cat()
            .indices(opensearch::cat::CatIndicesParts::None)
            .format("json")
            .send()
            .await?;

        let indices: Vec<Value> = response.json().await?;

        Ok(indices
            .into_iter()
            .map(|v| IndexInfo {
                name: v["index"].as_str().unwrap_or("").to_string(),
                health: v["health"].as_str().unwrap_or("").to_string(),
                status: v["status"].as_str().unwrap_or("").to_string(),
                docs_count: v["docs.count"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                store_size: v["store.size"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }
}

/// Index settings for creating indices.
#[derive(Debug, Clone, Default)]
pub struct IndexSettings {
    /// Number of shards.
    pub number_of_shards: Option<i32>,
    /// Number of replicas.
    pub number_of_replicas: Option<i32>,
    /// Refresh interval.
    pub refresh_interval: Option<String>,
    /// Analysis settings.
    pub analysis: Option<Value>,
    /// Field mappings.
    pub mappings: Option<Mapping>,
}

impl IndexSettings {
    /// Create new index settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set number of shards.
    pub fn shards(mut self, shards: i32) -> Self {
        self.number_of_shards = Some(shards);
        self
    }

    /// Set number of replicas.
    pub fn replicas(mut self, replicas: i32) -> Self {
        self.number_of_replicas = Some(replicas);
        self
    }

    /// Set refresh interval.
    pub fn refresh_interval(mut self, interval: impl Into<String>) -> Self {
        self.refresh_interval = Some(interval.into());
        self
    }

    /// Set mappings.
    pub fn mappings(mut self, mappings: Mapping) -> Self {
        self.mappings = Some(mappings);
        self
    }

    fn to_json(&self) -> Value {
        let mut body = serde_json::Map::new();
        let mut settings = serde_json::Map::new();

        if let Some(shards) = self.number_of_shards {
            settings.insert("number_of_shards".to_string(), json!(shards));
        }
        if let Some(replicas) = self.number_of_replicas {
            settings.insert("number_of_replicas".to_string(), json!(replicas));
        }
        if let Some(interval) = &self.refresh_interval {
            settings.insert("refresh_interval".to_string(), json!(interval));
        }
        if let Some(analysis) = &self.analysis {
            settings.insert("analysis".to_string(), analysis.clone());
        }

        if !settings.is_empty() {
            body.insert("settings".to_string(), Value::Object(settings));
        }

        if let Some(mappings) = &self.mappings {
            body.insert("mappings".to_string(), mappings.to_json());
        }

        Value::Object(body)
    }
}

/// Field mapping configuration.
#[derive(Debug, Clone, Default)]
pub struct Mapping {
    /// Field definitions.
    pub properties: HashMap<String, MappingField>,
    /// Dynamic mapping setting.
    pub dynamic: Option<String>,
}

impl Mapping {
    /// Create a new mapping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a field.
    pub fn field(mut self, name: impl Into<String>, field: MappingField) -> Self {
        self.properties.insert(name.into(), field);
        self
    }

    /// Set dynamic mapping.
    pub fn dynamic(mut self, dynamic: impl Into<String>) -> Self {
        self.dynamic = Some(dynamic.into());
        self
    }

    fn to_json(&self) -> Value {
        let mut mapping = serde_json::Map::new();

        if let Some(dynamic) = &self.dynamic {
            mapping.insert("dynamic".to_string(), json!(dynamic));
        }

        let mut properties = serde_json::Map::new();
        for (name, field) in &self.properties {
            properties.insert(name.clone(), field.to_json());
        }
        mapping.insert("properties".to_string(), Value::Object(properties));

        Value::Object(mapping)
    }
}

/// Field mapping definition.
#[derive(Debug, Clone)]
pub struct MappingField {
    /// Field type.
    pub field_type: FieldType,
    /// Analyzer.
    pub analyzer: Option<String>,
    /// Search analyzer.
    pub search_analyzer: Option<String>,
    /// Whether to index the field.
    pub index: Option<bool>,
    /// Whether to store the field.
    pub store: Option<bool>,
    /// Null value.
    pub null_value: Option<Value>,
    /// Nested properties (for object/nested types).
    pub properties: Option<HashMap<String, MappingField>>,
}

impl MappingField {
    /// Create a new text field.
    pub fn text() -> Self {
        Self {
            field_type: FieldType::Text,
            analyzer: None,
            search_analyzer: None,
            index: None,
            store: None,
            null_value: None,
            properties: None,
        }
    }

    /// Create a new keyword field.
    pub fn keyword() -> Self {
        Self {
            field_type: FieldType::Keyword,
            ..Self::text()
        }
    }

    /// Create a new integer field.
    pub fn integer() -> Self {
        Self {
            field_type: FieldType::Integer,
            ..Self::text()
        }
    }

    /// Create a new long field.
    pub fn long() -> Self {
        Self {
            field_type: FieldType::Long,
            ..Self::text()
        }
    }

    /// Create a new float field.
    pub fn float() -> Self {
        Self {
            field_type: FieldType::Float,
            ..Self::text()
        }
    }

    /// Create a new double field.
    pub fn double() -> Self {
        Self {
            field_type: FieldType::Double,
            ..Self::text()
        }
    }

    /// Create a new boolean field.
    pub fn boolean() -> Self {
        Self {
            field_type: FieldType::Boolean,
            ..Self::text()
        }
    }

    /// Create a new date field.
    pub fn date() -> Self {
        Self {
            field_type: FieldType::Date,
            ..Self::text()
        }
    }

    /// Create a new object field.
    pub fn object() -> Self {
        Self {
            field_type: FieldType::Object,
            ..Self::text()
        }
    }

    /// Create a new nested field.
    pub fn nested() -> Self {
        Self {
            field_type: FieldType::Nested,
            ..Self::text()
        }
    }

    /// Set analyzer.
    pub fn analyzer(mut self, analyzer: impl Into<String>) -> Self {
        self.analyzer = Some(analyzer.into());
        self
    }

    /// Set search analyzer.
    pub fn search_analyzer(mut self, analyzer: impl Into<String>) -> Self {
        self.search_analyzer = Some(analyzer.into());
        self
    }

    /// Add nested property.
    pub fn property(mut self, name: impl Into<String>, field: MappingField) -> Self {
        self.properties
            .get_or_insert_with(HashMap::new)
            .insert(name.into(), field);
        self
    }

    fn to_json(&self) -> Value {
        let mut field = serde_json::Map::new();

        field.insert("type".to_string(), json!(self.field_type.as_str()));

        if let Some(analyzer) = &self.analyzer {
            field.insert("analyzer".to_string(), json!(analyzer));
        }
        if let Some(search_analyzer) = &self.search_analyzer {
            field.insert("search_analyzer".to_string(), json!(search_analyzer));
        }
        if let Some(index) = self.index {
            field.insert("index".to_string(), json!(index));
        }
        if let Some(store) = self.store {
            field.insert("store".to_string(), json!(store));
        }
        if let Some(null_value) = &self.null_value {
            field.insert("null_value".to_string(), null_value.clone());
        }
        if let Some(properties) = &self.properties {
            let mut props = serde_json::Map::new();
            for (name, prop) in properties {
                props.insert(name.clone(), prop.to_json());
            }
            field.insert("properties".to_string(), Value::Object(props));
        }

        Value::Object(field)
    }
}

/// Field types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    /// Full-text searchable field.
    Text,
    /// Exact match keyword field.
    Keyword,
    /// 64-bit integer.
    Long,
    /// 32-bit integer.
    Integer,
    /// 16-bit integer.
    Short,
    /// 8-bit integer.
    Byte,
    /// Double precision float.
    Double,
    /// Single precision float.
    Float,
    /// Boolean.
    Boolean,
    /// Date.
    Date,
    /// Binary data.
    Binary,
    /// Integer range.
    IntegerRange,
    /// Float range.
    FloatRange,
    /// Long range.
    LongRange,
    /// Double range.
    DoubleRange,
    /// Date range.
    DateRange,
    /// IP address.
    Ip,
    /// Completion suggester.
    Completion,
    /// Geo point.
    GeoPoint,
    /// Geo shape.
    GeoShape,
    /// Token count.
    TokenCount,
    /// Nested object.
    Nested,
    /// Object.
    Object,
    /// Flattened.
    Flattened,
    /// Search-as-you-type.
    SearchAsYouType,
}

impl FieldType {
    fn as_str(&self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::Keyword => "keyword",
            FieldType::Long => "long",
            FieldType::Integer => "integer",
            FieldType::Short => "short",
            FieldType::Byte => "byte",
            FieldType::Double => "double",
            FieldType::Float => "float",
            FieldType::Boolean => "boolean",
            FieldType::Date => "date",
            FieldType::Binary => "binary",
            FieldType::IntegerRange => "integer_range",
            FieldType::FloatRange => "float_range",
            FieldType::LongRange => "long_range",
            FieldType::DoubleRange => "double_range",
            FieldType::DateRange => "date_range",
            FieldType::Ip => "ip",
            FieldType::Completion => "completion",
            FieldType::GeoPoint => "geo_point",
            FieldType::GeoShape => "geo_shape",
            FieldType::TokenCount => "token_count",
            FieldType::Nested => "nested",
            FieldType::Object => "object",
            FieldType::Flattened => "flattened",
            FieldType::SearchAsYouType => "search_as_you_type",
        }
    }
}

/// Index information.
#[derive(Debug, Clone)]
pub struct IndexInfo {
    /// Index name.
    pub name: String,
    /// Health status.
    pub health: String,
    /// Open/closed status.
    pub status: String,
    /// Document count.
    pub docs_count: u64,
    /// Store size.
    pub store_size: String,
}
