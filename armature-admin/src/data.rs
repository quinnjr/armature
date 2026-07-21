//! Pluggable data source for the admin interface.
//!
//! The admin dashboard does not ship an ORM. Instead, it renders whatever a
//! [`DataSource`] hands back. Applications wire their own persistence layer by
//! implementing this trait; the crate ships an [`InMemoryDataSource`] stub that
//! is used by default (and in tests) so the generated router is fully
//! functional out of the box.

use crate::error::AdminError;
use crate::model::ModelDefinition;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;

/// A resolved query for a list view.
///
/// `order_by` entries are pre-rendered `"field ASC"` / `"field DESC"` clauses
/// (see [`crate::model::OrderingField::as_sql`] and
/// [`crate::SortOrder::as_sql`]) so a SQL-backed data source can splice them
/// directly, while the in-memory stub parses the leading field name.
#[derive(Debug, Clone, Default)]
pub struct DataQuery {
    /// Zero-based row offset.
    pub offset: usize,
    /// Maximum number of rows to return.
    pub limit: usize,
    /// Ordering clauses, most-significant first.
    pub order_by: Vec<String>,
    /// Free-text search query (matched against the model's `search_fields`).
    pub search: Option<String>,
    /// Exact-match filters (`field -> value`).
    pub filters: HashMap<String, String>,
}

/// A page of rows plus the unpaginated total.
#[derive(Debug, Clone, Default)]
pub struct DataPage {
    /// The rows for the requested page (each a JSON object).
    pub rows: Vec<Value>,
    /// Total number of rows matching the query, ignoring pagination.
    pub total: usize,
}

/// Pluggable backing store for admin models.
///
/// All methods take the [`ModelDefinition`] so a single implementation can
/// serve every registered model.
#[async_trait]
pub trait DataSource: Send + Sync {
    /// Fetch a page of rows for a list view.
    async fn list(&self, model: &ModelDefinition, query: &DataQuery) -> DataPage;

    /// Fetch a single record by primary-key value.
    async fn get(&self, model: &ModelDefinition, id: &str) -> Option<Value>;

    /// Total number of records for a model (used by the dashboard summaries).
    async fn count(&self, model: &ModelDefinition) -> usize;

    /// Create a record, returning its primary-key value.
    async fn create(&self, model: &ModelDefinition, data: Value) -> Result<String, AdminError>;

    /// Update an existing record.
    async fn update(
        &self,
        model: &ModelDefinition,
        id: &str,
        data: Value,
    ) -> Result<(), AdminError>;

    /// Delete a record.
    async fn delete(&self, model: &ModelDefinition, id: &str) -> Result<(), AdminError>;
}

/// An in-memory [`DataSource`] used as the default backing store.
///
/// Rows are keyed by model name; each row is a JSON object. This is a real,
/// fully-functional store (create/read/update/delete, search, filter,
/// ordering and pagination) — just not a persistent one.
#[derive(Default)]
pub struct InMemoryDataSource {
    tables: RwLock<HashMap<String, Vec<Value>>>,
}

impl InMemoryDataSource {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a row for a model (test/setup helper). The row must be a JSON
    /// object; a primary key is generated if absent.
    pub fn seed(&self, model_name: impl Into<String>, mut row: Value) {
        let model_name = model_name.into();
        if let Value::Object(map) = &mut row
            && !map.contains_key("id")
        {
            map.insert(
                "id".to_string(),
                Value::String(uuid::Uuid::new_v4().to_string()),
            );
        }
        self.tables.write().entry(model_name).or_default().push(row);
    }

    /// Number of rows currently stored for a model.
    pub fn len(&self, model_name: &str) -> usize {
        self.tables
            .read()
            .get(model_name)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    fn pk_of(model: &ModelDefinition, row: &Value) -> Option<String> {
        row.get(&model.primary_key).map(value_to_plain_string)
    }
}

#[async_trait]
impl DataSource for InMemoryDataSource {
    async fn list(&self, model: &ModelDefinition, query: &DataQuery) -> DataPage {
        let tables = self.tables.read();
        let all = match tables.get(&model.name) {
            Some(rows) => rows,
            None => return DataPage::default(),
        };

        // Filter (exact match) + search (substring over search_fields).
        let mut matched: Vec<Value> = all
            .iter()
            .filter(|row| {
                query.filters.iter().all(|(field, want)| {
                    row.get(field)
                        .map(|v| value_to_plain_string(v) == *want)
                        .unwrap_or(false)
                })
            })
            .filter(|row| match &query.search {
                None => true,
                Some(needle) => {
                    let needle = needle.to_lowercase();
                    model.search_fields.iter().any(|field| {
                        row.get(field)
                            .map(|v| value_to_plain_string(v).to_lowercase().contains(&needle))
                            .unwrap_or(false)
                    })
                }
            })
            .cloned()
            .collect();

        // Ordering: honor the leading clause ("field ASC"/"field DESC").
        if let Some(first) = query.order_by.first() {
            let mut parts = first.split_whitespace();
            if let Some(field) = parts.next() {
                let descending = parts.next().map(|d| d.eq_ignore_ascii_case("DESC")) == Some(true);
                matched.sort_by(|a, b| {
                    let av = a.get(field).map(value_to_plain_string).unwrap_or_default();
                    let bv = b.get(field).map(value_to_plain_string).unwrap_or_default();
                    if descending { bv.cmp(&av) } else { av.cmp(&bv) }
                });
            }
        }

        let total = matched.len();
        let rows = if query.limit == 0 {
            matched.into_iter().skip(query.offset).collect()
        } else {
            matched
                .into_iter()
                .skip(query.offset)
                .take(query.limit)
                .collect()
        };
        DataPage { rows, total }
    }

    async fn get(&self, model: &ModelDefinition, id: &str) -> Option<Value> {
        let tables = self.tables.read();
        tables.get(&model.name).and_then(|rows| {
            rows.iter()
                .find(|row| Self::pk_of(model, row).as_deref() == Some(id))
                .cloned()
        })
    }

    async fn count(&self, model: &ModelDefinition) -> usize {
        self.len(&model.name)
    }

    async fn create(&self, model: &ModelDefinition, mut data: Value) -> Result<String, AdminError> {
        let map = data
            .as_object_mut()
            .ok_or_else(|| AdminError::Validation("record must be a JSON object".to_string()))?;

        let id = match map.get(&model.primary_key) {
            Some(v) if !v.is_null() => value_to_plain_string(v),
            _ => {
                let id = uuid::Uuid::new_v4().to_string();
                map.insert(model.primary_key.clone(), Value::String(id.clone()));
                id
            }
        };

        self.tables
            .write()
            .entry(model.name.clone())
            .or_default()
            .push(data);
        Ok(id)
    }

    async fn update(
        &self,
        model: &ModelDefinition,
        id: &str,
        data: Value,
    ) -> Result<(), AdminError> {
        let mut tables = self.tables.write();
        let rows = tables
            .get_mut(&model.name)
            .ok_or_else(|| AdminError::RecordNotFound {
                model: model.name.clone(),
                id: id.to_string(),
            })?;

        let slot = rows
            .iter_mut()
            .find(|row| Self::pk_of(model, row).as_deref() == Some(id))
            .ok_or_else(|| AdminError::RecordNotFound {
                model: model.name.clone(),
                id: id.to_string(),
            })?;

        // Merge provided fields onto the existing record, preserving the PK.
        if let (Some(existing), Some(incoming)) = (slot.as_object_mut(), data.as_object()) {
            for (k, v) in incoming {
                if k == &model.primary_key {
                    continue;
                }
                existing.insert(k.clone(), v.clone());
            }
        } else {
            *slot = data;
        }
        Ok(())
    }

    async fn delete(&self, model: &ModelDefinition, id: &str) -> Result<(), AdminError> {
        let mut tables = self.tables.write();
        let rows = tables
            .get_mut(&model.name)
            .ok_or_else(|| AdminError::RecordNotFound {
                model: model.name.clone(),
                id: id.to_string(),
            })?;

        let before = rows.len();
        rows.retain(|row| Self::pk_of(model, row).as_deref() != Some(id));
        if rows.len() == before {
            return Err(AdminError::RecordNotFound {
                model: model.name.clone(),
                id: id.to_string(),
            });
        }
        Ok(())
    }
}

/// Render a JSON scalar to a plain (unescaped) string for comparison/keys.
pub(crate) fn value_to_plain_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{FieldDefinition, FieldType};

    fn user_model() -> ModelDefinition {
        ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String).searchable())
            .search_fields(["name"])
            .list_display(["id", "name"])
            .build()
    }

    #[tokio::test]
    async fn stub_list_paginates_and_totals() {
        let ds = InMemoryDataSource::new();
        let model = user_model();
        for i in 0..5 {
            ds.seed(
                "user",
                serde_json::json!({ "id": i, "name": format!("u{i}") }),
            );
        }

        let page = ds
            .list(
                &model,
                &DataQuery {
                    offset: 0,
                    limit: 2,
                    ..Default::default()
                },
            )
            .await;

        assert_eq!(page.rows.len(), 2, "limit must cap returned rows");
        assert_eq!(page.total, 5, "total must ignore pagination");
    }

    #[tokio::test]
    async fn stub_crud_roundtrip() {
        let ds = InMemoryDataSource::new();
        let model = user_model();

        let id = ds
            .create(&model, serde_json::json!({ "id": "7", "name": "Alice" }))
            .await
            .unwrap();
        assert_eq!(id, "7");
        assert_eq!(ds.get(&model, "7").await.unwrap()["name"], "Alice");

        ds.update(&model, "7", serde_json::json!({ "name": "Bob" }))
            .await
            .unwrap();
        assert_eq!(ds.get(&model, "7").await.unwrap()["name"], "Bob");

        ds.delete(&model, "7").await.unwrap();
        assert!(ds.get(&model, "7").await.is_none());
    }

    #[tokio::test]
    async fn stub_search_filters_rows() {
        let ds = InMemoryDataSource::new();
        let model = user_model();
        ds.seed("user", serde_json::json!({ "id": 1, "name": "Alice" }));
        ds.seed("user", serde_json::json!({ "id": 2, "name": "Bob" }));

        let page = ds
            .list(
                &model,
                &DataQuery {
                    search: Some("ali".to_string()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0]["name"], "Alice");
    }
}
