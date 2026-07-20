//! Regression tests for Workflow 6 (armature-files) image/pipeline
//! conformance findings. Fixtures are generated in-memory so these tests
//! run anywhere without external files or containers.

#![cfg(feature = "images")]

use armature_files::image::ImageOp;
use armature_files::{MultiSizeBuilder, OutputFormat, Pipeline, Position};
use bytes::Bytes;
use image::{ImageEncoder, Rgb, RgbImage, Rgba, RgbaImage};
use std::io::Cursor;

/// Encode a solid-color RGBA image as PNG bytes.
fn solid_png(width: u32, height: u32, color: [u8; 4]) -> Bytes {
    let img = RgbaImage::from_pixel(width, height, Rgba(color));
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    Bytes::from(buf)
}

/// Encode a solid-color RGB image as JPEG bytes.
fn solid_jpeg(width: u32, height: u32, color: [u8; 3]) -> Bytes {
    let img = RgbImage::from_pixel(width, height, Rgb(color));
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
        .unwrap();
    Bytes::from(buf)
}

/// Build a minimal TIFF/Exif blob (as consumed by
/// `image::codecs::jpeg::JpegEncoder::set_exif_metadata`, i.e. *without* the
/// leading `"Exif\0\0"` marker, which the encoder adds itself) containing
/// only the Orientation tag (0x0112, SHORT).
fn minimal_exif_orientation(orientation: u16) -> Vec<u8> {
    let mut exif = Vec::new();
    exif.extend_from_slice(b"II"); // little-endian byte order
    exif.extend_from_slice(&42u16.to_le_bytes()); // TIFF magic
    exif.extend_from_slice(&8u32.to_le_bytes()); // offset of IFD0
    exif.extend_from_slice(&1u16.to_le_bytes()); // 1 IFD entry
    exif.extend_from_slice(&0x0112u16.to_le_bytes()); // tag: Orientation
    exif.extend_from_slice(&3u16.to_le_bytes()); // type: SHORT
    exif.extend_from_slice(&1u32.to_le_bytes()); // count: 1
    exif.extend_from_slice(&orientation.to_le_bytes());
    exif.extend_from_slice(&[0, 0]); // pad SHORT value to 4 bytes
    exif.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    exif
}

/// Encode a JPEG with a distinguishable gradient and an Exif orientation tag.
fn jpeg_with_orientation(width: u32, height: u32, orientation: u16) -> Bytes {
    let img = RgbImage::from_fn(width, height, |x, y| {
        Rgb([
            (x * 255 / width.max(1)) as u8,
            (y * 255 / height.max(1)) as u8,
            128,
        ])
    });

    let mut buf = Vec::new();
    {
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        encoder
            .set_exif_metadata(minimal_exif_orientation(orientation))
            .expect("exif metadata should be accepted");
        encoder
            .encode_image(&img)
            .expect("jpeg encode should succeed");
    }
    Bytes::from(buf)
}

/// Finding #1 / #10: `MultiSizeBuilder::generate()` with the default
/// (`Original`) output format must succeed and produce one output per size,
/// preserving the source format — instead of erroring on every call because
/// `Original` hit the catch-all `UnsupportedFormat` branch.
#[tokio::test]
async fn multi_size_builder_generates_all_sizes_preserving_format() {
    let data = solid_png(300, 200, [10, 20, 30, 255]);

    let results = MultiSizeBuilder::new(data, "photo.png")
        .with_thumbnails()
        .generate()
        .await
        .expect("generate() should succeed for the default Original format");

    assert_eq!(results.len(), 3);
    let names: Vec<&str> = results.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, vec!["thumb_small", "thumb_medium", "thumb_large"]);

    for (_, result) in &results {
        assert_eq!(result.metadata.extension.as_deref(), Some("png"));
        assert_eq!(result.metadata.mime_type, "image/png");
        // Re-decode to make sure the bytes are a valid, format-preserved image.
        let decoded = image::load_from_memory_with_format(&result.data, image::ImageFormat::Png)
            .expect("output bytes should decode as PNG");
        assert!(decoded.width() > 0 && decoded.height() > 0);
    }
}

/// Finding #1: `Pipeline::convert(OutputFormat::Original)` must round-trip
/// (not error), preserving the detected source format.
#[tokio::test]
async fn convert_to_original_round_trips() {
    let data = solid_jpeg(64, 48, [200, 100, 50]);

    let result = Pipeline::new()
        .load_bytes(data, "photo.jpg")
        .convert(OutputFormat::Original)
        .execute()
        .await
        .expect("Convert(Original) must be a format-preserving no-op, not an error");

    assert_eq!(result.metadata.extension.as_deref(), Some("jpg"));
    image::load_from_memory_with_format(&result.data, image::ImageFormat::Jpeg)
        .expect("output should still decode as JPEG");
}

/// Finding #3: converting a non-image input must error rather than silently
/// returning the original bytes mislabeled as converted.
#[tokio::test]
async fn convert_non_image_input_errors() {
    let data = Bytes::from_static(b"this is not an image, it's plain text data");

    let err = Pipeline::new()
        .load_bytes(data, "notes.txt")
        .convert(OutputFormat::Zip)
        .execute()
        .await
        .expect_err("converting a non-image input to Zip must fail, not pass through");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("unsupported") || msg.contains("cannot"),
        "unexpected error message: {msg}"
    );
}

/// Finding #4: `OutputFormat::WebP` no longer carries an ignored `quality`
/// field (the API was simplified to match the lossless-only implementation,
/// rather than silently ignoring a quality setting). This is primarily a
/// compile-time check — `OutputFormat::WebP { quality: .. }` would no longer
/// compile — but we also confirm conversion still produces valid WebP bytes.
#[tokio::test]
async fn webp_conversion_has_no_ignored_quality_param() {
    let data = solid_png(40, 40, [5, 6, 7, 255]);

    let result = Pipeline::new()
        .load_bytes(data, "photo.png")
        .convert(OutputFormat::WebP) // no `{ quality }` field anymore
        .execute()
        .await
        .expect("webp conversion should succeed");

    assert_eq!(result.metadata.extension.as_deref(), Some("webp"));
    image::load_from_memory_with_format(&result.data, image::ImageFormat::WebP)
        .expect("output should decode as WebP");
}

/// Finding #5: `ImageOp::AutoOrient` must actually read and apply the Exif
/// orientation tag, instead of being a documented-but-unimplemented no-op.
#[tokio::test]
async fn auto_orient_applies_exif_rotation() {
    // Exif orientation 6 = "rotate 90 CW to display": for a stored 6x4 image
    // this means the displayed result is rotated, swapping width and height.
    let data = jpeg_with_orientation(6, 4, 6);

    // Sanity check: a naive decode without honoring Exif keeps the stored 6x4.
    let naive = image::load_from_memory(&data).unwrap();
    assert_eq!((naive.width(), naive.height()), (6, 4));

    let result = Pipeline::new()
        .load_bytes(data, "photo.jpg")
        .image(ImageOp::AutoOrient)
        .execute()
        .await
        .expect("auto-orient should succeed");

    assert_eq!(
        (result.metadata.width, result.metadata.height),
        (Some(4), Some(6)),
        "AutoOrient should have swapped width/height per the Exif orientation tag"
    );
}

/// Finding #2: the text watermark must render actual glyph shapes, not a
/// striped/solid box. On a solid background, real anti-aliased glyph
/// rendering produces many distinct blended shades along glyph edges,
/// whereas the old striped-box implementation only ever produced two
/// colors (untouched background + one fully-blended stripe color).
#[tokio::test]
async fn text_watermark_renders_glyph_shapes_not_a_box() {
    let data = solid_png(200, 100, [255, 255, 255, 255]);

    let result = Pipeline::new()
        .load_bytes(data, "photo.png")
        .image(ImageOp::TextWatermark {
            text: "Hi".to_string(),
            position: Position::Center,
            font_size: 48.0,
            color: [0, 0, 0, 255],
        })
        .execute()
        .await
        .expect("watermark should succeed");

    let decoded = image::load_from_memory_with_format(&result.data, image::ImageFormat::Png)
        .expect("output should decode")
        .to_rgba8();

    let mut distinct_colors = std::collections::HashSet::new();
    for pixel in decoded.pixels() {
        distinct_colors.insert(pixel.0);
    }

    assert!(
        distinct_colors.len() > 4,
        "expected anti-aliased glyph rendering to produce a range of blended \
         shades, only found {} distinct colors (looks like a solid/striped box)",
        distinct_colors.len()
    );
}
