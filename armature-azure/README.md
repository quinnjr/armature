# armature-azure

Microsoft Azure services integration for the Armature framework.

## Features

- **Blob Storage** - Object storage
- **Queue Storage** - Storage queues
- **Cosmos DB** - Global database
- **Service Bus** - Message queues and topics
- **Key Vault** - Secrets management

Each service is behind a Cargo feature (`blob`, `queue`, `cosmos`, `servicebus`,
`keyvault`, or the `all` group) and is only compiled and initialized when enabled.

## Installation

```toml
[dependencies]
armature-azure = { version = "0.1", features = ["blob", "cosmos"] }
```

## Quick Start

Services are configured with `AzureConfig::builder()` and constructed through
`AzureServices::new`, which hands back the raw Azure SDK clients.

```rust,no_run
use armature_azure::{AzureConfig, AzureServices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AzureConfig::builder()
        .storage_account("mystorageaccount")
        .cosmos_endpoint("https://myaccount.documents.azure.com:443/")
        .cosmos_database("mydb")
        .servicebus_connection_string(
            "Endpoint=sb://mybus.servicebus.windows.net/;\
             SharedAccessKeyName=RootManageSharedAccessKey;SharedAccessKey=<key>",
        )
        .enable_blob()
        .enable_cosmos()
        .enable_servicebus()
        .build();

    let services = AzureServices::new(config).await?;

    // Blob Storage: raw azure_storage_blob::BlobServiceClient
    let blob = services.blob_service()?;
    let _containers = blob.list_containers(None)?;

    // Cosmos DB: raw azure_data_cosmos clients
    let db = services.cosmos_database()?;
    let _container = db.container_client("items");

    // Service Bus: per-entity SAS clients
    let queue = services.servicebus()?.queue("orders")?;
    queue.send_message("hello", None).await?;

    Ok(())
}
```

## Authentication

Storage, Cosmos DB and Key Vault use Microsoft Entra ID (AAD) token
credentials via `CredentialsSource` (the Azure SDK 1.0 line dropped
connection-string and shared-key auth for these services):

- Default / developer-tools chain (Azure CLI, Azure Developer CLI)
- Managed Identity
- Service Principal (client secret)
- Azure CLI credential

Service Bus authenticates with a Shared Access Signature (SAS), supplied as a
`servicebus_connection_string` or a `service_config("servicebus")` block with
`policy_name` + `shared_access_key`.

## License

MIT OR Apache-2.0
