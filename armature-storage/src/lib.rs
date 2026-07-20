//! # Armature Storage
//!
//! Multipart file upload handling and storage backends for Armature applications.
//!
//! ## Features
//!
//! - **Multipart Upload**: Parse multipart/form-data requests
//! - **File Validation**: Type, size, extension, and content validation
//! - **Local Storage**: Filesystem-based storage
//! - **S3 Storage**: AWS S3 compatible storage (optional)
//! - **GCS Storage**: Google Cloud Storage (optional)
//! - **Azure Blob**: Azure Blob Storage (optional)
//! - **Streaming**: Memory-efficient streaming uploads
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_storage::{Multipart, UploadedFile, FileValidator, LocalStorage, Storage};
//!
//! #[controller("/files")]
//! struct FileController;
//!
//! #[controller_impl]
//! impl FileController {
//!     #[post("/upload")]
//!     async fn upload(
//!         &self,
//!         multipart: Multipart,
//!         #[inject] storage: LocalStorage,
//!     ) -> Result<Json<UploadResponse>, HttpError> {
//!         let validator = FileValidator::new()
//!             .max_size(10 * 1024 * 1024) // 10 MB
//!             .allowed_types(&["image/jpeg", "image/png"])
//!             .allowed_extensions(&["jpg", "jpeg", "png"]);
//!
//!         let mut files = Vec::new();
//!         let mut stream = multipart.into_stream();
//!
//!         while let Some(field) = stream.next_field().await? {
//!             if field.name() == Some("file") {
//!                 let file = UploadedFile::from_field(field).await?;
//!                 validator.validate(&file)?;
//!
//!                 let metadata = storage.put_file(&file).await?;
//!                 files.push(metadata);
//!             }
//!         }
//!
//!         Ok(Json(UploadResponse { files }))
//!     }
//! }
//! ```
//!
//! ## With S3 Storage
//!
//! ```rust,ignore
//! use armature_storage::{S3Storage, S3Config};
//!
//! #[module_impl]
//! impl StorageModule {
//!     #[provider]
//!     async fn s3_storage() -> S3Storage {
//!         S3Storage::new(S3Config {
//!             bucket: "my-bucket".to_string(),
//!             region: "us-east-1".to_string(),
//!             ..Default::default()
//!         }).await.unwrap()
//!     }
//! }
//! ```

mod error;
mod file;
mod local;
mod multipart;
mod storage;
mod validation;

#[cfg(feature = "s3")]
mod s3;

#[cfg(feature = "gcs")]
mod gcs;

#[cfg(feature = "azure")]
mod azure;

pub use error::{Result, StorageError};
pub use file::{FileInfo, UploadedFile};
pub use local::{LocalStorage, LocalStorageConfig};
pub use multipart::{Multipart, MultipartConstraints, MultipartField, MultipartStream};
pub use storage::{
    Storage, StorageConfig, StorageMetadata, calculate_checksum, generate_unique_key,
    sanitize_filename,
};
pub use validation::{FileValidator, ValidationError, ValidationRule};

#[cfg(feature = "s3")]
pub use s3::{S3Config, S3Storage};

#[cfg(feature = "gcs")]
pub use gcs::{GcsConfig, GcsStorage};

#[cfg(feature = "azure")]
pub use azure::{AzureBlobConfig, AzureBlobStorage};

// Re-export useful types
pub use bytes::Bytes;
pub use mime::Mime;

/// Prelude for common imports.
///
/// ```
/// use armature_storage::prelude::*;
/// ```
pub mod prelude {
    pub use crate::error::{Result, StorageError};
    pub use crate::file::{FileInfo, UploadedFile};
    pub use crate::local::{LocalStorage, LocalStorageConfig};
    pub use crate::multipart::{Multipart, MultipartConstraints, MultipartField, MultipartStream};
    pub use crate::storage::{Storage, StorageConfig, StorageMetadata};
    pub use crate::validation::{FileValidator, ValidationError, ValidationRule};

    #[cfg(feature = "s3")]
    pub use crate::s3::{S3Config, S3Storage};

    #[cfg(feature = "gcs")]
    pub use crate::gcs::{GcsConfig, GcsStorage};

    #[cfg(feature = "azure")]
    pub use crate::azure::{AzureBlobConfig, AzureBlobStorage};

    pub use bytes::Bytes;
}
