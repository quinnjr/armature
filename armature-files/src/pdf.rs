//! PDF generation and manipulation
//!
//! Provides a fluent API for creating and modifying PDF documents.
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_files::pdf::{PdfBuilder, FontSize};
//!
//! let pdf = PdfBuilder::new()
//!     .title("Invoice #12345")
//!     .add_heading("Invoice", FontSize::H1)
//!     .add_text("Thank you for your purchase!")
//!     .build()?;
//! ```

use crate::{FileMetadata, FileResult, ProcessingResult};
use bytes::Bytes;
use lopdf::dictionary;
use lopdf::{Document, Object, Stream, StringFormat};

/// Font sizes for PDF text
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSize {
    /// Heading 1 (24pt)
    H1,
    /// Heading 2 (20pt)
    H2,
    /// Heading 3 (16pt)
    H3,
    /// Normal text (12pt)
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

impl Default for FontSize {
    fn default() -> Self {
        Self::Normal
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSize {
    /// A4 (210 × 297 mm = 595 × 842 points)
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

impl Default for PageSize {
    fn default() -> Self {
        Self::A4
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

/// Page content
#[derive(Debug, Clone, Default)]
struct PageContent {
    texts: Vec<TextOp>,
}

/// PDF document builder
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

    /// Set page size
    pub fn page_size(mut self, size: PageSize) -> Self {
        self.page_size = size;
        let (_, height) = size.dimensions();
        self.cursor_y = height - self.margins.top;
        self
    }

    /// Set page orientation
    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    /// Set margins
    pub fn margins(mut self, margins: Margins) -> Self {
        self.margins = margins;
        let (_, height) = self.page_size.dimensions();
        self.cursor_y = height - margins.top;
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

    /// Add a new page internally
    fn add_page_internal(&mut self) {
        self.pages.push(PageContent::default());
        let (_, height) = self.get_page_dimensions();
        self.cursor_y = height - self.margins.top;
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

    /// Add a heading
    pub fn add_heading(mut self, text: &str, size: FontSize) -> Self {
        let line_height = size.line_height();
        self.ensure_space(line_height);

        if let Some(page) = self.pages.last_mut() {
            page.texts.push(TextOp {
                text: text.to_string(),
                x: self.margins.left,
                y: self.cursor_y,
                font_size: size.points(),
            });
        }

        self.cursor_y -= line_height;
        self
    }

    /// Add text paragraph
    pub fn add_text(self, text: &str) -> Self {
        self.add_text_styled(text, FontSize::Normal)
    }

    /// Add text with custom font size
    pub fn add_text_styled(mut self, text: &str, size: FontSize) -> Self {
        let line_height = size.line_height();

        // Simple word wrapping (approximate)
        let chars_per_line = (self.content_width() / (size.points() * 0.6)) as usize;

        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let mut current_line = String::new();

            for word in words {
                let test_line = if current_line.is_empty() {
                    word.to_string()
                } else {
                    format!("{} {}", current_line, word)
                };

                if test_line.len() > chars_per_line && !current_line.is_empty() {
                    self.ensure_space(line_height);
                    if let Some(page) = self.pages.last_mut() {
                        page.texts.push(TextOp {
                            text: current_line.clone(),
                            x: self.margins.left,
                            y: self.cursor_y,
                            font_size: size.points(),
                        });
                    }
                    self.cursor_y -= line_height;
                    current_line = word.to_string();
                } else {
                    current_line = test_line;
                }
            }

            // Write remaining text
            if !current_line.is_empty() {
                self.ensure_space(line_height);
                if let Some(page) = self.pages.last_mut() {
                    page.texts.push(TextOp {
                        text: current_line,
                        x: self.margins.left,
                        y: self.cursor_y,
                        font_size: size.points(),
                    });
                }
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

    /// Add a horizontal line
    pub fn add_horizontal_line(mut self) -> Self {
        self.ensure_space(10.0);
        // Lines are not directly supported in this simplified version
        // We'll skip the line but add space
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
                if let Some(page) = self.pages.last_mut() {
                    page.texts.push(TextOp {
                        text: cell.to_string(),
                        x,
                        y: self.cursor_y,
                        font_size: FontSize::Normal.points(),
                    });
                }
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

        // Add font (Helvetica is a built-in font)
        doc.objects.insert(
            font_id,
            Object::Dictionary(dictionary! {
                "Type" => "Font",
                "Subtype" => "Type1",
                "BaseFont" => "Helvetica",
            }),
        );

        // Create pages
        for page_content in &self.pages {
            let content_id = doc.new_object_id();
            let page_id = doc.new_object_id();

            // Build content stream
            let mut content = String::new();
            content.push_str("BT\n"); // Begin text
            content.push_str("/F1 12 Tf\n"); // Default font

            for text_op in &page_content.texts {
                // Set font size
                content.push_str(&format!("/F1 {} Tf\n", text_op.font_size));
                // Move to position
                content.push_str(&format!("{} {} Td\n", text_op.x, text_op.y));
                // Show text (escape parentheses)
                let escaped = text_op
                    .text
                    .replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)");
                content.push_str(&format!("({}) Tj\n", escaped));
                // Reset position for next text
                content.push_str(&format!("{} {} Td\n", -text_op.x, -text_op.y));
            }

            content.push_str("ET\n"); // End text

            // Add content stream
            doc.objects.insert(
                content_id,
                Object::Stream(Stream::new(dictionary! {}, content.into_bytes())),
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
                Object::String(title.as_bytes().to_vec(), StringFormat::Literal),
            );
        }
        if let Some(author) = &self.author {
            info_dict.set(
                "Author",
                Object::String(author.as_bytes().to_vec(), StringFormat::Literal),
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
        let mut buffer = Vec::new();
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

    /// Build and save to file
    pub async fn save(self, path: impl AsRef<std::path::Path>) -> FileResult<ProcessingResult> {
        let result = self.build()?;
        result.save(path).await?;
        Ok(result)
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
