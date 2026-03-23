//! View structures for admin pages

use crate::{
    ListParams,
    field::FieldDefinition,
    model::ModelDefinition,
    ui::{Breadcrumb, FilterDef, Pagination, TableColumn, TableRow},
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
    /// Create a new list view
    pub fn new(model: &ModelDefinition, params: ListParams) -> Self {
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
            rows: Vec::new(), // Would be populated from database
            pagination: Pagination::new(params.page(), 25, 0),
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

    /// Set rows (from database query)
    pub fn with_rows(mut self, rows: Vec<TableRow>, total: usize) -> Self {
        self.rows = rows;
        self.pagination = Pagination::new(self.pagination.page, self.pagination.per_page, total);
        self
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
                Breadcrumb::new(&model.verbose_name).url(&format!("/admin/{}", model.name)),
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
                Breadcrumb::new(&model.verbose_name).url(&format!("/admin/{}", model.name)),
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

        let view = ListView::new(&model, ListParams::default());

        assert_eq!(view.model_name, "user");
        assert_eq!(view.columns.len(), 3);
        assert!(view.has_search);
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
