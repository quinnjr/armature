# armature-storage

Multipart file upload handling and object storage for the Armature framework.

## Features

- **Multiple Providers** — S3, Azure Blob, GCS, local filesystem
- **Unified API** — the same [`Storage`] trait for every provider
- **Multipart Uploads** — `multipart/form-data` parsing with enforced limits
- **Content Validation** — size, extension, and magic-byte content-type checks
- **Presigned URLs** — S3 and GCS only (Azure and local storage return `None`)

## Installation

```toml
[dependencies]
armature-storage = "0.2"

# Cloud backends are optional features:
# armature-storage = { version = "0.2", features = ["s3"] }
```

Features: `s3`, `gcs`, `azure`, and `all-clouds`.

## Quick Start

Every backend implements the `Storage` trait, so this code is identical
whichever one you construct:

```rust
use armature_storage::{Bytes, LocalStorage, Storage};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
let dir = tempfile::tempdir()?;
let storage = LocalStorage::with_path(dir.path()).await?;

// Upload
storage.put("files/doc.txt", Bytes::from("hello")).await?;

// Download
let data = storage.get("files/doc.txt").await?;
assert_eq!(data, "hello");

// Metadata, without downloading the body
let meta = storage.head("files/doc.txt").await?;
assert_eq!(meta.size, 5);

// List (recursive; nested keys are included)
let keys: Vec<String> = storage.list(None).await?.into_iter().map(|m| m.key).collect();
assert_eq!(keys, ["files/doc.txt"]);

// Copy, then delete
storage.copy("files/doc.txt", "files/copy.txt").await?;
storage.delete("files/doc.txt").await?;
# Ok(())
# }
```

Keys are untrusted input: any key that is absolute or contains a `..`
component is rejected with `StorageError::InvalidFileName` rather than
escaping the storage root.

```rust
use armature_storage::{Bytes, LocalStorage, Storage, StorageError};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let dir = tempfile::tempdir()?;
# let storage = LocalStorage::with_path(dir.path()).await?;
let err = storage.put("../../etc/passwd", Bytes::from("nope")).await.unwrap_err();
assert!(matches!(err, StorageError::InvalidFileName(_)));
# Ok(())
# }
```

## Temporary URLs

`temporary_url` returns `Ok(None)` on backends that cannot sign one, so check
the `Option` rather than assuming a URL:

```rust
use armature_storage::{LocalStorage, Storage};
use std::time::Duration;

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let dir = tempfile::tempdir()?;
# let storage = LocalStorage::with_path(dir.path()).await?;
// S3 and GCS return Some(signed_url); Azure and local storage return None.
let signed = storage.temporary_url("files/doc.txt", Duration::from_secs(3600)).await?;
assert!(signed.is_none());

// Or use the backend's configured default lifetime:
let signed = storage.temporary_url_default("files/doc.txt").await?;
assert!(signed.is_none());
# Ok(())
# }
```

## Validation

Type allowlists check the file's **content**, not just the client-declared
`Content-Type`, so an upload cannot pass by lying about its type:

```rust
use armature_storage::{Bytes, FileValidator, UploadedFile};

let validator = FileValidator::new()
    .max_size(10 * 1024 * 1024)
    .images_only();

// A PDF renamed to .png is rejected even though it declares image/png.
let liar = UploadedFile::from_bytes(Bytes::from_static(b"%PDF-1.7\n"), "photo.png");
assert!(validator.validate(&liar).is_err());
```

## Multipart

`Multipart::new` and `Multipart::from_request` apply
`MultipartConstraints::default()` — 100 MB total, 50 MB per field, 100 fields,
10 files. Use `with_constraints` to tune the limits, or the explicit
`unconstrained` / `from_request_unconstrained` constructors to opt out.

```rust
use armature_storage::{Multipart, MultipartConstraints};

let constraints = MultipartConstraints::default()
    .max_files(3)
    .max_total_size(5 * 1024 * 1024);

let body = futures::stream::empty::<Result<armature_storage::Bytes, std::io::Error>>();
let multipart = Multipart::with_constraints(body, "boundary", constraints);
```

## Providers

### Local filesystem

```rust
use armature_storage::{LocalStorage, LocalStorageConfig};

# #[tokio::main]
# async fn main() -> Result<(), Box<dyn std::error::Error>> {
# let dir = tempfile::tempdir()?;
let config = LocalStorageConfig::new(dir.path())
    .with_base_url("https://cdn.example.com/uploads")
    .with_prefix("tenant-a");
let storage = LocalStorage::new(config).await?;
# Ok(())
# }
```

### AWS S3

```rust
# #[cfg(feature = "s3")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_storage::{S3Config, S3Storage};
use std::time::Duration;

let config = S3Config::new("my-bucket")
    .region("us-east-1")
    .aes256_encryption()
    .presigned_duration(Duration::from_secs(900));
let storage = S3Storage::new(config).await?;
# Ok(())
# }
```

Point `endpoint` at an S3-compatible service (MinIO, R2) to use one instead.

### Google Cloud Storage

```rust
# #[cfg(feature = "gcs")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_storage::{GcsConfig, GcsStorage};

let config = GcsConfig::new("my-bucket").project_id("my-billing-project");
let storage = GcsStorage::new(config).await?;
# Ok(())
# }
```

Authenticates with Application Default Credentials.

### Azure Blob Storage

```rust
# #[cfg(feature = "azure")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use armature_storage::{AzureBlobConfig, AzureBlobStorage};

let config = AzureBlobConfig::new("myaccount", "mycontainer");
let storage = AzureBlobStorage::new(config).await?;
# Ok(())
# }
```

`azure_storage_blob` 1.0 authenticates with AAD (Entra ID) token credentials
only — connection-string and shared-key auth are not supported by the SDK.
Azure also cannot sign SAS URLs through this SDK yet, so `temporary_url`
returns `None`.

## License

MIT OR Apache-2.0
