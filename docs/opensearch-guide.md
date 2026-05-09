# OpenSearch Guide

Integration guide for OpenSearch with the Armature framework.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Document Operations](#document-operations)
- [Search](#search)
- [Query DSL](#query-dsl)
- [Aggregations](#aggregations)
- [Index Management](#index-management)
- [Bulk Operations](#bulk-operations)
- [AWS OpenSearch Service](#aws-opensearch-service)
- [Best Practices](#best-practices)
- [Examples](#examples)

---

## Overview

`armature-opensearch` provides a high-level client for OpenSearch with full support for:

- Document indexing, searching, and management
- Query DSL builder for complex queries
- Aggregations for analytics
- Index lifecycle management
- Bulk operations for high throughput
- AWS OpenSearch Service authentication

---

## Features

- ✅ Type-safe document operations with the `Document` trait
- ✅ Fluent query DSL builder
- ✅ Search with pagination, sorting, highlighting
- ✅ Aggregations (terms, histogram, metrics)
- ✅ Index management (create, delete, mappings)
- ✅ Bulk operations (index, delete)
- ✅ AWS OpenSearch Service support
- ✅ TLS support (rustls/native-tls)

---

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-opensearch = "0.1"
```

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `rustls` | ✅ | TLS via rustls |
| `native-tls` | | TLS via native-tls |
| `aws-auth` | | AWS OpenSearch Service authentication |
| `bulk-stream` | | Streaming bulk operations |

```toml
# With AWS auth
armature-opensearch = { version = "0.1", features = ["aws-auth"] }

# With native TLS
armature-opensearch = { version = "0.1", default-features = false, features = ["native-tls"] }
```

---

## Quick Start

### Define a Document

```rust
use armature_opensearch::{Document, OpenSearchClient, OpenSearchConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Article {
    title: String,
    body: String,
    author: String,
    tags: Vec<String>,
    published_at: String,
}

impl Document for Article {
    fn index_name() -> &'static str {
        "articles"
    }
}
```

### Create a Client

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = OpenSearchClient::new(
        OpenSearchConfig::new("http://localhost:9200")
    )?;

    // Index a document
    let article = Article {
        title: "Getting Started with OpenSearch".to_string(),
        body: "OpenSearch is a powerful search engine...".to_string(),
        author: "Alice".to_string(),
        tags: vec!["tutorial".to_string(), "search".to_string()],
        published_at: "2024-12-20".to_string(),
    };

    client.index("article-1", &article).await?;

    // Search
    let results: Vec<Article> = client
        .search()
        .query_string("getting started")
        .size(10)
        .execute()
        .await?;

    println!("Found {} articles", results.len());
    Ok(())
}
```

---

## Configuration

### Basic Configuration

```rust
use armature_opensearch::OpenSearchConfig;

let config = OpenSearchConfig::new("http://localhost:9200");
```

### With Authentication

```rust
let config = OpenSearchConfig::new("https://opensearch.example.com")
    .with_basic_auth("admin", "password");
```

### With TLS

```rust
use armature_opensearch::TlsConfig;

let config = OpenSearchConfig::new("https://opensearch.example.com")
    .with_tls(TlsConfig::with_ca_cert("/path/to/ca.crt"));
```

### Cluster Configuration

```rust
let config = OpenSearchConfig::cluster(vec![
    "http://node1:9200".to_string(),
    "http://node2:9200".to_string(),
    "http://node3:9200".to_string(),
]);
```

### Configuration Options

```rust
let config = OpenSearchConfig::new("http://localhost:9200")
    .with_basic_auth("user", "pass")
    .with_connect_timeout(Duration::from_secs(10))
    .with_request_timeout(Duration::from_secs(30))
    .with_compression(true)
    .with_max_retries(3);
```

---

## Document Operations

### Index a Document

```rust
// With explicit ID
client.index("doc-1", &article).await?;

// With auto-generated ID
let id = client.index_auto_id(&article).await?;
```

### Get a Document

```rust
let article: Option<Article> = client.get("doc-1").await?;

if let Some(article) = article {
    println!("Found: {}", article.title);
}
```

### Check if Document Exists

```rust
let exists = client.exists::<Article>("doc-1").await?;
```

### Update a Document

```rust
// Full update
article.title = "Updated Title".to_string();
client.update("doc-1", &article).await?;

// Partial update
use serde_json::json;
client.partial_update::<Article>("doc-1", json!({
    "title": "New Title"
})).await?;
```

### Delete a Document

```rust
let deleted = client.delete::<Article>("doc-1").await?;
```

### Delete by Query

```rust
use serde_json::json;

let deleted_count = client.delete_by_query(
    "articles",
    json!({ "term": { "author": "Alice" } })
).await?;
```

---

## Search

### Basic Search

```rust
let results: Vec<Article> = client
    .search()
    .index("articles")
    .query_string("opensearch tutorial")
    .execute()
    .await?;
```

### With Pagination

```rust
let results: Vec<Article> = client
    .search()
    .index("articles")
    .from(0)
    .size(20)
    .execute()
    .await?;
```

### With Sorting

```rust
use armature_opensearch::SortOrder;

let results: Vec<Article> = client
    .search()
    .index("articles")
    .sort_by("published_at", SortOrder::Desc)
    .sort_by_score(SortOrder::Desc)
    .execute()
    .await?;
```

### With Source Filtering

```rust
let results: Vec<Article> = client
    .search()
    .index("articles")
    .source_includes(vec!["title".to_string(), "author".to_string()])
    .source_excludes(vec!["body".to_string()])
    .execute()
    .await?;
```

### With Highlighting

```rust
let results = client
    .search()
    .index("articles")
    .query_string("opensearch")
    .highlight(vec!["title".to_string(), "body".to_string()])
    .execute_with_meta::<Article>()
    .await?;

for hit in results.hits {
    if let Some(highlight) = &hit.meta.highlight {
        if let Some(titles) = highlight.get("title") {
            println!("Highlighted title: {:?}", titles);
        }
    }
}
```

### Search Result with Metadata

```rust
let result = client
    .search()
    .index("articles")
    .query_string("tutorial")
    .execute_with_meta::<Article>()
    .await?;

println!("Total hits: {}", result.total);
println!("Max score: {:?}", result.max_score);
println!("Took: {}ms", result.took_ms);

for hit in result.hits {
    println!("ID: {}, Score: {:?}", hit.meta.id, hit.meta.score);
    println!("Title: {}", hit.doc.title);
}
```

### Count Documents

```rust
let count = client
    .search()
    .index("articles")
    .query_string("tutorial")
    .count()
    .await?;
```

---

## Query DSL

### Match Query

```rust
use armature_opensearch::{Query, MatchQuery};

let results: Vec<Article> = client
    .search()
    .query(Query::Match(MatchQuery::new("title", "opensearch tutorial")))
    .execute()
    .await?;
```

### Term Query (Exact Match)

```rust
use armature_opensearch::{Query, TermQuery};

let results: Vec<Article> = client
    .search()
    .query(Query::Term(TermQuery::new("author", "Alice")))
    .execute()
    .await?;
```

### Range Query

```rust
use armature_opensearch::{Query, RangeQuery};
use serde_json::json;

let results: Vec<Article> = client
    .search()
    .query(Query::Range(
        RangeQuery::new("published_at")
            .gte(json!("2024-01-01"))
            .lte(json!("2024-12-31"))
    ))
    .execute()
    .await?;
```

### Bool Query (Compound)

```rust
use armature_opensearch::{Query, MatchQuery, TermQuery, BoolQueryBuilder};

let query = BoolQueryBuilder::new()
    .must(Query::Match(MatchQuery::new("title", "opensearch")))
    .must(Query::Term(TermQuery::new("author", "Alice")))
    .should(Query::Match(MatchQuery::new("tags", "tutorial")))
    .must_not(Query::Term(TermQuery::new("status", "draft")))
    .filter(Query::Range(RangeQuery::new("published_at").gte(json!("2024-01-01"))))
    .minimum_should_match(1)
    .build();

let results: Vec<Article> = client
    .search()
    .query(query)
    .execute()
    .await?;
```

### Query String Query

```rust
// Supports Lucene query syntax
let results: Vec<Article> = client
    .search()
    .query_string("title:opensearch AND author:Alice")
    .execute()
    .await?;
```

### Raw JSON Query

```rust
use serde_json::json;

let results: Vec<Article> = client
    .search()
    .query_json(json!({
        "bool": {
            "must": [
                { "match": { "title": "opensearch" } }
            ],
            "filter": [
                { "term": { "status": "published" } }
            ]
        }
    }))
    .execute()
    .await?;
```

---

## Aggregations

### Terms Aggregation

```rust
use armature_opensearch::Aggregation;

let result = client
    .search()
    .index("articles")
    .aggregation("by_author", Aggregation::terms("author"))
    .size(0)  // Only return aggregations
    .execute_with_meta::<Article>()
    .await?;

if let Some(aggs) = result.aggregations {
    println!("Aggregations: {:?}", aggs);
}
```

### Metrics Aggregations

```rust
let result = client
    .search()
    .index("articles")
    .aggregation("avg_views", Aggregation::avg("view_count"))
    .aggregation("max_views", Aggregation::Max { field: "view_count".to_string() })
    .aggregation("total_views", Aggregation::Sum { field: "view_count".to_string() })
    .execute_with_meta::<Article>()
    .await?;
```

### Date Histogram

```rust
let result = client
    .search()
    .index("articles")
    .aggregation("articles_over_time", Aggregation::DateHistogram {
        field: "published_at".to_string(),
        calendar_interval: Some("month".to_string()),
        fixed_interval: None,
    })
    .execute_with_meta::<Article>()
    .await?;
```

---

## Index Management

### Create an Index

```rust
use armature_opensearch::{IndexSettings, Mapping, MappingField};

let settings = IndexSettings::new()
    .shards(3)
    .replicas(1)
    .refresh_interval("1s")
    .mappings(
        Mapping::new()
            .field("title", MappingField::text().analyzer("standard"))
            .field("author", MappingField::keyword())
            .field("published_at", MappingField::date())
            .field("view_count", MappingField::integer())
    );

client.indices().create("articles", settings).await?;
```

### Check if Index Exists

```rust
let exists = client.indices().exists("articles").await?;
```

### Delete an Index

```rust
client.indices().delete("articles").await?;
```

### Update Mappings

```rust
let mapping = Mapping::new()
    .field("new_field", MappingField::keyword());

client.indices().put_mapping("articles", mapping).await?;
```

### Index Aliases

```rust
// Create alias
client.indices().create_alias("articles", "articles-read").await?;

// Delete alias
client.indices().delete_alias("articles", "articles-read").await?;
```

### List Indices

```rust
let indices = client.indices().list().await?;

for index in indices {
    println!("{}: {} docs, {}", index.name, index.docs_count, index.store_size);
}
```

### Refresh Index

```rust
// Make recent changes searchable
client.refresh("articles").await?;
```

---

## Bulk Operations

### Bulk Index

```rust
let docs = vec![
    ("doc-1".to_string(), article1),
    ("doc-2".to_string(), article2),
    ("doc-3".to_string(), article3),
];

let indexed_count = client.bulk_index(docs).await?;
```

### Bulk Delete

```rust
let ids = vec![
    "doc-1".to_string(),
    "doc-2".to_string(),
    "doc-3".to_string(),
];

let deleted_count = client.bulk_delete::<Article>(ids).await?;
```

---

## AWS OpenSearch Service

Enable the `aws-auth` feature:

```toml
armature-opensearch = { version = "0.1", features = ["aws-auth"] }
```

### Configuration

```rust
let config = OpenSearchConfig::new("https://search-mydomain-xxxx.us-east-1.es.amazonaws.com")
    .with_aws_region("us-east-1");

let client = OpenSearchClient::new(config)?;
```

AWS credentials are automatically loaded from the environment (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY) or IAM role.

---

## Best Practices

### 1. Use Appropriate Analyzers

```rust
// Full-text search fields
MappingField::text().analyzer("standard")

// Exact match fields (IDs, enums, tags)
MappingField::keyword()
```

### 2. Use Filters for Non-Scoring Queries

```rust
// ✅ Good - uses filter for exact match (faster, cached)
BoolQueryBuilder::new()
    .must(Query::Match(MatchQuery::new("title", "search")))
    .filter(Query::Term(TermQuery::new("status", "published")))
    .build()

// ❌ Less efficient - scoring on exact match
BoolQueryBuilder::new()
    .must(Query::Match(MatchQuery::new("title", "search")))
    .must(Query::Term(TermQuery::new("status", "published")))
    .build()
```

### 3. Use Bulk Operations for High Throughput

```rust
// ✅ Good - single bulk request
client.bulk_index(docs).await?;

// ❌ Slow - individual requests
for (id, doc) in docs {
    client.index(&id, &doc).await?;
}
```

### 4. Refresh After Bulk Operations

```rust
client.bulk_index(docs).await?;
client.refresh("articles").await?;  // Make searchable immediately
```

### 5. Use Pagination for Large Result Sets

```rust
let mut from = 0;
let size = 100;

loop {
    let results: Vec<Article> = client
        .search()
        .from(from)
        .size(size)
        .execute()
        .await?;

    if results.is_empty() {
        break;
    }

    // Process results
    from += size;
}
```

---

## Examples

### Full-Text Search with Filters

```rust
let results: Vec<Article> = client
    .search()
    .index("articles")
    .query(
        BoolQueryBuilder::new()
            .must(Query::Match(MatchQuery::new("body", "rust programming")))
            .filter(Query::Term(TermQuery::new("published", true)))
            .filter(Query::Range(RangeQuery::new("published_at").gte(json!("2024-01-01"))))
            .build()
    )
    .sort_by("_score", SortOrder::Desc)
    .sort_by("published_at", SortOrder::Desc)
    .highlight(vec!["title".to_string(), "body".to_string()])
    .size(20)
    .execute()
    .await?;
```

### Analytics Dashboard

```rust
let result = client
    .search()
    .index("articles")
    .aggregation("by_author", Aggregation::terms("author"))
    .aggregation("by_month", Aggregation::DateHistogram {
        field: "published_at".to_string(),
        calendar_interval: Some("month".to_string()),
        fixed_interval: None,
    })
    .aggregation("avg_views", Aggregation::avg("view_count"))
    .aggregation("total_articles", Aggregation::Cardinality { field: "_id".to_string() })
    .size(0)
    .execute_with_meta::<Article>()
    .await?;
```

### Health Check

```rust
if client.ping().await? {
    let health = client.health().await?;
    println!("Cluster status: {}", health["status"]);
}
```

---

## Summary

### Key APIs

```rust
// Client
OpenSearchClient::new(config)?

// Documents
client.index(id, &doc).await?
client.get::<T>(id).await?
client.update(id, &doc).await?
client.delete::<T>(id).await?

// Search
client.search().query(...).execute::<T>().await?

// Index Management
client.indices().create(name, settings).await?
client.indices().delete(name).await?

// Bulk
client.bulk_index(docs).await?
client.bulk_delete::<T>(ids).await?
```

### Environment Variables

For AWS OpenSearch Service:
```bash
AWS_ACCESS_KEY_ID=xxx
AWS_SECRET_ACCESS_KEY=xxx
AWS_REGION=us-east-1
```

---

**Happy searching!** 🔍

