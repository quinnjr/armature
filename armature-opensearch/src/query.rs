//! Query DSL builder for OpenSearch.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Query builder for constructing OpenSearch queries.
#[derive(Debug, Clone, Default)]
pub struct QueryBuilder {
    query: Option<Query>,
}

impl QueryBuilder {
    /// Create a new query builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the query.
    pub fn query(mut self, query: Query) -> Self {
        self.query = Some(query);
        self
    }

    /// Build a match_all query.
    pub fn match_all(self) -> Self {
        self.query(Query::MatchAll)
    }

    /// Build a match query.
    pub fn match_query(self, field: impl Into<String>, value: impl Into<String>) -> Self {
        self.query(Query::Match(MatchQuery {
            field: field.into(),
            query: value.into(),
            operator: None,
            fuzziness: None,
        }))
    }

    /// Build a term query.
    pub fn term(self, field: impl Into<String>, value: impl Into<Value>) -> Self {
        self.query(Query::Term(TermQuery {
            field: field.into(),
            value: value.into(),
        }))
    }

    /// Build a bool query.
    pub fn bool_query(self) -> BoolQueryBuilder {
        BoolQueryBuilder::new()
    }

    /// Build the query as JSON.
    pub fn build(self) -> Value {
        self.query
            .map(|q| q.to_json())
            .unwrap_or(json!({ "match_all": {} }))
    }
}

/// Query types supported by OpenSearch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Query {
    /// Match all documents.
    MatchAll,
    /// Full-text match query.
    Match(MatchQuery),
    /// Term query for exact matches.
    Term(TermQuery),
    /// Terms query for multiple exact matches.
    Terms(TermsQuery),
    /// Range query.
    Range(RangeQuery),
    /// Bool query for combining queries.
    Bool(BoolQuery),
    /// Query string query.
    QueryString(QueryStringQuery),
    /// Prefix query.
    Prefix(PrefixQuery),
    /// Wildcard query.
    Wildcard(WildcardQuery),
    /// Exists query.
    Exists(ExistsQuery),
    /// Nested query.
    Nested(NestedQuery),
    /// Raw JSON query.
    Raw(Value),
}

impl Query {
    /// Convert query to JSON.
    pub fn to_json(&self) -> Value {
        match self {
            Query::MatchAll => json!({ "match_all": {} }),
            Query::Match(m) => m.to_json(),
            Query::Term(t) => t.to_json(),
            Query::Terms(t) => t.to_json(),
            Query::Range(r) => r.to_json(),
            Query::Bool(b) => b.to_json(),
            Query::QueryString(q) => q.to_json(),
            Query::Prefix(p) => p.to_json(),
            Query::Wildcard(w) => w.to_json(),
            Query::Exists(e) => e.to_json(),
            Query::Nested(n) => n.to_json(),
            Query::Raw(v) => v.clone(),
        }
    }
}

/// Match query for full-text search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchQuery {
    /// Field to search.
    pub field: String,
    /// Search query.
    pub query: String,
    /// Operator (and/or).
    pub operator: Option<String>,
    /// Fuzziness for typo tolerance.
    pub fuzziness: Option<String>,
}

impl MatchQuery {
    /// Create a new match query.
    pub fn new(field: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            query: query.into(),
            operator: None,
            fuzziness: None,
        }
    }

    /// Set the operator.
    pub fn operator(mut self, op: impl Into<String>) -> Self {
        self.operator = Some(op.into());
        self
    }

    /// Set fuzziness.
    pub fn fuzziness(mut self, fuzz: impl Into<String>) -> Self {
        self.fuzziness = Some(fuzz.into());
        self
    }

    fn to_json(&self) -> Value {
        let mut query = json!({ "query": self.query });

        if let Some(op) = &self.operator {
            query["operator"] = json!(op);
        }
        if let Some(fuzz) = &self.fuzziness {
            query["fuzziness"] = json!(fuzz);
        }

        json!({ "match": { &self.field: query } })
    }
}

/// Term query for exact matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermQuery {
    /// Field name.
    pub field: String,
    /// Exact value to match.
    pub value: Value,
}

impl TermQuery {
    /// Create a new term query.
    pub fn new(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            field: field.into(),
            value: value.into(),
        }
    }

    fn to_json(&self) -> Value {
        json!({ "term": { &self.field: self.value } })
    }
}

/// Terms query for matching multiple values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermsQuery {
    /// Field name.
    pub field: String,
    /// Values to match.
    pub values: Vec<Value>,
}

impl TermsQuery {
    /// Create a new terms query.
    pub fn new(field: impl Into<String>, values: Vec<Value>) -> Self {
        Self {
            field: field.into(),
            values,
        }
    }

    fn to_json(&self) -> Value {
        json!({ "terms": { &self.field: self.values } })
    }
}

/// Range query for numeric/date ranges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeQuery {
    /// Field name.
    pub field: String,
    /// Greater than.
    pub gt: Option<Value>,
    /// Greater than or equal.
    pub gte: Option<Value>,
    /// Less than.
    pub lt: Option<Value>,
    /// Less than or equal.
    pub lte: Option<Value>,
    /// Date format (for date fields).
    pub format: Option<String>,
}

impl RangeQuery {
    /// Create a new range query.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            gt: None,
            gte: None,
            lt: None,
            lte: None,
            format: None,
        }
    }

    /// Set greater than.
    pub fn gt(mut self, value: impl Into<Value>) -> Self {
        self.gt = Some(value.into());
        self
    }

    /// Set greater than or equal.
    pub fn gte(mut self, value: impl Into<Value>) -> Self {
        self.gte = Some(value.into());
        self
    }

    /// Set less than.
    pub fn lt(mut self, value: impl Into<Value>) -> Self {
        self.lt = Some(value.into());
        self
    }

    /// Set less than or equal.
    pub fn lte(mut self, value: impl Into<Value>) -> Self {
        self.lte = Some(value.into());
        self
    }

    /// Set date format.
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    fn to_json(&self) -> Value {
        let mut range = serde_json::Map::new();

        if let Some(v) = &self.gt {
            range.insert("gt".to_string(), v.clone());
        }
        if let Some(v) = &self.gte {
            range.insert("gte".to_string(), v.clone());
        }
        if let Some(v) = &self.lt {
            range.insert("lt".to_string(), v.clone());
        }
        if let Some(v) = &self.lte {
            range.insert("lte".to_string(), v.clone());
        }
        if let Some(v) = &self.format {
            range.insert("format".to_string(), json!(v));
        }

        json!({ "range": { &self.field: range } })
    }
}

/// Bool query for combining multiple queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoolQuery {
    /// Must match (AND).
    pub must: Vec<Query>,
    /// Should match (OR).
    pub should: Vec<Query>,
    /// Must not match (NOT).
    pub must_not: Vec<Query>,
    /// Filter (non-scoring).
    pub filter: Vec<Query>,
    /// Minimum should match.
    pub minimum_should_match: Option<i32>,
}

impl BoolQuery {
    /// Create a new bool query.
    pub fn new() -> Self {
        Self::default()
    }

    fn to_json(&self) -> Value {
        let mut bool_query = serde_json::Map::new();

        if !self.must.is_empty() {
            bool_query.insert(
                "must".to_string(),
                Value::Array(self.must.iter().map(|q| q.to_json()).collect()),
            );
        }
        if !self.should.is_empty() {
            bool_query.insert(
                "should".to_string(),
                Value::Array(self.should.iter().map(|q| q.to_json()).collect()),
            );
        }
        if !self.must_not.is_empty() {
            bool_query.insert(
                "must_not".to_string(),
                Value::Array(self.must_not.iter().map(|q| q.to_json()).collect()),
            );
        }
        if !self.filter.is_empty() {
            bool_query.insert(
                "filter".to_string(),
                Value::Array(self.filter.iter().map(|q| q.to_json()).collect()),
            );
        }
        if let Some(min) = self.minimum_should_match {
            bool_query.insert("minimum_should_match".to_string(), json!(min));
        }

        json!({ "bool": bool_query })
    }
}

/// Builder for bool queries.
#[derive(Debug, Clone, Default)]
pub struct BoolQueryBuilder {
    query: BoolQuery,
}

impl BoolQueryBuilder {
    /// Create a new bool query builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a must clause.
    pub fn must(mut self, query: Query) -> Self {
        self.query.must.push(query);
        self
    }

    /// Add a should clause.
    pub fn should(mut self, query: Query) -> Self {
        self.query.should.push(query);
        self
    }

    /// Add a must_not clause.
    pub fn must_not(mut self, query: Query) -> Self {
        self.query.must_not.push(query);
        self
    }

    /// Add a filter clause.
    pub fn filter(mut self, query: Query) -> Self {
        self.query.filter.push(query);
        self
    }

    /// Set minimum should match.
    pub fn minimum_should_match(mut self, min: i32) -> Self {
        self.query.minimum_should_match = Some(min);
        self
    }

    /// Build the query.
    pub fn build(self) -> Query {
        Query::Bool(self.query)
    }
}

/// Query string query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStringQuery {
    /// Query string.
    pub query: String,
    /// Default field.
    pub default_field: Option<String>,
    /// Fields to search.
    pub fields: Option<Vec<String>>,
}

impl QueryStringQuery {
    /// Create a new query string query.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            default_field: None,
            fields: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut qs = json!({ "query": self.query });

        if let Some(df) = &self.default_field {
            qs["default_field"] = json!(df);
        }
        if let Some(fields) = &self.fields {
            qs["fields"] = json!(fields);
        }

        json!({ "query_string": qs })
    }
}

/// Prefix query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixQuery {
    /// Field name.
    pub field: String,
    /// Prefix value.
    pub value: String,
}

impl PrefixQuery {
    fn to_json(&self) -> Value {
        json!({ "prefix": { &self.field: self.value } })
    }
}

/// Wildcard query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WildcardQuery {
    /// Field name.
    pub field: String,
    /// Wildcard pattern.
    pub value: String,
}

impl WildcardQuery {
    fn to_json(&self) -> Value {
        json!({ "wildcard": { &self.field: self.value } })
    }
}

/// Exists query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExistsQuery {
    /// Field name.
    pub field: String,
}

impl ExistsQuery {
    fn to_json(&self) -> Value {
        json!({ "exists": { "field": self.field } })
    }
}

/// Nested query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NestedQuery {
    /// Path to nested field.
    pub path: String,
    /// Inner query.
    pub query: Box<Query>,
    /// Score mode.
    pub score_mode: Option<String>,
}

impl NestedQuery {
    fn to_json(&self) -> Value {
        let mut nested = json!({
            "path": self.path,
            "query": self.query.to_json()
        });

        if let Some(mode) = &self.score_mode {
            nested["score_mode"] = json!(mode);
        }

        json!({ "nested": nested })
    }
}
