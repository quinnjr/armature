//! View structures for admin pages

use crate::{
    ListParams,
    config::AdminConfig,
    data::value_to_plain_string,
    field::{FieldDefinition, FieldType},
    model::ModelDefinition,
    ui::{Breadcrumb, CellType, FilterDef, Pagination, TableCell, TableColumn, TableRow},
};
use serde::{Deserialize, Serialize};

/// List view for a model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListView {
    /// Model name
    pub model_name: String,
    /// Verbose name (plural)
    pub verbose_name: String,
    /// Page title
    pub title: String,
    /// Breadcrumbs
    pub breadcrumbs: Vec<Breadcrumb>,
    /// Table columns
    pub columns: Vec<TableColumn>,
    /// Table rows
    pub rows: Vec<TableRow>,
    /// Pagination
    pub pagination: Pagination,
    /// Filters
    pub filters: Vec<FilterDef>,
    /// Current search query
    pub search_query: Option<String>,
    /// Can add new records?
    pub can_add: bool,
    /// Can delete records?
    pub can_delete: bool,
    /// Can export?
    pub can_export: bool,
    /// Add URL
    pub add_url: String,
    /// Search placeholder
    pub search_placeholder: String,
    /// Has search enabled?
    pub has_search: bool,
    /// Has filters?
    pub has_filters: bool,
}

impl ListView {
    /// Create a new list view.
    ///
    /// `per_page` is the already-resolved page size (see
    /// [`ListParams::resolve_per_page`]); rows are filled in separately from a
    /// [`crate::data::DataSource`] via [`ListView::with_json_rows`].
    pub fn new(model: &ModelDefinition, params: ListParams, per_page: usize) -> Self {
        let columns = model
            .display_fields()
            .iter()
            .map(|f| TableColumn {
                field: f.name.clone(),
                label: f.label.clone(),
                sortable: f.sortable,
                sort_direction: if params.sort.as_deref() == Some(&f.name) {
                    Some(match params.order {
                        Some(crate::SortOrder::Desc) => crate::ui::SortDirection::Desc,
                        _ => crate::ui::SortDirection::Asc,
                    })
                } else {
                    None
                },
                css_class: None,
                width: None,
            })
            .collect();

        let filters = model
            .filterable_fields()
            .iter()
            .map(|f| FilterDef {
                field: f.name.clone(),
                label: f.label.clone(),
                filter_type: match f.field_type {
                    crate::field::FieldType::Boolean => crate::ui::FilterType::Boolean,
                    crate::field::FieldType::Enum => crate::ui::FilterType::Select,
                    crate::field::FieldType::Date | crate::field::FieldType::DateTime => {
                        crate::ui::FilterType::DateRange
                    }
                    _ => crate::ui::FilterType::Text,
                },
                choices: f
                    .choices
                    .as_ref()
                    .map(|choices| {
                        choices
                            .iter()
                            .map(|c| crate::ui::FilterChoice {
                                value: c.value.clone(),
                                label: c.label.clone(),
                                count: None,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                current: params.filters.get(&f.name).cloned(),
            })
            .collect();

        Self {
            model_name: model.name.clone(),
            verbose_name: model.verbose_name.clone(),
            title: model.verbose_name.clone(),
            breadcrumbs: vec![
                Breadcrumb::new("Dashboard").url("/admin"),
                Breadcrumb::new(&model.verbose_name),
            ],
            columns,
            rows: Vec::new(), // Populated from the data source via `with_json_rows`.
            pagination: Pagination::new(params.page(), per_page.max(1), 0),
            filters,
            search_query: params.search,
            can_add: model.can_add,
            can_delete: model.can_delete,
            can_export: model.can_export,
            add_url: format!("/admin/{}/add", model.name),
            search_placeholder: format!("Search {}...", model.search_fields.join(", ")),
            has_search: !model.search_fields.is_empty(),
            has_filters: !model.list_filter.is_empty(),
        }
    }

    /// Set pre-built rows (from a data source), recomputing pagination.
    pub fn with_rows(mut self, rows: Vec<TableRow>, total: usize) -> Self {
        self.rows = rows;
        self.pagination = Pagination::new(self.pagination.page, self.pagination.per_page, total);
        self
    }

    /// Populate rows from raw JSON records returned by a
    /// [`crate::data::DataSource`], rendering each display column's cell and
    /// honoring the configured date/datetime formats.
    pub fn with_json_rows(
        self,
        model: &ModelDefinition,
        config: &AdminConfig,
        records: &[serde_json::Value],
        total: usize,
    ) -> Self {
        let rows = records
            .iter()
            .map(|record| build_table_row(model, config, record))
            .collect();
        self.with_rows(rows, total)
    }
}

/// Build a [`TableRow`] for one record, one cell per display field.
fn build_table_row(
    model: &ModelDefinition,
    config: &AdminConfig,
    record: &serde_json::Value,
) -> TableRow {
    let id = record
        .get(&model.primary_key)
        .map(value_to_plain_string)
        .unwrap_or_default();

    let cells = model
        .display_fields()
        .iter()
        .map(|field| {
            let value = record
                .get(&field.name)
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let (rendered, cell_type) = render_cell(field, config, &value);
            TableCell {
                field: field.name.clone(),
                value,
                rendered,
                cell_type,
            }
        })
        .collect();

    TableRow {
        id,
        cells,
        selected: false,
        css_class: None,
    }
}

/// Render a single cell's HTML + classify its cell type, applying the
/// configured date/datetime formats for temporal fields.
fn render_cell(
    field: &FieldDefinition,
    config: &AdminConfig,
    value: &serde_json::Value,
) -> (String, CellType) {
    match field.field_type {
        FieldType::Date => {
            let raw = value_to_plain_string(value);
            (html_escape(&config.format_date(&raw)), CellType::Date)
        }
        FieldType::DateTime => {
            let raw = value_to_plain_string(value);
            (
                html_escape(&config.format_datetime(&raw)),
                CellType::DateTime,
            )
        }
        FieldType::Boolean => (render_value(value), CellType::Boolean),
        FieldType::Integer | FieldType::BigInteger | FieldType::Float | FieldType::Decimal => {
            (render_value(value), CellType::Number)
        }
        FieldType::Email => (html_escape(&value_to_plain_string(value)), CellType::Email),
        FieldType::Url => (html_escape(&value_to_plain_string(value)), CellType::Url),
        _ => (render_value(value), CellType::Text),
    }
}

/// Detail view for a model record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailView {
    /// Model name
    pub model_name: String,
    /// Verbose name (singular)
    pub verbose_name: String,
    /// Page title
    pub title: String,
    /// Breadcrumbs
    pub breadcrumbs: Vec<Breadcrumb>,
    /// Record ID
    pub id: String,
    /// Field values
    pub fields: Vec<FieldValue>,
    /// Fieldsets
    pub fieldsets: Vec<ViewFieldset>,
    /// Can edit?
    pub can_edit: bool,
    /// Can delete?
    pub can_delete: bool,
    /// Edit URL
    pub edit_url: String,
    /// Delete URL
    pub delete_url: String,
    /// List URL
    pub list_url: String,
    /// Inlines (related data)
    pub inlines: Vec<InlineView>,
}

impl DetailView {
    /// Create a new detail view
    pub fn new(model: &ModelDefinition, id: String) -> Self {
        Self {
            model_name: model.name.clone(),
            verbose_name: model.verbose_name_singular.clone(),
            title: format!("{} #{}", model.verbose_name_singular, id),
            breadcrumbs: vec![
                Breadcrumb::new("Dashboard").url("/admin"),
                Breadcrumb::new(&model.verbose_name).url(format!("/admin/{}", model.name)),
                Breadcrumb::new(&id),
            ],
            id: id.clone(),
            fields: Vec::new(), // Would be populated from database
            fieldsets: Vec::new(),
            can_edit: model.can_edit,
            can_delete: model.can_delete,
            edit_url: format!("/admin/{}/{}/edit", model.name, id),
            delete_url: format!("/admin/{}/{}/delete", model.name, id),
            list_url: format!("/admin/{}", model.name),
            inlines: Vec::new(),
        }
    }

    /// Set field values
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        if let Some(obj) = data.as_object() {
            self.fields = obj
                .iter()
                .map(|(k, v)| FieldValue {
                    name: k.clone(),
                    label: k.replace('_', " "),
                    value: v.clone(),
                    rendered: render_value(v),
                    readonly: false,
                })
                .collect();
        }
        self
    }
}

/// Create view for adding a new record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateView {
    /// Model name
    pub model_name: String,
    /// Verbose name (singular)
    pub verbose_name: String,
    /// Page title
    pub title: String,
    /// Breadcrumbs
    pub breadcrumbs: Vec<Breadcrumb>,
    /// Form fields
    pub fields: Vec<FormField>,
    /// Fieldsets
    pub fieldsets: Vec<ViewFieldset>,
    /// Submit URL
    pub submit_url: String,
    /// Cancel URL
    pub cancel_url: String,
    /// Inlines
    pub inlines: Vec<InlineView>,
}

impl CreateView {
    /// Create a new create view
    pub fn new(model: &ModelDefinition) -> Self {
        let fields = model
            .form_fields()
            .iter()
            .map(|f| FormField::from_definition(f))
            .collect();

        Self {
            model_name: model.name.clone(),
            verbose_name: model.verbose_name_singular.clone(),
            title: format!("Add {}", model.verbose_name_singular),
            breadcrumbs: vec![
                Breadcrumb::new("Dashboard").url("/admin"),
                Breadcrumb::new(&model.verbose_name).url(format!("/admin/{}", model.name)),
                Breadcrumb::new("Add"),
            ],
            fields,
            fieldsets: Vec::new(),
            submit_url: format!("/admin/{}/add", model.name),
            cancel_url: format!("/admin/{}", model.name),
            inlines: Vec::new(),
        }
    }
}

/// Edit view for modifying a record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditView {
    /// Model name
    pub model_name: String,
    /// Verbose name
    pub verbose_name: String,
    /// Page title
    pub title: String,
    /// Breadcrumbs
    pub breadcrumbs: Vec<Breadcrumb>,
    /// Record ID
    pub id: String,
    /// Form fields
    pub fields: Vec<FormField>,
    /// Fieldsets
    pub fieldsets: Vec<ViewFieldset>,
    /// Submit URL
    pub submit_url: String,
    /// Cancel URL
    pub cancel_url: String,
    /// Delete URL
    pub delete_url: String,
    /// Can delete?
    pub can_delete: bool,
    /// Inlines
    pub inlines: Vec<InlineView>,
}

impl EditView {
    /// Create a new edit view (form fields default to empty values).
    pub fn new(model: &ModelDefinition, id: String) -> Self {
        let fields = model
            .form_fields()
            .iter()
            .map(|f| FormField::from_definition(f))
            .collect();

        Self {
            model_name: model.name.clone(),
            verbose_name: model.verbose_name_singular.clone(),
            title: format!("Edit {} #{}", model.verbose_name_singular, id),
            breadcrumbs: vec![
                Breadcrumb::new("Dashboard").url("/admin"),
                Breadcrumb::new(&model.verbose_name).url(format!("/admin/{}", model.name)),
                Breadcrumb::new(&id).url(format!("/admin/{}/{}", model.name, id)),
                Breadcrumb::new("Edit"),
            ],
            id: id.clone(),
            fields,
            fieldsets: Vec::new(),
            submit_url: format!("/admin/{}/{}/edit", model.name, id),
            cancel_url: format!("/admin/{}/{}", model.name, id),
            delete_url: format!("/admin/{}/{}/delete", model.name, id),
            can_delete: model.can_delete,
            inlines: Vec::new(),
        }
    }

    /// Populate the form fields with the current record's values.
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        if let Some(obj) = data.as_object() {
            for field in &mut self.fields {
                if let Some(v) = obj.get(&field.name) {
                    field.value = v.clone();
                }
            }
        }
        self
    }
}

/// Field value for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldValue {
    /// Field name
    pub name: String,
    /// Display label
    pub label: String,
    /// Raw value
    pub value: serde_json::Value,
    /// Rendered HTML
    pub rendered: String,
    /// Is readonly?
    pub readonly: bool,
}

/// Form field for editing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    /// Field name
    pub name: String,
    /// Display label
    pub label: String,
    /// Widget type
    pub widget: String,
    /// Current value
    pub value: serde_json::Value,
    /// Is required?
    pub required: bool,
    /// Is readonly?
    pub readonly: bool,
    /// Help text
    pub help_text: Option<String>,
    /// Placeholder
    pub placeholder: Option<String>,
    /// Choices (for select)
    pub choices: Option<Vec<(String, String)>>,
    /// Validation errors
    pub errors: Vec<String>,
    /// HTML attributes
    pub attrs: std::collections::HashMap<String, String>,
}

impl FormField {
    /// Create from field definition
    pub fn from_definition(field: &FieldDefinition) -> Self {
        let mut attrs = std::collections::HashMap::new();

        if let Some(max_len) = field.max_length {
            attrs.insert("maxlength".to_string(), max_len.to_string());
        }
        if let Some(min) = field.min_value {
            attrs.insert("min".to_string(), min.to_string());
        }
        if let Some(max) = field.max_value {
            attrs.insert("max".to_string(), max.to_string());
        }

        Self {
            name: field.name.clone(),
            label: field.label.clone(),
            widget: format!("{:?}", field.widget).to_lowercase(),
            value: serde_json::Value::Null,
            required: field.required,
            readonly: field.readonly,
            help_text: field.help_text.clone(),
            placeholder: field.placeholder.clone(),
            choices: field.choices.as_ref().map(|c| {
                c.iter()
                    .map(|ch| (ch.value.clone(), ch.label.clone()))
                    .collect()
            }),
            errors: Vec::new(),
            attrs,
        }
    }

    /// Set value
    pub fn with_value(mut self, value: serde_json::Value) -> Self {
        self.value = value;
        self
    }

    /// Add error
    pub fn add_error(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
    }
}

/// Fieldset for organizing form fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewFieldset {
    /// Fieldset name
    pub name: Option<String>,
    /// Description
    pub description: Option<String>,
    /// Fields in this fieldset
    pub fields: Vec<String>,
    /// Is collapsible?
    pub collapsible: bool,
    /// Is collapsed?
    pub collapsed: bool,
}

/// Inline view for related data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineView {
    /// Model name
    pub model_name: String,
    /// Verbose name
    pub verbose_name: String,
    /// Rows
    pub rows: Vec<InlineRow>,
    /// Extra empty rows
    pub extra: usize,
    /// Can delete?
    pub can_delete: bool,
    /// Fields to display
    pub fields: Vec<String>,
}

/// Inline row
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineRow {
    /// Row ID (if existing)
    pub id: Option<String>,
    /// Field values
    pub fields: Vec<FormField>,
    /// Is new?
    pub is_new: bool,
    /// Delete marker
    pub delete: bool,
}

/// Render a value for display
fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "—".to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                r#"<span class="badge badge-success">Yes</span>"#.to_string()
            } else {
                r#"<span class="badge badge-error">No</span>"#.to_string()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => html_escape(s),
        serde_json::Value::Array(arr) => {
            format!("[{} items]", arr.len())
        }
        serde_json::Value::Object(_) => "[Object]".to_string(),
    }
}

/// HTML escape a string
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::FieldType;

    #[test]
    fn test_create_list_view() {
        let model = ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String).searchable())
            .field(FieldDefinition::new("email", FieldType::Email))
            .list_display(["id", "name", "email"])
            .search_fields(["name", "email"])
            .build();

        let view = ListView::new(&model, ListParams::default(), 25);

        assert_eq!(view.model_name, "user");
        assert_eq!(view.columns.len(), 3);
        assert!(view.has_search);
        assert_eq!(view.pagination.per_page, 25);
    }

    #[test]
    fn test_list_view_json_rows() {
        let model = ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String))
            .list_display(["id", "name"])
            .build();

        let records = vec![
            serde_json::json!({ "id": 1, "name": "Alice" }),
            serde_json::json!({ "id": 2, "name": "Bob" }),
        ];
        let view = ListView::new(&model, ListParams::default(), 25).with_json_rows(
            &model,
            &AdminConfig::default(),
            &records,
            2,
        );

        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.rows[0].id, "1");
        assert_eq!(view.rows[0].cells[1].rendered, "Alice");
        assert_eq!(view.pagination.total_items, 2);
    }

    #[test]
    fn test_edit_view_with_data() {
        let model = ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String))
            .build();

        let view = EditView::new(&model, "5".to_string())
            .with_data(serde_json::json!({ "name": "Carol" }));

        assert_eq!(view.id, "5");
        let name = view.fields.iter().find(|f| f.name == "name").unwrap();
        assert_eq!(name.value, serde_json::json!("Carol"));
    }

    #[test]
    fn test_render_value() {
        assert_eq!(render_value(&serde_json::Value::Null), "—");
        assert!(render_value(&serde_json::Value::Bool(true)).contains("Yes"));
        assert_eq!(render_value(&serde_json::json!(42)), "42");
        assert_eq!(render_value(&serde_json::json!("test")), "test");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
    }
}
