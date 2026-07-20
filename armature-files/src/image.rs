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
    DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader, Rgba, RgbaImage,
    imageops::FilterType,
};
use imageproc::drawing::draw_text_mut;
use std::io::Cursor;

/// Embedded fallback font used for text watermarks (DejaVu Sans, Bitstream
/// Vera License — see `assets/fonts/DejaVuSans-LICENSE.txt`).
///
/// Only compiled in under the (default-on) `embedded-font` feature; with it
/// disabled the caller must supply its own font via
/// [`ImageOp::TextWatermark::font`] or [`crate::Pipeline::with_font`].
#[cfg(feature = "embedded-font")]
static WATERMARK_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");

/// Default JPEG quality used whenever a JPEG must be (re-)encoded and no
/// explicit quality was requested.
pub const DEFAULT_JPEG_QUALITY: u8 = 85;

/// Resource limits applied when decoding untrusted image bytes.
///
/// Without these, a 40-byte PNG header declaring `65535x65535` forces the
/// decoder into a ~17 GB RGBA allocation, OOM-killing the process. The
/// defaults cap both the declared dimensions and the total allocation the
/// decoder is permitted to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// Maximum accepted image width in pixels.
    pub max_width: u32,
    /// Maximum accepted image height in pixels.
    pub max_height: u32,
    /// Maximum total bytes the decoder may allocate.
    pub max_alloc_bytes: u64,
}

impl DecodeLimits {
    /// Default width/height cap (16384 px on each axis).
    pub const DEFAULT_MAX_DIMENSION: u32 = 16_384;
    /// Default decoder allocation cap (256 MiB).
    pub const DEFAULT_MAX_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

    fn to_image_limits(self) -> image::Limits {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(self.max_width);
        limits.max_image_height = Some(self.max_height);
        limits.max_alloc = Some(self.max_alloc_bytes);
        limits
    }
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_width: Self::DEFAULT_MAX_DIMENSION,
            max_height: Self::DEFAULT_MAX_DIMENSION,
            max_alloc_bytes: Self::DEFAULT_MAX_ALLOC_BYTES,
        }
    }
}

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
    /// Add text watermark.
    ///
    /// `color`'s alpha channel is the *global* opacity of the watermark: the
    /// glyphs are rasterized at full opacity into a scratch layer and that
    /// layer is then composited into the image's RGB channels. The image's own
    /// alpha channel is left untouched, so the watermark survives formats that
    /// do not carry alpha (JPEG, BMP, GIF).
    TextWatermark {
        text: String,
        position: Position,
        font_size: f32,
        color: [u8; 4], // RGBA
        /// TrueType/OpenType font to render with. `None` falls back to the
        /// font set via [`crate::Pipeline::with_font`], and failing that to
        /// the embedded DejaVu Sans (requires the `embedded-font` feature).
        font: Option<Bytes>,
    },
    /// Add image watermark/overlay
    ImageWatermark {
        overlay: Bytes,
        position: Position,
        opacity: f32,
    },
    /// Apply auto-orientation based on the source image's EXIF orientation tag.
    ///
    /// # Ordering
    ///
    /// This operation is **position-independent**: it is always applied at
    /// decode time, before every other operation in the chain, regardless of
    /// where it appears. `.rotate(90).image(AutoOrient)` therefore applies the
    /// EXIF orientation *first* and then rotates, exactly as
    /// `.image(AutoOrient).rotate(90)` does.
    ///
    /// This is not an oversight. The orientation tag is only readable from the
    /// decoder, not from an already-decoded `DynamicImage`, so by the time an
    /// operation chain is being folded the tag no longer exists to consult.
    /// Applying it at decode time is also the semantically right thing: EXIF
    /// orientation describes how the *stored* pixels relate to the intended
    /// display, so it belongs with the decode, not partway through a chain of
    /// deliberate transforms.
    AutoOrient,
    /// Strip EXIF/ICC metadata.
    ///
    /// This is a **no-op at the pixel level**: every path in this crate that
    /// produces output bytes re-encodes through `image`, which writes a fresh
    /// buffer and never carries EXIF or ICC chunks over from the source. The
    /// variant exists so the intent is explicit (and pinned by a test), not
    /// because it performs additional work.
    StripMetadata,
}

impl ImageOp {
    /// A short, stable label for this operation, suitable for logs and for
    /// [`crate::ProcessingResult::operations`].
    ///
    /// Deliberately *not* `{:?}`: `ImageWatermark` carries the whole overlay
    /// buffer, and `Bytes`'s `Debug` escapes every single byte — formatting a
    /// 2 MB overlay would allocate a multi-megabyte `String` just to label an
    /// operation.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Resize { .. } => "resize",
            Self::ResizeFit { .. } => "resize_fit",
            Self::ResizeFill { .. } => "resize_fill",
            Self::Crop { .. } => "crop",
            Self::Rotate { .. } => "rotate",
            Self::FlipHorizontal => "flip_horizontal",
            Self::FlipVertical => "flip_vertical",
            Self::Brightness(_) => "brightness",
            Self::Contrast(_) => "contrast",
            Self::Grayscale => "grayscale",
            Self::Invert => "invert",
            Self::Blur(_) => "blur",
            Self::Sharpen => "sharpen",
            Self::TextWatermark { .. } => "text_watermark",
            Self::ImageWatermark { .. } => "image_watermark",
            Self::AutoOrient => "auto_orient",
            Self::StripMetadata => "strip_metadata",
        }
    }
}

impl std::fmt::Display for ImageOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Process an image with the given operation, using the default
/// [`DecodeLimits`].
///
/// Prefer [`process_image_async`] from inside async code: decode, transform and
/// re-encode are pure CPU with no I/O to yield on (a Lanczos3 resize of a
/// 4000x3000 JPEG runs 200-600 ms) and block the calling runtime worker for the
/// whole duration.
pub fn process_image(data: &Bytes, op: &ImageOp, metadata: &mut FileMetadata) -> FileResult<Bytes> {
    process_image_with_limits(data, op, metadata, DecodeLimits::default())
}

/// Process an image with the given operation on a blocking thread.
///
/// The async counterpart to [`process_image`]. Returns the updated metadata
/// alongside the encoded bytes, since the work happens on another thread and
/// cannot write through a `&mut` borrow.
pub async fn process_image_async(
    data: Bytes,
    op: ImageOp,
    metadata: FileMetadata,
    limits: DecodeLimits,
) -> FileResult<(Bytes, FileMetadata)> {
    tokio::task::spawn_blocking(move || {
        let mut metadata = metadata;
        let encoded = process_image_with_limits(&data, &op, &mut metadata, limits)?;
        Ok::<_, FileError>((encoded, metadata))
    })
    .await
    .map_err(|e| FileError::Image(format!("image processing task failed: {e}")))?
}

/// Process an image with the given operation and explicit decode limits.
pub fn process_image_with_limits(
    data: &Bytes,
    op: &ImageOp,
    metadata: &mut FileMetadata,
    limits: DecodeLimits,
) -> FileResult<Bytes> {
    let mut img = if matches!(op, ImageOp::AutoOrient) {
        // AutoOrient needs the raw EXIF orientation tag, which is only
        // available from the decoder, not from an already-decoded image.
        load_image_oriented(data, limits)?
    } else {
        load_image(data, limits)?
    };

    img = apply_operation(img, op, limits)?;

    // Update metadata
    let (width, height) = img.dimensions();
    metadata.width = Some(width);
    metadata.height = Some(height);

    // Encode back to the same (detected) format
    encode_image(&img, data, metadata)
}

/// Convert image to a different format, using the default [`DecodeLimits`].
///
/// Prefer [`convert_format_async`] from inside async code: this is a full
/// decode plus re-encode and blocks the calling runtime worker throughout.
///
/// Note that unlike [`crate::Pipeline`], this function honors
/// [`OutputFormat::Original`] by *re-encoding* in the source format rather than
/// passing the input bytes through — see the variant's documentation.
pub fn convert_format(
    data: &Bytes,
    format: OutputFormat,
    metadata: &mut FileMetadata,
) -> FileResult<Bytes> {
    convert_format_with_limits(data, format, metadata, DecodeLimits::default())
}

/// Convert an image to a different format on a blocking thread.
///
/// The async counterpart to [`convert_format`].
pub async fn convert_format_async(
    data: Bytes,
    format: OutputFormat,
    metadata: FileMetadata,
    limits: DecodeLimits,
) -> FileResult<(Bytes, FileMetadata)> {
    tokio::task::spawn_blocking(move || {
        let mut metadata = metadata;
        let encoded = convert_format_with_limits(&data, format, &mut metadata, limits)?;
        Ok::<_, FileError>((encoded, metadata))
    })
    .await
    .map_err(|e| FileError::Image(format!("format conversion task failed: {e}")))?
}

/// Convert image to a different format with explicit decode limits.
pub fn convert_format_with_limits(
    data: &Bytes,
    format: OutputFormat,
    metadata: &mut FileMetadata,
    limits: DecodeLimits,
) -> FileResult<Bytes> {
    let img = load_image(data, limits)?;

    if matches!(format, OutputFormat::Original) {
        // `Original` means "keep the source format" — re-encode using the
        // detected/original format instead of falling into the catch-all
        // `UnsupportedFormat` error in `encode_image_format`.
        return encode_image(&img, data, metadata);
    }

    // Update metadata for new format
    metadata.mime_type = format.mime_type().to_string();
    metadata.extension = Some(format.extension().to_string());

    encode_image_format(&img, format)
}

/// Build a format-guessing `ImageReader` with `limits` applied.
///
/// `ImageReader::into_decoder` propagates the reader's limits to the decoder,
/// so this covers both the plain and the EXIF-oriented load paths.
fn limited_reader(data: &Bytes, limits: DecodeLimits) -> FileResult<ImageReader<Cursor<&Bytes>>> {
    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| FileError::Image(format!("Failed to detect format: {}", e)))?;
    reader.limits(limits.to_image_limits());
    Ok(reader)
}

/// Load an image from bytes, rejecting anything exceeding `limits`.
fn load_image(data: &Bytes, limits: DecodeLimits) -> FileResult<DynamicImage> {
    limited_reader(data, limits)?
        .decode()
        .map_err(|e| FileError::Image(format!("Failed to decode: {}", e)))
}

/// Load an image from bytes and apply its EXIF orientation (if any),
/// rejecting anything exceeding `limits`.
fn load_image_oriented(data: &Bytes, limits: DecodeLimits) -> FileResult<DynamicImage> {
    let mut decoder = limited_reader(data, limits)?
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
///
/// Errors with [`FileError::UnsupportedFormat`] when neither content sniffing
/// nor the filename extension yields a format this crate can encode. Silently
/// falling back to PNG (as this used to) mislabels the output: the bytes would
/// be PNG while `metadata.mime_type` still claimed e.g. `image/avif`.
fn detected_output_format(data: &Bytes, metadata: &FileMetadata) -> FileResult<OutputFormat> {
    if let Ok(fmt) = detect_format(data)
        && let Some(output_format) = image_format_to_output(fmt)
    {
        return Ok(output_format);
    }

    extension_output_format(metadata.extension.as_deref())
}

/// Map an `image::ImageFormat` (detected from bytes) to our `OutputFormat`.
///
/// This is the single source of truth for the format table; the
/// extension-based fallback delegates through it.
fn image_format_to_output(fmt: ImageFormat) -> Option<OutputFormat> {
    match fmt {
        ImageFormat::Jpeg => Some(OutputFormat::Jpeg {
            quality: DEFAULT_JPEG_QUALITY,
        }),
        ImageFormat::Png => Some(OutputFormat::Png),
        ImageFormat::WebP => Some(OutputFormat::WebP),
        ImageFormat::Gif => Some(OutputFormat::Gif),
        ImageFormat::Bmp => Some(OutputFormat::Bmp),
        ImageFormat::Ico => Some(OutputFormat::Ico),
        ImageFormat::Tiff => Some(OutputFormat::Tiff),
        _ => None,
    }
}

/// Fall back to deriving the format from a filename extension, via the same
/// table as [`image_format_to_output`].
fn extension_output_format(ext: Option<&str>) -> FileResult<OutputFormat> {
    ext.and_then(ImageFormat::from_extension)
        .and_then(image_format_to_output)
        .ok_or_else(|| {
            FileError::UnsupportedFormat(format!(
                "cannot determine an encodable image format (extension: {})",
                ext.unwrap_or("<none>")
            ))
        })
}

/// Apply an operation to an already-decoded image.
///
/// Exposed crate-wide so `Pipeline::execute` can fold a whole chain of
/// operations over one decode instead of decoding and re-encoding per op.
///
/// `limits` applies to any *further* decoding an operation has to do — today
/// that is `ImageOp::ImageWatermark`, whose overlay is a second buffer of
/// untrusted image bytes and therefore needs the same protection as the primary
/// input rather than the hardcoded defaults.
pub(crate) fn apply_operation(
    img: DynamicImage,
    op: &ImageOp,
    limits: DecodeLimits,
) -> FileResult<DynamicImage> {
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
            // `checked_add`, not `x + width`: the operands are caller-supplied
            // (`Pipeline::crop` passes them straight through), so
            // `crop(u32::MAX, 0, 10, 10)` panics in debug and, worse, wraps in
            // release to a small value that *passes* a naive bounds check and
            // reaches `crop_imm` with a nonsensical origin.
            let overflows_x = x.checked_add(*width).is_none_or(|r| r > img_width);
            let overflows_y = y.checked_add(*height).is_none_or(|r| r > img_height);
            if overflows_x || overflows_y {
                return Err(FileError::InvalidDimensions {
                    width: *width,
                    height: *height,
                });
            }
            Ok(img.crop_imm(*x, *y, *width, *height))
        }
        ImageOp::Rotate { degrees } => {
            let normalized = degrees.rem_euclid(360.0);

            // Only take the lossless fast paths for *exact* quarter turns.
            // `normalized as u32` truncates, so 90.5° would otherwise be
            // silently rendered as a plain 90° rotation.
            if normalized.fract() == 0.0 {
                match normalized as u32 {
                    0 => return Ok(img),
                    90 => return Ok(img.rotate90()),
                    180 => return Ok(img.rotate180()),
                    270 => return Ok(img.rotate270()),
                    _ => {}
                }
            }

            // Arbitrary angle: rasterize through imageproc.
            let rgba = img.to_rgba8();
            let rotated = imageproc::geometric_transformations::rotate_about_center(
                &rgba,
                normalized.to_radians(),
                imageproc::geometric_transformations::Interpolation::Bilinear,
                imageproc::geometric_transformations::Border::Constant(Rgba([0, 0, 0, 0])),
            );
            Ok(DynamicImage::ImageRgba8(rotated))
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
            font,
        } => apply_text_watermark(img, text, *position, *font_size, *color, font.as_deref()),
        ImageOp::ImageWatermark {
            overlay,
            position,
            opacity,
        } => apply_image_watermark(img, overlay, *position, *opacity, limits),
        ImageOp::AutoOrient => {
            // The EXIF orientation is read and applied by `load_image_oriented`
            // before this function runs (see `process_image`), so by the time
            // we get here the pixels are already correctly oriented.
            Ok(img)
        }
        // See the `ImageOp::StripMetadata` docs: re-encoding always emits a
        // fresh, metadata-free buffer, so there is nothing to do here.
        ImageOp::StripMetadata => Ok(img),
    }
}

/// Error returned when a text watermark has no font to render with.
#[cfg(not(feature = "embedded-font"))]
fn missing_font_error() -> FileError {
    FileError::InvalidOperation(
        "text watermark requires a font: enable the `embedded-font` feature \
         or supply one via `Pipeline::with_font` / `ImageOp::TextWatermark { font, .. }`"
            .into(),
    )
}

/// The parsed embedded fallback font, parsed at most once per process.
///
/// `FontRef::try_from_slice` re-parses the whole 760 KB TTF (building the table
/// directory and glyph indices) on every call, so watermarking N images used to
/// pay that N times for a font that never changes.
#[cfg(feature = "embedded-font")]
fn embedded_font() -> FileResult<&'static FontRef<'static>> {
    static FONT: std::sync::OnceLock<Result<FontRef<'static>, String>> = std::sync::OnceLock::new();

    FONT.get_or_init(|| FontRef::try_from_slice(WATERMARK_FONT_BYTES).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| FileError::Image(format!("Failed to load watermark font: {e}")))
}

/// Apply a text watermark to an image.
///
/// Renders real glyphs (via `imageproc::drawing::draw_text_mut` + `ab_glyph`)
/// honoring both `text` and `font_size`.
///
/// The glyphs are rasterized at **full opacity** into a transparent scratch
/// layer, and that layer is then composited into the target's RGB channels
/// scaled by `color[3]`. Drawing directly with `Rgba(color)` instead would
/// write the requested opacity into the *alpha channel* of the result, so a
/// 25%-opacity black watermark would come out as fully opaque black on any
/// format that drops alpha (JPEG, BMP, GIF).
fn apply_text_watermark(
    img: DynamicImage,
    text: &str,
    position: Position,
    font_size: f32,
    color: [u8; 4],
    font_bytes: Option<&[u8]>,
) -> FileResult<DynamicImage> {
    let mut rgba = img.to_rgba8();
    let (img_width, img_height) = rgba.dimensions();

    // An explicitly supplied font is parsed here; the embedded fallback comes
    // from a process-wide `OnceLock` so it is parsed at most once.
    let supplied;
    let font: &FontRef<'_> = match font_bytes {
        Some(bytes) => {
            supplied = FontRef::try_from_slice(bytes)
                .map_err(|e| FileError::Image(format!("Failed to load watermark font: {}", e)))?;
            &supplied
        }
        #[cfg(feature = "embedded-font")]
        None => embedded_font()?,
        #[cfg(not(feature = "embedded-font"))]
        None => return Err(missing_font_error()),
    };

    let scale = PxScale::from(font_size.max(1.0));

    let (text_width, text_height) = imageproc::drawing::text_size(scale, font, text);

    // `calculate` saturates, so oversized text anchors at the origin rather
    // than underflowing; clamp again so the draw origin is always in-bounds.
    let (x, y) = position.calculate(img_width, img_height, text_width, text_height, 10);
    let x = x.min(img_width.saturating_sub(1));
    let y = y.min(img_height.saturating_sub(1));

    // Rasterize the glyph coverage into a transparent scratch layer.
    let mut layer = RgbaImage::new(img_width, img_height);
    draw_text_mut(
        &mut layer,
        Rgba([color[0], color[1], color[2], 255]),
        x as i32,
        y as i32,
        scale,
        font,
        text,
    );

    // Composite: glyph coverage * requested global opacity.
    //
    // The layer is read for *coverage only* (`source[3]`); the colour comes
    // from `color`. imageproc 0.27 composites with straight, non-premultiplied
    // alpha — an edge pixel at coverage `v` comes out as
    // `[r, g, b, 255*v]`, not `[v*r, v*g, v*b, 255*v]` — so taking the colour
    // from the layer instead would happen to give the same result today. It
    // would stop doing so the moment imageproc switched to premultiplied
    // output, at which point `alpha * source[ch]` would silently become
    // `v^2 * color[ch]`: correct at full coverage, but antialiased edges
    // blending toward `0.5 * color`, i.e. grey fringes around a white
    // watermark on a dark photo. Not depending on that convention costs
    // nothing. `antialiased_watermark_edges_are_not_double_darkened` pins it.
    let global_alpha = color[3] as f32 / 255.0;
    for (target, source) in rgba.pixels_mut().zip(layer.pixels()) {
        let alpha = (source[3] as f32 / 255.0) * global_alpha;
        if alpha <= 0.0 {
            continue;
        }
        for channel in 0..3 {
            target[channel] = ((1.0 - alpha) * target[channel] as f32
                + alpha * color[channel] as f32)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }

    Ok(DynamicImage::ImageRgba8(rgba))
}

/// Apply an image watermark/overlay.
///
/// `limits` is the caller's configured [`DecodeLimits`], not the default set:
/// the overlay is a second buffer of caller-supplied, potentially untrusted
/// image bytes going through the very decode path those limits exist to
/// protect. Hardcoding the defaults here meant a caller who tightened the
/// limits for an untrusted upload path still permitted a 16384x16384 /
/// 256 MiB overlay decode.
fn apply_image_watermark(
    img: DynamicImage,
    overlay_data: &Bytes,
    position: Position,
    opacity: f32,
    limits: DecodeLimits,
) -> FileResult<DynamicImage> {
    let overlay = load_image(overlay_data, limits)?;
    let mut base = img.to_rgba8();

    let (base_width, base_height) = base.dimensions();
    let (overlay_width, overlay_height) = overlay.dimensions();

    // `calculate` saturates; clamp again so the blend origin is in-bounds even
    // when the overlay is larger than the base image.
    let (x, y) = position.calculate(base_width, base_height, overlay_width, overlay_height, 10);
    let x = x.min(base_width.saturating_sub(1));
    let y = y.min(base_height.saturating_sub(1));

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
pub(crate) fn decode_image(data: &Bytes, limits: DecodeLimits) -> FileResult<DynamicImage> {
    load_image(data, limits)
}

/// Decode image bytes, applying the EXIF orientation tag if present.
pub(crate) fn decode_image_oriented(
    data: &Bytes,
    limits: DecodeLimits,
) -> FileResult<DynamicImage> {
    load_image_oriented(data, limits)
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

/// Encode an image back to bytes, preserving the detected/original format,
/// and update `metadata` to describe the format actually written.
///
/// The metadata update is not cosmetic: `image` can decode formats this crate
/// cannot re-encode (AVIF, QOI, PNM, DDS, HDR, Farbfeld, TGA). Previously the
/// fallback silently produced PNG bytes while leaving `mime_type` at e.g.
/// `image/avif`, so a downstream `Storage::put` served a PNG under a
/// `Content-Type` browsers refuse to render. Now the format is either
/// preserved and the metadata kept in sync, or the call fails outright.
pub(crate) fn encode_image(
    img: &DynamicImage,
    data: &Bytes,
    metadata: &mut FileMetadata,
) -> FileResult<Bytes> {
    let format = detected_output_format(data, metadata)?;
    metadata.mime_type = format.mime_type().to_string();
    metadata.extension = Some(format.extension().to_string());
    encode_image_format(img, format)
}

/// Rough byte estimate for the encoded form of `img` in `format`, used to seed
/// the encode buffer.
///
/// Starting from `Vec::new()` means ~23 reallocate-and-copy doubling steps to
/// reach an 8 MB output. The estimate only has to be the right order of
/// magnitude; it is clamped so a wrong guess never becomes a large speculative
/// allocation.
fn encoded_size_estimate(img: &DynamicImage, format: OutputFormat) -> usize {
    const MIN: usize = 8 * 1024;
    const MAX: usize = 16 * 1024 * 1024;

    let (width, height) = img.dimensions();
    let pixels = (width as usize).saturating_mul(height as usize);

    let estimate = match format {
        // Lossy: well under a byte per pixel at ordinary quality settings.
        OutputFormat::Jpeg { .. } => pixels / 2,
        // Uncompressed / weakly compressed containers.
        OutputFormat::Bmp | OutputFormat::Tiff => pixels.saturating_mul(3),
        // Lossless compressors: about a byte per pixel is a safe over-estimate
        // for photographic content and generous for flat content.
        _ => pixels,
    };

    estimate.clamp(MIN, MAX)
}

/// Encode an image to a specific format
fn encode_image_format(img: &DynamicImage, format: OutputFormat) -> FileResult<Bytes> {
    let mut buffer = Vec::with_capacity(encoded_size_estimate(img, format));

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
