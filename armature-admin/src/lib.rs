//! Admin Dashboard Generator for Armature Framework
//!
//! Auto-generates a complete CRUD admin interface from your models,
//! similar to Django Admin or Rails Admin.
//!
//! ## Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                    Admin Dashboard                               │
//! │                                                                  │
//! │  ┌──────────────────────────────────────────────────────────┐  │
//! │  │  Navigation    │  Content Area                           │  │
//! │  │  ───────────   │  ─────────────                          │  │
//! │  │  Dashboard     │  ┌────────────────────────────────────┐ │  │
//! │  │  Users         │  │  Users List                        │ │  │
//! │  │  Products      │  │  ─────────────────────────────────  │ │  │
//! │  │  Orders        │  │  [Search] [Filter] [+Add]          │ │  │
//! │  │  Settings      │  │  ┌────┬────────┬────────┬───────┐  │ │  │
//! │  │                │  │  │ ID │ Name   │ Email  │ Actions│  │ │  │
//! │  │                │  │  ├────┼────────┼────────┼───────┤  │ │  │
//! │  │                │  │  │ 1  │ Alice  │ a@...  │ ✏️ 🗑️ │  │ │  │
//! │  │                │  │  │ 2  │ Bob    │ b@...  │ ✏️ 🗑️ │  │ │  │
//! │  │                │  │  └────┴────────┴────────┴───────┘  │ │  │
//! │  │                │  │  [◀ Prev] Page 1 of 10 [Next ▶]   │ │  │
//! │  │                │  └────────────────────────────────────┘ │  │
//! │  └──────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_admin::{Admin, AdminModel, Field};
//!
//! #[derive(AdminModel)]
//! #[admin(list_display = ["id", "name", "email"])]
//! #[admin(search_fields = ["name", "email"])]
//! struct User {
//!     #[admin(primary_key)]
//!     id: i64,
//!     #[admin(required)]
//!     name: String,
//!     #[admin(widget = "email")]
//!     email: String,
//!     #[admin(readonly)]
//!     created_at: DateTime<Utc>,
//! }
//!
//! let admin = Admin::new()
//!     .title("My Admin")
//!     .register::<User>()
//!     .build();
//!
//! // Mount at /admin
//! app.mount("/admin", admin.routes());
//! ```

pub mod config;
pub mod dashboard;
pub mod error;
pub mod field;
pub mod model;
pub mod registry;
pub mod ui;
pub mod views;

pub use config::*;
pub use dashboard::*;
pub use error::*;
pub use field::*;
pub use model::*;
pub use registry::*;
pub use ui::*;
pub use views::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Admin instance builder
pub struct Admin {
    /// Admin configuration
    config: AdminConfig,
    /// Model registry
    registry: ModelRegistry,
}

impl Admin {
    /// Create a new admin builder
    pub fn new() -> Self {
        Self {
            config: AdminConfig::default(),
            registry: ModelRegistry::new(),
        }
    }

    /// Set the admin title
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.config.title = title.into();
        self
    }

    /// Set the base URL path
    pub fn base_path(mut self, path: impl Into<String>) -> Self {
        self.config.base_path = path.into();
        self
    }

    /// Set the theme
    pub fn theme(mut self, theme: Theme) -> Self {
        self.config.theme = theme;
        self
    }

    /// Set items per page
    pub fn items_per_page(mut self, count: usize) -> Self {
        self.config.items_per_page = count;
        self
    }

    /// Enable/disable authentication
    pub fn require_auth(mut self, required: bool) -> Self {
        self.config.require_auth = required;
        self
    }

    /// Register a model with the admin
    pub fn register_model(mut self, model: ModelDefinition) -> Self {
        self.registry.register(model);
        self
    }

    /// Build the admin instance
    pub fn build(self) -> AdminInstance {
        AdminInstance {
            config: Arc::new(self.config),
            registry: Arc::new(self.registry),
        }
    }
}

impl Default for Admin {
    fn default() -> Self {
        Self::new()
    }
}

/// Built admin instance
#[derive(Clone)]
pub struct AdminInstance {
    /// Configuration
    pub config: Arc<AdminConfig>,
    /// Model registry
    pub registry: Arc<ModelRegistry>,
}

impl AdminInstance {
    /// Get the configuration
    pub fn config(&self) -> &AdminConfig {
        &self.config
    }

    /// Get the model registry
    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }

    /// Generate routes for the admin interface
    pub fn routes(&self) -> AdminRoutes {
        AdminRoutes::new(self.clone())
    }

    /// Get a model definition by name
    pub fn get_model(&self, name: &str) -> Option<&ModelDefinition> {
        self.registry.get(name)
    }

    /// List all registered models
    pub fn models(&self) -> Vec<&ModelDefinition> {
        self.registry.all()
    }
}

/// Admin route handler
pub struct AdminRoutes {
    admin: AdminInstance,
}

impl AdminRoutes {
    /// Create new admin routes
    pub fn new(admin: AdminInstance) -> Self {
        Self { admin }
    }

    /// Get the base path
    pub fn base_path(&self) -> &str {
        &self.admin.config.base_path
    }

    /// Handle dashboard request
    pub async fn dashboard(&self) -> DashboardView {
        DashboardView::new(&self.admin)
    }

    /// Handle model list request
    pub async fn list(&self, model_name: &str, params: ListParams) -> Option<ListView> {
        self.admin
            .get_model(model_name)
            .map(|model| ListView::new(model, params))
    }

    /// Handle model detail/edit request
    pub async fn detail(&self, model_name: &str, id: &str) -> Option<DetailView> {
        self.admin
            .get_model(model_name)
            .map(|model| DetailView::new(model, id.to_string()))
    }

    /// Handle create request
    pub async fn create(&self, model_name: &str) -> Option<CreateView> {
        self.admin
            .get_model(model_name)
            .map(|model| CreateView::new(model))
    }
}

/// Parameters for list view
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListParams {
    /// Current page (1-indexed)
    pub page: Option<usize>,
    /// Items per page
    pub per_page: Option<usize>,
    /// Sort field
    pub sort: Option<String>,
    /// Sort direction
    pub order: Option<SortOrder>,
    /// Search query
    pub search: Option<String>,
    /// Filters
    pub filters: HashMap<String, String>,
}

impl ListParams {
    /// Get effective page number
    pub fn page(&self) -> usize {
        self.page.unwrap_or(1).max(1)
    }

    /// Get effective items per page
    pub fn per_page(&self, default: usize) -> usize {
        self.per_page.unwrap_or(default).min(100)
    }

    /// Get offset for pagination
    pub fn offset(&self, default_per_page: usize) -> usize {
        (self.page() - 1) * self.per_page(default_per_page)
    }
}

/// Sort order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    /// Get SQL representation
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }

    /// Toggle order
    pub fn toggle(&self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_builder() {
        let admin = Admin::new()
            .title("Test Admin")
            .base_path("/admin")
            .items_per_page(25)
            .build();

        assert_eq!(admin.config.title, "Test Admin");
        assert_eq!(admin.config.base_path, "/admin");
        assert_eq!(admin.config.items_per_page, 25);
    }

    #[test]
    fn test_list_params() {
        let params = ListParams {
            page: Some(2),
            per_page: Some(20),
            ..Default::default()
        };

        assert_eq!(params.page(), 2);
        assert_eq!(params.per_page(10), 20);
        assert_eq!(params.offset(10), 20);
    }

    #[test]
    fn test_sort_order() {
        assert_eq!(SortOrder::Asc.toggle(), SortOrder::Desc);
        assert_eq!(SortOrder::Desc.toggle(), SortOrder::Asc);
    }
}
