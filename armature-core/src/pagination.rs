//! Pagination, filtering, sorting, and field selection utilities
//!
//! This module provides comprehensive utilities for API queries including:
//! - Offset and cursor-based pagination
//! - Multi-field sorting
//! - Query parameter filtering
//! - Full-text search integration
//! - Sparse fieldsets (field selection)
//!
//! # Quick Start
//!
//! ```
//! use armature_core::*;
//! use std::collections::HashMap;
//!
//! let query_params: HashMap<String, String> = HashMap::new();
//!
//! // Parse pagination from query params
//! let pagination = OffsetPagination::from_query_params(&query_params);
//!
//! // Parse sorting
//! let sorting = SortParams::from_query(&query_params);
//!
//! // Parse filters
//! let filters = FilterParams::from_query(&query_params);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// Default page size
pub const DEFAULT_PAGE_SIZE: usize = 20;

/// Maximum page size
pub const MAX_PAGE_SIZE: usize = 100;

// ============================================================================
// PAGINATION
// ============================================================================

/// Offset-based pagination parameters
///
/// Uses page number and page size (per_page).
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("page".to_string(), "2".to_string());
/// params.insert("per_page".to_string(), "50".to_string());
///
/// let pagination = OffsetPagination::from_query_params(&params);
/// assert_eq!(pagination.page, 2);
/// assert_eq!(pagination.per_page, 50);
/// assert_eq!(pagination.offset(), 50);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffsetPagination {
    /// Current page number (1-indexed)
    pub page: usize,

    /// Items per page
    pub per_page: usize,
}

impl OffsetPagination {
    /// Create new offset pagination
    pub fn new(page: usize, per_page: usize) -> Self {
        let page = page.max(1);
        let per_page = per_page.clamp(1, MAX_PAGE_SIZE);

        Self { page, per_page }
    }

    /// Parse from query parameters
    ///
    /// Looks for `page` and `per_page` (or `limit`) parameters.
    pub fn from_query_params(params: &HashMap<String, String>) -> Self {
        let page = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);

        let per_page = params
            .get("per_page")
            .or_else(|| params.get("limit"))
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PAGE_SIZE);

        Self::new(page, per_page)
    }

    /// Calculate offset for database queries
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_core::*;
    ///
    /// let pagination = OffsetPagination::new(1, 20);
    /// assert_eq!(pagination.offset(), 0);
    ///
    /// let pagination = OffsetPagination::new(2, 20);
    /// assert_eq!(pagination.offset(), 20);
    ///
    /// let pagination = OffsetPagination::new(3, 50);
    /// assert_eq!(pagination.offset(), 100);
    /// ```
    pub fn offset(&self) -> usize {
        (self.page - 1) * self.per_page
    }

    /// Get limit for database queries
    pub fn limit(&self) -> usize {
        self.per_page
    }

    /// Calculate total pages
    pub fn total_pages(&self, total_items: usize) -> usize {
        total_items.div_ceil(self.per_page)
    }

    /// Check if there's a next page
    pub fn has_next(&self, total_items: usize) -> bool {
        self.page < self.total_pages(total_items)
    }

    /// Check if there's a previous page
    pub fn has_prev(&self) -> bool {
        self.page > 1
    }
}

impl Default for OffsetPagination {
    fn default() -> Self {
        Self::new(1, DEFAULT_PAGE_SIZE)
    }
}

/// Cursor-based pagination parameters
///
/// Uses opaque cursors for pagination (better for real-time data).
///
/// # Examples
///
/// ```
/// use armature_core::*;
///
/// let pagination = CursorPagination::new(Some("cursor123".to_string()), 20);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPagination {
    /// Cursor for next page
    pub cursor: Option<String>,

    /// Items per page
    pub limit: usize,
}

impl CursorPagination {
    /// Create new cursor pagination
    pub fn new(cursor: Option<String>, limit: usize) -> Self {
        let limit = limit.clamp(1, MAX_PAGE_SIZE);
        Self { cursor, limit }
    }

    /// Parse from query parameters
    ///
    /// Looks for `cursor` and `limit` parameters.
    pub fn from_query_params(params: &HashMap<String, String>) -> Self {
        let cursor = params.get("cursor").cloned();
        let limit = params
            .get("limit")
            .and_then(|l| l.parse().ok())
            .unwrap_or(DEFAULT_PAGE_SIZE);

        Self::new(cursor, limit)
    }
}

impl Default for CursorPagination {
    fn default() -> Self {
        Self::new(None, DEFAULT_PAGE_SIZE)
    }
}

/// Pagination response metadata
///
/// Contains information about the pagination state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    /// Current page (offset pagination)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,

    /// Items per page
    pub per_page: usize,

    /// Total number of items
    pub total: usize,

    /// Total number of pages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pages: Option<usize>,

    /// Whether there's a next page
    pub has_next: bool,

    /// Whether there's a previous page
    pub has_prev: bool,

    /// Next cursor (cursor pagination)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Paginated response wrapper
///
/// Generic wrapper for paginated API responses.
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct User {
///     id: u64,
///     name: String,
/// }
///
/// let users = vec![
///     User { id: 1, name: "Alice".to_string() },
///     User { id: 2, name: "Bob".to_string() },
/// ];
///
/// let pagination = OffsetPagination::new(1, 20);
/// let response = PaginatedResponse::new(users, pagination, 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    /// The data items
    pub data: Vec<T>,

    /// Pagination metadata
    pub meta: PaginationMeta,
}

impl<T> PaginatedResponse<T> {
    /// Create paginated response from offset pagination
    pub fn new(data: Vec<T>, pagination: OffsetPagination, total: usize) -> Self {
        Self {
            data,
            meta: PaginationMeta {
                page: Some(pagination.page),
                per_page: pagination.per_page,
                total,
                total_pages: Some(pagination.total_pages(total)),
                has_next: pagination.has_next(total),
                has_prev: pagination.has_prev(),
                next_cursor: None,
            },
        }
    }

    /// Create paginated response from cursor pagination
    pub fn with_cursor(
        data: Vec<T>,
        limit: usize,
        total: usize,
        next_cursor: Option<String>,
    ) -> Self {
        Self {
            data,
            meta: PaginationMeta {
                page: None,
                per_page: limit,
                total,
                total_pages: None,
                has_next: next_cursor.is_some(),
                has_prev: false, // Cursor pagination typically doesn't track prev
                next_cursor,
            },
        }
    }
}

// ============================================================================
// SORTING
// ============================================================================

/// Sort direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    /// Ascending order
    Asc,
    /// Descending order
    Desc,
}

impl SortDirection {
    /// Convert to SQL ORDER BY string
    pub fn to_sql(&self) -> &'static str {
        match self {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        }
    }
}

impl FromStr for SortDirection {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "desc" | "descending" | "-" => SortDirection::Desc,
            _ => SortDirection::Asc,
        })
    }
}

/// Single sort field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortField {
    /// Field name to sort by
    pub field: String,

    /// Sort direction
    pub direction: SortDirection,
}

impl SortField {
    /// Create new sort field
    pub fn new(field: impl Into<String>, direction: SortDirection) -> Self {
        Self {
            field: field.into(),
            direction,
        }
    }

    /// Ascending sort
    pub fn asc(field: impl Into<String>) -> Self {
        Self::new(field, SortDirection::Asc)
    }

    /// Descending sort
    pub fn desc(field: impl Into<String>) -> Self {
        Self::new(field, SortDirection::Desc)
    }

    /// Convert to SQL ORDER BY clause
    pub fn to_sql(&self) -> String {
        format!("{} {}", self.field, self.direction.to_sql())
    }
}

impl FromStr for SortField {
    type Err = std::convert::Infallible;

    /// Parse from string (e.g., "name", "-created_at", "+email")
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if let Some(field) = s.strip_prefix('-') {
            Self::desc(field)
        } else if let Some(field) = s.strip_prefix('+') {
            Self::asc(field)
        } else {
            Self::asc(s)
        })
    }
}

/// Multi-field sorting parameters
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("sort".to_string(), "-created_at,name".to_string());
///
/// let sorting = SortParams::from_query(&params);
/// assert_eq!(sorting.fields.len(), 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortParams {
    /// Sort fields
    pub fields: Vec<SortField>,
}

impl SortParams {
    /// Create new sort params
    pub fn new(fields: Vec<SortField>) -> Self {
        Self { fields }
    }

    /// Parse from query parameters
    ///
    /// Looks for `sort` or `order_by` parameter.
    /// Format: "field1,-field2,+field3" (- prefix = DESC, + or no prefix = ASC)
    pub fn from_query(params: &HashMap<String, String>) -> Self {
        let sort_str = params
            .get("sort")
            .or_else(|| params.get("order_by"))
            .map(|s| s.as_str())
            .unwrap_or("");

        if sort_str.is_empty() {
            return Self::new(vec![]);
        }

        let fields = sort_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<SortField>().unwrap())
            .collect();

        Self::new(fields)
    }

    /// Check if sorting is specified
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Convert to SQL ORDER BY clause
    pub fn to_sql(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }

        Some(
            self.fields
                .iter()
                .map(|f| f.to_sql())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

impl Default for SortParams {
    fn default() -> Self {
        Self::new(vec![])
    }
}

// ============================================================================
// FILTERING
// ============================================================================

/// Filter operator
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterOperator {
    /// Equal (=)
    Eq,
    /// Not equal (!=)
    Ne,
    /// Greater than (>)
    Gt,
    /// Greater than or equal (>=)
    Gte,
    /// Less than (<)
    Lt,
    /// Less than or equal (<=)
    Lte,
    /// In list
    In,
    /// Not in list
    NotIn,
    /// Contains substring
    Contains,
    /// Starts with
    StartsWith,
    /// Ends with
    EndsWith,
    /// Is null
    IsNull,
    /// Is not null
    IsNotNull,
}

impl FilterOperator {
    /// Parse from string suffix
    pub fn from_suffix(suffix: &str) -> Option<Self> {
        match suffix {
            "eq" => Some(FilterOperator::Eq),
            "ne" | "neq" => Some(FilterOperator::Ne),
            "gt" => Some(FilterOperator::Gt),
            "gte" | "ge" => Some(FilterOperator::Gte),
            "lt" => Some(FilterOperator::Lt),
            "lte" | "le" => Some(FilterOperator::Lte),
            "in" => Some(FilterOperator::In),
            "not_in" | "nin" => Some(FilterOperator::NotIn),
            "contains" | "like" => Some(FilterOperator::Contains),
            "starts_with" | "startswith" => Some(FilterOperator::StartsWith),
            "ends_with" | "endswith" => Some(FilterOperator::EndsWith),
            "is_null" | "isnull" => Some(FilterOperator::IsNull),
            "is_not_null" | "isnotnull" | "not_null" => Some(FilterOperator::IsNotNull),
            _ => None,
        }
    }

    /// Convert to SQL operator
    pub fn to_sql(&self) -> &'static str {
        match self {
            FilterOperator::Eq => "=",
            FilterOperator::Ne => "!=",
            FilterOperator::Gt => ">",
            FilterOperator::Gte => ">=",
            FilterOperator::Lt => "<",
            FilterOperator::Lte => "<=",
            FilterOperator::In => "IN",
            FilterOperator::NotIn => "NOT IN",
            FilterOperator::Contains => "LIKE",
            FilterOperator::StartsWith => "LIKE",
            FilterOperator::EndsWith => "LIKE",
            FilterOperator::IsNull => "IS NULL",
            FilterOperator::IsNotNull => "IS NOT NULL",
        }
    }
}

/// Single filter condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterCondition {
    /// Field name
    pub field: String,

    /// Filter operator
    pub operator: FilterOperator,

    /// Filter value(s)
    pub value: Option<String>,
}

impl FilterCondition {
    /// Create new filter condition
    pub fn new(field: impl Into<String>, operator: FilterOperator, value: Option<String>) -> Self {
        Self {
            field: field.into(),
            operator,
            value,
        }
    }
}

/// Query parameter filters
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("status".to_string(), "active".to_string());
/// params.insert("age__gte".to_string(), "18".to_string());
/// params.insert("name__contains".to_string(), "john".to_string());
///
/// let filters = FilterParams::from_query(&params);
/// assert_eq!(filters.conditions.len(), 3);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterParams {
    /// Filter conditions
    pub conditions: Vec<FilterCondition>,
}

impl FilterParams {
    /// Create new filter params
    pub fn new(conditions: Vec<FilterCondition>) -> Self {
        Self { conditions }
    }

    /// Parse from query parameters
    ///
    /// Supports formats:
    /// - `field=value` → field = value
    /// - `field__op=value` → field op value (e.g., age__gte=18)
    pub fn from_query(params: &HashMap<String, String>) -> Self {
        let mut conditions = Vec::new();

        // Skip known pagination/sorting params
        let skip_params = [
            "page", "per_page", "limit", "cursor", "sort", "order_by", "fields", "q", "search",
        ];

        for (key, value) in params {
            if skip_params.contains(&key.as_str()) {
                continue;
            }

            // Parse field__operator format
            if let Some((field, op_str)) = key.split_once("__")
                && let Some(operator) = FilterOperator::from_suffix(op_str)
            {
                conditions.push(FilterCondition::new(field, operator, Some(value.clone())));
                continue;
            }

            // Default to equality
            conditions.push(FilterCondition::new(
                key,
                FilterOperator::Eq,
                Some(value.clone()),
            ));
        }

        Self::new(conditions)
    }

    /// Check if filters are empty
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    /// Get filter for specific field
    pub fn get(&self, field: &str) -> Option<&FilterCondition> {
        self.conditions.iter().find(|c| c.field == field)
    }
}

impl Default for FilterParams {
    fn default() -> Self {
        Self::new(vec![])
    }
}

// ============================================================================
// SEARCH
// ============================================================================

/// Search parameters
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("q".to_string(), "search term".to_string());
///
/// let search = SearchParams::from_query(&params);
/// assert_eq!(search.query, Some("search term".to_string()));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    /// Search query
    pub query: Option<String>,

    /// Fields to search in
    pub fields: Vec<String>,
}

impl SearchParams {
    /// Create new search params
    pub fn new(query: Option<String>, fields: Vec<String>) -> Self {
        Self { query, fields }
    }

    /// Parse from query parameters
    ///
    /// Looks for `q` or `search` parameter.
    pub fn from_query(params: &HashMap<String, String>) -> Self {
        let query = params.get("q").or_else(|| params.get("search")).cloned();

        let fields = params
            .get("search_fields")
            .map(|s| s.split(',').map(|f| f.trim().to_string()).collect())
            .unwrap_or_default();

        Self::new(query, fields)
    }

    /// Check if search is active
    pub fn is_active(&self) -> bool {
        self.query.as_ref().map(|q| !q.is_empty()).unwrap_or(false)
    }
}

impl Default for SearchParams {
    fn default() -> Self {
        Self::new(None, vec![])
    }
}

// ============================================================================
// FIELD SELECTION
// ============================================================================

/// Field selection parameters (sparse fieldsets)
///
/// Allows clients to request only specific fields.
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("fields".to_string(), "id,name,email".to_string());
///
/// let fields = FieldSelection::from_query(&params);
/// assert!(fields.should_include("name"));
/// assert!(!fields.should_include("password"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSelection {
    /// Fields to include
    pub include: Option<Vec<String>>,

    /// Fields to exclude
    pub exclude: Option<Vec<String>>,
}

impl FieldSelection {
    /// Create new field selection
    pub fn new(include: Option<Vec<String>>, exclude: Option<Vec<String>>) -> Self {
        Self { include, exclude }
    }

    /// Include specific fields
    pub fn include(fields: Vec<String>) -> Self {
        Self::new(Some(fields), None)
    }

    /// Exclude specific fields
    pub fn exclude(fields: Vec<String>) -> Self {
        Self::new(None, Some(fields))
    }

    /// Parse from query parameters
    ///
    /// Supports:
    /// - `fields=id,name,email` → include only these fields
    /// - `exclude=password,secret` → exclude these fields
    pub fn from_query(params: &HashMap<String, String>) -> Self {
        let include = params.get("fields").map(|s| {
            s.split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect()
        });

        let exclude = params.get("exclude").map(|s| {
            s.split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect()
        });

        Self::new(include, exclude)
    }

    /// Check if a field should be included
    pub fn should_include(&self, field: &str) -> bool {
        // If include is specified, field must be in it
        if let Some(ref include) = self.include
            && !include.contains(&field.to_string())
        {
            return false;
        }

        // If exclude is specified, field must not be in it
        if let Some(ref exclude) = self.exclude
            && exclude.contains(&field.to_string())
        {
            return false;
        }

        true
    }

    /// Check if field selection is active
    pub fn is_active(&self) -> bool {
        self.include.is_some() || self.exclude.is_some()
    }
}

impl Default for FieldSelection {
    fn default() -> Self {
        Self::new(None, None)
    }
}

// ============================================================================
// COMBINED QUERY PARAMS
// ============================================================================

/// Combined query parameters for pagination, sorting, filtering, and search
///
/// # Examples
///
/// ```
/// use armature_core::*;
/// use std::collections::HashMap;
///
/// let mut params = HashMap::new();
/// params.insert("page".to_string(), "2".to_string());
/// params.insert("sort".to_string(), "-created_at".to_string());
/// params.insert("status".to_string(), "active".to_string());
///
/// let query = QueryParams::from_hashmap(&params);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParams {
    /// Pagination
    pub pagination: OffsetPagination,

    /// Sorting
    pub sort: SortParams,

    /// Filters
    pub filter: FilterParams,

    /// Search
    pub search: SearchParams,

    /// Field selection
    pub fields: FieldSelection,
}

impl QueryParams {
    /// Parse all query parameters
    pub fn from_hashmap(params: &HashMap<String, String>) -> Self {
        Self {
            pagination: OffsetPagination::from_query_params(params),
            sort: SortParams::from_query(params),
            filter: FilterParams::from_query(params),
            search: SearchParams::from_query(params),
            fields: FieldSelection::from_query(params),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offset_pagination() {
        let p = OffsetPagination::new(1, 20);
        assert_eq!(p.offset(), 0);
        assert_eq!(p.limit(), 20);

        let p = OffsetPagination::new(2, 20);
        assert_eq!(p.offset(), 20);

        let p = OffsetPagination::new(3, 50);
        assert_eq!(p.offset(), 100);
    }

    #[test]
    fn test_offset_pagination_total_pages() {
        let p = OffsetPagination::new(1, 20);
        assert_eq!(p.total_pages(100), 5);
        assert_eq!(p.total_pages(95), 5);
        assert_eq!(p.total_pages(101), 6);
    }

    #[test]
    fn test_sort_field_from_str() {
        let f: SortField = "name".parse().unwrap();
        assert_eq!(f.field, "name");
        assert_eq!(f.direction, SortDirection::Asc);

        let f: SortField = "-created_at".parse().unwrap();
        assert_eq!(f.field, "created_at");
        assert_eq!(f.direction, SortDirection::Desc);

        let f: SortField = "+email".parse().unwrap();
        assert_eq!(f.field, "email");
        assert_eq!(f.direction, SortDirection::Asc);
    }

    #[test]
    fn test_sort_params_from_query() {
        let mut params = HashMap::new();
        params.insert("sort".to_string(), "-created_at,name,+email".to_string());

        let sort = SortParams::from_query(&params);
        assert_eq!(sort.fields.len(), 3);
        assert_eq!(sort.fields[0].field, "created_at");
        assert_eq!(sort.fields[0].direction, SortDirection::Desc);
        assert_eq!(sort.fields[1].field, "name");
        assert_eq!(sort.fields[1].direction, SortDirection::Asc);
    }

    #[test]
    fn test_filter_params_from_query() {
        let mut params = HashMap::new();
        params.insert("status".to_string(), "active".to_string());
        params.insert("age__gte".to_string(), "18".to_string());

        let filters = FilterParams::from_query(&params);
        assert_eq!(filters.conditions.len(), 2);

        let status_filter = filters.get("status").unwrap();
        assert_eq!(status_filter.operator, FilterOperator::Eq);

        let age_filter = filters.get("age").unwrap();
        assert_eq!(age_filter.operator, FilterOperator::Gte);
    }

    #[test]
    fn test_field_selection() {
        let mut params = HashMap::new();
        params.insert("fields".to_string(), "id,name,email".to_string());

        let fields = FieldSelection::from_query(&params);
        assert!(fields.should_include("name"));
        assert!(!fields.should_include("password"));
    }

    #[test]
    fn test_field_selection_exclude() {
        let mut params = HashMap::new();
        params.insert("exclude".to_string(), "password,secret".to_string());

        let fields = FieldSelection::from_query(&params);
        assert!(fields.should_include("name"));
        assert!(!fields.should_include("password"));
    }
}
