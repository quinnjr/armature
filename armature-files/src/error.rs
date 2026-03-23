//! Error types for file processing

use thiserror::Error;

/// File processing error types
#[derive(Error, Debug)]
pub enum FileError {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Image processing error
    #[error("Image error: {0}")]
    Image(String),

    /// PDF error
    #[error("PDF error: {0}")]
    Pdf(String),

    /// Archive error
    #[error("Archive error: {0}")]
    Archive(String),

    /// Format not supported
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Invalid operation
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    /// File not found
    #[error("File not found: {0}")]
    NotFound(String),

    /// File too large
    #[error("File too large: {size} bytes (max: {max} bytes)")]
    FileTooLarge { size: u64, max: u64 },

    /// Invalid dimensions
    #[error("Invalid dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },

    /// Pipeline error
    #[error("Pipeline error: {0}")]
    Pipeline(String),

    /// Encoding error
    #[error("Encoding error: {0}")]
    Encoding(String),
}

#[cfg(feature = "images")]
impl From<::image::ImageError> for FileError {
    fn from(err: ::image::ImageError) -> Self {
        FileError::Image(err.to_string())
    }
}

/// Result type for file operations
pub type FileResult<T> = Result<T, FileError>;
