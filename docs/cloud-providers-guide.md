# Cloud Providers Guide

Armature provides unified cloud provider integrations for AWS, GCP, and Azure with dynamic service loading and dependency injection.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [AWS Integration](#aws-integration)
- [GCP Integration](#gcp-integration)
- [Azure Integration](#azure-integration)
- [Dependency Injection](#dependency-injection)
- [Best Practices](#best-practices)
- [Summary](#summary)

## Overview

The cloud provider crates (`armature-aws`, `armature-gcp`, `armature-azure`) provide:

- **Dynamic Service Loading**: Only compile and load the services you need
- **Feature Flags**: Fine-grained control over dependencies
- **Lazy Initialization**: Services are created on-demand
- **DI Integration**: Register cloud services in your application's DI container
- **Unified Configuration**: Environment-based and programmatic configuration

## Features

- ✅ AWS: S3, DynamoDB, SQS, SNS, SES, Lambda, Secrets Manager, KMS, Cognito, and more
- ✅ GCP: Cloud Storage, Pub/Sub, Firestore, Spanner, BigQuery
- ✅ Azure: Blob Storage, Queue Storage, Cosmos DB, Service Bus, Key Vault
- ✅ Dynamic service loading based on configuration
- ✅ Feature flags for each service
- ✅ DI container integration

## AWS Integration

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-aws = { version = "0.1", features = ["s3", "dynamodb", "sqs"] }
```

### Available Features

| Feature | Services |
|---------|----------|
| `s3` | S3 Object Storage |
| `dynamodb` | DynamoDB NoSQL Database |
| `sqs` | Simple Queue Service |
| `sns` | Simple Notification Service |
| `ses` | Simple Email Service |
| `lambda` | Lambda Functions |
| `secrets-manager` | Secrets Manager |
| `ssm` | Systems Manager Parameter Store |
| `cloudwatch` | CloudWatch Metrics/Logs |
| `kinesis` | Kinesis Data Streams |
| `kms` | Key Management Service |
| `cognito` | Cognito User Pools |
| `storage` | S3 + DynamoDB |
| `messaging` | SQS + SNS + Kinesis |
| `all` | All services |

### Basic Usage

```rust
use armature_aws::{AwsServices, AwsConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure services
    let config = AwsConfig::builder()
        .region("us-east-1")
        .enable_s3()
        .enable_dynamodb()
        .build();

    // Initialize (lazy - clients created on first access)
    let services = AwsServices::new(config).await?;

    // Use S3
    let s3 = services.s3()?;
    let buckets = s3.list_buckets().send().await?;
    println!("Buckets: {:?}", buckets);

    // Use DynamoDB
    let dynamo = services.dynamodb()?;
    let tables = dynamo.list_tables().send().await?;
    println!("Tables: {:?}", tables);

    Ok(())
}
```

### Environment Configuration

```rust
// Reads AWS_REGION, AWS_DEFAULT_REGION, AWS_ENDPOINT_URL
let config = AwsConfig::from_env()
    .enable_s3()
    .enable_sqs()
    .build();
```

### LocalStack Support

```rust
let config = AwsConfig::builder()
    .region("us-east-1")
    .localstack() // Sets endpoint to http://localhost:4566
    .enable_s3()
    .build();
```

## GCP Integration

### Installation

```toml
[dependencies]
armature-gcp = { version = "0.1", features = ["storage", "pubsub"] }
```

### Available Features

| Feature | Services |
|---------|----------|
| `storage` | Cloud Storage |
| `pubsub` | Pub/Sub Messaging |
| `firestore` | Firestore Database |
| `spanner` | Cloud Spanner |
| `bigquery` | BigQuery Analytics |
| `secret-manager` | Secret Manager |
| `data` | Storage + Firestore + Spanner + BigQuery |
| `all` | All services |

### Basic Usage

```rust
use armature_gcp::{GcpServices, GcpConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = GcpConfig::builder()
        .project_id("my-project")
        .enable_storage()
        .enable_pubsub()
        .build();

    let services = GcpServices::new(config).await?;

    // Use Cloud Storage
    let storage = services.storage()?;

    // Use Pub/Sub
    let pubsub = services.pubsub()?;

    Ok(())
}
```

### Environment Configuration

```rust
// Reads GOOGLE_CLOUD_PROJECT, GOOGLE_APPLICATION_CREDENTIALS
let config = GcpConfig::from_env()
    .enable_storage()
    .build();
```

## Azure Integration

### Installation

```toml
[dependencies]
armature-azure = { version = "0.1", features = ["blob", "cosmos"] }
```

### Available Features

| Feature | Services |
|---------|----------|
| `blob` | Blob Storage |
| `queue` | Queue Storage |
| `cosmos` | Cosmos DB |
| `servicebus` | Service Bus |
| `keyvault` | Key Vault |
| `storage` | Blob + Queue |
| `all` | All services |

### Basic Usage

```rust
use armature_azure::{AzureServices, AzureConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AzureConfig::builder()
        .storage_account("mystorageaccount")
        .enable_blob()
        .enable_cosmos()
        .cosmos_endpoint("https://mydb.documents.azure.com:443/")
        .build();

    let services = AzureServices::new(config).await?;

    // Use Blob Storage
    let blob = services.blob_service()?;

    // Use Cosmos DB
    let cosmos = services.cosmos()?;

    Ok(())
}
```

### Environment Configuration

```rust
// Reads AZURE_STORAGE_ACCOUNT, AZURE_COSMOS_ENDPOINT, etc.
let config = AzureConfig::from_env()
    .enable_blob()
    .build();
```

## Dependency Injection

The cloud provider crates integrate seamlessly with Armature's DI system.

### Registering Cloud Services

```rust
use armature_framework::prelude::*;
use armature_aws::{AwsServices, AwsConfig};

#[module]
struct CloudModule;

#[module_impl]
impl CloudModule {
    // Register AWS services as a singleton
    #[provider(singleton)]
    async fn aws_services() -> Arc<AwsServices> {
        let config = AwsConfig::from_env()
            .enable_s3()
            .enable_sqs()
            .enable_dynamodb()
            .build();
        AwsServices::new(config).await.unwrap()
    }

    // Expose individual clients from the service container
    #[provider]
    fn s3_client(services: &Arc<AwsServices>) -> aws_sdk_s3::Client {
        services.s3().unwrap()
    }

    #[provider]
    fn sqs_client(services: &Arc<AwsServices>) -> aws_sdk_sqs::Client {
        services.sqs().unwrap()
    }

    #[provider]
    fn dynamodb_client(services: &Arc<AwsServices>) -> aws_sdk_dynamodb::Client {
        services.dynamodb().unwrap()
    }
}
```

### Using in Controllers

```rust
#[controller("/files")]
struct FileController;

#[controller_impl]
impl FileController {
    #[post("/upload")]
    async fn upload(
        &self,
        #[inject] s3: aws_sdk_s3::Client,
        body: Bytes,
    ) -> Result<Json<UploadResponse>, HttpError> {
        s3.put_object()
            .bucket("my-bucket")
            .key("uploaded-file")
            .body(body.into())
            .send()
            .await
            .map_err(|e| HttpError::internal(e.to_string()))?;

        Ok(Json(UploadResponse { success: true }))
    }
}
```

### Multi-Cloud Setup

```rust
#[module]
struct MultiCloudModule;

#[module_impl]
impl MultiCloudModule {
    #[provider(singleton)]
    async fn aws() -> Arc<AwsServices> {
        let config = AwsConfig::from_env()
            .enable_s3()
            .build();
        AwsServices::new(config).await.unwrap()
    }

    #[provider(singleton)]
    async fn gcp() -> Arc<GcpServices> {
        let config = GcpConfig::from_env()
            .enable_storage()
            .build();
        GcpServices::new(config).await.unwrap()
    }

    #[provider(singleton)]
    async fn azure() -> Arc<AzureServices> {
        let config = AzureConfig::from_env()
            .enable_blob()
            .build();
        AzureServices::new(config).await.unwrap()
    }
}
```

### Using with Other Armature Crates

Other Armature crates can depend on cloud providers through DI:

```rust
// In armature-storage, use S3 from armature-aws
use armature_aws::aws_sdk_s3;

pub struct S3StorageBackend {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl S3StorageBackend {
    // Client injected from DI container
    pub fn new(client: aws_sdk_s3::Client, bucket: String) -> Self {
        Self { client, bucket }
    }
}

// In your application module
#[module_impl]
impl StorageModule {
    #[provider]
    fn s3_backend(
        client: aws_sdk_s3::Client,
    ) -> S3StorageBackend {
        S3StorageBackend::new(client, "my-bucket".to_string())
    }
}
```

## Best Practices

### 1. Use Feature Flags

Only enable the services you need to minimize compile times and binary size:

```toml
# Good - only what you need
armature-aws = { version = "0.1", features = ["s3", "sqs"] }

# Avoid unless you need everything
armature-aws = { version = "0.1", features = ["all"] }
```

### 2. Use Environment Configuration

```rust
// Prefer environment-based config for flexibility
let config = AwsConfig::from_env()
    .enable_s3()
    .build();
```

### 3. Register Services as Singletons

```rust
#[provider(singleton)]  // Important!
async fn aws_services() -> Arc<AwsServices> {
    // ...
}
```

### 4. Handle Missing Services Gracefully

```rust
match services.s3() {
    Ok(client) => { /* use client */ }
    Err(AwsError::ServiceNotConfigured(_)) => {
        tracing::warn!("S3 not configured, skipping");
    }
    Err(e) => return Err(e.into()),
}
```

### 5. Use Local Emulators for Testing

```rust
#[cfg(test)]
let config = AwsConfig::builder()
    .localstack()
    .enable_s3()
    .build();

#[cfg(test)]
let config = AzureConfig::builder()
    .use_emulator()
    .enable_blob()
    .build();
```

## Summary

**Key Concepts:**

1. **Dynamic Loading**: Services compile and initialize only when enabled
2. **Feature Flags**: Fine-grained dependency control
3. **DI Integration**: Cloud services live in the application DI container
4. **Lazy Initialization**: Clients created on first access
5. **Environment Config**: Reads from standard cloud environment variables

**Crate Feature Groups:**

| Crate | Storage | Messaging | Security |
|-------|---------|-----------|----------|
| AWS | `storage` | `messaging` | `security` |
| GCP | `data` | `messaging` | - |
| Azure | `storage` | `messaging` | `security` |

