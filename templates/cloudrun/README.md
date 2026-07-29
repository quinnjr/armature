# Google Cloud Run Deployment Templates

Deploy Armature applications to Google Cloud Run.

## Files

| File | Description |
|------|-------------|
| `Dockerfile` | Optimized distroless container image |
| `Dockerfile.cloudbuild` | Cloud Build optimized Dockerfile |
| `cloudbuild.yaml` | Cloud Build configuration |
| `docker-compose.yml` | Local development with GCP emulators |

## Quick Start

> **Binary name:** The Dockerfiles build with `--bin ${BIN_NAME}` and default
> `BIN_NAME` to `app`. If your crate's `[[bin]]`/package name is not literally
> `app`, pass it as a build arg, e.g.:
> `docker build --build-arg BIN_NAME=<your-crate-name> -t myapp .`
> Neither `gcloud builds submit --tag` nor the provided `cloudbuild.yaml`
> forward Docker build args, so when building through Cloud Build either
> name your crate's binary `app`, or add
> `--build-arg=BIN_NAME=<your-crate-name>` to the `docker build` step in
> `cloudbuild.yaml` yourself.

### Using gcloud

```bash
# Build and push
gcloud builds submit --tag gcr.io/PROJECT_ID/armature-app

# Deploy
gcloud run deploy armature-app \
  --image gcr.io/PROJECT_ID/armature-app \
  --platform managed \
  --region us-central1 \
  --allow-unauthenticated
```

### Using Cloud Build

```bash
# Deploy with cloudbuild.yaml
gcloud builds submit --config=cloudbuild.yaml

# With custom substitutions
gcloud builds submit \
  --config=cloudbuild.yaml \
  --substitutions=_SERVICE=my-service,_REGION=europe-west1,_MEMORY=512Mi
```

### Using Artifact Registry (Recommended)

```bash
# Create repository
gcloud artifacts repositories create armature \
  --repository-format=docker \
  --location=us-central1

# Build and push
gcloud builds submit \
  --tag us-central1-docker.pkg.dev/PROJECT_ID/armature/app

# Deploy
gcloud run deploy armature-app \
  --image us-central1-docker.pkg.dev/PROJECT_ID/armature/app \
  --platform managed \
  --region us-central1
```

## Production Settings

```bash
gcloud run deploy armature-app \
  --image gcr.io/PROJECT_ID/armature-app \
  --platform managed \
  --region us-central1 \
  --memory 256Mi \
  --cpu 1 \
  --min-instances 1 \
  --max-instances 10 \
  --timeout 300s \
  --concurrency 80 \
  --set-env-vars "RUST_LOG=info,DATABASE_URL=..." \
  --allow-unauthenticated
```

## Local Development

```bash
# Start GCP emulators
docker-compose up -d

# Access app
curl http://localhost:8080/

# Pub/Sub emulator
export PUBSUB_EMULATOR_HOST=localhost:8085

# Firestore emulator
export FIRESTORE_EMULATOR_HOST=localhost:8086
```

## Armature Integration

```rust
use armature_cloudrun::{CloudRunConfig, init_tracing};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let config = CloudRunConfig::from_env();

    Application::create::<AppModule>()
        .listen(&config.bind_address())
        .await
}
```

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PORT` | HTTP port | `8080` |
| `K_SERVICE` | Service name | set by Cloud Run |
| `K_REVISION` | Revision name | set by Cloud Run |
| `GOOGLE_CLOUD_PROJECT` | Project ID | set by Cloud Run |

