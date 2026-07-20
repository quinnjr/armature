#![doc = include_str!("../README.md")]
//!
//! ## Handling an upload
//!
//! A request handler typically parses the multipart body, validates each file,
//! and writes the ones it accepts to storage. This is the whole flow, free of
//! any particular web framework:
//!
//! ```rust,no_run
//! # async fn example(multipart: armature_storage::Multipart) -> Result<(), Box<dyn std::error::Error>> {
//! use armature_storage::{
//!     FileValidator, LocalStorage, Storage, StorageMetadata, UploadedFile,
//! };
//!
//! let storage = LocalStorage::with_path("./uploads").await?;
//!
//! let validator = FileValidator::new()
//!     .max_size(10 * 1024 * 1024) // 10 MB
//!     // Checks the file's magic bytes, not the client-declared type.
//!     .allowed_types_detected(&["image/jpeg", "image/png"])
//!     .allowed_extensions(&["jpg", "jpeg", "png"]);
//!
//! let mut stored: Vec<StorageMetadata> = Vec::new();
//! let mut stream = multipart.into_stream();
//!
//! while let Some(field) = stream.next_field().await? {
//!     if field.name() == Some("file") {
//!         let file = UploadedFile::from_field(field).await?;
//!         validator.validate(&file)?;
//!         stored.push(storage.put_file(&file).await?);
//!     }
//! }
//! # Ok(())
//! # }
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
pub use multipart::{
    Multipart, MultipartConstraints, MultipartData, MultipartField, MultipartStream,
    parse_multipart,
};
pub use storage::{
    DEFAULT_URL_DURATION, Storage, StorageConfig, StorageMetadata, calculate_checksum,
    generate_unique_key, sanitize_filename,
};
pub use validation::{
    FileValidator, FileValidatorFn, ValidationError, ValidationRule, size, sniff_content_type,
};

#[cfg(feature = "s3")]
pub use s3::{S3Config, S3Storage};

#[cfg(feature = "gcs")]
pub use gcs::{GcsConfig, GcsStorage};

#[cfg(feature = "azure")]
pub use azure::{AzureBlobConfig, AzureBlobStorage};

// Re-export useful types
pub use bytes::Bytes;
pub use mime::Mime;

/// Re-exported so callers can name the types in [`parse_multipart`]'s
/// signature without taking their own `http` dependency.
pub use http;

/// Prelude for common imports.
///
/// ```
/// use armature_storage::prelude::*;
/// ```
pub mod prelude {
    pub use crate::error::{Result, StorageError};
    pub use crate::file::{FileInfo, UploadedFile};
    pub use crate::local::{LocalStorage, LocalStorageConfig};
    pub use crate::multipart::{
        Multipart, MultipartConstraints, MultipartData, MultipartField, MultipartStream,
        parse_multipart,
    };
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
