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
pub mod data;
pub mod error;
pub mod field;
pub mod model;
pub mod registry;
pub mod render;
pub mod ui;
pub mod views;

pub use config::*;
pub use dashboard::*;
pub use data::{DataPage, DataQuery, DataSource, InMemoryDataSource};
pub use error::*;
pub use field::*;
pub use model::*;
pub use registry::*;
pub use ui::*;
pub use views::*;

use armature_core::{Error, HttpRequest, HttpResponse, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Admin instance builder
pub struct Admin {
    /// Admin configuration
    config: AdminConfig,
    /// Model registry
    registry: ModelRegistry,
    /// Backing data source
    data_source: Arc<dyn DataSource>,
}

impl Admin {
    /// Create a new admin builder
    pub fn new() -> Self {
        Self {
            config: AdminConfig::default(),
            registry: ModelRegistry::new(),
            data_source: Arc::new(InMemoryDataSource::new()),
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

    /// Set the maximum items-per-page a client may request
    pub fn max_items_per_page(mut self, count: usize) -> Self {
        self.config.max_items_per_page = count;
        self
    }

    /// Enable/disable authentication
    pub fn require_auth(mut self, required: bool) -> Self {
        self.config.require_auth = required;
        self
    }

    /// Set the backing data source (defaults to an in-memory store)
    pub fn data_source(mut self, source: Arc<dyn DataSource>) -> Self {
        self.data_source = source;
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
            data_source: self.data_source,
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
    /// Backing data source
    pub data_source: Arc<dyn DataSource>,
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

    /// Get the backing data source
    pub fn data_source(&self) -> &Arc<dyn DataSource> {
        &self.data_source
    }

    /// Build a mountable [`armature_core::Router`] for the admin interface.
    ///
    /// Every route renders HTML through the view structs and is backed by the
    /// configured [`DataSource`]. When `require_auth` is set, mutating and
    /// browsing routes require an `Authorization` header.
    ///
    /// Registered routes (relative to `base_path`, default `/admin`):
    /// - `GET  {base}` — dashboard
    /// - `GET  {base}/:model` — list
    /// - `GET  {base}/:model/add` + `POST {base}/:model/add` — create
    /// - `GET  {base}/:model/:id` — detail
    /// - `GET  {base}/:model/:id/edit` + `POST {base}/:model/:id/edit` — update
    /// - `POST {base}/:model/:id/delete` (also `DELETE {base}/:model/:id`) — delete
    pub fn routes(&self) -> Router {
        let base = self.config.base_path.trim_end_matches('/').to_string();
        let inst = Arc::new(self.clone());
        let mut router = Router::new();

        macro_rules! h {
            ($f:ident) => {{
                let inst = inst.clone();
                move |req: HttpRequest| {
                    let inst = inst.clone();
                    async move { $f(inst, req).await }
                }
            }};
        }

        router.get(base.clone(), h!(handle_dashboard));
        router.get(format!("{base}/:model"), h!(handle_list));
        // Static `/add` and `/:id/edit` must precede the `:id` param routes so
        // they are matched first (the router returns the first matching route).
        router.get(format!("{base}/:model/add"), h!(handle_create_form));
        router.post(format!("{base}/:model/add"), h!(handle_create_submit));
        router.get(format!("{base}/:model/:id/edit"), h!(handle_edit_form));
        router.post(format!("{base}/:model/:id/edit"), h!(handle_update_submit));
        router.post(format!("{base}/:model/:id/delete"), h!(handle_delete));
        router.delete(format!("{base}/:model/:id"), h!(handle_delete));
        router.get(format!("{base}/:model/:id"), h!(handle_detail));

        router
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

/// Returns `true` if the request may proceed given the auth policy.
///
/// # This is a PRESENCE check, not authentication
///
/// When `require_auth` is disabled every request passes. When it is enabled this
/// function only checks that a **non-empty `Authorization` header is present** —
/// it does **not** parse, validate, or verify the credential in any way. Any
/// non-empty value (e.g. `Authorization: x`) is accepted.
///
/// This is deliberate: the admin router does not implement authentication. Real
/// verification (validating a session, bearer token, or signature and rejecting
/// forged/expired credentials) MUST be performed by upstream authentication
/// middleware mounted ahead of these routes. Enabling `require_auth(true)`
/// without such middleware provides **no security** — it merely rejects requests
/// that omit the header entirely.
fn is_authorized(config: &AdminConfig, req: &HttpRequest) -> bool {
    if !config.require_auth {
        return true;
    }
    req.headers
        .get_ignore_case("authorization")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// Enforce the auth policy and resolve the `:model` path param to its
/// [`ModelDefinition`], centralizing the identical 401/404 handling that opens
/// every model-scoped handler.
///
/// Returns the resolved model on success, or the ready-to-return error response
/// (401 when unauthorized, 404 when the model is unknown).
fn authorize_and_resolve_model(
    inst: &AdminInstance,
    req: &HttpRequest,
) -> Result<ModelDefinition, Box<HttpResponse>> {
    if !is_authorized(&inst.config, req) {
        return Err(Box::new(html_response(
            401,
            render::render_unauthorized(&inst.config),
        )));
    }
    let model_name = req.param("model").cloned().unwrap_or_default();
    match inst.get_model(&model_name) {
        Some(m) => Ok(m.clone()),
        None => Err(Box::new(html_response(
            404,
            render::render_not_found(&inst.config),
        ))),
    }
}

/// Build [`ListParams`] from the request's query string.
fn list_params_from_request(req: &HttpRequest) -> ListParams {
    let get = |k: &str| req.query_param(k);
    let mut filters = HashMap::new();
    for (k, v) in req.query().iter() {
        if let Some(field) = k.strip_prefix("filter.") {
            filters.insert(field.to_string(), v.to_owned());
        }
    }
    ListParams {
        page: get("page").and_then(|p| p.parse().ok()),
        per_page: get("per_page").and_then(|p| p.parse().ok()),
        sort: get("sort").map(str::to_owned),
        order: match get("order") {
            Some("desc") | Some("DESC") => Some(SortOrder::Desc),
            Some("asc") | Some("ASC") => Some(SortOrder::Asc),
            _ => None,
        },
        search: get("search").map(str::to_owned),
        filters,
    }
}

/// Parse a submitted form (or JSON body) into a JSON object for the data source.
fn parse_body(req: &HttpRequest) -> serde_json::Value {
    let is_json = req
        .headers
        .get_ignore_case("content-type")
        .map(|ct| ct.contains("application/json"))
        .unwrap_or(false);

    if is_json && let Ok(value) = req.json::<serde_json::Value>() {
        return value;
    }

    if let Ok(map) = req.form_map() {
        let obj: serde_json::Map<String, serde_json::Value> = map
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect();
        return serde_json::Value::Object(obj);
    }

    serde_json::Value::Object(Default::default())
}

fn html_response(status: u16, body: String) -> HttpResponse {
    let mut resp = HttpResponse::html(body);
    resp.status = status;
    resp
}

/// Quote a single CSV field per RFC 4180 (double internal quotes; wrap in quotes
/// when the value contains a comma, quote, or newline).
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Render the model's `list_display` columns for the given rows as CSV text.
///
/// The header row uses the display fields' labels; each data row emits the
/// plain (unescaped) value of each display field.
fn render_list_csv(model: &ModelDefinition, rows: &[serde_json::Value]) -> String {
    let fields = model.display_fields();
    let mut out = String::new();

    let header: Vec<String> = fields.iter().map(|f| csv_field(&f.label)).collect();
    out.push_str(&header.join(","));
    out.push_str("\r\n");

    for row in rows {
        let cells: Vec<String> = fields
            .iter()
            .map(|f| {
                let raw = row
                    .get(&f.name)
                    .map(data::value_to_plain_string)
                    .unwrap_or_default();
                csv_field(&raw)
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }

    out
}

async fn handle_dashboard(
    inst: Arc<AdminInstance>,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    if !is_authorized(&inst.config, &req) {
        return Ok(html_response(
            401,
            render::render_unauthorized(&inst.config),
        ));
    }
    let mut view = DashboardView::new(&inst);
    // Populate live counts from the data source.
    let mut total = 0usize;
    for summary in &mut view.model_summaries {
        if let Some(model) = inst.get_model(&summary.name) {
            let count = inst.data_source.count(model).await;
            summary.count = count;
            total += count;
        }
    }
    if let Some(first) = view.stats.first_mut() {
        first.value = total.to_string();
    }
    Ok(HttpResponse::html(render::render_dashboard(
        &view,
        &inst.config,
    )))
}

async fn handle_list(inst: Arc<AdminInstance>, req: HttpRequest) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };

    let params = list_params_from_request(&req);
    let per_page = params.resolve_per_page(&inst.config);
    let offset = params.resolve_offset(per_page);

    // Ordering clauses, honoring the sort param else the model default.
    let order_by = if let Some(sort) = &params.sort {
        vec![format!(
            "{} {}",
            sort,
            params.order.unwrap_or_default().as_sql()
        )]
    } else {
        model.ordering.iter().map(|o| o.as_sql()).collect()
    };

    // CSV export of the current query (honoring search/filters/ordering). A
    // `limit` of 0 tells the data source to return every matching row rather
    // than a single page.
    if inst.config.enable_export && req.query_param("export") == Some("csv") {
        let query = DataQuery {
            offset: 0,
            limit: 0,
            order_by,
            search: params.search.clone(),
            filters: params.filters.clone(),
        };
        let page = inst.data_source.list(&model, &query).await;
        let csv = render_list_csv(&model, &page.rows);
        return Ok(HttpResponse::new(200)
            .with_header(
                "Content-Type".to_string(),
                "text/csv; charset=utf-8".to_string(),
            )
            .with_header(
                "Content-Disposition".to_string(),
                format!("attachment; filename=\"{}.csv\"", model.name),
            )
            .with_body(csv.into_bytes()));
    }

    let query = DataQuery {
        offset,
        limit: per_page,
        order_by,
        search: params.search.clone(),
        filters: params.filters.clone(),
    };

    let page = inst.data_source.list(&model, &query).await;
    let view = ListView::new(&model, params, per_page).with_json_rows(
        &model,
        &inst.config,
        &page.rows,
        page.total,
    );
    Ok(HttpResponse::html(render::render_list(&view, &inst.config)))
}

async fn handle_detail(inst: Arc<AdminInstance>, req: HttpRequest) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };
    let id = req.param("id").cloned().unwrap_or_default();

    match inst.data_source.get(&model, &id).await {
        Some(record) => {
            let view = DetailView::new(&model, id).with_data(record);
            Ok(HttpResponse::html(render::render_detail(
                &view,
                &inst.config,
            )))
        }
        None => Ok(html_response(404, render::render_not_found(&inst.config))),
    }
}

async fn handle_create_form(
    inst: Arc<AdminInstance>,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };
    let view = CreateView::new(&model);
    Ok(HttpResponse::html(render::render_create(
        &view,
        &inst.config,
    )))
}

async fn handle_create_submit(
    inst: Arc<AdminInstance>,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };
    if !model.can_add {
        return Ok(html_response(
            403,
            render::render_unauthorized(&inst.config),
        ));
    }
    let data = parse_body(&req);
    match inst.data_source.create(&model, data).await {
        Ok(id) => Ok(HttpResponse::redirect(format!(
            "{}/{}/{}",
            inst.config.base_path, model.name, id
        ))),
        Err(e) => Ok(html_response(
            400,
            render::render_error(&inst.config, &e.to_string()),
        )),
    }
}

async fn handle_edit_form(
    inst: Arc<AdminInstance>,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };
    let id = req.param("id").cloned().unwrap_or_default();
    match inst.data_source.get(&model, &id).await {
        Some(record) => {
            let view = EditView::new(&model, id).with_data(record);
            Ok(HttpResponse::html(render::render_edit(&view, &inst.config)))
        }
        None => Ok(html_response(404, render::render_not_found(&inst.config))),
    }
}

async fn handle_update_submit(
    inst: Arc<AdminInstance>,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };
    let id = req.param("id").cloned().unwrap_or_default();
    if !model.can_edit {
        return Ok(html_response(
            403,
            render::render_unauthorized(&inst.config),
        ));
    }
    let data = parse_body(&req);
    match inst.data_source.update(&model, &id, data).await {
        Ok(()) => Ok(HttpResponse::redirect(format!(
            "{}/{}/{}",
            inst.config.base_path, model.name, id
        ))),
        Err(e) => Ok(html_response(
            400,
            render::render_error(&inst.config, &e.to_string()),
        )),
    }
}

async fn handle_delete(inst: Arc<AdminInstance>, req: HttpRequest) -> Result<HttpResponse, Error> {
    let model = match authorize_and_resolve_model(&inst, &req) {
        Ok(m) => m,
        Err(resp) => return Ok(*resp),
    };
    let id = req.param("id").cloned().unwrap_or_default();
    if !model.can_delete {
        return Ok(html_response(
            403,
            render::render_unauthorized(&inst.config),
        ));
    }
    match inst.data_source.delete(&model, &id).await {
        Ok(()) => Ok(HttpResponse::redirect(format!(
            "{}/{}",
            inst.config.base_path, model.name
        ))),
        Err(e) => Ok(html_response(
            404,
            render::render_error(&inst.config, &e.to_string()),
        )),
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

    /// Resolve the effective page size, honoring the requested `per_page`, the
    /// config default (`items_per_page`), and the cap (`max_items_per_page`).
    pub fn resolve_per_page(&self, config: &AdminConfig) -> usize {
        let cap = config.max_items_per_page.max(1);
        self.per_page.unwrap_or(config.items_per_page).clamp(1, cap)
    }

    /// Compute the row offset for a resolved page size.
    pub fn resolve_offset(&self, per_page: usize) -> usize {
        (self.page() - 1) * per_page
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
        assert_eq!(params.resolve_per_page(&AdminConfig::default()), 20);
        assert_eq!(params.resolve_offset(20), 20);
    }

    #[test]
    fn test_sort_order() {
        assert_eq!(SortOrder::Asc.toggle(), SortOrder::Desc);
        assert_eq!(SortOrder::Desc.toggle(), SortOrder::Asc);
    }

    // ---- Router regression tests (fail against the pre-fix code) ----

    use armature_core::HttpRequest;

    fn user_model() -> ModelDefinition {
        ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String).searchable())
            .search_fields(["name"])
            .list_display(["id", "name"])
            .build()
    }

    fn req(method: &str, path: &str) -> HttpRequest {
        HttpRequest::new(method.to_string(), path.to_string())
    }

    /// `routes()` must return a real router with registered, responding routes.
    #[tokio::test]
    async fn routes_are_registered_and_respond() {
        let admin = Admin::new()
            .require_auth(false)
            .register_model(user_model())
            .build();

        let router = admin.routes();
        assert!(
            router.routes.len() >= 6,
            "expected multiple registered routes, got {}",
            router.routes.len()
        );

        // Dashboard, list, and create form all respond with 200 HTML.
        for path in ["/admin", "/admin/user", "/admin/user/add"] {
            let resp = router.route(req("GET", path)).await.unwrap();
            assert_eq!(resp.status, 200, "route {path} should respond 200");
            assert!(!resp.body.is_empty(), "route {path} should have a body");
        }
    }

    /// A stub data source must populate list rows (was hardcoded `Vec::new()`).
    #[tokio::test]
    async fn stub_data_source_populates_rows() {
        let ds = Arc::new(InMemoryDataSource::new());
        ds.seed("user", serde_json::json!({ "id": 1, "name": "Alice" }));
        ds.seed("user", serde_json::json!({ "id": 2, "name": "Bob" }));

        let admin = Admin::new()
            .require_auth(false)
            .data_source(ds.clone())
            .register_model(user_model())
            .build();

        let resp = admin
            .routes()
            .route(req("GET", "/admin/user"))
            .await
            .unwrap();
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        assert!(
            body.contains("Alice"),
            "list body should contain seeded rows"
        );
        assert!(body.contains("Bob"));
    }

    /// Pagination must honor `per_page` and cap at `max_items_per_page`.
    #[tokio::test]
    async fn pagination_respects_per_page_and_max() {
        let ds = Arc::new(InMemoryDataSource::new());
        for i in 0..30 {
            ds.seed(
                "user",
                serde_json::json!({ "id": i, "name": format!("u{i}") }),
            );
        }

        let admin = Admin::new()
            .require_auth(false)
            .items_per_page(10)
            .max_items_per_page(5)
            .data_source(ds.clone())
            .register_model(user_model())
            .build();
        let router = admin.routes();

        // Requesting per_page=3 yields 3 rows.
        let resp = router
            .route(req("GET", "/admin/user?per_page=3"))
            .await
            .unwrap();
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        let row_count = body.matches("<tr>").count() - 1; // minus header row
        assert_eq!(row_count, 3, "per_page=3 must return 3 rows");

        // Requesting per_page=1000 is capped at max_items_per_page (5).
        let resp = router
            .route(req("GET", "/admin/user?per_page=1000"))
            .await
            .unwrap();
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        let row_count = body.matches("<tr>").count() - 1;
        assert_eq!(
            row_count, 5,
            "per_page must be capped at max_items_per_page"
        );
    }

    /// `require_auth` must guard routes.
    #[tokio::test]
    async fn require_auth_guards_routes() {
        let admin = Admin::new()
            .require_auth(true)
            .register_model(user_model())
            .build();
        let router = admin.routes();

        // No Authorization header -> 401.
        let resp = router.route(req("GET", "/admin/user")).await.unwrap();
        assert_eq!(resp.status, 401, "unauthenticated request must be blocked");

        // With Authorization header -> 200.
        let mut authed = req("GET", "/admin/user");
        authed.headers.insert("Authorization", "Bearer token");
        let resp = router.route(authed).await.unwrap();
        assert_eq!(resp.status, 200, "authenticated request must pass");
    }

    /// Update and delete handlers must mutate the data source.
    #[tokio::test]
    async fn update_and_delete_handlers_work() {
        let ds = Arc::new(InMemoryDataSource::new());
        ds.seed("user", serde_json::json!({ "id": "1", "name": "Alice" }));

        let admin = Admin::new()
            .require_auth(false)
            .data_source(ds.clone())
            .register_model(user_model())
            .build();
        let router = admin.routes();

        // Update via POST form body.
        let mut update = req("POST", "/admin/user/1/edit");
        update.set_body(b"name=Renamed".to_vec());
        let resp = router.route(update).await.unwrap();
        assert!(
            (300..400).contains(&resp.status),
            "successful update should redirect, got {}",
            resp.status
        );
        assert_eq!(ds.get(&user_model(), "1").await.unwrap()["name"], "Renamed");

        // Delete via POST.
        let resp = router
            .route(req("POST", "/admin/user/1/delete"))
            .await
            .unwrap();
        assert!((300..400).contains(&resp.status));
        assert!(ds.get(&user_model(), "1").await.is_none());
    }

    /// End-to-end: the search box rendered on the list page must drive filtering
    /// when its form is submitted. This ties the *rendered form's* param name to
    /// the handler — the exact seam where `name="q"` silently broke search.
    #[tokio::test]
    async fn search_form_param_filters_end_to_end() {
        let ds = Arc::new(InMemoryDataSource::new());
        ds.seed("user", serde_json::json!({ "id": "1", "name": "Alice" }));
        ds.seed("user", serde_json::json!({ "id": "2", "name": "Bob" }));

        let admin = Admin::new()
            .require_auth(false)
            .data_source(ds.clone())
            .register_model(user_model())
            .build();
        let router = admin.routes();

        // Render the list page and pull the search input's `name` straight out of
        // the emitted HTML, exactly as a browser would submit it.
        let resp = router.route(req("GET", "/admin/user")).await.unwrap();
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        let form = body
            .split("admin-search")
            .nth(1)
            .expect("search form must be rendered");
        let after = form
            .split("name=\"")
            .nth(1)
            .expect("input must have a name");
        let param = &after[..after.find('"').unwrap()];
        assert_eq!(
            param, "search",
            "rendered search input name must match the handler's query key"
        );

        // Submit that param and assert real filtering occurs.
        let path = format!("/admin/user?{param}=Alice");
        let resp = router.route(req("GET", &path)).await.unwrap();
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        assert!(body.contains("Alice"), "search must keep the matching row");
        assert!(
            !body.contains("Bob"),
            "search must filter out non-matching rows"
        );
    }

    /// The Export control must produce real CSV of the current query, not just
    /// re-render HTML.
    #[tokio::test]
    async fn export_csv_returns_csv_rows() {
        let ds = Arc::new(InMemoryDataSource::new());
        ds.seed("user", serde_json::json!({ "id": "1", "name": "Alice" }));
        ds.seed("user", serde_json::json!({ "id": "2", "name": "Bob" }));

        let admin = Admin::new()
            .require_auth(false)
            .data_source(ds.clone())
            .register_model(user_model())
            .build();
        let router = admin.routes();

        let resp = router
            .route(req("GET", "/admin/user?export=csv"))
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        let ct = resp
            .headers
            .get("Content-Type")
            .cloned()
            .unwrap_or_default();
        assert!(ct.contains("text/csv"), "export must be served as CSV");
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        assert!(
            !body.contains("<table"),
            "CSV export must not re-render HTML"
        );
        // Header row (labels) + both data rows.
        assert!(body.contains("Alice"));
        assert!(body.contains("Bob"));
        assert!(body.lines().count() >= 3, "header + two data rows");

        // Export honors the active search filter.
        let resp = router
            .route(req("GET", "/admin/user?export=csv&search=Alice"))
            .await
            .unwrap();
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        assert!(body.contains("Alice"));
        assert!(
            !body.contains("Bob"),
            "CSV export must honor the search query"
        );
    }

    /// The edit route must render an EditView (CRUD is wired, not just claimed).
    #[tokio::test]
    async fn edit_form_renders() {
        let ds = Arc::new(InMemoryDataSource::new());
        ds.seed("user", serde_json::json!({ "id": "1", "name": "Alice" }));

        let admin = Admin::new()
            .require_auth(false)
            .data_source(ds.clone())
            .register_model(user_model())
            .build();

        let resp = admin
            .routes()
            .route(req("GET", "/admin/user/1/edit"))
            .await
            .unwrap();
        assert_eq!(resp.status, 200);
        let body = String::from_utf8(resp.body.to_vec()).unwrap();
        assert!(body.contains("Alice"), "edit form should prefill values");
        assert!(body.contains("<form"), "edit form should render a form");
    }
}
