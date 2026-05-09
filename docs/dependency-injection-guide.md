# Dependency Injection Guide

Armature's dependency injection system lets you use **any Rust crate directly** without special integration packages. Unlike other frameworks that require adapter crates (`framework-sqlx`, `framework-redis`, etc.), Armature works with the Rust ecosystem as-is.

## Overview

With Armature DI, you can inject:

- ✅ **Database clients** (SQLx, Diesel, SeaORM, SurrealDB)
- ✅ **HTTP clients** (reqwest, ureq, hyper)
- ✅ **Cloud SDKs** (AWS SDK, Google Cloud, Azure)
- ✅ **Message queues** (RabbitMQ, Kafka, Redis)
- ✅ **Any `Clone + Send + Sync + 'static` type**

No wrappers. No adapters. Just your crates.

## Quick Example

```rust
use armature_framework::prelude::*;
use sqlx::PgPool;
use aws_sdk_s3::Client as S3Client;
use reqwest::Client as HttpClient;

#[module]
struct AppModule;

#[module_impl]
impl AppModule {
    // Register a PostgreSQL connection pool
    #[provider]
    async fn database() -> PgPool {
        PgPool::connect("postgres://localhost/myapp")
            .await
            .expect("Failed to connect to database")
    }

    // Register AWS S3 client
    #[provider]
    async fn s3_client() -> S3Client {
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        S3Client::new(&config)
    }

    // Register HTTP client
    #[provider]
    fn http_client() -> HttpClient {
        HttpClient::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap()
    }
}

#[controller("/users")]
struct UserController;

#[controller_impl]
impl UserController {
    // All dependencies are automatically injected!
    #[get("/:id")]
    async fn get_user(
        &self,
        #[path] id: i32,
        #[inject] db: PgPool,
        #[inject] s3: S3Client,
    ) -> Result<Json<User>, HttpError> {
        let user = sqlx::query_as!(User, "SELECT * FROM users WHERE id = $1", id)
            .fetch_one(&db)
            .await?;

        Ok(Json(user))
    }
}
```

## Database Connections

### SQLx (PostgreSQL, MySQL, SQLite)

```rust
use sqlx::{PgPool, MySqlPool, SqlitePool};

#[module]
struct DatabaseModule;

#[module_impl]
impl DatabaseModule {
    #[provider]
    async fn postgres() -> PgPool {
        PgPool::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .expect("Failed to connect to PostgreSQL")
    }

    #[provider]
    async fn mysql() -> MySqlPool {
        MySqlPool::connect(&std::env::var("MYSQL_URL").unwrap())
            .await
            .expect("Failed to connect to MySQL")
    }

    #[provider]
    async fn sqlite() -> SqlitePool {
        SqlitePool::connect("sqlite:./data.db")
            .await
            .expect("Failed to connect to SQLite")
    }
}

// Usage in controller
#[controller_impl]
impl UserController {
    #[get("/")]
    async fn list_users(
        &self,
        #[inject] db: PgPool,
    ) -> Result<Json<Vec<User>>, HttpError> {
        let users = sqlx::query_as!(User, "SELECT * FROM users")
            .fetch_all(&db)
            .await?;
        Ok(Json(users))
    }
}
```

### Diesel

```rust
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;

type DbPool = Pool<ConnectionManager<PgConnection>>;

#[module_impl]
impl DatabaseModule {
    #[provider]
    fn diesel_pool() -> DbPool {
        let manager = ConnectionManager::<PgConnection>::new(
            std::env::var("DATABASE_URL").unwrap()
        );
        Pool::builder()
            .max_size(10)
            .build(manager)
            .expect("Failed to create pool")
    }
}

// Usage
#[post("/users")]
async fn create_user(
    &self,
    #[body] input: CreateUser,
    #[inject] pool: DbPool,
) -> Result<Json<User>, HttpError> {
    let conn = pool.get()?;
    let user = diesel::insert_into(users::table)
        .values(&input)
        .get_result(&conn)?;
    Ok(Json(user))
}
```

### SeaORM

```rust
use sea_orm::{Database, DatabaseConnection};

#[module_impl]
impl DatabaseModule {
    #[provider]
    async fn sea_orm() -> DatabaseConnection {
        Database::connect(&std::env::var("DATABASE_URL").unwrap())
            .await
            .expect("Failed to connect")
    }
}

// Usage
#[get("/posts")]
async fn list_posts(
    &self,
    #[inject] db: DatabaseConnection,
) -> Result<Json<Vec<Post>>, HttpError> {
    let posts = Post::find().all(&db).await?;
    Ok(Json(posts))
}
```

### SurrealDB

```rust
use surrealdb::Surreal;
use surrealdb::engine::remote::ws::{Client, Ws};

#[module_impl]
impl DatabaseModule {
    #[provider]
    async fn surrealdb() -> Surreal<Client> {
        let db = Surreal::new::<Ws>("localhost:8000").await.unwrap();
        db.signin(surrealdb::opt::auth::Root {
            username: "root",
            password: "root",
        }).await.unwrap();
        db.use_ns("myapp").use_db("main").await.unwrap();
        db
    }
}
```

## HTTP Clients

### reqwest

```rust
use reqwest::Client;

#[module_impl]
impl HttpModule {
    #[provider]
    fn http_client() -> Client {
        Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .gzip(true)
            .build()
            .unwrap()
    }
}

// Usage
#[get("/external")]
async fn fetch_external(
    &self,
    #[inject] client: Client,
) -> Result<Json<ExternalData>, HttpError> {
    let response = client
        .get("https://api.example.com/data")
        .send()
        .await?
        .json()
        .await?;
    Ok(Json(response))
}
```

### Armature HTTP Client (with retry/circuit breaker)

```rust
use armature_http_client::{HttpClient, HttpClientConfig, RetryConfig};

#[module_impl]
impl HttpModule {
    #[provider]
    fn resilient_client() -> HttpClient {
        let config = HttpClientConfig::builder()
            .timeout(std::time::Duration::from_secs(30))
            .retry(RetryConfig::exponential(3, std::time::Duration::from_millis(100)))
            .circuit_breaker(Default::default())
            .build();
        HttpClient::new(config)
    }
}
```

## Cloud Provider SDKs

### AWS SDK

```rust
use aws_sdk_s3::Client as S3Client;
use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_sqs::Client as SqsClient;
use aws_sdk_sns::Client as SnsClient;
use aws_sdk_ses::Client as SesClient;

#[module]
struct AwsModule;

#[module_impl]
impl AwsModule {
    // Shared AWS config - loaded once
    #[provider]
    async fn aws_config() -> aws_config::SdkConfig {
        aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
    }

    #[provider]
    async fn s3(#[inject] config: aws_config::SdkConfig) -> S3Client {
        S3Client::new(&config)
    }

    #[provider]
    async fn dynamodb(#[inject] config: aws_config::SdkConfig) -> DynamoClient {
        DynamoClient::new(&config)
    }

    #[provider]
    async fn sqs(#[inject] config: aws_config::SdkConfig) -> SqsClient {
        SqsClient::new(&config)
    }

    #[provider]
    async fn sns(#[inject] config: aws_config::SdkConfig) -> SnsClient {
        SnsClient::new(&config)
    }

    #[provider]
    async fn ses(#[inject] config: aws_config::SdkConfig) -> SesClient {
        SesClient::new(&config)
    }
}

// Usage - upload file to S3
#[post("/upload")]
async fn upload_file(
    &self,
    #[body] data: Bytes,
    #[inject] s3: S3Client,
) -> Result<Json<UploadResponse>, HttpError> {
    let key = format!("uploads/{}", uuid::Uuid::new_v4());

    s3.put_object()
        .bucket("my-bucket")
        .key(&key)
        .body(data.into())
        .send()
        .await?;

    Ok(Json(UploadResponse { key }))
}

// Usage - query DynamoDB
#[get("/items/:id")]
async fn get_item(
    &self,
    #[path] id: String,
    #[inject] dynamo: DynamoClient,
) -> Result<Json<Item>, HttpError> {
    let result = dynamo
        .get_item()
        .table_name("items")
        .key("id", aws_sdk_dynamodb::types::AttributeValue::S(id))
        .send()
        .await?;

    let item = result.item.ok_or(HttpError::not_found("Item not found"))?;
    Ok(Json(Item::from_dynamodb(item)))
}
```

### Google Cloud

```rust
use google_cloud_storage::client::Client as GcsClient;
use google_cloud_pubsub::client::Client as PubSubClient;

#[module]
struct GcpModule;

#[module_impl]
impl GcpModule {
    #[provider]
    async fn gcs() -> GcsClient {
        GcsClient::default().await.unwrap()
    }

    #[provider]
    async fn pubsub() -> PubSubClient {
        PubSubClient::default().await.unwrap()
    }
}

// Usage
#[post("/publish")]
async fn publish_message(
    &self,
    #[body] message: PublishRequest,
    #[inject] pubsub: PubSubClient,
) -> Result<StatusCode, HttpError> {
    let topic = pubsub.topic("my-topic");
    let publisher = topic.new_publisher(None);

    publisher
        .publish(message.data.into())
        .await?;

    Ok(StatusCode::ACCEPTED)
}
```

### Azure

```rust
use azure_storage_blobs::prelude::*;
use azure_identity::DefaultAzureCredential;

#[module]
struct AzureModule;

#[module_impl]
impl AzureModule {
    #[provider]
    async fn blob_client() -> ContainerClient {
        let credential = DefaultAzureCredential::default();
        let account = std::env::var("AZURE_STORAGE_ACCOUNT").unwrap();

        BlobServiceClient::new(account, credential)
            .container_client("my-container")
    }
}
```

## Message Queues

### Redis

```rust
use redis::Client as RedisClient;

#[module_impl]
impl CacheModule {
    #[provider]
    fn redis() -> RedisClient {
        RedisClient::open("redis://localhost:6379").unwrap()
    }
}

// Usage
#[get("/cached/:key")]
async fn get_cached(
    &self,
    #[path] key: String,
    #[inject] redis: RedisClient,
) -> Result<Json<Value>, HttpError> {
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let value: Option<String> = redis::cmd("GET")
        .arg(&key)
        .query_async(&mut conn)
        .await?;

    match value {
        Some(v) => Ok(Json(serde_json::from_str(&v)?)),
        None => Err(HttpError::not_found("Key not found")),
    }
}
```

### RabbitMQ (lapin)

```rust
use lapin::{Connection, ConnectionProperties, Channel};

#[module_impl]
impl MessagingModule {
    #[provider]
    async fn rabbitmq_channel() -> Channel {
        let conn = Connection::connect(
            "amqp://localhost:5672",
            ConnectionProperties::default(),
        ).await.unwrap();

        conn.create_channel().await.unwrap()
    }
}

// Usage
#[post("/events")]
async fn publish_event(
    &self,
    #[body] event: Event,
    #[inject] channel: Channel,
) -> Result<StatusCode, HttpError> {
    channel
        .basic_publish(
            "events",
            "event.created",
            Default::default(),
            serde_json::to_vec(&event)?.as_slice(),
            Default::default(),
        )
        .await?;

    Ok(StatusCode::ACCEPTED)
}
```

### Apache Kafka (rdkafka)

```rust
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;

#[module_impl]
impl MessagingModule {
    #[provider]
    fn kafka_producer() -> FutureProducer {
        ClientConfig::new()
            .set("bootstrap.servers", "localhost:9092")
            .set("message.timeout.ms", "5000")
            .create()
            .unwrap()
    }
}

// Usage
#[post("/kafka/publish")]
async fn publish_kafka(
    &self,
    #[body] message: KafkaMessage,
    #[inject] producer: FutureProducer,
) -> Result<StatusCode, HttpError> {
    let record = FutureRecord::to("my-topic")
        .payload(&serde_json::to_string(&message)?)
        .key(&message.key);

    producer.send(record, std::time::Duration::from_secs(5)).await?;
    Ok(StatusCode::ACCEPTED)
}
```

## Search Engines

### Elasticsearch

```rust
use elasticsearch::{Elasticsearch, http::transport::Transport};

#[module_impl]
impl SearchModule {
    #[provider]
    fn elasticsearch() -> Elasticsearch {
        let transport = Transport::single_node("http://localhost:9200").unwrap();
        Elasticsearch::new(transport)
    }
}

// Usage
#[get("/search")]
async fn search(
    &self,
    #[query] q: String,
    #[inject] es: Elasticsearch,
) -> Result<Json<SearchResults>, HttpError> {
    let response = es
        .search(elasticsearch::SearchParts::Index(&["products"]))
        .body(serde_json::json!({
            "query": {
                "match": { "name": q }
            }
        }))
        .send()
        .await?;

    let results: SearchResults = response.json().await?;
    Ok(Json(results))
}
```

### Meilisearch

```rust
use meilisearch_sdk::Client as MeiliClient;

#[module_impl]
impl SearchModule {
    #[provider]
    fn meilisearch() -> MeiliClient {
        MeiliClient::new("http://localhost:7700", Some("masterKey"))
    }
}
```

## Email Services

### Lettre (SMTP)

```rust
use lettre::{SmtpTransport, Transport};

#[module_impl]
impl EmailModule {
    #[provider]
    fn smtp() -> SmtpTransport {
        SmtpTransport::relay("smtp.example.com")
            .unwrap()
            .credentials(lettre::transport::smtp::authentication::Credentials::new(
                std::env::var("SMTP_USER").unwrap(),
                std::env::var("SMTP_PASS").unwrap(),
            ))
            .build()
    }
}
```

## Combining Multiple Services

Real applications use multiple services together:

```rust
#[module]
#[imports(DatabaseModule, AwsModule, CacheModule)]
struct AppModule;

#[controller("/orders")]
struct OrderController;

#[controller_impl]
impl OrderController {
    #[post("/")]
    async fn create_order(
        &self,
        #[body] order: CreateOrder,
        #[inject] db: PgPool,           // Database
        #[inject] redis: RedisClient,    // Cache
        #[inject] sqs: SqsClient,        // Message queue
        #[inject] s3: S3Client,          // File storage
    ) -> Result<Json<Order>, HttpError> {
        // 1. Save to database
        let order = sqlx::query_as!(
            Order,
            "INSERT INTO orders (customer_id, total) VALUES ($1, $2) RETURNING *",
            order.customer_id,
            order.total
        )
        .fetch_one(&db)
        .await?;

        // 2. Invalidate cache
        let mut conn = redis.get_multiplexed_async_connection().await?;
        redis::cmd("DEL")
            .arg(format!("customer:{}:orders", order.customer_id))
            .query_async::<()>(&mut conn)
            .await?;

        // 3. Send to processing queue
        sqs.send_message()
            .queue_url("https://sqs.../order-processing")
            .message_body(serde_json::to_string(&order)?)
            .send()
            .await?;

        // 4. Generate and store invoice PDF
        let invoice_pdf = generate_invoice(&order);
        s3.put_object()
            .bucket("invoices")
            .key(format!("order-{}.pdf", order.id))
            .body(invoice_pdf.into())
            .send()
            .await?;

        Ok(Json(order))
    }
}
```

## Why No Integration Crates?

Other frameworks require special integration crates because they:

1. **Wrap types in framework-specific containers**
2. **Require special initialization hooks**
3. **Need framework-specific error handling**

Armature's DI is different:

| Other Frameworks | Armature |
|-----------------|----------|
| `framework-sqlx = "0.1"` | `sqlx = "0.8"` |
| `framework-redis = "0.2"` | `redis = "0.27"` |
| `framework-aws = "0.1"` | `aws-sdk-s3 = "1.0"` |
| Special `FromRequest` traits | Standard Rust types |
| Framework-specific errors | Your crate's errors |

**Benefits:**

- ✅ Use the latest crate versions immediately
- ✅ No waiting for integration crate updates
- ✅ No version conflicts between framework and crate
- ✅ Standard Rust documentation applies
- ✅ Easier to migrate to/from Armature
- ✅ Test services independently

## Best Practices

### 1. Use Async Providers for I/O

```rust
// ✅ Good - async provider for network I/O
#[provider]
async fn database() -> PgPool {
    PgPool::connect("...").await.unwrap()
}

// ❌ Avoid - blocking in sync provider
#[provider]
fn database() -> PgPool {
    futures::executor::block_on(PgPool::connect("...")).unwrap()
}
```

### 2. Share Configuration

```rust
#[provider]
async fn aws_config() -> aws_config::SdkConfig {
    aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
}

// Inject config into other providers
#[provider]
async fn s3(#[inject] config: aws_config::SdkConfig) -> S3Client {
    S3Client::new(&config)
}
```

### 3. Use Connection Pools

```rust
// ✅ Good - connection pool (shared across requests)
#[provider]
async fn database() -> PgPool {
    PgPool::connect_lazy("...").unwrap()
}

// ❌ Avoid - new connection per request
#[get("/")]
async fn handler() -> Result<...> {
    let conn = PgConnection::connect("...").await?; // Don't do this!
}
```

### 4. Handle Initialization Failures

```rust
#[provider]
async fn critical_service() -> CriticalService {
    CriticalService::connect("...")
        .await
        .expect("Critical service must be available at startup")
}

// Or return Option for optional services
#[provider]
async fn optional_service() -> Option<OptionalService> {
    OptionalService::connect("...").await.ok()
}
```

## Summary

Armature's DI lets you use the entire Rust ecosystem directly:

```rust
// Just add crates to Cargo.toml
sqlx = "0.8"
aws-sdk-s3 = "1.0"
redis = "0.27"
reqwest = "0.12"

// Register in module
#[provider]
async fn my_service() -> MyService { ... }

// Inject anywhere
#[get("/")]
async fn handler(#[inject] service: MyService) { ... }
```

No wrappers. No adapters. No integration crates. Just Rust.

