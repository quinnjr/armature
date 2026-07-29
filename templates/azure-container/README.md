# Azure Container Deployment Templates

Deploy Armature applications to Azure Container Apps and Azure Functions.

## Files

| File | Description |
|------|-------------|
| `Dockerfile` | Container Apps optimized image |
| `Dockerfile.functions` | Azure Functions custom handler |
| `containerapp.yaml` | Container Apps deployment config |
| `docker-compose.yml` | Local development with Azurite |

## Azure Container Apps

### Quick Start

> **Binary name:** The Dockerfile builds with `--bin ${BIN_NAME}` and
> defaults `BIN_NAME` to `app`. If your crate's `[[bin]]`/package name is
> not literally `app`, pass it as a build arg, e.g.:
> `docker build --build-arg BIN_NAME=<your-crate-name> -t myapp .`

```bash
# Build (pass --build-arg BIN_NAME=<your-crate-name> if it isn't "app")
docker build --build-arg BIN_NAME=<your-crate-name> -t armature-app -f Dockerfile .

# Tag for ACR
docker tag armature-app myregistry.azurecr.io/armature-app:latest

# Push
az acr login --name myregistry
docker push myregistry.azurecr.io/armature-app:latest

# Deploy
az containerapp create \
  --name armature-app \
  --resource-group my-rg \
  --environment my-env \
  --image myregistry.azurecr.io/armature-app:latest \
  --target-port 80 \
  --ingress external
```

### Using YAML Configuration

```bash
# Edit containerapp.yaml with your settings
az containerapp create \
  --name armature-app \
  --resource-group my-rg \
  --yaml containerapp.yaml
```

### Production Settings

```bash
az containerapp create \
  --name armature-app \
  --resource-group my-rg \
  --environment my-env \
  --image myregistry.azurecr.io/armature-app:latest \
  --target-port 80 \
  --ingress external \
  --cpu 0.5 \
  --memory 1.0Gi \
  --min-replicas 1 \
  --max-replicas 10 \
  --env-vars "RUST_LOG=info" "DATABASE_URL=secretref:database-url"
```

## Azure Functions

### Quick Start

> **Binary name:** `Dockerfile.functions` also builds with `--bin
> ${BIN_NAME}` (default `app`). Pass `--build-arg BIN_NAME=<your-crate-name>`
> if your crate's binary isn't literally named `app`.

```bash
# Build (pass --build-arg BIN_NAME=<your-crate-name> if it isn't "app")
docker build --build-arg BIN_NAME=<your-crate-name> -t armature-functions -f Dockerfile.functions .

# Push
docker push myregistry.azurecr.io/armature-functions:latest

# Create Function App
az functionapp create \
  --name my-function-app \
  --resource-group my-rg \
  --storage-account mystorageaccount \
  --plan my-plan \
  --deployment-container-image-name myregistry.azurecr.io/armature-functions:latest \
  --functions-version 4
```

### Required Configuration Files

**host.json:**
```json
{
  "version": "2.0",
  "customHandler": {
    "description": {
      "defaultExecutablePath": "handler"
    },
    "enableForwardingHttpRequest": true
  }
}
```

**api/function.json:**
```json
{
  "bindings": [
    {
      "type": "httpTrigger",
      "direction": "in",
      "name": "req",
      "methods": ["get", "post", "put", "delete"],
      "route": "{*route}"
    },
    {
      "type": "http",
      "direction": "out",
      "name": "$return"
    }
  ]
}
```

## Local Development

```bash
# Start Azurite and dependencies
docker-compose up -d

# Access app
curl http://localhost:80/

# Azurite endpoints
# Blob: http://localhost:10000
# Queue: http://localhost:10001
# Table: http://localhost:10002
```

## Armature Integration

### Container Apps

```rust
use armature_cloudrun::CloudRunConfig; // Works for any container platform

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("PORT").unwrap_or_else(|_| "80".to_string());

    Application::create::<AppModule>()
        .listen(&format!("0.0.0.0:{}", port))
        .await
}
```

### Azure Functions

```rust
use armature_azure_functions::{AzureFunctionsRuntime, init_tracing};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let app = Application::create::<AppModule>();
    AzureFunctionsRuntime::new(app).run().await
}
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PORT` | HTTP port | `80` |
| `AZURE_FUNCTIONS_ENVIRONMENT` | Environment | `Development` |
| `WEBSITE_SITE_NAME` | App name | set by Azure |
| `APPLICATIONINSIGHTS_CONNECTION_STRING` | App Insights | optional |

