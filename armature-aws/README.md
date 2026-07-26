# armature-aws

AWS cloud services integration for the Armature framework.

## Features

- **S3** - Object storage
- **DynamoDB** - NoSQL database
- **SQS/SNS** - Message queues and topics
- **Secrets Manager** - Secure secrets
- **Parameter Store** - Configuration management
- **CloudWatch** - Logging and metrics

## Installation

```toml
[dependencies]
armature-aws = "0.1"
```

## Quick Start

`armature-aws` does not wrap the AWS SDK. You configure which services to
enable, construct an `AwsServices` container, and pull raw
`aws_sdk_*::Client`s out of it via accessors like `services.s3()`.

```rust
use armature_aws::{AwsConfig, AwsServices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Choose region + which services to load.
    let config = AwsConfig::builder()
        .region("us-east-1")
        .enable_s3()
        .enable_dynamodb()
        .enable_sqs()
        .build();

    let services = AwsServices::new(config).await?;

    // Accessors return the raw AWS SDK clients.
    let s3 = services.s3()?;
    let buckets = s3.list_buckets().send().await?;

    let dynamo = services.dynamodb()?;
    let tables = dynamo.list_tables().send().await?;

    let sqs = services.sqs()?;
    sqs.send_message()
        .queue_url("https://sqs.us-east-1.amazonaws.com/123456789012/my-queue")
        .message_body("hello")
        .send()
        .await?;

    Ok(())
}
```

Each service requires its Cargo feature (e.g. `features = ["s3", "dynamodb",
"sqs"]`); accessors for services that were not enabled return an error.
Configuration can also be read from the environment with `AwsConfig::from_env()`.

## Credential Chain

Credentials are loaded from:
1. Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
2. AWS credentials file (`~/.aws/credentials`)
3. IAM role (ECS, Lambda, EC2)

## License

MIT OR Apache-2.0

