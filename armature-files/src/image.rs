//! Image processing operations
//!
//! Provides comprehensive image manipulation including:
//! - Resizing (with various filters)
//! - Cropping
//! - Rotation and flipping
//! - Color adjustments
//! - Watermarks (text and image)
//! - Format conversion

use crate::{FileError, FileMetadata, FileResult, OutputFormat, Position};
use ab_glyph::{FontRef, PxScale};
use bytes::Bytes;
use image::{
    DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader, Rgba,
    imageops::FilterType,
};
use imageproc::drawing::draw_text_mut;
use std::io::Cursor;

/// Embedded fallback font used for text watermarks (DejaVu Sans, Bitstream
/// Vera License — see `assets/fonts/DejaVuSans-LICENSE.txt`).
static WATERMARK_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");

/// Image resize filter quality
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeFilter {
    /// Fastest, lowest quality
    Nearest,
    /// Fast, decent quality
    Triangle,
    /// Good balance
    CatmullRom,
    /// Best quality, slowest
    #[default]
    Lanczos3,
}

impl ResizeFilter {
    fn to_filter_type(self) -> FilterType {
        match self {
            Self::Nearest => FilterType::Nearest,
            Self::Triangle => FilterType::Triangle,
            Self::CatmullRom => FilterType::CatmullRom,
            Self::Lanczos3 => FilterType::Lanczos3,
        }
    }
}

/// Image operations
#[derive(Debug, Clone)]
pub enum ImageOp {
    /// Resize to exact dimensions
    Resize {
        width: u32,
        height: u32,
        filter: ResizeFilter,
    },
    /// Resize to fit within bounds while maintaining aspect ratio
    ResizeFit {
        max_width: u32,
        max_height: u32,
        filter: ResizeFilter,
    },
    /// Resize to fill bounds (may crop)
    ResizeFill {
        width: u32,
        height: u32,
        filter: ResizeFilter,
    },
    /// Crop a region
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Rotate by degrees (supports 90, 180, 270, or arbitrary)
    Rotate { degrees: f32 },
    /// Flip horizontally
    FlipHorizontal,
    /// Flip vertically
    FlipVertical,
    /// Adjust brightness (-100 to 100)
    Brightness(i32),
    /// Adjust contrast (-100 to 100)
    Contrast(i32),
    /// Convert to grayscale
    Grayscale,
    /// Invert colors
    Invert,
    /// Apply blur (sigma)
    Blur(f32),
    /// Sharpen the image
    Sharpen,
    /// Add text watermark
    TextWatermark {
        text: String,
        position: Position,
        font_size: f32,
        color: [u8; 4], // RGBA
    },
    /// Add image watermark/overlay
    ImageWatermark {
        overlay: Bytes,
        position: Position,
        opacity: f32,
    },
    /// Apply auto-orientation based on EXIF
    AutoOrient,
    /// Strip EXIF metadata
    StripMetadata,
}

/// Process an image with the given operation
pub fn process_image(data: &Bytes, op: &ImageOp, metadata: &mut FileMetadata) -> FileResult<Bytes> {
    let mut img = if matches!(op, ImageOp::AutoOrient) {
        // AutoOrient needs the raw EXIF orientation tag, which is only
        // available from the decoder, not from an already-decoded image.
        load_image_oriented(data)?
    } else {
        load_image(data)?
    };

    img = apply_operation(img, op)?;

    // Update metadata
    let (width, height) = img.dimensions();
    metadata.width = Some(width);
    metadata.height = Some(height);

    // Encode back to the same (detected) format
    encode_image(&img, data, metadata)
}

/// Convert image to a different format
pub fn convert_format(
    data: &Bytes,
    format: OutputFormat,
    metadata: &mut FileMetadata,
) -> FileResult<Bytes> {
    let img = load_image(data)?;

    if matches!(format, OutputFormat::Original) {
        // `Original` means "keep the source format" — re-encode using the
        // detected/original format instead of falling into the catch-all
        // `UnsupportedFormat` error in `encode_image_format`. Metadata is
        // intentionally left untouched since the format did not change.
        return encode_image(&img, data, metadata);
    }

    // Update metadata for new format
    metadata.mime_type = format.mime_type().to_string();
    metadata.extension = Some(format.extension().to_string());

    encode_image_format(&img, format)
}

/// Load an image from bytes
fn load_image(data: &Bytes) -> FileResult<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| FileError::Image(format!("Failed to detect format: {}", e)))?;

    reader
        .decode()
        .map_err(|e| FileError::Image(format!("Failed to decode: {}", e)))
}

/// Load an image from bytes and apply its EXIF orientation (if any).
fn load_image_oriented(data: &Bytes) -> FileResult<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| FileError::Image(format!("Failed to detect format: {}", e)))?;

    let mut decoder = reader
        .into_decoder()
        .map_err(|e| FileError::Image(format!("Failed to create decoder: {}", e)))?;

    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);

    let mut img = DynamicImage::from_decoder(decoder)
        .map_err(|e| FileError::Image(format!("Failed to decode: {}", e)))?;

    img.apply_orientation(orientation);
    Ok(img)
}

/// Detect the original `OutputFormat` of an already-loaded image, used to
/// preserve format on `OutputFormat::Original` conversions and when
/// re-encoding after an in-place operation.
fn detected_output_format(data: &Bytes, metadata: &FileMetadata) -> OutputFormat {
    if let Ok(fmt) = detect_format(data)
        && let Some(output_format) = image_format_to_output(fmt)
    {
        return output_format;
    }

    extension_output_format(metadata.extension.as_deref())
}

/// Map an `image::ImageFormat` (detected from bytes) to our `OutputFormat`.
fn image_format_to_output(fmt: ImageFormat) -> Option<OutputFormat> {
    match fmt {
        ImageFormat::Jpeg => Some(OutputFormat::Jpeg { quality: 85 }),
        ImageFormat::Png => Some(OutputFormat::Png),
        ImageFormat::WebP => Some(OutputFormat::WebP),
        ImageFormat::Gif => Some(OutputFormat::Gif),
        ImageFormat::Bmp => Some(OutputFormat::Bmp),
        ImageFormat::Ico => Some(OutputFormat::Ico),
        ImageFormat::Tiff => Some(OutputFormat::Tiff),
        _ => None,
    }
}

/// Fall back to guessing the format from a filename extension.
fn extension_output_format(ext: Option<&str>) -> OutputFormat {
    match ext {
        Some("jpg") | Some("jpeg") => OutputFormat::Jpeg { quality: 85 },
        Some("png") => OutputFormat::Png,
        Some("webp") => OutputFormat::WebP,
        Some("gif") => OutputFormat::Gif,
        Some("bmp") => OutputFormat::Bmp,
        Some("tiff") | Some("tif") => OutputFormat::Tiff,
        _ => OutputFormat::Png, // Default to PNG for unknown formats
    }
}

/// Apply an operation to an image
fn apply_operation(img: DynamicImage, op: &ImageOp) -> FileResult<DynamicImage> {
    match op {
        ImageOp::Resize {
            width,
            height,
            filter,
        } => Ok(img.resize_exact(*width, *height, filter.to_filter_type())),
        ImageOp::ResizeFit {
            max_width,
            max_height,
            filter,
        } => Ok(img.resize(*max_width, *max_height, filter.to_filter_type())),
        ImageOp::ResizeFill {
            width,
            height,
            filter,
        } => Ok(img.resize_to_fill(*width, *height, filter.to_filter_type())),
        ImageOp::Crop {
            x,
            y,
            width,
            height,
        } => {
            let (img_width, img_height) = img.dimensions();
            if *x + *width > img_width || *y + *height > img_height {
                return Err(FileError::InvalidDimensions {
                    width: *width,
                    height: *height,
                });
            }
            Ok(img.crop_imm(*x, *y, *width, *height))
        }
        ImageOp::Rotate { degrees } => {
            let normalized = degrees.rem_euclid(360.0);
            match normalized as u32 {
                0 => Ok(img),
                90 => Ok(img.rotate90()),
                180 => Ok(img.rotate180()),
                270 => Ok(img.rotate270()),
                _ => {
                    // For arbitrary rotation, use imageproc
                    #[cfg(feature = "images")]
                    {
                        let rgba = img.to_rgba8();
                        let rotated = imageproc::geometric_transformations::rotate_about_center(
                            &rgba,
                            degrees.to_radians(),
                            imageproc::geometric_transformations::Interpolation::Bilinear,
                            imageproc::geometric_transformations::Border::Constant(Rgba([
                                0, 0, 0, 0,
                            ])),
                        );
                        Ok(DynamicImage::ImageRgba8(rotated))
                    }
                    #[cfg(not(feature = "images"))]
                    {
                        Err(FileError::InvalidOperation(
                            "Arbitrary rotation requires imageproc feature".into(),
                        ))
                    }
                }
            }
        }
        ImageOp::FlipHorizontal => Ok(img.fliph()),
        ImageOp::FlipVertical => Ok(img.flipv()),
        ImageOp::Brightness(value) => Ok(img.brighten(*value)),
        ImageOp::Contrast(value) => Ok(img.adjust_contrast(*value as f32)),
        ImageOp::Grayscale => Ok(img.grayscale()),
        ImageOp::Invert => {
            let mut img = img;
            img.invert();
            Ok(img)
        }
        ImageOp::Blur(sigma) => Ok(img.blur(*sigma)),
        ImageOp::Sharpen => Ok(img.unsharpen(1.0, 1)),
        ImageOp::TextWatermark {
            text,
            position,
            font_size,
            color,
        } => apply_text_watermark(img, text, *position, *font_size, *color),
        ImageOp::ImageWatermark {
            overlay,
            position,
            opacity,
        } => apply_image_watermark(img, overlay, *position, *opacity),
        ImageOp::AutoOrient => {
            // The EXIF orientation is read and applied by `load_image_oriented`
            // before this function runs (see `process_image`), so by the time
            // we get here the pixels are already correctly oriented.
            Ok(img)
        }
        ImageOp::StripMetadata => {
            // Re-encoding via `image`/`imageproc` always produces a fresh
            // buffer with no EXIF/ICC metadata carried over, so this is
            // effectively a no-op at this stage.
            Ok(img)
        }
    }
}

/// Apply a text watermark to an image
///
/// Renders real glyphs (via `imageproc::drawing::draw_text_mut` + `ab_glyph`,
/// using an embedded DejaVu Sans font) honoring both `text` and `font_size`,
/// alpha-blended onto the image using `color`'s alpha channel.
fn apply_text_watermark(
    img: DynamicImage,
    text: &str,
    position: Position,
    font_size: f32,
    color: [u8; 4],
) -> FileResult<DynamicImage> {
    let mut rgba = img.to_rgba8();
    let (img_width, img_height) = rgba.dimensions();

    let font = FontRef::try_from_slice(WATERMARK_FONT_BYTES)
        .map_err(|e| FileError::Image(format!("Failed to load watermark font: {}", e)))?;
    let scale = PxScale::from(font_size.max(1.0));

    let (text_width, text_height) = imageproc::drawing::text_size(scale, &font, text);

    let (x, y) = position.calculate(img_width, img_height, text_width, text_height, 10);

    draw_text_mut(
        &mut rgba,
        Rgba(color),
        x as i32,
        y as i32,
        scale,
        &font,
        text,
    );

    Ok(DynamicImage::ImageRgba8(rgba))
}

/// Apply an image watermark/overlay
fn apply_image_watermark(
    img: DynamicImage,
    overlay_data: &Bytes,
    position: Position,
    opacity: f32,
) -> FileResult<DynamicImage> {
    let overlay = load_image(overlay_data)?;
    let mut base = img.to_rgba8();

    let (base_width, base_height) = base.dimensions();
    let (overlay_width, overlay_height) = overlay.dimensions();

    let (x, y) = position.calculate(base_width, base_height, overlay_width, overlay_height, 10);

    // Blend the overlay onto the base image
    let overlay_rgba = overlay.to_rgba8();
    for dy in 0..overlay_height.min(base_height.saturating_sub(y)) {
        for dx in 0..overlay_width.min(base_width.saturating_sub(x)) {
            let overlay_pixel = overlay_rgba.get_pixel(dx, dy);
            let base_pixel = base.get_pixel_mut(x + dx, y + dy);

            let alpha = (overlay_pixel[3] as f32 / 255.0) * opacity;
            base_pixel[0] =
                ((1.0 - alpha) * base_pixel[0] as f32 + alpha * overlay_pixel[0] as f32) as u8;
            base_pixel[1] =
                ((1.0 - alpha) * base_pixel[1] as f32 + alpha * overlay_pixel[1] as f32) as u8;
            base_pixel[2] =
                ((1.0 - alpha) * base_pixel[2] as f32 + alpha * overlay_pixel[2] as f32) as u8;
        }
    }

    Ok(DynamicImage::ImageRgba8(base))
}

/// Decode image bytes into a `DynamicImage`.
///
/// Exposed crate-wide so callers that need to produce multiple derived
/// images from one source (e.g. `MultiSizeBuilder`) can decode once instead
/// of re-decoding per variant.
pub(crate) fn decode_image(data: &Bytes) -> FileResult<DynamicImage> {
    load_image(data)
}

/// Resize `img` to fit within `max_width`/`max_height` while preserving
/// aspect ratio (mirrors `ImageOp::ResizeFit`).
pub(crate) fn resize_fit(img: &DynamicImage, max_width: u32, max_height: u32) -> DynamicImage {
    img.resize(max_width, max_height, FilterType::Lanczos3)
}

/// Encode an already-decoded/resized `img` to `output_format`, updating
/// `metadata` accordingly. `source_data` is the original file's raw bytes,
/// used only to detect the original format when `output_format` is
/// `OutputFormat::Original`.
pub(crate) fn encode_variant(
    img: &DynamicImage,
    source_data: &Bytes,
    output_format: OutputFormat,
    metadata: &mut FileMetadata,
) -> FileResult<Bytes> {
    let (width, height) = img.dimensions();
    metadata.width = Some(width);
    metadata.height = Some(height);

    if matches!(output_format, OutputFormat::Original) {
        return encode_image(img, source_data, metadata);
    }

    metadata.mime_type = output_format.mime_type().to_string();
    metadata.extension = Some(output_format.extension().to_string());
    encode_image_format(img, output_format)
}

/// Encode an image back to bytes, preserving the detected/original format.
fn encode_image(img: &DynamicImage, data: &Bytes, metadata: &FileMetadata) -> FileResult<Bytes> {
    let format = detected_output_format(data, metadata);
    encode_image_format(img, format)
}

/// Encode an image to a specific format
fn encode_image_format(img: &DynamicImage, format: OutputFormat) -> FileResult<Bytes> {
    let mut buffer = Vec::new();

    match format {
        OutputFormat::Jpeg { quality } => {
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality);
            img.write_with_encoder(encoder)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::Png => {
            img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::WebP => {
            // The `image` crate only supports lossless WebP encoding; there
            // is no quality/lossy knob to honor (see `OutputFormat::WebP` docs).
            let encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut buffer);
            img.write_with_encoder(encoder)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::Gif => {
            img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Gif)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::Bmp => {
            img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Bmp)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::Ico => {
            img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Ico)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::Tiff => {
            img.write_to(&mut Cursor::new(&mut buffer), ImageFormat::Tiff)
                .map_err(|e| FileError::Encoding(e.to_string()))?;
        }
        OutputFormat::Avif { .. } => {
            // AVIF support depends on features
            return Err(FileError::UnsupportedFormat(
                "AVIF encoding not available".into(),
            ));
        }
        _ => {
            return Err(FileError::UnsupportedFormat(format!("{:?}", format)));
        }
    }

    Ok(Bytes::from(buffer))
}

/// Get image dimensions without fully decoding
pub fn get_dimensions(data: &Bytes) -> FileResult<(u32, u32)> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| FileError::Image(format!("Failed to detect format: {}", e)))?;

    reader
        .into_dimensions()
        .map_err(|e| FileError::Image(format!("Failed to get dimensions: {}", e)))
}

/// Detect the image format
pub fn detect_format(data: &Bytes) -> FileResult<ImageFormat> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| FileError::Image(format!("Failed to detect format: {}", e)))?;

    reader
        .format()
        .ok_or_else(|| FileError::UnsupportedFormat("Unknown image format".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Actual image tests would require test fixtures

    #[test]
    fn test_resize_filter_default() {
        assert_eq!(ResizeFilter::default(), ResizeFilter::Lanczos3);
    }

    #[test]
    fn test_position_calculation() {
        let (x, y) = Position::Center.calculate(100, 100, 20, 20, 0);
        assert_eq!((x, y), (40, 40));
    }
}
