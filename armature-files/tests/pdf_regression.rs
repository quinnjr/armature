//! Regression tests for Workflow 6 (armature-files) PDF conformance
//! findings.

#![cfg(feature = "pdf")]

use armature_files::pdf::{FontSize, Margins, Orientation, PageSize, PdfBuilder, TextAlign};
use lopdf::Document;

/// Extract the decompressed content stream of the first page as a string.
fn first_page_content(pdf_bytes: &[u8]) -> String {
    let doc = Document::load_mem(pdf_bytes).expect("generated PDF should be loadable");
    let pages = doc.get_pages();
    let (_, page_id) = pages
        .iter()
        .next()
        .expect("document should have at least one page");
    let content = doc
        .get_page_content(*page_id)
        .expect("should be able to read/decompress page content");
    String::from_utf8_lossy(&content).into_owned()
}

/// Finding #6: `PdfBuilder::add_horizontal_line` must emit a real stroked
/// line in the content stream (moveto/lineto/stroke), not just advance the
/// cursor without drawing anything.
#[test]
fn horizontal_line_emits_a_real_stroke() {
    let result = PdfBuilder::new()
        .title("Line Test")
        .add_text("above the line")
        .add_horizontal_line()
        .add_text("below the line")
        .build()
        .expect("pdf build should succeed");

    let content = first_page_content(&result.data);

    assert!(
        content.split_whitespace().any(|tok| tok == "S"),
        "expected a stroke ('S') operator in the content stream, got:\n{content}"
    );
    assert!(
        content.split_whitespace().any(|tok| tok == "m"),
        "expected a moveto ('m') operator in the content stream, got:\n{content}"
    );
    assert!(
        content.split_whitespace().any(|tok| tok == "l"),
        "expected a lineto ('l') operator in the content stream, got:\n{content}"
    );
}

/// Extract the first non-negative `x` from an `"x y Td"` line — i.e. the
/// first absolute text placement (subsequent negated `Td` lines are the
/// origin-reset moves and are always <= 0 for our fixtures).
fn first_absolute_td_x(content: &str) -> f32 {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_suffix(" Td") {
            let mut parts = rest.split_whitespace();
            let x: f32 = parts
                .next()
                .expect("Td line should have an x component")
                .parse()
                .expect("x should be numeric");
            if x >= 0.0 {
                return x;
            }
        }
    }
    panic!("no absolute Td line found in content stream:\n{content}");
}

/// Finding #8: `TextAlign` must actually change where text is placed instead
/// of being an exported-but-unused enum (all text always left-aligned).
#[test]
fn text_align_changes_text_x_position() {
    let build = |align: TextAlign| {
        PdfBuilder::new()
            .add_text_aligned("hello", FontSize::Normal, align)
            .build()
            .expect("pdf build should succeed")
    };

    let left_x = first_absolute_td_x(&first_page_content(&build(TextAlign::Left).data));
    let center_x = first_absolute_td_x(&first_page_content(&build(TextAlign::Center).data));
    let right_x = first_absolute_td_x(&first_page_content(&build(TextAlign::Right).data));

    assert!(
        left_x < center_x,
        "center-aligned text (x={center_x}) should start further right than left-aligned text (x={left_x})"
    );
    assert!(
        center_x < right_x,
        "right-aligned text (x={right_x}) should start further right than center-aligned text (x={center_x})"
    );
}

/// Extract the first `y` from an `"x y Td"` line — the first absolute text
/// placement on the page.
fn first_absolute_td_y(content: &str) -> f32 {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_suffix(" Td") {
            let mut parts = rest.split_whitespace();
            let x: f32 = parts.next().unwrap().parse().unwrap();
            let y: f32 = parts.next().unwrap().parse().unwrap();
            if x >= 0.0 {
                return y;
            }
        }
    }
    panic!("no absolute Td line found in content stream:\n{content}");
}

/// Read the MediaBox of the first page as `[x0, y0, x1, y1]`.
fn first_page_media_box(pdf_bytes: &[u8]) -> [f32; 4] {
    let doc = Document::load_mem(pdf_bytes).expect("generated PDF should be loadable");
    let pages = doc.get_pages();
    let page_id = *pages.values().next().expect("at least one page");
    let dict = doc.get_dictionary(page_id).unwrap();
    let media_box = dict.get(b"MediaBox").unwrap().as_array().unwrap();
    let mut out = [0.0f32; 4];
    for (slot, obj) in out.iter_mut().zip(media_box) {
        *slot = obj.as_float().unwrap_or(obj.as_i64().unwrap_or(0) as f32);
    }
    out
}

/// Finding #10: `orientation()` must re-derive the text cursor.
///
/// `page_size()` and `margins()` both recompute `cursor_y` when they change
/// the layout; `orientation()` only set the field. On A4 that left the cursor
/// at 770 while the MediaBox height became 595, so every piece of content was
/// emitted 175 pt *above* the top of the page — a blank PDF. `ensure_space`
/// never noticed because it only compares against `margins.bottom`.
#[test]
fn landscape_content_stays_inside_the_media_box() {
    let result = PdfBuilder::new()
        .orientation(Orientation::Landscape)
        .add_text("first line on a landscape page")
        .build()
        .expect("pdf build should succeed");

    let media_box = first_page_media_box(&result.data);
    let page_height = media_box[3];
    assert_eq!(page_height, 595.0, "A4 landscape should be 842x595");

    let y = first_absolute_td_y(&first_page_content(&result.data));
    assert!(
        y > 0.0 && y <= page_height,
        "first text baseline y={y} falls outside the landscape MediaBox \
         (height {page_height}) — the cursor was left at the portrait height"
    );
}

/// The same must hold when orientation is set *before* the page size, and
/// after margins are changed.
#[test]
fn cursor_tracks_every_layout_setter() {
    let result = PdfBuilder::new()
        .orientation(Orientation::Landscape)
        .page_size(PageSize::Legal)
        .margins(Margins::uniform(36.0))
        .add_text("legal landscape with half-inch margins")
        .build()
        .expect("pdf build should succeed");

    let media_box = first_page_media_box(&result.data);
    assert_eq!([media_box[2], media_box[3]], [1008.0, 612.0]);

    let y = first_absolute_td_y(&first_page_content(&result.data));
    assert_eq!(
        y,
        612.0 - 36.0,
        "the cursor should sit one margin below the landscape page top"
    );
}

/// Finding #13: non-ASCII text must be transcoded to WinAnsiEncoding, and the
/// font dictionary must declare that encoding.
///
/// Emitting raw UTF-8 into a StandardEncoding string renders each byte of a
/// multi-byte sequence as its own wrong glyph — the crate's own `"© 2025"`
/// example came out as mojibake, with no error reported.
#[test]
fn latin1_text_is_winansi_encoded() {
    let result = PdfBuilder::new()
        .add_text("© 2025 Café")
        .build()
        .expect("Latin-1 text should build");

    let doc = Document::load_mem(&result.data).unwrap();
    let has_winansi = doc.objects.values().any(|obj| {
        obj.as_dict().is_ok_and(|dict| {
            dict.get(b"Encoding")
                .and_then(|e| e.as_name())
                .is_ok_and(|name| name == b"WinAnsiEncoding")
        })
    });
    assert!(
        has_winansi,
        "the font dictionary must declare /Encoding /WinAnsiEncoding"
    );

    let content = first_page_content(&result.data);
    // U+00A9 (©) is 0xA9 in WinAnsi = \251 octal. Its UTF-8 form (0xC2 0xA9)
    // would instead show up as \302\251.
    assert!(
        content.contains("\\251"),
        "expected the WinAnsi byte for '©', got:\n{content}"
    );
    assert!(
        !content.contains("\\302"),
        "raw UTF-8 leaked into the content stream:\n{content}"
    );
}

/// Characters outside WinAnsi must produce an error, not mojibake.
#[test]
fn unrepresentable_characters_error_rather_than_render_wrong() {
    let err = PdfBuilder::new()
        .add_text("こんにちは")
        .build()
        .expect_err("CJK cannot be encoded in WinAnsiEncoding and must not be faked");

    assert!(
        err.to_string().contains("WinAnsiEncoding"),
        "unexpected error: {err}"
    );
}

/// Finding #12: text metrics must count glyphs (with real Helvetica advance
/// widths), not UTF-8 bytes.
///
/// `"Café"` is 5 bytes but 4 glyphs, and CJK is 3 bytes per glyph. Measuring
/// bytes made right-aligned non-ASCII overestimate its own width until
/// `(content_width - text_width).max(0.0)` clamped to `0.0` — silently
/// rendering right-aligned headings as left-aligned.
#[test]
fn right_alignment_works_for_non_ascii_text() {
    let build = |text: &str, align: TextAlign| {
        PdfBuilder::new()
            .add_text_aligned(text, FontSize::Normal, align)
            .build()
            .expect("pdf build should succeed")
    };

    let left_x = first_absolute_td_x(&first_page_content(&build("Café", TextAlign::Left).data));
    let right_x = first_absolute_td_x(&first_page_content(&build("Café", TextAlign::Right).data));

    assert!(
        right_x > left_x,
        "right-aligned accented text (x={right_x}) collapsed back to the left \
         margin (x={left_x})"
    );

    // An accented string must measure the same as its unaccented equivalent
    // of the same glyph count and widths ('e' and 'é' share Helvetica's 556).
    let plain_x = first_absolute_td_x(&first_page_content(&build("Cafe", TextAlign::Right).data));
    assert!(
        (right_x - plain_x).abs() < 0.01,
        "'Café' (right_x={right_x}) and 'Cafe' (right_x={plain_x}) have the same \
         glyph widths and must align identically; a byte-length metric would \
         push the accented one left"
    );
}

/// The advance widths must be per-glyph, not a flat constant: a caps-heavy
/// string is meaningfully wider than an `i`/`l`-heavy one of the same length.
#[test]
fn text_width_uses_per_glyph_advance_widths() {
    let wide = PdfBuilder::estimate_text_width("WWWW", 12.0);
    let narrow = PdfBuilder::estimate_text_width("llll", 12.0);

    assert!(
        wide > narrow * 3.0,
        "'WWWW' ({wide}) should be far wider than 'llll' ({narrow}); a flat \
         0.6-em monospace constant would make them identical"
    );
}
