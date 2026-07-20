//! Regression tests for Workflow 6 (armature-files) PDF conformance
//! findings.

#![cfg(feature = "pdf")]

use armature_files::pdf::{FontSize, PdfBuilder, TextAlign};
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
