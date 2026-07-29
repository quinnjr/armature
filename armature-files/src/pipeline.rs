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
/// ```rust,no_run
/// # #[cfg(feature = "images")]
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// use armature_files::image::{ImageOp, ResizeFilter};
/// use armature_files::Pipeline;
///
/// let image_data = std::fs::read("photo.jpg")?;
///
/// let result = Pipeline::new()
///     .load_bytes(image_data, "photo.jpg")
///     .image(ImageOp::Resize {
///         width: 800,
///         height: 600,
///         filter: ResizeFilter::Lanczos3,
///     })
///     // Encoder quality is a property of the output format, not an
///     // image operation.
///     .to_jpeg(85)
///     .execute()
///     .await?;
/// # let _ = result;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Pipeline {
    /// Input data
    data: Option<Bytes>,
    /// Input metadata
    metadata: Option<FileMetadata>,
    /// Operations to perform
    operations: Vec<PipelineOp>,
    /// Maximum file size (in bytes)
    max_size: Option<u64>,
    /// Resource limits applied when decoding untrusted image bytes
    #[cfg(feature = "images")]
    decode_limits: crate::image::DecodeLimits,
    /// Font used for text watermarks that do not carry their own
    #[cfg(feature = "images")]
    font: Option<Bytes>,
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

/// Fill in the pipeline-level default font for a `TextWatermark` that did not
/// specify one of its own. Every other operation passes through unchanged.
#[cfg(feature = "images")]
fn apply_default_font(
    op: crate::image::ImageOp,
    default_font: &Option<Bytes>,
) -> crate::image::ImageOp {
    match op {
        crate::image::ImageOp::TextWatermark {
            text,
            position,
            font_size,
            color,
            font,
        } => crate::image::ImageOp::TextWatermark {
            text,
            position,
            font_size,
            color,
            font: font.or_else(|| default_font.clone()),
        },
        other => other,
    }
}

impl Pipeline {
    /// Create a new empty pipeline
    pub fn new() -> Self {
        Self {
            data: None,
            metadata: None,
            operations: Vec::new(),
            max_size: None,
            #[cfg(feature = "images")]
            decode_limits: crate::image::DecodeLimits::default(),
            #[cfg(feature = "images")]
            font: None,
        }
    }

    /// Override the resource limits applied when decoding image input.
    ///
    /// Defaults to [`crate::image::DecodeLimits::default`], which caps both
    /// dimensions and total decoder allocation so a maliciously crafted header
    /// cannot force a multi-gigabyte allocation.
    #[cfg(feature = "images")]
    pub fn decode_limits(mut self, limits: crate::image::DecodeLimits) -> Self {
        self.decode_limits = limits;
        self
    }

    /// Supply the TrueType/OpenType font used to render text watermarks that
    /// do not carry their own `font`.
    ///
    /// Required when the `embedded-font` feature is disabled; otherwise it
    /// overrides the embedded DejaVu Sans fallback.
    #[cfg(feature = "images")]
    pub fn with_font(mut self, font: impl Into<Bytes>) -> Self {
        self.font = Some(font.into());
        self
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
            font: None,
        })
    }

    /// Convert to a specific format
    pub fn convert(mut self, format: OutputFormat) -> Self {
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
    ///
    /// Note: WebP encoding is lossless only (the `image` crate does not
    /// support quality-controlled/lossy WebP encoding), so there is no
    /// `quality` parameter here.
    pub fn to_webp(self) -> Self {
        self.convert(OutputFormat::WebP)
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
        if let Some(max) = self.max_size
            && data.len() as u64 > max
        {
            return Err(FileError::FileTooLarge {
                size: data.len() as u64,
                max,
            });
        }

        let mut operation_names = Vec::new();
        // Only reassigned on the image path.
        #[cfg_attr(not(feature = "images"), allow(unused_mut))]
        let mut current_data = data;

        // Plan the run: collect operation labels, gather the image operations
        // so they can be folded over a *single* decode, and resolve the final
        // target format. Decoding and re-encoding per operation (as this used
        // to) costs N full codec round-trips and, for JPEG, N generations of
        // recompression.
        #[cfg(feature = "images")]
        let mut image_ops: Vec<crate::image::ImageOp> = Vec::new();
        #[cfg(feature = "images")]
        let mut target_format: Option<OutputFormat> = None;

        for op in self.operations {
            match op {
                #[cfg(feature = "images")]
                PipelineOp::Image(image_op) => {
                    // NOT `{:?}`: `ImageWatermark` carries the whole overlay
                    // buffer and `Bytes`'s `Debug` escapes every byte.
                    operation_names.push(format!("image:{}", image_op.name()));
                    image_ops.push(image_op);
                }
                PipelineOp::Convert(format) => {
                    operation_names.push(format!("convert:{}", format.extension()));

                    if matches!(format, OutputFormat::Original) {
                        // The bytes are already in the source format — round
                        // tripping them through the codec would recompress
                        // (and for JPEG, degrade) for no reason at all.
                        continue;
                    }

                    #[cfg(feature = "images")]
                    {
                        if !metadata.is_image() {
                            // Non-image input with a real target format: we
                            // have no encoder for that combination. Fail loudly
                            // instead of silently returning the input bytes
                            // mislabeled as converted.
                            return Err(FileError::UnsupportedFormat(format!(
                                "cannot convert non-image input (mime: {}) to {:?}",
                                metadata.mime_type, format
                            )));
                        }
                        target_format = Some(format);
                    }
                    #[cfg(not(feature = "images"))]
                    {
                        return Err(FileError::UnsupportedFormat(format!(
                            "cannot convert to {:?}: the `images` feature is not enabled",
                            format
                        )));
                    }
                }
                PipelineOp::Custom(name) => {
                    operation_names.push(format!("custom:{}", name));
                }
            }
        }

        #[cfg(feature = "images")]
        if !image_ops.is_empty() || target_format.is_some() {
            let limits = self.decode_limits;
            let default_font = self.font;
            let source = current_data.clone();
            let mut work_metadata = metadata;

            // Decoding, resizing and re-encoding are pure CPU (a Lanczos3
            // resize of a 4000x3000 JPEG runs 200-600 ms) with no I/O to
            // yield on. Running that inline would block a runtime worker and
            // stall every other task sharing the reactor.
            let (encoded, out_metadata) = tokio::task::spawn_blocking(move || {
                // AutoOrient needs the raw EXIF tag, which is only reachable
                // from the decoder, so it has to be honored at decode time.
                let needs_orientation = image_ops
                    .iter()
                    .any(|op| matches!(op, crate::image::ImageOp::AutoOrient));

                let mut img = if needs_orientation {
                    crate::image::decode_image_oriented(&source, limits)?
                } else {
                    crate::image::decode_image(&source, limits)?
                };

                for op in image_ops {
                    let op = apply_default_font(op, &default_font);
                    img = crate::image::apply_operation(img, &op, limits)?;
                }

                let encoded = crate::image::encode_variant(
                    &img,
                    &source,
                    target_format.unwrap_or(OutputFormat::Original),
                    &mut work_metadata,
                )?;

                Ok::<_, FileError>((encoded, work_metadata))
            })
            .await
            .map_err(|e| FileError::Pipeline(format!("image processing task failed: {e}")))??;

            current_data = encoded;
            metadata = out_metadata;
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
// `data`/`metadata` are only read by `generate`, which requires `images`.
#[cfg_attr(not(feature = "images"), allow(dead_code))]
pub struct MultiSizeBuilder {
    data: Bytes,
    metadata: FileMetadata,
    sizes: Vec<(String, u32, u32)>,
    output_format: OutputFormat,
    #[cfg(feature = "images")]
    decode_limits: crate::image::DecodeLimits,
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
            #[cfg(feature = "images")]
            decode_limits: crate::image::DecodeLimits::default(),
        }
    }

    /// Override the resource limits applied when decoding the source image.
    #[cfg(feature = "images")]
    pub fn decode_limits(mut self, limits: crate::image::DecodeLimits) -> Self {
        self.decode_limits = limits;
        self
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
    ///
    /// Decodes the source image once and resizes/encodes each variant from
    /// that single decode, rather than re-decoding the source per size.
    ///
    /// The whole decode/resize/encode batch runs on a blocking thread: it is
    /// pure CPU work with no I/O to await, and `with_responsive()` alone means
    /// four Lanczos3 resizes — enough to stall a runtime worker for seconds.
    #[cfg(feature = "images")]
    pub async fn generate(self) -> FileResult<Vec<(String, ProcessingResult)>> {
        tokio::task::spawn_blocking(move || {
            let source_img = crate::image::decode_image(&self.data, self.decode_limits)?;

            let mut results = Vec::new();
            for (name, width, height) in self.sizes {
                let variant_start = Instant::now();

                let resized = crate::image::resize_fit(&source_img, width, height);

                let mut metadata = self.metadata.clone();
                let data = crate::image::encode_variant(
                    &resized,
                    &self.data,
                    self.output_format,
                    &mut metadata,
                )?;
                metadata.size = data.len() as u64;

                results.push((
                    name,
                    ProcessingResult {
                        data,
                        metadata,
                        operations: vec![
                            format!("resize_fit:{}x{}", width, height),
                            format!("convert:{}", self.output_format.extension()),
                        ],
                        processing_time_ms: variant_start.elapsed().as_millis() as u64,
                    },
                ));
            }

            Ok::<_, FileError>(results)
        })
        .await
        .map_err(|e| FileError::Pipeline(format!("multi-size generation task failed: {e}")))?
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
