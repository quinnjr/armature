# armature-gcp

Google Cloud Platform services integration for the Armature framework.

## Features

Each service is behind a Cargo feature flag; only the ones you enable are
compiled.

- **Cloud Storage** (`storage`) - Object storage
- **Pub/Sub** (`pubsub`) - Message queues
- **Spanner** (`spanner`) - Distributed SQL database
- **BigQuery** (`bigquery`) - Data warehouse
- **Secret Manager** (`secret-manager`) - Secrets management (REST)
- **Cloud Run** (`cloud-run`) - Service administration (REST)
- **Cloud Functions** (`cloud-functions`) - Function administration (REST)

## Installation

```toml
[dependencies]
armature-gcp = { version = "0.1", features = ["storage", "pubsub"] }
```

## Quick Start

Configure which services to load with `GcpConfig`, construct the `GcpServices`
container, then pull typed clients out of it. The container returns the raw
upstream SDK clients (for `storage`/`pubsub`/`spanner`/`bigquery`) or the
crate's REST clients (for `secret-manager`/`cloud-run`/`cloud-functions`).

```rust,ignore
use armature_gcp::{GcpConfig, GcpServices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = GcpConfig::builder()
        .project_id("my-project")
        .enable_storage()
        .enable_pubsub()
        .enable_secret_manager()
        .build();

    // Enabled clients are initialized eagerly here.
    let services = GcpServices::new(config).await?;

    // Raw google-cloud-storage / google-cloud-pubsub SDK clients:
    let _storage = services.storage()?;
    let _pubsub = services.pubsub()?;

    // REST client for Secret Manager:
    let secrets = services.secret_manager()?;
    let _payload = secrets.access_secret_version("my-secret", "latest").await?;

    Ok(())
}
```

## Authentication

Credentials are selected via `GcpConfig`'s `CredentialsSource` and threaded into
every client:

```rust,ignore
use armature_gcp::{CredentialsSource, GcpConfig};

let config = GcpConfig::builder()
    .project_id("my-project")
    // or .service_account_file("/path/key.json")
    // or .credentials(CredentialsSource::AccessToken("ya29...".into()))
    .service_account_json(std::fs::read_to_string("key.json")?)
    .enable_secret_manager()
    .build();
# Ok::<(), Box<dyn std::error::Error>>(())
```

The supported sources are Application Default Credentials (the default), a
service-account file, inline service-account JSON, the GCE/Cloud Run metadata
server, and a static access token. Static access tokens are honored by the REST
services; the gRPC SDKs (storage/pubsub) support the remaining variants.

### From the environment

`GcpConfig::from_env()` seeds a builder from the standard environment variables:
`GOOGLE_CLOUD_PROJECT` / `GCP_PROJECT` / `GCLOUD_PROJECT` for the project id,
`GOOGLE_APPLICATION_CREDENTIALS` for a service-account file, and
`PUBSUB_EMULATOR_HOST` / `STORAGE_EMULATOR_HOST` for an emulator endpoint
override.

## License

MIT OR Apache-2.0

