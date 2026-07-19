# armature-opensearch

OpenSearch integration for the Armature framework.

## Features

- **Document CRUD** - Index, get, update, delete documents
- **Search** - Full-text and structured queries
- **Query DSL** - Type-safe query builder
- **Bulk Operations** - Efficient batch processing
- **Index Management** - Create, configure, delete indices
- **AWS OpenSearch** - AWS authentication support

## Installation

```toml
[dependencies]
armature-opensearch = "0.1"
```

## Quick Start

```rust,ignore
use armature_opensearch::{OpenSearchClient, OpenSearchConfig, Document, Query};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Product {
    name: String,
    category: String,
    price: f64,
}

impl Document for Product {
    fn index_name() -> &'static str {
        "products"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = OpenSearchConfig::new("http://localhost:9200");
    let client = OpenSearchClient::new(config)?;

    // Index a document
    let product = Product {
        name: "Laptop".to_string(),
        category: "electronics".to_string(),
        price: 999.0,
    };
    client.index("1", &product).await?;

    // Search
    let results: Vec<Product> = client
        .search()
        .index("products")
        .match_field("name", "laptop")
        .size(10)
        .execute()
        .await?;

    // Bulk operations
    use armature_opensearch::BulkOperation;

    client
        .bulk_execute(vec![
            BulkOperation::Index {
                id: "2".to_string(),
                doc: product.clone(),
            },
            BulkOperation::Delete {
                id: "old-id".to_string(),
            },
        ])
        .await?;

    Ok(())
}
```

## Query DSL

```rust,ignore
use armature_opensearch::{Query, QueryBuilder};

let query = QueryBuilder::new()
    .bool_query()
    .must(Query::Match(armature_opensearch::MatchQuery::new(
        "category",
        "electronics",
    )))
    .filter(Query::Range(
        armature_opensearch::RangeQuery::new("price").gte(100).lte(500),
    ))
    .should(Query::Term(armature_opensearch::TermQuery::new(
        "featured", true,
    )))
    .build();
```

## License

MIT OR Apache-2.0
