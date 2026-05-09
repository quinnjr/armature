# Pagination & Filtering Guide

Comprehensive guide to pagination, sorting, filtering, search, and field selection in Armature.

## Table of Contents

- [Overview](#overview)
- [Pagination](#pagination)
- [Sorting](#sorting)
- [Filtering](#filtering)
- [Search](#search)
- [Field Selection](#field-selection)
- [Combined Queries](#combined-queries)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

---

## Overview

Armature provides powerful utilities for building flexible, performant APIs with:

- **Offset Pagination** - Traditional page-based pagination
- **Cursor Pagination** - For real-time/streaming data
- **Multi-field Sorting** - Sort by multiple fields with direction
- **Query Filtering** - Rich filter operators
- **Full-text Search** - Search integration points
- **Field Selection** - Sparse fieldsets (GraphQL-like)

All features parse from standard query parameters and work together seamlessly.

---

## Pagination

### Offset Pagination

Traditional page-based pagination using page number and page size.

#### Usage

```rust
use armature_core::*;
use std::collections::HashMap;

// Parse from query params
let mut params = HashMap::new();
params.insert("page".to_string(), "2".to_string());
params.insert("per_page".to_string(), "50".to_string());

let pagination = OffsetPagination::from_query_params(&params);

// Use in database query
let offset = pagination.offset(); // 50
let limit = pagination.limit();   // 50
```

#### Query Parameters

```
GET /users?page=2&per_page=50
```

| Parameter | Description | Default |
|-----------|-------------|---------|
| `page` | Page number (1-indexed) | 1 |
| `per_page` or `limit` | Items per page | 20 |

### Cursor Pagination

Opaque cursor-based pagination for real-time data and infinite scroll.

#### Query Parameters

```
GET /users?cursor=eyJpZCI6MTIzfQ&limit=20
```

| Parameter | Description | Default |
|-----------|-------------|---------|
| `cursor` | Opaque cursor for next page | None |
| `limit` | Items per page | 20 |

---

## Sorting

Multi-field sorting with ascending/descending order.

### Usage

```rust
let params = HashMap::from([
    ("sort".to_string(), "-created_at,name".to_string())
]);

let sorting = SortParams::from_query(&params);
```

### Query Parameters

```
GET /users?sort=-created_at,+name,email
```

**Format:** Comma-separated field names with optional prefix:
- `-field` → Descending (DESC)
- `+field` or `field` → Ascending (ASC)

### Examples

| Query | Meaning |
|-------|---------|
| `?sort=name` | Sort by name ascending |
| `?sort=-created_at` | Sort by created_at descending |
| `?sort=-age,name` | Sort by age DESC, then name ASC |
| `?sort=+email,-created_at` | Sort by email ASC, then created_at DESC |

---

## Filtering

Rich query parameter filtering with multiple operators.

### Query Parameters

**Format:** `field__operator=value`

```
GET /users?status=active&age__gte=18&name__contains=john
```

### Supported Operators

| Operator | Query Param | SQL | Example |
|----------|-------------|-----|---------|
| Equal | `field=value` | `=` | `status=active` |
| Not Equal | `field__ne=value` | `!=` | `status__ne=inactive` |
| Greater Than | `field__gt=value` | `>` | `age__gt=18` |
| Greater or Equal | `field__gte=value` | `>=` | `age__gte=18` |
| Less Than | `field__lt=value` | `<` | `age__lt=65` |
| Less or Equal | `field__lte=value` | `<=` | `age__lte=65` |
| In List | `field__in=val1,val2` | `IN` | `status__in=active,pending` |
| Not In | `field__not_in=val1,val2` | `NOT IN` | `role__not_in=admin` |
| Contains | `field__contains=value` | `LIKE %value%` | `name__contains=john` |
| Starts With | `field__starts_with=value` | `LIKE value%` | `email__starts_with=admin` |
| Ends With | `field__ends_with=value` | `LIKE %value` | `domain__ends_with=.com` |
| Is Null | `field__is_null=true` | `IS NULL` | `deleted_at__is_null=true` |
| Is Not Null | `field__is_not_null=true` | `IS NOT NULL` | `email__is_not_null=true` |

---

## Search

Full-text search integration.

### Query Parameters

```
GET /users?q=search+term
GET /users?search=john+doe
GET /users?q=keyword&search_fields=name,email,bio
```

| Parameter | Description |
|-----------|-------------|
| `q` or `search` | Search query |
| `search_fields` | Comma-separated fields to search in |

---

## Field Selection

Sparse fieldsets allow clients to request only specific fields (like GraphQL).

### Query Parameters

```
GET /users?fields=id,name,email
GET /users?exclude=password,secret
```

| Parameter | Description |
|-----------|-------------|
| `fields` | Comma-separated fields to include |
| `exclude` | Comma-separated fields to exclude |

---

## Combined Queries

All query features work together seamlessly.

### Example: Complete Query

```
GET /users?page=2&per_page=20&sort=-created_at,name&status=active&age__gte=25&q=developer&fields=id,name,email
```

Breakdown:
- **Pagination:** Page 2, 20 items per page
- **Sorting:** By created_at DESC, then name ASC
- **Filtering:** Active users aged 25+
- **Search:** Contains "developer"
- **Fields:** Return only id, name, email

### Parsing All Parameters

```rust
use armature_core::*;

let query = QueryParams::from_hashmap(&req.query_params);

// Access all parsed parameters
let pagination = query.pagination; // OffsetPagination
let sorting = query.sort;          // SortParams
let filters = query.filter;        // FilterParams
let search = query.search;         // SearchParams
let fields = query.fields;         // FieldSelection
```

---

## Best Practices

### 1. Set Maximum Page Size

```rust
pub const MAX_PAGE_SIZE: usize = 100;

let per_page = per_page.clamp(1, MAX_PAGE_SIZE);
```

### 2. Provide Default Sorting

```rust
let sorting = SortParams::from_query(&params);

if sorting.is_empty() {
    sorting = SortParams::new(vec![SortField::desc("created_at")]);
}
```

### 3. Validate Filter Fields

```rust
const ALLOWED_FILTERS: &[&str] = &["status", "age", "role"];

for condition in &filters.conditions {
    if !ALLOWED_FILTERS.contains(&condition.field.as_str()) {
        return Err(Error::BadRequest(format!(
            "Filtering by '{}' is not allowed",
            condition.field
        )));
    }
}
```

### 4. Index Database Columns

```sql
-- Index filtered columns
CREATE INDEX idx_users_status ON users(status);
CREATE INDEX idx_users_created_at ON users(created_at);

-- Composite index for common queries
CREATE INDEX idx_users_status_created ON users(status, created_at DESC);
```

---

## Summary

**Key Points:**

1. **Offset Pagination** - Traditional page-based (page/per_page)
2. **Cursor Pagination** - For real-time data (cursor/limit)
3. **Multi-field Sorting** - `-field` for DESC, `+field` or `field` for ASC
4. **Rich Filtering** - `field__operator=value` format
5. **Search Integration** - `q` or `search` parameter
6. **Field Selection** - `fields` or `exclude` parameters
7. **Combined Queries** - All features work together

**Quick Reference:**

```rust
// Parse all parameters at once
let query = QueryParams::from_hashmap(&req.query_params);

// Or individually
let pagination = OffsetPagination::from_query_params(&params);
let sorting = SortParams::from_query(&params);
let filters = FilterParams::from_query(&params);
let search = SearchParams::from_query(&params);
let fields = FieldSelection::from_query(&params);
```

**Common Patterns:**

```
# Pagination
?page=2&per_page=50

# Sorting (- = DESC)
?sort=-created_at,name

# Filtering
?status=active&age__gte=18

# Search
?q=keyword

# Fields
?fields=id,name,email

# Combined
?page=1&sort=-age&status=active&fields=id,name
```

