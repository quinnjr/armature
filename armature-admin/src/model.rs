//! Model definitions for admin

use crate::field::{FieldDefinition, FieldType};
use serde::{Deserialize, Serialize};

/// Model definition for admin registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDefinition {
    /// Model name (used in URLs)
    pub name: String,
    /// Display name (plural)
    pub verbose_name: String,
    /// Singular display name
    pub verbose_name_singular: String,
    /// Database table name
    pub table_name: String,
    /// Fields
    pub fields: Vec<FieldDefinition>,
    /// Primary key field name
    pub primary_key: String,
    /// Fields to display in list view
    pub list_display: Vec<String>,
    /// Fields that are searchable
    pub search_fields: Vec<String>,
    /// Default ordering
    pub ordering: Vec<OrderingField>,
    /// Fields that can be filtered
    pub list_filter: Vec<String>,
    /// Read-only fields
    pub readonly_fields: Vec<String>,
    /// Fields excluded from forms
    pub exclude: Vec<String>,
    /// Fieldsets for form organization
    pub fieldsets: Vec<Fieldset>,
    /// Actions available for this model
    pub actions: Vec<AdminAction>,
    /// Icon (for sidebar)
    pub icon: Option<String>,
    /// Custom list template
    pub list_template: Option<String>,
    /// Custom detail template
    pub detail_template: Option<String>,
    /// Custom form template
    pub form_template: Option<String>,
    /// Inline models (for related data)
    pub inlines: Vec<InlineDefinition>,
    /// Can add new records?
    pub can_add: bool,
    /// Can edit records?
    pub can_edit: bool,
    /// Can delete records?
    pub can_delete: bool,
    /// Can export records?
    pub can_export: bool,
}

impl ModelDefinition {
    /// Create a new model definition builder
    pub fn builder(name: impl Into<String>) -> ModelDefinitionBuilder {
        ModelDefinitionBuilder::new(name)
    }

    /// Get a field by name
    pub fn get_field(&self, name: &str) -> Option<&FieldDefinition> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Get display fields
    pub fn display_fields(&self) -> Vec<&FieldDefinition> {
        self.list_display
            .iter()
            .filter_map(|name| self.get_field(name))
            .collect()
    }

    /// Get searchable fields
    pub fn searchable_fields(&self) -> Vec<&FieldDefinition> {
        self.search_fields
            .iter()
            .filter_map(|name| self.get_field(name))
            .collect()
    }

    /// Get filterable fields
    pub fn filterable_fields(&self) -> Vec<&FieldDefinition> {
        self.list_filter
            .iter()
            .filter_map(|name| self.get_field(name))
            .collect()
    }

    /// Get editable fields for forms
    pub fn form_fields(&self) -> Vec<&FieldDefinition> {
        self.fields
            .iter()
            .filter(|f| !f.primary_key && !self.exclude.contains(&f.name))
            .collect()
    }

    /// Get primary key field
    pub fn pk_field(&self) -> Option<&FieldDefinition> {
        self.get_field(&self.primary_key)
    }
}

/// Builder for model definitions
pub struct ModelDefinitionBuilder {
    name: String,
    verbose_name: Option<String>,
    verbose_name_singular: Option<String>,
    table_name: Option<String>,
    fields: Vec<FieldDefinition>,
    primary_key: String,
    list_display: Vec<String>,
    search_fields: Vec<String>,
    ordering: Vec<OrderingField>,
    list_filter: Vec<String>,
    readonly_fields: Vec<String>,
    exclude: Vec<String>,
    fieldsets: Vec<Fieldset>,
    actions: Vec<AdminAction>,
    icon: Option<String>,
    inlines: Vec<InlineDefinition>,
    can_add: bool,
    can_edit: bool,
    can_delete: bool,
    can_export: bool,
}

impl ModelDefinitionBuilder {
    /// Create a new builder
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            name: name.clone(),
            verbose_name: None,
            verbose_name_singular: None,
            table_name: None,
            fields: Vec::new(),
            primary_key: "id".to_string(),
            list_display: Vec::new(),
            search_fields: Vec::new(),
            ordering: vec![OrderingField::desc("id")],
            list_filter: Vec::new(),
            readonly_fields: Vec::new(),
            exclude: Vec::new(),
            fieldsets: Vec::new(),
            actions: Vec::new(),
            icon: None,
            inlines: Vec::new(),
            can_add: true,
            can_edit: true,
            can_delete: true,
            can_export: true,
        }
    }

    /// Set verbose name
    pub fn verbose_name(mut self, name: impl Into<String>) -> Self {
        self.verbose_name = Some(name.into());
        self
    }

    /// Set table name
    pub fn table_name(mut self, name: impl Into<String>) -> Self {
        self.table_name = Some(name.into());
        self
    }

    /// Add a field
    pub fn field(mut self, field: FieldDefinition) -> Self {
        if field.primary_key {
            self.primary_key = field.name.clone();
        }
        self.fields.push(field);
        self
    }

    /// Add ID field (common pattern)
    pub fn id_field(self) -> Self {
        self.field(
            FieldDefinition::new("id", FieldType::BigInteger)
                .primary_key()
                .label("ID"),
        )
    }

    /// Add timestamp fields
    pub fn timestamps(self) -> Self {
        self.field(
            FieldDefinition::new("created_at", FieldType::DateTime)
                .readonly()
                .label("Created"),
        )
        .field(
            FieldDefinition::new("updated_at", FieldType::DateTime)
                .readonly()
                .label("Updated"),
        )
    }

    /// Set list display fields
    pub fn list_display(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.list_display = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Set search fields
    pub fn search_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.search_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Set ordering
    pub fn ordering(mut self, fields: impl IntoIterator<Item = OrderingField>) -> Self {
        self.ordering = fields.into_iter().collect();
        self
    }

    /// Set list filter fields
    pub fn list_filter(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.list_filter = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Set readonly fields
    pub fn readonly_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.readonly_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Set excluded fields
    pub fn exclude(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.exclude = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Add a fieldset
    pub fn fieldset(mut self, fieldset: Fieldset) -> Self {
        self.fieldsets.push(fieldset);
        self
    }

    /// Add an action
    pub fn action(mut self, action: AdminAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Set icon
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Add inline
    pub fn inline(mut self, inline: InlineDefinition) -> Self {
        self.inlines.push(inline);
        self
    }

    /// Disable adding
    pub fn no_add(mut self) -> Self {
        self.can_add = false;
        self
    }

    /// Disable editing
    pub fn no_edit(mut self) -> Self {
        self.can_edit = false;
        self
    }

    /// Disable deleting
    pub fn no_delete(mut self) -> Self {
        self.can_delete = false;
        self
    }

    /// Build the model definition
    pub fn build(self) -> ModelDefinition {
        let verbose_name = self.verbose_name.unwrap_or_else(|| {
            // Pluralize name
            let name = self.name.replace('_', " ");
            if name.ends_with('s') {
                name
            } else {
                format!("{}s", name)
            }
        });

        let verbose_name_singular = self
            .verbose_name_singular
            .unwrap_or_else(|| self.name.replace('_', " "));

        let table_name = self
            .table_name
            .unwrap_or_else(|| self.name.to_lowercase().replace(' ', "_"));

        // If no list_display set, use all fields
        let list_display = if self.list_display.is_empty() {
            self.fields.iter().take(5).map(|f| f.name.clone()).collect()
        } else {
            self.list_display
        };

        ModelDefinition {
            name: self.name,
            verbose_name,
            verbose_name_singular,
            table_name,
            fields: self.fields,
            primary_key: self.primary_key,
            list_display,
            search_fields: self.search_fields,
            ordering: self.ordering,
            list_filter: self.list_filter,
            readonly_fields: self.readonly_fields,
            exclude: self.exclude,
            fieldsets: self.fieldsets,
            actions: self.actions,
            icon: self.icon,
            list_template: None,
            detail_template: None,
            form_template: None,
            inlines: self.inlines,
            can_add: self.can_add,
            can_edit: self.can_edit,
            can_delete: self.can_delete,
            can_export: self.can_export,
        }
    }
}

/// Ordering field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderingField {
    /// Field name
    pub field: String,
    /// Is descending?
    pub descending: bool,
}

impl OrderingField {
    /// Ascending order
    pub fn asc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            descending: false,
        }
    }

    /// Descending order
    pub fn desc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            descending: true,
        }
    }

    /// Get SQL representation
    pub fn as_sql(&self) -> String {
        format!(
            "{} {}",
            self.field,
            if self.descending { "DESC" } else { "ASC" }
        )
    }
}

/// Fieldset for organizing form fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fieldset {
    /// Fieldset name
    pub name: Option<String>,
    /// Fields in this set
    pub fields: Vec<String>,
    /// CSS classes
    pub classes: Vec<String>,
    /// Description
    pub description: Option<String>,
    /// Is collapsible?
    pub collapsible: bool,
    /// Is initially collapsed?
    pub collapsed: bool,
}

impl Fieldset {
    /// Create a new fieldset
    pub fn new(fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            name: None,
            fields: fields.into_iter().map(Into::into).collect(),
            classes: Vec::new(),
            description: None,
            collapsible: false,
            collapsed: false,
        }
    }

    /// Named fieldset
    pub fn named(
        name: impl Into<String>,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            name: Some(name.into()),
            fields: fields.into_iter().map(Into::into).collect(),
            classes: Vec::new(),
            description: None,
            collapsible: false,
            collapsed: false,
        }
    }

    /// Set description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Make collapsible
    pub fn collapsible(mut self) -> Self {
        self.collapsible = true;
        self
    }

    /// Start collapsed
    pub fn collapsed(mut self) -> Self {
        self.collapsible = true;
        self.collapsed = true;
        self
    }
}

/// Admin action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminAction {
    /// Action name (identifier)
    pub name: String,
    /// Display label
    pub label: String,
    /// Description
    pub description: Option<String>,
    /// Icon
    pub icon: Option<String>,
    /// Is dangerous (requires confirmation)?
    pub dangerous: bool,
    /// Requires selection?
    pub requires_selection: bool,
}

impl AdminAction {
    /// Create a new action
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: None,
            icon: None,
            dangerous: false,
            requires_selection: true,
        }
    }

    /// Delete action preset
    pub fn delete() -> Self {
        Self {
            name: "delete".to_string(),
            label: "Delete selected".to_string(),
            description: Some("Permanently delete selected items".to_string()),
            icon: Some("trash".to_string()),
            dangerous: true,
            requires_selection: true,
        }
    }

    /// Export action preset
    pub fn export() -> Self {
        Self {
            name: "export".to_string(),
            label: "Export".to_string(),
            description: Some("Export selected items to CSV".to_string()),
            icon: Some("download".to_string()),
            dangerous: false,
            requires_selection: false,
        }
    }

    /// Set description
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set icon
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Mark as dangerous
    pub fn dangerous(mut self) -> Self {
        self.dangerous = true;
        self
    }
}

/// Inline definition for related models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InlineDefinition {
    /// Related model name
    pub model: String,
    /// Foreign key field in related model
    pub fk_field: String,
    /// Fields to display
    pub fields: Vec<String>,
    /// Extra rows to show
    pub extra: usize,
    /// Maximum rows
    pub max_num: Option<usize>,
    /// Minimum rows
    pub min_num: usize,
    /// Can delete?
    pub can_delete: bool,
    /// Verbose name
    pub verbose_name: Option<String>,
}

impl InlineDefinition {
    /// Create a new inline
    pub fn new(model: impl Into<String>, fk_field: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            fk_field: fk_field.into(),
            fields: Vec::new(),
            extra: 3,
            max_num: None,
            min_num: 0,
            can_delete: true,
            verbose_name: None,
        }
    }

    /// Set fields
    pub fn fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Set extra rows
    pub fn extra(mut self, extra: usize) -> Self {
        self.extra = extra;
        self
    }

    /// Set max rows
    pub fn max_num(mut self, max: usize) -> Self {
        self.max_num = Some(max);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_builder() {
        let model = ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String).required())
            .field(FieldDefinition::new("email", FieldType::Email))
            .timestamps()
            .list_display(["id", "name", "email"])
            .search_fields(["name", "email"])
            .build();

        assert_eq!(model.name, "user");
        assert_eq!(model.verbose_name, "users");
        assert_eq!(model.primary_key, "id");
        assert_eq!(model.fields.len(), 5);
    }

    #[test]
    fn test_fieldset() {
        let fieldset = Fieldset::named("Personal Info", ["name", "email"])
            .description("User's personal information")
            .collapsible();

        assert_eq!(fieldset.name, Some("Personal Info".to_string()));
        assert_eq!(fieldset.fields.len(), 2);
        assert!(fieldset.collapsible);
    }

    #[test]
    fn test_ordering() {
        let desc = OrderingField::desc("created_at");
        assert_eq!(desc.as_sql(), "created_at DESC");

        let asc = OrderingField::asc("name");
        assert_eq!(asc.as_sql(), "name ASC");
    }
}
