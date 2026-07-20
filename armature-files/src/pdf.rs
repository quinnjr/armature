//! PDF generation and manipulation
//!
//! Provides a fluent API for creating and modifying PDF documents.
//!
//! # Example
//!
//! ```rust
//! # fn example() -> Result<(), armature_files::FileError> {
//! use armature_files::pdf::{FontSize, PdfBuilder};
//!
//! let pdf = PdfBuilder::new()
//!     .title("Invoice #12345")
//!     .add_heading("Invoice", FontSize::H1)
//!     .add_text("Thank you for your purchase!")
//!     .build()?;
//!
//! assert!(pdf.data.starts_with(b"%PDF"));
//! # Ok(())
//! # }
//! # example().unwrap();
//! ```

use crate::{FileMetadata, FileResult, ProcessingResult};
use bytes::Bytes;
use lopdf::dictionary;
use lopdf::{Document, Object, Stream, StringFormat};

/// WinAnsiEncoding (code page 1252) transcoding and Base-14 Helvetica glyph
/// metrics.
///
/// The generated PDFs use the built-in Helvetica Type1 font with an explicit
/// `/Encoding /WinAnsiEncoding`. Emitting raw UTF-8 bytes into such a string
/// renders each byte of a multi-byte sequence as a separate wrong glyph — the
/// crate's own `"© 2025"` example came out as two pieces of mojibake with no
/// error reported anywhere.
mod winansi {
    /// Unicode scalars that WinAnsi places in the 0x80..=0x9F range, which is
    /// where it diverges from Latin-1.
    const HIGH_CONTROL_RANGE: [(u8, char); 27] = [
        (0x80, '\u{20AC}'), // Euro
        (0x82, '\u{201A}'), // quotesinglbase
        (0x83, '\u{0192}'), // florin
        (0x84, '\u{201E}'), // quotedblbase
        (0x85, '\u{2026}'), // ellipsis
        (0x86, '\u{2020}'), // dagger
        (0x87, '\u{2021}'), // daggerdbl
        (0x88, '\u{02C6}'), // circumflex
        (0x89, '\u{2030}'), // perthousand
        (0x8A, '\u{0160}'), // Scaron
        (0x8B, '\u{2039}'), // guilsinglleft
        (0x8C, '\u{0152}'), // OE
        (0x8E, '\u{017D}'), // Zcaron
        (0x91, '\u{2018}'), // quoteleft
        (0x92, '\u{2019}'), // quoteright
        (0x93, '\u{201C}'), // quotedblleft
        (0x94, '\u{201D}'), // quotedblright
        (0x95, '\u{2022}'), // bullet
        (0x96, '\u{2013}'), // endash
        (0x97, '\u{2014}'), // emdash
        (0x98, '\u{02DC}'), // tilde
        (0x99, '\u{2122}'), // trademark
        (0x9A, '\u{0161}'), // scaron
        (0x9B, '\u{203A}'), // guilsinglright
        (0x9C, '\u{0153}'), // oe
        (0x9E, '\u{017E}'), // zcaron
        (0x9F, '\u{0178}'), // Ydieresis
    ];

    /// Map a Unicode scalar to its WinAnsiEncoding byte, if representable.
    pub(super) fn encode_char(ch: char) -> Option<u8> {
        match ch {
            // ASCII printable range is identical.
            ' '..='~' => Some(ch as u8),
            // Latin-1 supplement is identical above 0x9F.
            '\u{00A0}'..='\u{00FF}' => Some(ch as u32 as u8),
            _ => HIGH_CONTROL_RANGE
                .iter()
                .find(|(_, mapped)| *mapped == ch)
                .map(|(byte, _)| *byte),
        }
    }

    /// Transcode a whole string, reporting the first unrepresentable char.
    pub(super) fn encode_str(text: &str) -> Result<Vec<u8>, char> {
        text.chars()
            .map(|ch| encode_char(ch).ok_or(ch))
            .collect::<Result<Vec<u8>, char>>()
    }

    /// Helvetica advance widths in 1/1000 em, indexed by WinAnsi code point.
    /// Taken from the Adobe Core 14 AFM for Helvetica. Zero means "no glyph
    /// at this code point".
    #[rustfmt::skip]
    pub(super) const HELVETICA_WIDTHS: [u16; 256] = [
        // 0x00-0x1F: control codes, no glyphs
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        // 0x20-0x2F: space ! " # $ % & ' ( ) * + , - . /
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
        // 0x30-0x3F: 0-9 : ; < = > ?
        556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
        // 0x40-0x4F: @ A-O
        1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
        // 0x50-0x5F: P-Z [ \ ] ^ _
        667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
        // 0x60-0x6F: ` a-o
        333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
        // 0x70-0x7F: p-z { | } ~ DEL
        556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, 0,
        // 0x80-0x8F
        556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0,
        // 0x90-0x9F
        0, 222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667,
        // 0xA0-0xAF
        278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333,
        // 0xB0-0xBF
        400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611,
        // 0xC0-0xCF
        667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
        // 0xD0-0xDF
        722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
        // 0xE0-0xEF
        556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
        // 0xF0-0xFF
        556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
    ];

    /// Advance width of `ch` in 1/1000 em, falling back to the width of `n`
    /// for characters Helvetica/WinAnsi cannot represent.
    pub(super) fn char_width(ch: char) -> f32 {
        const FALLBACK: u16 = 556; // width of 'n'
        match encode_char(ch) {
            Some(byte) => {
                let width = HELVETICA_WIDTHS[byte as usize];
                (if width == 0 { FALLBACK } else { width }) as f32
            }
            None => FALLBACK as f32,
        }
    }
}

/// Encode a document-metadata string as a PDF *text string*.
///
/// ASCII passes through as-is; anything else is emitted as UTF-16BE with the
/// `FE FF` byte-order mark, which every conforming reader understands. Raw
/// UTF-8 in this position is not valid PDF and renders as mojibake.
fn pdf_text_string(text: &str) -> Vec<u8> {
    if text.is_ascii() {
        return text.as_bytes().to_vec();
    }

    let mut out = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// Font sizes for PDF text
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FontSize {
    /// Heading 1 (24pt)
    H1,
    /// Heading 2 (20pt)
    H2,
    /// Heading 3 (16pt)
    H3,
    /// Normal text (12pt)
    #[default]
    Normal,
    /// Small text (10pt)
    Small,
    /// Footnote (8pt)
    Footnote,
    /// Custom size in points
    Custom(f32),
}

impl FontSize {
    /// Get the size in points
    pub fn points(&self) -> f32 {
        match self {
            Self::H1 => 24.0,
            Self::H2 => 20.0,
            Self::H3 => 16.0,
            Self::Normal => 12.0,
            Self::Small => 10.0,
            Self::Footnote => 8.0,
            Self::Custom(size) => *size,
        }
    }

    /// Get line height multiplier
    pub fn line_height(&self) -> f32 {
        self.points() * 1.5
    }
}

/// Text alignment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Page size presets
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PageSize {
    /// A4 (210 × 297 mm = 595 × 842 points)
    #[default]
    A4,
    /// Letter (8.5 × 11 in = 612 × 792 points)
    Letter,
    /// Legal (8.5 × 14 in = 612 × 1008 points)
    Legal,
    /// Custom size in points
    Custom { width: f32, height: f32 },
}

impl PageSize {
    /// Get dimensions in points (1 point = 1/72 inch)
    pub fn dimensions(&self) -> (f32, f32) {
        match self {
            Self::A4 => (595.0, 842.0),
            Self::Letter => (612.0, 792.0),
            Self::Legal => (612.0, 1008.0),
            Self::Custom { width, height } => (*width, *height),
        }
    }
}

/// Page orientation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// Margin settings in points
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Margins {
    /// Create uniform margins (72 points = 1 inch)
    pub fn uniform(margin: f32) -> Self {
        Self {
            top: margin,
            right: margin,
            bottom: margin,
            left: margin,
        }
    }

    /// Create from inches
    pub fn inches(value: f32) -> Self {
        Self::uniform(value * 72.0)
    }
}

impl Default for Margins {
    fn default() -> Self {
        Self::inches(1.0) // 1 inch margins
    }
}

/// Text operation to add to a page
#[derive(Debug, Clone)]
struct TextOp {
    text: String,
    x: f32,
    y: f32,
    font_size: f32,
}

/// A straight line (used for e.g. horizontal rules) to draw on a page.
#[derive(Debug, Clone, Copy)]
struct LineOp {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

/// Page content
#[derive(Debug, Clone, Default)]
struct PageContent {
    texts: Vec<TextOp>,
    lines: Vec<LineOp>,
}

/// PDF document builder.
///
/// # Character set
///
/// Text is rendered with the built-in Helvetica Type1 font declared with
/// `/Encoding /WinAnsiEncoding`, so the supported repertoire is Latin-1 plus
/// the CP1252 punctuation block (curly quotes, dashes, `€`, `™`, …). Anything
/// outside it — CJK, Cyrillic, Greek, emoji — makes [`PdfBuilder::build`]
/// return [`crate::FileError::Pdf`] rather than emit mojibake. Supporting
/// those requires embedding a TrueType font with an Identity-H CMap.
pub struct PdfBuilder {
    pages: Vec<PageContent>,
    page_size: PageSize,
    orientation: Orientation,
    margins: Margins,
    cursor_y: f32,
    title: Option<String>,
    author: Option<String>,
}

impl PdfBuilder {
    /// Create a new PDF builder
    pub fn new() -> Self {
        let page_size = PageSize::default();
        let margins = Margins::default();
        let (_, height) = page_size.dimensions();

        Self {
            pages: vec![PageContent::default()],
            page_size,
            orientation: Orientation::Portrait,
            margins,
            cursor_y: height - margins.top,
            title: None,
            author: None,
        }
    }

    /// Set document title
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set document author
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Reset the text cursor to the top of the current page layout.
    ///
    /// Must be called by every setter that changes the page geometry. Note it
    /// uses the orientation-aware [`Self::get_page_dimensions`], not
    /// `PageSize::dimensions` — otherwise a landscape page would place the
    /// cursor at the *portrait* height, i.e. above the top of the MediaBox,
    /// and every subsequent `ensure_space` check (which only ever compares
    /// against `margins.bottom`) would fail to notice.
    fn reset_cursor(&mut self) {
        let (_, height) = self.get_page_dimensions();
        self.cursor_y = height - self.margins.top;
    }

    /// Set page size
    pub fn page_size(mut self, size: PageSize) -> Self {
        self.page_size = size;
        self.reset_cursor();
        self
    }

    /// Set page orientation
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self.reset_cursor();
        self
    }

    /// Set margins
    pub fn margins(mut self, margins: Margins) -> Self {
        self.margins = margins;
        self.reset_cursor();
        self
    }

    /// Get page dimensions accounting for orientation
    fn get_page_dimensions(&self) -> (f32, f32) {
        let (w, h) = self.page_size.dimensions();
        match self.orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        }
    }

    /// Get content area width
    fn content_width(&self) -> f32 {
        let (width, _) = self.get_page_dimensions();
        width - self.margins.left - self.margins.right
    }

    /// Check if we need a new page
    fn ensure_space(&mut self, needed_height: f32) {
        if self.cursor_y - needed_height < self.margins.bottom {
            self.add_page_internal();
        }
    }

    /// The page drawing operations are currently appended to.
    ///
    /// `pages` is seeded with one entry in [`Self::new`] (which `Default`
    /// delegates to) and is only ever pushed to — there is no `pop`, `clear`,
    /// `drain`, `truncate`, `retain` or reassignment anywhere in this module —
    /// so the vector is non-empty by construction. The `if let Some(page) =
    /// self.pages.last_mut()` this replaces was therefore unreachable in its
    /// `None` arm, and that arm silently discarded the drawing operation.
    fn current_page(&mut self) -> &mut PageContent {
        self.pages
            .last_mut()
            .expect("pages is seeded in new() and only ever pushed to")
    }

    /// Add a new page internally
    fn add_page_internal(&mut self) {
        self.pages.push(PageContent::default());
        self.reset_cursor();
    }

    /// Add a new page
    pub fn add_page(mut self) -> Self {
        self.add_page_internal();
        self
    }

    /// Add a page break
    pub fn add_page_break(self) -> Self {
        self.add_page()
    }

    /// Rendered width of `text` in points, using the Base-14 Helvetica AFM
    /// advance widths.
    ///
    /// Deliberately not `text.len()`: that counts UTF-8 *bytes*, so `"Café"`
    /// measured 5 instead of 4 glyphs and CJK measured 3x its glyph count —
    /// wrapping accented Latin ~2x too early and pushing right-aligned
    /// non-ASCII text back to the margin. The old flat `0.6` monospace
    /// constant was also off by up to ~30% for caps-heavy or `i`/`l`-heavy
    /// strings.
    pub fn estimate_text_width(text: &str, font_size: f32) -> f32 {
        let thousandths: f32 = text.chars().map(winansi::char_width).sum();
        thousandths / 1000.0 * font_size
    }

    /// Compute the x position for a line of `text` at `font_size` given
    /// `align`, clamped to not go left of the margin.
    fn aligned_x(&self, text: &str, font_size: f32, align: TextAlign) -> f32 {
        self.aligned_x_for_width(Self::estimate_text_width(text, font_size), align)
    }

    /// [`Self::aligned_x`] for a line whose width the caller already knows.
    ///
    /// The word-wrapper tracks a running width as it appends words, so it would
    /// otherwise measure every line a second time purely to place it.
    fn aligned_x_for_width(&self, text_width: f32, align: TextAlign) -> f32 {
        let content_width = self.content_width();
        match align {
            TextAlign::Left => self.margins.left,
            TextAlign::Center => self.margins.left + ((content_width - text_width) / 2.0).max(0.0),
            TextAlign::Right => self.margins.left + (content_width - text_width).max(0.0),
        }
    }

    /// Add a heading
    pub fn add_heading(self, text: &str, size: FontSize) -> Self {
        self.add_heading_aligned(text, size, TextAlign::Left)
    }

    /// Add a heading with explicit text alignment
    pub fn add_heading_aligned(mut self, text: &str, size: FontSize, align: TextAlign) -> Self {
        let line_height = size.line_height();
        self.ensure_space(line_height);

        // Both `cursor_y` and `aligned_x` read `self` immutably, so they must
        // be evaluated *before* `current_page` takes the mutable borrow.
        let x = self.aligned_x(text, size.points(), align);
        let y = self.cursor_y;
        let font_size = size.points();
        self.current_page().texts.push(TextOp {
            text: text.to_string(),
            x,
            y,
            font_size,
        });

        self.cursor_y -= line_height;
        self
    }

    /// Add text paragraph
    pub fn add_text(self, text: &str) -> Self {
        self.add_text_styled(text, FontSize::Normal)
    }

    /// Add text with custom font size
    pub fn add_text_styled(self, text: &str, size: FontSize) -> Self {
        self.add_text_aligned(text, size, TextAlign::Left)
    }

    /// Add text with custom font size and explicit alignment.
    ///
    /// Wrapping is measured in points via the real Helvetica advance widths,
    /// not by comparing a UTF-8 byte length against a character count.
    ///
    /// The accumulated line and its width are both carried forward as words are
    /// appended. Building `format!("{current_line} {word}")` per word and
    /// re-measuring the whole prefix — then measuring it a *third* time in
    /// `aligned_x` — made a single paragraph O(n^2) in its word count, since
    /// advance widths are additive and the prefix never changes.
    pub fn add_text_aligned(mut self, text: &str, size: FontSize, align: TextAlign) -> Self {
        let line_height = size.line_height();
        let font_size = size.points();
        let content_width = self.content_width();
        // Advance width of the separator added ahead of each appended word.
        let space_width = Self::estimate_text_width(" ", font_size);

        for line in text.lines() {
            let mut current_line = String::new();
            let mut current_width = 0.0f32;

            for word in line.split_whitespace() {
                let word_width = Self::estimate_text_width(word, font_size);

                if current_line.is_empty() {
                    current_line.push_str(word);
                    current_width = word_width;
                    continue;
                }

                let candidate_width = current_width + space_width + word_width;
                if candidate_width > content_width {
                    self.ensure_space(line_height);
                    let x = self.aligned_x_for_width(current_width, align);
                    let y = self.cursor_y;
                    self.current_page().texts.push(TextOp {
                        text: std::mem::take(&mut current_line),
                        x,
                        y,
                        font_size,
                    });
                    self.cursor_y -= line_height;

                    current_line.push_str(word);
                    current_width = word_width;
                } else {
                    current_line.push(' ');
                    current_line.push_str(word);
                    current_width = candidate_width;
                }
            }

            // Write remaining text
            if !current_line.is_empty() {
                self.ensure_space(line_height);
                let x = self.aligned_x_for_width(current_width, align);
                let y = self.cursor_y;
                self.current_page().texts.push(TextOp {
                    text: current_line,
                    x,
                    y,
                    font_size,
                });
                self.cursor_y -= line_height;
            }
        }

        self
    }

    /// Add vertical space
    pub fn add_space(mut self, points: f32) -> Self {
        self.cursor_y -= points;
        self
    }

    /// Add a horizontal line (a real stroked line across the content width)
    pub fn add_horizontal_line(mut self) -> Self {
        self.ensure_space(10.0);

        let y = self.cursor_y;
        let x1 = self.margins.left;
        let x2 = x1 + self.content_width();
        self.current_page().lines.push(LineOp {
            x1,
            y1: y,
            x2,
            y2: y,
        });

        self.cursor_y -= 10.0;
        self
    }

    /// Add a simple table
    pub fn add_table(mut self, rows: &[Vec<&str>]) -> Self {
        if rows.is_empty() {
            return self;
        }

        let num_cols = rows[0].len();
        if num_cols == 0 {
            return self;
        }

        let col_width = self.content_width() / num_cols as f32;
        let row_height = FontSize::Normal.line_height();

        for row in rows {
            self.ensure_space(row_height);

            for (col_idx, cell) in row.iter().enumerate() {
                let x = self.margins.left + (col_idx as f32 * col_width) + 5.0;
                let y = self.cursor_y;
                self.current_page().texts.push(TextOp {
                    text: cell.to_string(),
                    x,
                    y,
                    font_size: FontSize::Normal.points(),
                });
            }
            self.cursor_y -= row_height;
        }

        self
    }

    /// Build the final PDF using lopdf
    pub fn build(self) -> FileResult<ProcessingResult> {
        let start = std::time::Instant::now();

        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.new_object_id();
        let mut page_ids = Vec::new();
        let (page_width, page_height) = self.get_page_dimensions();

        // Add font (Helvetica is a built-in font).
        //
        // `/Encoding /WinAnsiEncoding` is required: without it the font falls
        // back to StandardEncoding, where the bytes we emit for anything above
        // ASCII (including `©`) map to different glyphs entirely.
        doc.objects.insert(
            font_id,
            Object::Dictionary(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
                "Encoding" => "WinAnsiEncoding",
            }),
        );

        // Create pages
        for page_content in &self.pages {
            let content_id = doc.new_object_id();
            let page_id = doc.new_object_id();

            // Build content stream. This is a byte buffer, not a `String`:
            // the text strings are WinAnsi-encoded and are not valid UTF-8.
            let mut content: Vec<u8> = Vec::new();

            // Path-painting operators (e.g. horizontal rules) must live
            // outside the BT/ET text object.
            for line_op in &page_content.lines {
                content.extend_from_slice(b"1 w\n"); // 1pt line width
                content.extend_from_slice(format!("{} {} m\n", line_op.x1, line_op.y1).as_bytes());
                content.extend_from_slice(format!("{} {} l\n", line_op.x2, line_op.y2).as_bytes());
                content.extend_from_slice(b"S\n"); // stroke
            }

            content.extend_from_slice(b"BT\n"); // Begin text
            content.extend_from_slice(b"/F1 12 Tf\n"); // Default font

            for text_op in &page_content.texts {
                // Transcode to WinAnsiEncoding, which is what the font
                // dictionary declares. Unrepresentable characters are a hard
                // error — silently emitting mojibake is worse than failing.
                let encoded = winansi::encode_str(&text_op.text).map_err(|ch| {
                    crate::FileError::Pdf(format!(
                        "character {ch:?} (U+{:04X}) cannot be encoded in WinAnsiEncoding; \
                         the built-in Helvetica font used by PdfBuilder is limited to \
                         Latin-1 plus the CP1252 punctuation set",
                        ch as u32
                    ))
                })?;

                // Set font size
                content.extend_from_slice(format!("/F1 {} Tf\n", text_op.font_size).as_bytes());
                // Move to position
                content.extend_from_slice(format!("{} {} Td\n", text_op.x, text_op.y).as_bytes());
                // Show text (escape the literal-string metacharacters)
                content.push(b'(');
                for byte in encoded {
                    match byte {
                        b'\\' | b'(' | b')' => {
                            content.push(b'\\');
                            content.push(byte);
                        }
                        0x20..=0x7E => content.push(byte),
                        // Non-ASCII bytes are legal raw in a literal string
                        // but octal escapes survive every parser and any
                        // downstream re-encoding.
                        other => content.extend_from_slice(format!("\\{other:03o}").as_bytes()),
                    }
                }
                content.extend_from_slice(b") Tj\n");
                // Reset position for next text
                content.extend_from_slice(format!("{} {} Td\n", -text_op.x, -text_op.y).as_bytes());
            }

            content.extend_from_slice(b"ET\n"); // End text

            // Add content stream
            doc.objects.insert(
                content_id,
                Object::Stream(Stream::new(dictionary! {}, content)),
            );

            // Add page
            doc.objects.insert(
                page_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), page_width.into(), page_height.into()],
                    "Contents" => content_id,
                    "Resources" => dictionary! {
                        "Font" => dictionary! {
                            "F1" => font_id,
                        },
                    },
                }),
            );
            page_ids.push(page_id);
        }

        // Add pages object
        let kids: Vec<Object> = page_ids.iter().map(|id| (*id).into()).collect();
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => page_ids.len() as i64,
            }),
        );

        // Add catalog
        let catalog_id = doc.new_object_id();
        doc.objects.insert(
            catalog_id,
            Object::Dictionary(dictionary! {
                "Type" => "Catalog",
                "Pages" => pages_id,
            }),
        );

        // Add info
        let info_id = doc.new_object_id();
        let mut info_dict = lopdf::Dictionary::new();
        if let Some(title) = &self.title {
            info_dict.set(
                "Title",
                Object::String(pdf_text_string(title), StringFormat::Literal),
            );
        }
        if let Some(author) = &self.author {
            info_dict.set(
                "Author",
                Object::String(pdf_text_string(author), StringFormat::Literal),
            );
        }
        info_dict.set(
            "Producer",
            Object::String(b"Armature Files".to_vec(), StringFormat::Literal),
        );
        doc.objects.insert(info_id, Object::Dictionary(info_dict));

        // Set trailer
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set("Info", info_id);

        // Compress and save
        doc.compress();

        // Seed the serialization buffer instead of doubling from zero. Roughly
        // one object header plus its content stream per page, floored so small
        // documents still get a single allocation.
        let text_bytes: usize = self
            .pages
            .iter()
            .flat_map(|page| page.texts.iter())
            .map(|op| op.text.len() + 64)
            .sum();
        let estimate = (4096 + text_bytes + self.pages.len() * 512).min(16 * 1024 * 1024);

        let mut buffer = Vec::with_capacity(estimate);
        doc.save_to(&mut buffer)
            .map_err(|e| crate::FileError::Pdf(e.to_string()))?;

        let bytes = Bytes::from(buffer);

        Ok(ProcessingResult {
            data: bytes.clone(),
            metadata: FileMetadata {
                filename: self.title.unwrap_or_else(|| "document.pdf".to_string()),
                mime_type: "application/pdf".to_string(),
                size: bytes.len() as u64,
                extension: Some("pdf".to_string()),
                width: None,
                height: None,
                pages: Some(self.pages.len() as u32),
            },
            operations: vec!["pdf:generate".to_string()],
            processing_time_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Build and save to file.
    ///
    /// The build itself (content-stream assembly plus lopdf's compression and
    /// serialization) is pure CPU work, so it runs on a blocking thread rather
    /// than stalling a runtime worker.
    pub async fn save(self, path: impl AsRef<std::path::Path>) -> FileResult<ProcessingResult> {
        let result = self.build_async().await?;
        result.save(path).await?;
        Ok(result)
    }

    /// Build the PDF on a blocking thread.
    ///
    /// Prefer this over [`Self::build`] from inside async code: `build` is
    /// CPU-bound and blocks the calling runtime worker for its whole duration.
    pub async fn build_async(self) -> FileResult<ProcessingResult> {
        tokio::task::spawn_blocking(move || self.build())
            .await
            .map_err(|e| crate::FileError::Pdf(format!("pdf build task failed: {e}")))?
    }
}

impl Default for PdfBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Quick PDF generation helpers
pub mod quick {
    use super::*;

    /// Create a simple text document
    pub fn text_document(title: &str, content: &str) -> FileResult<ProcessingResult> {
        PdfBuilder::new()
            .title(title)
            .add_heading(title, FontSize::H1)
            .add_space(10.0)
            .add_text(content)
            .build()
    }

    /// Create an invoice-style document
    pub fn invoice(
        invoice_number: &str,
        company: &str,
        items: &[(&str, &str, &str)],
        total: &str,
    ) -> FileResult<ProcessingResult> {
        let mut builder = PdfBuilder::new()
            .title(format!("Invoice {}", invoice_number))
            .add_heading("INVOICE", FontSize::H1)
            .add_space(10.0)
            .add_text(&format!("Invoice #: {}", invoice_number))
            .add_text(&format!("From: {}", company))
            .add_horizontal_line()
            .add_space(10.0);

        let mut rows: Vec<Vec<&str>> = vec![vec!["Description", "Qty", "Price"]];
        for (desc, qty, price) in items {
            rows.push(vec![*desc, *qty, *price]);
        }
        rows.push(vec!["", "Total:", total]);

        builder = builder.add_table(&rows);

        builder.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_size_points() {
        assert_eq!(FontSize::H1.points(), 24.0);
        assert_eq!(FontSize::Normal.points(), 12.0);
        assert_eq!(FontSize::Custom(18.0).points(), 18.0);
    }

    #[test]
    fn test_page_size_dimensions() {
        let (w, h) = PageSize::A4.dimensions();
        assert_eq!(w, 595.0);
        assert_eq!(h, 842.0);
    }

    #[test]
    fn test_margins_uniform() {
        let margins = Margins::uniform(72.0);
        assert_eq!(margins.top, 72.0);
        assert_eq!(margins.right, 72.0);
        assert_eq!(margins.bottom, 72.0);
        assert_eq!(margins.left, 72.0);
    }
}
