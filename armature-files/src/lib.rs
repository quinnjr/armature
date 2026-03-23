//! File Processing Pipeline for Armature Framework
//!
//! Provides a fluent API for file processing operations including:
//! - Image manipulation (resize, crop, rotate, format conversion)
//! - PDF generation and manipulation
//! - Archive operations (zip/unzip)
//! - Format detection and conversion
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_files::{Pipeline, ImageOp, OutputFormat};
//!
//! // Image processing pipeline
//! let result = Pipeline::new()
//!     .load("input.jpg")
//!     .image(ImageOp::Resize { width: 800, height: 600 })
//!     .image(ImageOp::Watermark { text: "© 2025".into(), position: Position::BottomRight })
//!     .convert(OutputFormat::WebP { quality: 80 })
//!     .save("output.webp")
//!     .await?;
//!
//! // PDF generation
//! let pdf = PdfBuilder::new()
//!     .title("Report")
//!     .add_text("Hello, World!", FontSize::H1)
//!     .add_image("chart.png")
//!     .add_page_break()
//!     .add_table(data)
//!     .build()?;
//! ```
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                         Pipeline                             │
//! │  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐     │
//! │  │  Load   │──▶│ Process │──▶│ Convert │──▶│  Save   │     │
//! │  └─────────┘   └─────────┘   └─────────┘   └─────────┘     │
//! │       │              │              │              │         │
//! │       ▼              ▼              ▼              ▼         │
//! │  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌─────────┐     │
//! │  │  File   │   │  Image  │   │  Format │   │  File   │     │
//! │  │  Bytes  │   │   PDF   │   │  Codec  │   │ Storage │     │
//! │  └─────────┘   └─────────┘   └─────────┘   └─────────┘     │
//! └─────────────────────────────────────────────────────────────┘
//! ```

mod error;
mod pipeline;

#[cfg(feature = "images")]
pub mod image;

#[cfg(feature = "pdf")]
pub mod pdf;

#[cfg(feature = "archives")]
pub mod archive;

pub use error::*;
pub use pipeline::*;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// File metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// Original filename
    pub filename: String,
    /// MIME type
    pub mime_type: String,
    /// File size in bytes
    pub size: u64,
    /// File extension
    pub extension: Option<String>,
    /// Width (for images)
    pub width: Option<u32>,
    /// Height (for images)
    pub height: Option<u32>,
    /// Page count (for PDFs)
    pub pages: Option<u32>,
}

impl FileMetadata {
    /// Create metadata from a file path
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let extension = path.extension().map(|s| s.to_string_lossy().to_string());
        let mime_type = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        Self {
            filename,
            mime_type,
            size: 0,
            extension,
            width: None,
            height: None,
            pages: None,
        }
    }

    /// Create metadata from bytes with filename hint
    pub fn from_bytes(data: &[u8], filename: impl Into<String>) -> Self {
        let filename = filename.into();
        let extension = Path::new(&filename)
            .extension()
            .map(|s| s.to_string_lossy().to_string());
        let mime_type = mime_guess::from_path(&filename)
            .first_or_octet_stream()
            .to_string();

        Self {
            filename,
            mime_type,
            size: data.len() as u64,
            extension,
            width: None,
            height: None,
            pages: None,
        }
    }

    /// Check if this is an image file
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }

    /// Check if this is a PDF file
    pub fn is_pdf(&self) -> bool {
        self.mime_type == "application/pdf"
    }

    /// Check if this is an archive
    pub fn is_archive(&self) -> bool {
        matches!(
            self.mime_type.as_str(),
            "application/zip" | "application/x-tar" | "application/gzip"
        )
    }
}

/// Supported output formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    // Image formats
    Jpeg { quality: u8 },
    Png,
    WebP { quality: u8 },
    Gif,
    Bmp,
    Ico,
    Tiff,
    Avif { quality: u8 },

    // Document formats
    Pdf,

    // Archive formats
    Zip,

    // Keep original format
    Original,
}

impl OutputFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg { .. } => "jpg",
            Self::Png => "png",
            Self::WebP { .. } => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Ico => "ico",
            Self::Tiff => "tiff",
            Self::Avif { .. } => "avif",
            Self::Pdf => "pdf",
            Self::Zip => "zip",
            Self::Original => "",
        }
    }

    /// Get the MIME type for this format
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Jpeg { .. } => "image/jpeg",
            Self::Png => "image/png",
            Self::WebP { .. } => "image/webp",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Ico => "image/x-icon",
            Self::Tiff => "image/tiff",
            Self::Avif { .. } => "image/avif",
            Self::Pdf => "application/pdf",
            Self::Zip => "application/zip",
            Self::Original => "application/octet-stream",
        }
    }
}

/// Position for watermarks and overlays
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    /// Custom position (x, y from top-left)
    Custom(u32, u32),
}

impl Position {
    /// Calculate pixel coordinates given container and element dimensions
    pub fn calculate(
        &self,
        container_width: u32,
        container_height: u32,
        element_width: u32,
        element_height: u32,
        padding: u32,
    ) -> (u32, u32) {
        match self {
            Self::TopLeft => (padding, padding),
            Self::TopCenter => ((container_width - element_width) / 2, padding),
            Self::TopRight => (container_width - element_width - padding, padding),
            Self::CenterLeft => (padding, (container_height - element_height) / 2),
            Self::Center => (
                (container_width - element_width) / 2,
                (container_height - element_height) / 2,
            ),
            Self::CenterRight => (
                container_width - element_width - padding,
                (container_height - element_height) / 2,
            ),
            Self::BottomLeft => (padding, container_height - element_height - padding),
            Self::BottomCenter => (
                (container_width - element_width) / 2,
                container_height - element_height - padding,
            ),
            Self::BottomRight => (
                container_width - element_width - padding,
                container_height - element_height - padding,
            ),
            Self::Custom(x, y) => (*x, *y),
        }
    }
}

/// Result of a pipeline operation
#[derive(Debug, Clone)]
pub struct ProcessingResult {
    /// Processed file data
    pub data: Bytes,
    /// File metadata
    pub metadata: FileMetadata,
    /// Operations performed
    pub operations: Vec<String>,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
}

impl ProcessingResult {
    /// Save the result to a file
    pub async fn save(&self, path: impl AsRef<Path>) -> Result<(), FileError> {
        tokio::fs::write(path, &self.data)
            .await
            .map_err(FileError::Io)
    }

    /// Get the data as bytes
    pub fn bytes(&self) -> &Bytes {
        &self.data
    }

    /// Get the data as a Vec<u8>
    pub fn to_vec(&self) -> Vec<u8> {
        self.data.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_metadata_from_path() {
        let meta = FileMetadata::from_path("test.jpg");
        assert_eq!(meta.filename, "test.jpg");
        assert_eq!(meta.extension, Some("jpg".to_string()));
        assert!(meta.is_image());
    }

    #[test]
    fn test_output_format_extension() {
        assert_eq!(OutputFormat::Jpeg { quality: 85 }.extension(), "jpg");
        assert_eq!(OutputFormat::Png.extension(), "png");
        assert_eq!(OutputFormat::Pdf.extension(), "pdf");
    }

    #[test]
    fn test_position_calculate() {
        let (x, y) = Position::TopLeft.calculate(100, 100, 20, 20, 5);
        assert_eq!((x, y), (5, 5));

        let (x, y) = Position::Center.calculate(100, 100, 20, 20, 5);
        assert_eq!((x, y), (40, 40));

        let (x, y) = Position::BottomRight.calculate(100, 100, 20, 20, 5);
        assert_eq!((x, y), (75, 75));
    }
}
