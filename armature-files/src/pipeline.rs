//! File processing pipeline
//!
//! Provides a fluent builder API for chaining file operations.

use crate::{FileError, FileMetadata, FileResult, OutputFormat, ProcessingResult};
use bytes::Bytes;
use std::path::Path;
use std::time::Instant;

/// A file processing pipeline
///
/// # Example
///
/// ```rust,ignore
/// use armature_files::{Pipeline, ImageOp};
///
/// let result = Pipeline::new()
///     .load_bytes(image_data, "photo.jpg")
///     .image(ImageOp::Resize { width: 800, height: 600 })
///     .image(ImageOp::Quality(85))
///     .execute()
///     .await?;
/// ```
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Input data
    data: Option<Bytes>,
    /// Input metadata
    metadata: Option<FileMetadata>,
    /// Operations to perform
    operations: Vec<PipelineOp>,
    /// Output format
    output_format: OutputFormat,
    /// Maximum file size (in bytes)
    max_size: Option<u64>,
}

/// Pipeline operations
#[derive(Debug, Clone)]
pub enum PipelineOp {
    /// Image operation
    #[cfg(feature = "images")]
    Image(crate::image::ImageOp),
    /// Convert to format
    Convert(OutputFormat),
    /// Custom operation (name for logging)
    Custom(String),
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    /// Create a new empty pipeline
    pub fn new() -> Self {
        Self {
            data: None,
            metadata: None,
            operations: Vec::new(),
            output_format: OutputFormat::Original,
            max_size: None,
        }
    }

    /// Load data from a file path
    pub async fn load(mut self, path: impl AsRef<Path>) -> FileResult<Self> {
        let path = path.as_ref();
        let data = tokio::fs::read(path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                FileError::NotFound(path.display().to_string())
            } else {
                FileError::Io(e)
            }
        })?;

        let mut metadata = FileMetadata::from_path(path);
        metadata.size = data.len() as u64;

        self.data = Some(Bytes::from(data));
        self.metadata = Some(metadata);
        Ok(self)
    }

    /// Load data from bytes with a filename hint
    pub fn load_bytes(mut self, data: impl Into<Bytes>, filename: impl Into<String>) -> Self {
        let data = data.into();
        let filename = filename.into();
        let metadata = FileMetadata::from_bytes(&data, &filename);

        self.data = Some(data);
        self.metadata = Some(metadata);
        self
    }

    /// Set maximum allowed file size
    pub fn max_size(mut self, max_bytes: u64) -> Self {
        self.max_size = Some(max_bytes);
        self
    }

    /// Add an image operation
    #[cfg(feature = "images")]
    pub fn image(mut self, op: crate::image::ImageOp) -> Self {
        self.operations.push(PipelineOp::Image(op));
        self
    }

    /// Resize image (convenience method)
    #[cfg(feature = "images")]
    pub fn resize(self, width: u32, height: u32) -> Self {
        self.image(crate::image::ImageOp::Resize {
            width,
            height,
            filter: crate::image::ResizeFilter::Lanczos3,
        })
    }

    /// Resize image to fit within bounds (convenience method)
    #[cfg(feature = "images")]
    pub fn resize_fit(self, max_width: u32, max_height: u32) -> Self {
        self.image(crate::image::ImageOp::ResizeFit {
            max_width,
            max_height,
            filter: crate::image::ResizeFilter::Lanczos3,
        })
    }

    /// Crop image (convenience method)
    #[cfg(feature = "images")]
    pub fn crop(self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.image(crate::image::ImageOp::Crop {
            x,
            y,
            width,
            height,
        })
    }

    /// Rotate image (convenience method)
    #[cfg(feature = "images")]
    pub fn rotate(self, degrees: f32) -> Self {
        self.image(crate::image::ImageOp::Rotate { degrees })
    }

    /// Add watermark (convenience method)
    #[cfg(feature = "images")]
    pub fn watermark(self, text: impl Into<String>, position: crate::Position) -> Self {
        self.image(crate::image::ImageOp::TextWatermark {
            text: text.into(),
            position,
            font_size: 24.0,
            color: [255, 255, 255, 180],
        })
    }

    /// Convert to a specific format
    pub fn convert(mut self, format: OutputFormat) -> Self {
        self.output_format = format;
        self.operations.push(PipelineOp::Convert(format));
        self
    }

    /// Convert to JPEG (convenience method)
    pub fn to_jpeg(self, quality: u8) -> Self {
        self.convert(OutputFormat::Jpeg { quality })
    }

    /// Convert to PNG (convenience method)
    pub fn to_png(self) -> Self {
        self.convert(OutputFormat::Png)
    }

    /// Convert to WebP (convenience method)
    pub fn to_webp(self, quality: u8) -> Self {
        self.convert(OutputFormat::WebP { quality })
    }

    /// Execute the pipeline and return the result
    pub async fn execute(self) -> FileResult<ProcessingResult> {
        let start = Instant::now();

        let data = self
            .data
            .ok_or_else(|| FileError::Pipeline("No input data loaded".into()))?;
        let mut metadata = self
            .metadata
            .ok_or_else(|| FileError::Pipeline("No metadata available".into()))?;

        // Check max size
        if let Some(max) = self.max_size {
            if data.len() as u64 > max {
                return Err(FileError::FileTooLarge {
                    size: data.len() as u64,
                    max,
                });
            }
        }

        let mut operation_names = Vec::new();
        let mut current_data = data;

        // Process operations
        for op in &self.operations {
            match op {
                #[cfg(feature = "images")]
                PipelineOp::Image(image_op) => {
                    operation_names.push(format!("image:{:?}", image_op));
                    current_data =
                        crate::image::process_image(&current_data, image_op, &mut metadata)?;
                }
                PipelineOp::Convert(format) => {
                    operation_names.push(format!("convert:{}", format.extension()));
                    #[cfg(feature = "images")]
                    if metadata.is_image() {
                        current_data =
                            crate::image::convert_format(&current_data, *format, &mut metadata)?;
                    }
                }
                PipelineOp::Custom(name) => {
                    operation_names.push(format!("custom:{}", name));
                }
            }
        }

        // Update metadata
        metadata.size = current_data.len() as u64;

        Ok(ProcessingResult {
            data: current_data,
            metadata,
            operations: operation_names,
            processing_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Execute the pipeline and save to a file
    pub async fn save(self, path: impl AsRef<Path>) -> FileResult<ProcessingResult> {
        let result = self.execute().await?;
        result.save(path).await?;
        Ok(result)
    }
}

/// Builder for creating multiple output sizes (e.g., thumbnails)
#[derive(Debug, Clone)]
pub struct MultiSizeBuilder {
    data: Bytes,
    metadata: FileMetadata,
    sizes: Vec<(String, u32, u32)>,
    output_format: OutputFormat,
}

impl MultiSizeBuilder {
    /// Create a new multi-size builder
    pub fn new(data: impl Into<Bytes>, filename: impl Into<String>) -> Self {
        let data = data.into();
        let filename = filename.into();
        let metadata = FileMetadata::from_bytes(&data, &filename);

        Self {
            data,
            metadata,
            sizes: Vec::new(),
            output_format: OutputFormat::Original,
        }
    }

    /// Add a size variant
    pub fn add_size(mut self, name: impl Into<String>, width: u32, height: u32) -> Self {
        self.sizes.push((name.into(), width, height));
        self
    }

    /// Add common thumbnail sizes
    pub fn with_thumbnails(self) -> Self {
        self.add_size("thumb_small", 64, 64)
            .add_size("thumb_medium", 128, 128)
            .add_size("thumb_large", 256, 256)
    }

    /// Add common responsive sizes
    pub fn with_responsive(self) -> Self {
        self.add_size("xs", 320, 240)
            .add_size("sm", 640, 480)
            .add_size("md", 1024, 768)
            .add_size("lg", 1920, 1080)
    }

    /// Set output format
    pub fn format(mut self, format: OutputFormat) -> Self {
        self.output_format = format;
        self
    }

    /// Generate all sizes
    #[cfg(feature = "images")]
    pub async fn generate(self) -> FileResult<Vec<(String, ProcessingResult)>> {
        let mut results = Vec::new();

        for (name, width, height) in self.sizes {
            let pipeline = Pipeline::new()
                .load_bytes(self.data.clone(), &self.metadata.filename)
                .resize_fit(width, height)
                .convert(self.output_format);

            let result = pipeline.execute().await?;
            results.push((name, result));
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = Pipeline::new().max_size(1024 * 1024);

        assert!(pipeline.data.is_none());
        assert_eq!(pipeline.max_size, Some(1024 * 1024));
    }

    #[test]
    fn test_pipeline_load_bytes() {
        let data = vec![0u8; 100];
        let pipeline = Pipeline::new().load_bytes(data, "test.jpg");

        assert!(pipeline.data.is_some());
        assert!(pipeline.metadata.is_some());
        assert_eq!(pipeline.metadata.unwrap().filename, "test.jpg");
    }
}
