//! Field definitions for admin models

use serde::{Deserialize, Serialize};

/// Field definition for a model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDefinition {
    /// Field name
    pub name: String,
    /// Display label
    pub label: String,
    /// Field type
    pub field_type: FieldType,
    /// Widget type for rendering
    pub widget: WidgetType,
    /// Is this field required?
    pub required: bool,
    /// Is this field read-only?
    pub readonly: bool,
    /// Is this the primary key?
    pub primary_key: bool,
    /// Show in list view?
    pub list_display: bool,
    /// Searchable?
    pub searchable: bool,
    /// Filterable?
    pub filterable: bool,
    /// Sortable?
    pub sortable: bool,
    /// Default value
    pub default: Option<String>,
    /// Help text
    pub help_text: Option<String>,
    /// Placeholder text
    pub placeholder: Option<String>,
    /// Validation rules
    pub validators: Vec<ValidatorType>,
    /// Choices for select fields
    pub choices: Option<Vec<Choice>>,
    /// Foreign key reference
    pub foreign_key: Option<ForeignKeyRef>,
    /// Maximum length (for strings)
    pub max_length: Option<usize>,
    /// Minimum value (for numbers)
    pub min_value: Option<f64>,
    /// Maximum value (for numbers)
    pub max_value: Option<f64>,
}

impl FieldDefinition {
    /// Create a new field definition
    pub fn new(name: impl Into<String>, field_type: FieldType) -> Self {
        let name = name.into();
        let label = name
            .replace('_', " ")
            .split_whitespace()
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(c) => c.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        Self {
            name,
            label,
            widget: field_type.default_widget(),
            field_type,
            required: false,
            readonly: false,
            primary_key: false,
            list_display: true,
            searchable: false,
            filterable: false,
            sortable: true,
            default: None,
            help_text: None,
            placeholder: None,
            validators: Vec::new(),
            choices: None,
            foreign_key: None,
            max_length: None,
            min_value: None,
            max_value: None,
        }
    }

    /// Set field as required
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set field as read-only
    pub fn readonly(mut self) -> Self {
        self.readonly = true;
        self
    }

    /// Set as primary key
    pub fn primary_key(mut self) -> Self {
        self.primary_key = true;
        self.readonly = true;
        self
    }

    /// Set custom label
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Set widget type
    pub fn widget(mut self, widget: WidgetType) -> Self {
        self.widget = widget;
        self
    }

    /// Set help text
    pub fn help_text(mut self, text: impl Into<String>) -> Self {
        self.help_text = Some(text.into());
        self
    }

    /// Set placeholder
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Add validator
    pub fn validator(mut self, validator: ValidatorType) -> Self {
        self.validators.push(validator);
        self
    }

    /// Set choices
    pub fn choices(mut self, choices: Vec<Choice>) -> Self {
        self.choices = Some(choices);
        self.widget = WidgetType::Select;
        self
    }

    /// Set foreign key
    pub fn foreign_key(
        mut self,
        model: impl Into<String>,
        display_field: impl Into<String>,
    ) -> Self {
        self.foreign_key = Some(ForeignKeyRef {
            model: model.into(),
            display_field: display_field.into(),
        });
        self.widget = WidgetType::ForeignKey;
        self
    }

    /// Enable search
    pub fn searchable(mut self) -> Self {
        self.searchable = true;
        self
    }

    /// Enable filter
    pub fn filterable(mut self) -> Self {
        self.filterable = true;
        self
    }

    /// Hide from list
    pub fn hide_from_list(mut self) -> Self {
        self.list_display = false;
        self
    }

    /// Disable sorting
    pub fn no_sort(mut self) -> Self {
        self.sortable = false;
        self
    }

    /// Set max length
    pub fn max_length(mut self, len: usize) -> Self {
        self.max_length = Some(len);
        self
    }

    /// Set value range
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min_value = Some(min);
        self.max_value = Some(max);
        self
    }
}

/// Field types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    /// Integer
    Integer,
    /// Big integer
    BigInteger,
    /// Float
    Float,
    /// Decimal
    Decimal,
    /// String
    String,
    /// Text (long string)
    Text,
    /// Boolean
    Boolean,
    /// Date
    Date,
    /// Time
    Time,
    /// DateTime
    DateTime,
    /// UUID
    Uuid,
    /// JSON
    Json,
    /// Binary/Blob
    Binary,
    /// Email
    Email,
    /// URL
    Url,
    /// IP Address
    IpAddress,
    /// Enum/Choices
    Enum,
    /// Foreign key
    ForeignKey,
    /// Many-to-many
    ManyToMany,
}

impl FieldType {
    /// Get default widget for this field type
    pub fn default_widget(&self) -> WidgetType {
        match self {
            Self::Integer | Self::BigInteger => WidgetType::NumberInput,
            Self::Float | Self::Decimal => WidgetType::NumberInput,
            Self::String => WidgetType::TextInput,
            Self::Text => WidgetType::Textarea,
            Self::Boolean => WidgetType::Checkbox,
            Self::Date => WidgetType::DatePicker,
            Self::Time => WidgetType::TimePicker,
            Self::DateTime => WidgetType::DateTimePicker,
            Self::Uuid => WidgetType::TextInput,
            Self::Json => WidgetType::JsonEditor,
            Self::Binary => WidgetType::FileUpload,
            Self::Email => WidgetType::EmailInput,
            Self::Url => WidgetType::UrlInput,
            Self::IpAddress => WidgetType::TextInput,
            Self::Enum => WidgetType::Select,
            Self::ForeignKey => WidgetType::ForeignKey,
            Self::ManyToMany => WidgetType::MultiSelect,
        }
    }

    /// Get SQL type representation
    pub fn sql_type(&self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::BigInteger => "BIGINT",
            Self::Float => "REAL",
            Self::Decimal => "DECIMAL",
            Self::String => "VARCHAR",
            Self::Text => "TEXT",
            Self::Boolean => "BOOLEAN",
            Self::Date => "DATE",
            Self::Time => "TIME",
            Self::DateTime => "TIMESTAMP",
            Self::Uuid => "UUID",
            Self::Json => "JSON",
            Self::Binary => "BLOB",
            Self::Email => "VARCHAR",
            Self::Url => "VARCHAR",
            Self::IpAddress => "VARCHAR",
            Self::Enum => "VARCHAR",
            Self::ForeignKey => "INTEGER",
            Self::ManyToMany => "INTEGER",
        }
    }
}

/// Widget types for form rendering
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WidgetType {
    /// Text input
    TextInput,
    /// Password input
    PasswordInput,
    /// Email input
    EmailInput,
    /// URL input
    UrlInput,
    /// Number input
    NumberInput,
    /// Textarea
    Textarea,
    /// Rich text editor
    RichText,
    /// Checkbox
    Checkbox,
    /// Radio buttons
    Radio,
    /// Select dropdown
    Select,
    /// Multi-select
    MultiSelect,
    /// Date picker
    DatePicker,
    /// Time picker
    TimePicker,
    /// DateTime picker
    DateTimePicker,
    /// Color picker
    ColorPicker,
    /// File upload
    FileUpload,
    /// Image upload
    ImageUpload,
    /// Foreign key selector
    ForeignKey,
    /// JSON editor
    JsonEditor,
    /// Code editor
    CodeEditor,
    /// Hidden field
    Hidden,
    /// Read-only display
    ReadOnly,
}

/// Choice for select fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    /// Value stored
    pub value: String,
    /// Display label
    pub label: String,
    /// Is disabled?
    pub disabled: bool,
}

impl Choice {
    /// Create a new choice
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    /// Create from value (label = value)
    pub fn from_value(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            label: value.clone(),
            value,
            disabled: false,
        }
    }

    /// Set as disabled
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

/// Foreign key reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyRef {
    /// Related model name
    pub model: String,
    /// Field to display
    pub display_field: String,
}

/// Validator types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorType {
    /// Required field
    Required,
    /// Email format
    Email,
    /// URL format
    Url,
    /// Minimum length
    MinLength(usize),
    /// Maximum length
    MaxLength(usize),
    /// Minimum value
    MinValue(f64),
    /// Maximum value
    MaxValue(f64),
    /// Regex pattern
    Pattern(String),
    /// Custom validator name
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_field_definition() {
        let field = FieldDefinition::new("user_name", FieldType::String)
            .required()
            .searchable()
            .max_length(100);

        assert_eq!(field.name, "user_name");
        assert_eq!(field.label, "User Name");
        assert!(field.required);
        assert!(field.searchable);
        assert_eq!(field.max_length, Some(100));
    }

    #[test]
    fn test_choice() {
        let choice = Choice::new("active", "Active");
        assert_eq!(choice.value, "active");
        assert_eq!(choice.label, "Active");
        assert!(!choice.disabled);
    }

    #[test]
    fn test_field_type_widget() {
        assert_eq!(FieldType::String.default_widget(), WidgetType::TextInput);
        assert_eq!(FieldType::Boolean.default_widget(), WidgetType::Checkbox);
        assert_eq!(
            FieldType::DateTime.default_widget(),
            WidgetType::DateTimePicker
        );
    }
}
