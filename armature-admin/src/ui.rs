//! UI components for admin dashboard

use crate::config::Theme;
use serde::{Deserialize, Serialize};

/// Pagination info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    /// Current page (1-indexed)
    pub page: usize,
    /// Total pages
    pub total_pages: usize,
    /// Items per page
    pub per_page: usize,
    /// Total items
    pub total_items: usize,
    /// Has previous page
    pub has_prev: bool,
    /// Has next page
    pub has_next: bool,
    /// Start item number (for display)
    pub start_item: usize,
    /// End item number (for display)
    pub end_item: usize,
}

impl Pagination {
    /// Create pagination info
    pub fn new(page: usize, per_page: usize, total_items: usize) -> Self {
        let total_pages = (total_items + per_page - 1) / per_page;
        let page = page.min(total_pages).max(1);
        let start_item = (page - 1) * per_page + 1;
        let end_item = (start_item + per_page - 1).min(total_items);

        Self {
            page,
            total_pages,
            per_page,
            total_items,
            has_prev: page > 1,
            has_next: page < total_pages,
            start_item: if total_items > 0 { start_item } else { 0 },
            end_item,
        }
    }

    /// Get page numbers for pagination UI
    pub fn page_numbers(&self, window: usize) -> Vec<PageNumber> {
        let mut pages = Vec::new();

        if self.total_pages <= 0 {
            return pages;
        }

        // Always show first page
        pages.push(PageNumber::Page(1));

        let start = (self.page as i64 - window as i64).max(2) as usize;
        let end = (self.page + window).min(self.total_pages - 1);

        // Add ellipsis if needed
        if start > 2 {
            pages.push(PageNumber::Ellipsis);
        }

        // Add middle pages
        for p in start..=end {
            pages.push(PageNumber::Page(p));
        }

        // Add ellipsis before last page if needed
        if end < self.total_pages - 1 {
            pages.push(PageNumber::Ellipsis);
        }

        // Always show last page (if more than 1 page)
        if self.total_pages > 1 {
            pages.push(PageNumber::Page(self.total_pages));
        }

        pages
    }
}

/// Page number for pagination
#[derive(Debug, Clone, Copy)]
pub enum PageNumber {
    /// A specific page
    Page(usize),
    /// Ellipsis (...)
    Ellipsis,
}

/// Flash message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashMessage {
    /// Message type
    pub level: MessageLevel,
    /// Message content
    pub message: String,
    /// Auto-dismiss after seconds
    pub dismiss_after: Option<u32>,
}

impl FlashMessage {
    /// Create a success message
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            level: MessageLevel::Success,
            message: message.into(),
            dismiss_after: Some(5),
        }
    }

    /// Create an error message
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: MessageLevel::Error,
            message: message.into(),
            dismiss_after: None,
        }
    }

    /// Create a warning message
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            level: MessageLevel::Warning,
            message: message.into(),
            dismiss_after: Some(10),
        }
    }

    /// Create an info message
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            level: MessageLevel::Info,
            message: message.into(),
            dismiss_after: Some(5),
        }
    }
}

/// Message level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageLevel {
    Success,
    Error,
    Warning,
    Info,
}

impl MessageLevel {
    /// Get CSS class for this level
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Success => "alert-success",
            Self::Error => "alert-error",
            Self::Warning => "alert-warning",
            Self::Info => "alert-info",
        }
    }

    /// Get icon for this level
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Success => "check-circle",
            Self::Error => "x-circle",
            Self::Warning => "alert-triangle",
            Self::Info => "info",
        }
    }
}

/// Breadcrumb item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breadcrumb {
    /// Label
    pub label: String,
    /// URL (None for current page)
    pub url: Option<String>,
    /// Icon
    pub icon: Option<String>,
}

impl Breadcrumb {
    /// Create a breadcrumb
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            url: None,
            icon: None,
        }
    }

    /// With URL
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// With icon
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Table column for list view
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableColumn {
    /// Field name
    pub field: String,
    /// Display label
    pub label: String,
    /// Is sortable?
    pub sortable: bool,
    /// Current sort direction (if sorted)
    pub sort_direction: Option<SortDirection>,
    /// CSS class
    pub css_class: Option<String>,
    /// Width
    pub width: Option<String>,
}

/// Sort direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// Table row
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    /// Primary key value
    pub id: String,
    /// Cell values (field -> rendered value)
    pub cells: Vec<TableCell>,
    /// Is selected?
    pub selected: bool,
    /// Row CSS class
    pub css_class: Option<String>,
}

/// Table cell
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableCell {
    /// Field name
    pub field: String,
    /// Raw value
    pub value: serde_json::Value,
    /// Rendered HTML
    pub rendered: String,
    /// Cell type
    pub cell_type: CellType,
}

/// Cell type for rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellType {
    Text,
    Number,
    Boolean,
    Date,
    DateTime,
    Email,
    Url,
    Image,
    Badge,
    Actions,
}

/// Filter definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterDef {
    /// Field name
    pub field: String,
    /// Display label
    pub label: String,
    /// Filter type
    pub filter_type: FilterType,
    /// Available choices
    pub choices: Vec<FilterChoice>,
    /// Current value
    pub current: Option<String>,
}

/// Filter type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterType {
    /// Boolean (yes/no/all)
    Boolean,
    /// Select from choices
    Select,
    /// Date range
    DateRange,
    /// Number range
    NumberRange,
    /// Text search
    Text,
}

/// Filter choice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterChoice {
    /// Value
    pub value: String,
    /// Label
    pub label: String,
    /// Count of matching items
    pub count: Option<usize>,
}

/// Statistics card for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatCard {
    /// Card title
    pub title: String,
    /// Main value
    pub value: String,
    /// Change from previous period
    pub change: Option<StatChange>,
    /// Icon
    pub icon: Option<String>,
    /// Card color
    pub color: Option<String>,
    /// Link URL
    pub link: Option<String>,
}

/// Change indicator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatChange {
    /// Change value
    pub value: String,
    /// Is positive change?
    pub positive: bool,
    /// Period label (e.g., "vs last month")
    pub period: String,
}

/// Quick action button
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickAction {
    /// Label
    pub label: String,
    /// URL
    pub url: String,
    /// Icon
    pub icon: Option<String>,
    /// CSS class
    pub css_class: Option<String>,
}

/// Generate CSS for theme
pub fn generate_admin_css(theme: &Theme) -> String {
    let variables = theme.to_css_variables();

    format!(
        r#"{}

/* Admin Base Styles */
* {{
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}}

body {{
  font-family: var(--admin-font);
  background: var(--admin-bg);
  color: var(--admin-text);
  line-height: 1.5;
}}

/* Layout */
.admin-layout {{
  display: flex;
  min-height: 100vh;
}}

.admin-sidebar {{
  width: var(--admin-sidebar-width);
  background: var(--admin-surface);
  border-right: 1px solid var(--admin-border);
  display: flex;
  flex-direction: column;
}}

.admin-content {{
  flex: 1;
  overflow-x: auto;
}}

/* Navigation */
.admin-nav {{
  padding: 1rem;
}}

.admin-nav-item {{
  display: flex;
  align-items: center;
  padding: 0.75rem 1rem;
  color: var(--admin-text-muted);
  text-decoration: none;
  border-radius: var(--admin-radius);
  transition: all 0.15s;
}}

.admin-nav-item:hover,
.admin-nav-item.active {{
  background: var(--admin-primary);
  color: white;
}}

/* Cards */
.admin-card {{
  background: var(--admin-surface);
  border: 1px solid var(--admin-border);
  border-radius: var(--admin-radius);
  padding: 1.5rem;
}}

/* Tables */
.admin-table {{
  width: 100%;
  border-collapse: collapse;
}}

.admin-table th,
.admin-table td {{
  padding: 0.75rem 1rem;
  text-align: left;
  border-bottom: 1px solid var(--admin-border);
}}

.admin-table th {{
  font-weight: 600;
  color: var(--admin-text-muted);
  font-size: 0.875rem;
}}

.admin-table tr:hover {{
  background: rgba(255, 255, 255, 0.02);
}}

/* Forms */
.admin-input {{
  width: 100%;
  padding: 0.5rem 0.75rem;
  background: var(--admin-bg);
  border: 1px solid var(--admin-border);
  border-radius: var(--admin-radius);
  color: var(--admin-text);
  font-size: 0.875rem;
}}

.admin-input:focus {{
  outline: none;
  border-color: var(--admin-primary);
  box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.2);
}}

/* Buttons */
.admin-btn {{
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  font-size: 0.875rem;
  font-weight: 500;
  border-radius: var(--admin-radius);
  border: none;
  cursor: pointer;
  transition: all 0.15s;
}}

.admin-btn-primary {{
  background: var(--admin-primary);
  color: white;
}}

.admin-btn-primary:hover {{
  filter: brightness(1.1);
}}

.admin-btn-danger {{
  background: var(--admin-error);
  color: white;
}}

/* Alerts */
.admin-alert {{
  padding: 1rem;
  border-radius: var(--admin-radius);
  margin-bottom: 1rem;
}}

.alert-success {{
  background: rgba(34, 197, 94, 0.1);
  border: 1px solid var(--admin-success);
  color: var(--admin-success);
}}

.alert-error {{
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid var(--admin-error);
  color: var(--admin-error);
}}

/* Badges */
.admin-badge {{
  display: inline-flex;
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
  font-weight: 500;
  border-radius: 9999px;
}}

.badge-success {{
  background: rgba(34, 197, 94, 0.2);
  color: var(--admin-success);
}}

.badge-warning {{
  background: rgba(245, 158, 11, 0.2);
  color: var(--admin-warning);
}}

.badge-error {{
  background: rgba(239, 68, 68, 0.2);
  color: var(--admin-error);
}}

/* Pagination */
.admin-pagination {{
  display: flex;
  align-items: center;
  gap: 0.25rem;
}}

.admin-page-btn {{
  min-width: 2rem;
  height: 2rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--admin-radius);
  border: 1px solid var(--admin-border);
  background: transparent;
  color: var(--admin-text);
  cursor: pointer;
}}

.admin-page-btn.active {{
  background: var(--admin-primary);
  border-color: var(--admin-primary);
  color: white;
}}
"#,
        variables
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagination() {
        let pagination = Pagination::new(1, 10, 95);

        assert_eq!(pagination.total_pages, 10);
        assert!(pagination.has_next);
        assert!(!pagination.has_prev);
        assert_eq!(pagination.start_item, 1);
        assert_eq!(pagination.end_item, 10);
    }

    #[test]
    fn test_pagination_empty() {
        let pagination = Pagination::new(1, 10, 0);

        assert_eq!(pagination.total_pages, 0);
        assert!(!pagination.has_next);
        assert!(!pagination.has_prev);
        assert_eq!(pagination.start_item, 0);
    }

    #[test]
    fn test_flash_message() {
        let msg = FlashMessage::success("Record saved");
        assert_eq!(msg.level, MessageLevel::Success);
        assert_eq!(msg.dismiss_after, Some(5));
    }
}
