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
///
/// Stronger than "still decodes": `Original` means the bytes are *already* in
/// the requested format, so the pipeline must not touch the codec at all.
/// Decoding and re-encoding at the default quality silently recompressed
/// every `MultiSizeBuilder` run, compounding generation loss across runs.
#[tokio::test]
async fn convert_to_original_is_a_byte_identical_no_op() {
    let data = solid_jpeg(64, 48, [200, 100, 50]);

    let result = Pipeline::new()
        .load_bytes(data.clone(), "photo.jpg")
        .convert(OutputFormat::Original)
        .execute()
        .await
        .expect("Convert(Original) must be a format-preserving no-op, not an error");

    assert_eq!(result.metadata.extension.as_deref(), Some("jpg"));
    assert_eq!(
        result.data, data,
        "Convert(Original) must return the input bytes untouched, not recompress them"
    );
}

/// Finding #14: chaining N image operations must cost one decode and one
/// encode, not N of each. Byte-for-byte, `.grayscale().rotate(180)` through
/// the pipeline must equal the same two operations applied to a single decode
/// — which is only true if no intermediate re-encode happened.
#[tokio::test]
async fn chained_operations_encode_only_once() {
    let data = solid_jpeg(64, 48, [200, 100, 50]);

    let chained = Pipeline::new()
        .load_bytes(data.clone(), "photo.jpg")
        .image(ImageOp::Grayscale)
        .image(ImageOp::Rotate { degrees: 180.0 })
        .execute()
        .await
        .expect("chained operations should succeed");

    // Reference: one decode, both operations, one encode.
    let decoded = image::load_from_memory(&data).unwrap();
    let expected = decoded.grayscale().rotate180();
    let mut buf = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
    expected.write_with_encoder(encoder).unwrap();

    assert_eq!(
        &chained.data[..],
        &buf[..],
        "the pipeline should decode once and encode once; a byte mismatch means \
         an intermediate re-encode (and, for JPEG, a lost generation) crept back in"
    );
}

/// Finding #6: an operation label must not embed the operation's payload.
/// `format!("{:?}", op)` on an `ImageWatermark` dumps the whole overlay
/// buffer — `Bytes`'s `Debug` escapes every single byte — so a 2 MB overlay
/// allocated a multi-megabyte `String` just to name an operation.
#[tokio::test]
async fn operation_labels_do_not_embed_payloads() {
    let overlay = solid_png(8, 8, [255, 0, 0, 128]);
    let base = solid_png(64, 64, [255, 255, 255, 255]);

    let result = Pipeline::new()
        .load_bytes(base, "photo.png")
        .image(ImageOp::ImageWatermark {
            overlay: overlay.clone(),
            position: Position::Center,
            opacity: 0.5,
        })
        .execute()
        .await
        .expect("image watermark should succeed");

    assert_eq!(result.operations, vec!["image:image_watermark".to_string()]);
    for label in &result.operations {
        assert!(
            label.len() < 64,
            "operation label should be a short stable name, got {} bytes",
            label.len()
        );
    }
}

/// Finding #4: a header declaring absurd dimensions must be refused with a
/// clean error. `65535x65535` RGBA is a ~17 GB allocation; without decoder
/// limits a 40-byte file OOM-kills the process handling uploads.
#[tokio::test]
async fn oversized_image_header_is_rejected_not_allocated() {
    let err = Pipeline::new()
        .load_bytes(huge_png_header(), "huge.png")
        .resize(16, 16)
        .execute()
        .await
        .expect_err("a 65535x65535 declaration must not be decoded");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("limit"),
        "expected a decode-limit error, got: {err}"
    );
}

/// The limits are configurable, and a normal image is unaffected by them.
#[tokio::test]
async fn decode_limits_are_configurable() {
    use armature_files::image::DecodeLimits;

    let data = solid_png(64, 64, [1, 2, 3, 255]);

    let err = Pipeline::new()
        .load_bytes(data.clone(), "photo.png")
        .decode_limits(DecodeLimits {
            max_width: 32,
            max_height: 32,
            ..DecodeLimits::default()
        })
        .resize(16, 16)
        .execute()
        .await
        .expect_err("a 64x64 image must not pass a 32x32 dimension cap");
    assert!(err.to_string().to_lowercase().contains("limit"));

    Pipeline::new()
        .load_bytes(data, "photo.png")
        .resize(16, 16)
        .execute()
        .await
        .expect("the default limits must not reject an ordinary image");
}

/// Finding #3: watermark text wider than the image must not underflow the
/// position arithmetic. At the documented `font_size: 48.0` the rendered text
/// is ~430 px wide — far wider than an ordinary thumbnail — which used to
/// panic in debug builds and silently drop the watermark in release.
/// Relies on the embedded fallback font; see `custom_font_is_honored`
/// for the `embedded-font`-disabled path.
#[cfg(feature = "embedded-font")]
#[tokio::test]
async fn watermark_wider_than_the_image_does_not_underflow() {
    let data = solid_png(80, 40, [255, 255, 255, 255]);

    let result = Pipeline::new()
        .load_bytes(data, "thumb.png")
        .image(ImageOp::TextWatermark {
            text: "© 2025 Acme Corp".to_string(),
            position: Position::BottomRight,
            font_size: 48.0,
            color: [0, 0, 0, 255],
            font: None,
        })
        .execute()
        .await
        .expect("watermarking a thumbnail narrower than the text must not panic");

    let decoded = image::load_from_memory(&result.data).unwrap().to_rgba8();
    assert!(
        decoded.pixels().any(|p| p.0 != [255, 255, 255, 255]),
        "the watermark should still have been drawn, clamped to the image"
    );
}

/// Finding #7: `color`'s alpha is the *global opacity* of the watermark and
/// must be composited into RGB, not written into the alpha channel.
///
/// JPEG has no alpha channel, so a watermark that stored its opacity there
/// came out fully opaque — a 25%-opacity black mark rendered as solid black.
/// Relies on the embedded fallback font; see `custom_font_is_honored`
/// for the `embedded-font`-disabled path.
#[cfg(feature = "embedded-font")]
#[tokio::test]
async fn semi_transparent_watermark_stays_translucent_on_jpeg() {
    let data = solid_jpeg(200, 100, [255, 255, 255]);

    let result = Pipeline::new()
        .load_bytes(data, "photo.jpg")
        .image(ImageOp::TextWatermark {
            text: "Hi".to_string(),
            position: Position::Center,
            font_size: 48.0,
            color: [0, 0, 0, 64], // 25% opacity black
            font: None,
        })
        .execute()
        .await
        .expect("watermark should succeed");

    let decoded = image::load_from_memory_with_format(&result.data, image::ImageFormat::Jpeg)
        .expect("output should decode as JPEG")
        .to_rgb8();

    let darkest = decoded.pixels().map(|p| p.0[0]).min().unwrap();

    assert!(
        darkest > 60,
        "a 25%-opacity black watermark on white should leave the background \
         partially visible (~191), but the darkest pixel is {darkest} — the \
         opacity was written into the alpha channel and dropped by JPEG"
    );
    assert!(
        darkest < 250,
        "the watermark should actually be visible, darkest pixel is {darkest}"
    );
}

/// Finding #11: only *exact* quarter turns may take the lossless fast path.
/// `normalized as u32` truncates, so 90.5 deg silently rendered as a plain
/// 90 deg rotation and the fraction was discarded.
#[tokio::test]
async fn rotation_fast_path_requires_an_exact_quarter_turn() {
    let data = solid_png(60, 40, [10, 20, 30, 255]);

    let exact = Pipeline::new()
        .load_bytes(data.clone(), "photo.png")
        .rotate(90.0)
        .execute()
        .await
        .expect("90 deg rotation should succeed");

    let fractional = Pipeline::new()
        .load_bytes(data, "photo.png")
        .rotate(90.5)
        .execute()
        .await
        .expect("90.5 deg rotation should succeed");

    // The exact quarter turn swaps the axes; the arbitrary-angle rasterizer
    // rotates within the original canvas.
    assert_eq!(
        (exact.metadata.width, exact.metadata.height),
        (Some(40), Some(60)),
        "90 deg should take the lossless rotate90 fast path and swap the axes"
    );
    assert_eq!(
        (fractional.metadata.width, fractional.metadata.height),
        (Some(60), Some(40)),
        "90.5 deg must fall through to the arbitrary-angle path, not be \
         truncated to an exact 90 deg rotation"
    );
}

/// Finding #9: `process_image` must not return bytes in one format while
/// reporting another. `image` decodes formats this crate cannot re-encode, and
/// the old fallback quietly produced PNG bytes while `mime_type` still claimed
/// the source type — so a downstream `Storage::put` served a PNG under a
/// `Content-Type` browsers refuse to render.
#[tokio::test]
async fn metadata_describes_the_format_actually_encoded() {
    // PNG bytes carrying a misleading `.avif` filename: content sniffing wins,
    // and the reported MIME type must be corrected to match the real output.
    let data = solid_png(32, 32, [4, 5, 6, 255]);

    let result = Pipeline::new()
        .load_bytes(data, "mislabeled.avif")
        .image(ImageOp::Grayscale)
        .execute()
        .await
        .expect("processing should succeed");

    assert_eq!(result.metadata.mime_type, "image/png");
    assert_eq!(result.metadata.extension.as_deref(), Some("png"));
    image::load_from_memory_with_format(&result.data, image::ImageFormat::Png)
        .expect("the bytes really are PNG, as the metadata now says");
}

/// ...and when neither content sniffing nor the extension yields a format this
/// crate can encode, that is an error rather than a silent rewrite to PNG.
///
/// QOI stands in for the whole class (`image` also decodes AVIF, PNM, DDS,
/// HDR, Farbfeld and TGA, none of which `image_format_to_output` maps). The
/// call must fail — either at decode, because the codec is not compiled in, or
/// at encode, because there is no encoder — but it must never emit PNG bytes
/// labeled `image/qoi`.
#[tokio::test]
async fn unencodable_source_format_errors_instead_of_lying() {
    let qoi = qoi_fixture();
    let mut metadata = armature_files::FileMetadata::from_bytes(&qoi, "mystery.qoi");
    metadata.mime_type = "image/qoi".to_string();

    let err = armature_files::image::process_image(&qoi, &ImageOp::Grayscale, &mut metadata)
        .expect_err("a format with no encoder must not be silently rewritten to PNG");

    assert!(
        err.to_string().to_lowercase().contains("unsupported")
            || err.to_string().to_lowercase().contains("decode"),
        "unexpected error: {err}"
    );
}

/// Finding #18: `StripMetadata` claims EXIF is gone; pin that claim.
#[tokio::test]
async fn strip_metadata_emits_no_exif() {
    let data = jpeg_with_orientation(8, 6, 6);
    assert!(
        contains_exif_marker(&data),
        "fixture should actually carry an Exif segment"
    );

    let result = Pipeline::new()
        .load_bytes(data, "photo.jpg")
        .image(ImageOp::StripMetadata)
        .execute()
        .await
        .expect("strip metadata should succeed");

    assert!(
        !contains_exif_marker(&result.data),
        "output still carries an Exif segment"
    );
}

/// True if the JPEG stream contains an APP1/Exif segment.
fn contains_exif_marker(data: &[u8]) -> bool {
    data.windows(6).any(|w| w == b"Exif\0\0")
}

/// A PNG signature plus an IHDR declaring 65535x65535 RGBA — around 17 GB
/// once allocated, from 45 bytes on the wire.
fn huge_png_header() -> Bytes {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    let mut ihdr = Vec::from(*b"IHDR");
    ihdr.extend_from_slice(&65535u32.to_be_bytes()); // width
    ihdr.extend_from_slice(&65535u32.to_be_bytes()); // height
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace

    let mut png = Vec::from([0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&crc32(&ihdr).to_be_bytes());
    Bytes::from(png)
}

/// A minimal 1x1 QOI image — a format `image` decodes but this crate has no
/// encoder for.
fn qoi_fixture() -> Bytes {
    let mut data = Vec::from(*b"qoif");
    data.extend_from_slice(&1u32.to_be_bytes()); // width
    data.extend_from_slice(&1u32.to_be_bytes()); // height
    data.push(4); // channels: RGBA
    data.push(0); // colorspace: sRGB
    data.extend_from_slice(&[0xFE, 0x10, 0x20, 0x30]); // QOI_OP_RGB
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]); // end marker
    Bytes::from(data)
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
/// Relies on the embedded fallback font; see `custom_font_is_honored`
/// for the `embedded-font`-disabled path.
#[cfg(feature = "embedded-font")]
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
            font: None,
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

/// Finding #19: the vendored font must be substitutable.
///
/// `embedded-font` is default-on, but a downstream binary that does not want
/// 760 KB of DejaVu Sans compiled in must be able to turn it off and supply
/// its own font — via `Pipeline::with_font` or per-operation. This test runs
/// in both configurations because it never relies on the embedded fallback.
#[tokio::test]
async fn custom_font_is_honored() {
    let font = Bytes::from(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/DejaVuSans.ttf"
        ))
        .expect("the vendored font should be readable from the manifest dir"),
    );

    // Per-operation font.
    let per_op = Pipeline::new()
        .load_bytes(solid_png(200, 100, [255, 255, 255, 255]), "photo.png")
        .image(ImageOp::TextWatermark {
            text: "Hi".to_string(),
            position: Position::Center,
            font_size: 48.0,
            color: [0, 0, 0, 255],
            font: Some(font.clone()),
        })
        .execute()
        .await
        .expect("an explicitly supplied font should render without the embedded one");

    // Pipeline-level default font, filled in for an op that left `font: None`.
    let pipeline_level = Pipeline::new()
        .load_bytes(solid_png(200, 100, [255, 255, 255, 255]), "photo.png")
        .with_font(font)
        .image(ImageOp::TextWatermark {
            text: "Hi".to_string(),
            position: Position::Center,
            font_size: 48.0,
            color: [0, 0, 0, 255],
            font: None,
        })
        .execute()
        .await
        .expect("Pipeline::with_font should supply the font for ops that omit one");

    assert_eq!(
        per_op.data, pipeline_level.data,
        "the pipeline-level font should produce the same render as passing it per-operation"
    );

    let decoded = image::load_from_memory(&per_op.data).unwrap().to_rgba8();
    assert!(
        decoded.pixels().any(|p| p.0 != [255, 255, 255, 255]),
        "glyphs should actually have been rendered with the supplied font"
    );
}
