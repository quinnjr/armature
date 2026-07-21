//! HTML rendering for admin views.
//!
//! These functions turn the view data structures (built from the model
//! registry + [`crate::data::DataSource`]) into complete HTML documents so the
//! router handlers can respond with real pages.

use crate::config::AdminConfig;
use crate::dashboard::DashboardView;
use crate::ui::generate_admin_css;
use crate::views::{CreateView, DetailView, EditView, FormField, ListView};

/// Escape a string for safe inclusion in HTML text/attributes.
pub(crate) fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Wrap body content in a full HTML document with the themed admin stylesheet.
fn page(config: &AdminConfig, title: &str, body: &str) -> String {
    // Favicon link when configured.
    let favicon = match &config.favicon_url {
        Some(url) => format!("<link rel=\"icon\" href=\"{}\">\n", escape(url)),
        None => String::new(),
    };

    // Optional operator-supplied CSS, injected after the themed stylesheet so it
    // can override the defaults.
    let custom_css = match &config.custom_css {
        Some(css) => format!("<style>{css}</style>\n"),
        None => String::new(),
    };

    // Header logo when configured.
    let logo = match &config.logo_url {
        Some(url) => format!(
            "<img class=\"admin-logo\" src=\"{}\" alt=\"{}\">",
            escape(url),
            escape(&config.title)
        ),
        None => String::new(),
    };

    // Page footer when configured.
    let footer = match &config.footer_text {
        Some(text) => format!("\n<footer class=\"admin-footer\">{}</footer>", escape(text)),
        None => String::new(),
    };

    // Optional operator-supplied JS, injected near the end of the body.
    let custom_js = match &config.custom_js {
        Some(js) => format!("\n<script>{js}</script>"),
        None => String::new(),
    };

    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} · {app}</title>\n{favicon}<style>{css}</style>\n{custom_css}</head>\n\
         <body>\n<header class=\"admin-header\">{logo}</header>\n\
         <div class=\"admin-content\" data-admin-root>\n{body}\n</div>{footer}{custom_js}\n</body>\n</html>",
        title = escape(title),
        app = escape(&config.title),
        favicon = favicon,
        css = generate_admin_css(&config.theme),
        custom_css = custom_css,
        logo = logo,
        body = body,
        footer = footer,
        custom_js = custom_js,
    )
}

fn breadcrumbs_html(crumbs: &[crate::ui::Breadcrumb]) -> String {
    let items: Vec<String> = crumbs
        .iter()
        .map(|c| match &c.url {
            Some(url) => format!("<a href=\"{}\">{}</a>", escape(url), escape(&c.label)),
            None => format!("<span>{}</span>", escape(&c.label)),
        })
        .collect();
    format!(
        "<nav class=\"admin-breadcrumbs\">{}</nav>",
        items.join(" / ")
    )
}

/// Render the dashboard page.
pub fn render_dashboard(view: &DashboardView, config: &AdminConfig) -> String {
    let stats: Vec<String> = view
        .stats
        .iter()
        .map(|s| {
            format!(
                "<div class=\"admin-card admin-stat\"><span class=\"stat-title\">{}</span>\
                 <span class=\"stat-value\">{}</span></div>",
                escape(&s.title),
                escape(&s.value)
            )
        })
        .collect();

    let models: Vec<String> = view
        .model_summaries
        .iter()
        .map(|m| {
            format!(
                "<li><a href=\"{}\">{}</a> <span class=\"count\">{}</span></li>",
                escape(&m.url),
                escape(&m.verbose_name),
                m.count
            )
        })
        .collect();

    let actions: Vec<String> = view
        .quick_actions
        .iter()
        .map(|a| {
            format!(
                "<a class=\"admin-btn admin-btn-primary\" href=\"{}\">{}</a>",
                escape(&a.url),
                escape(&a.label)
            )
        })
        .collect();

    let body = format!(
        "<h1>{title}</h1>\n<div class=\"admin-stats-row\">{stats}</div>\n\
         <div class=\"admin-quick-actions\">{actions}</div>\n\
         <h2>Models</h2>\n<ul class=\"admin-model-list\">{models}</ul>",
        title = escape(&view.title),
        stats = stats.join(""),
        actions = actions.join(""),
        models = models.join(""),
    );
    page(config, &view.title, &body)
}

/// Render a model list page (table of rows + pagination).
pub fn render_list(view: &ListView, config: &AdminConfig) -> String {
    let headers: Vec<String> = view
        .columns
        .iter()
        .map(|c| format!("<th>{}</th>", escape(&c.label)))
        .collect();

    let rows: Vec<String> = view
        .rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = row
                .cells
                .iter()
                .map(|cell| format!("<td>{}</td>", cell.rendered))
                .collect();
            let view_url = format!(
                "{}/{}/{}",
                config.base_path,
                view.model_name,
                escape(&row.id)
            );
            format!(
                "<tr>{}<td><a href=\"{}\">View</a></td></tr>",
                cells.join(""),
                view_url
            )
        })
        .collect();

    let rows_html = if rows.is_empty() {
        format!(
            "<tr><td colspan=\"{}\" class=\"admin-empty\">No records</td></tr>",
            view.columns.len() + 1
        )
    } else {
        rows.join("")
    };

    let p = &view.pagination;
    let pagination = format!(
        "<div class=\"admin-pagination\">Page {} of {} · {} items (showing {}–{})</div>",
        p.page, p.total_pages, p.total_items, p.start_item, p.end_item
    );

    let add = if view.can_add {
        format!(
            "<a class=\"admin-btn admin-btn-primary\" href=\"{}\">Add {}</a>",
            escape(&view.add_url),
            escape(&view.verbose_name)
        )
    } else {
        String::new()
    };

    // The list route this view was rendered from, used as the target for the
    // search form and export link.
    let list_url = format!("{}/{}", config.base_path, view.model_name);

    // A search box that posts back to the list route with a `?q=` query param.
    let search = if config.enable_search {
        format!(
            "<form class=\"admin-search\" method=\"get\" action=\"{}\">\
             <input class=\"admin-input\" type=\"search\" name=\"q\" placeholder=\"Search\">\
             <button class=\"admin-btn\" type=\"submit\">Search</button></form>",
            escape(&list_url)
        )
    } else {
        String::new()
    };

    // An export link pointing at the list route with `?export=csv`.
    let export = if config.enable_export {
        format!(
            "<a class=\"admin-btn admin-export\" href=\"{}?export=csv\">Export</a>",
            escape(&list_url)
        )
    } else {
        String::new()
    };

    let body = format!(
        "{crumbs}\n<div class=\"admin-list-header\"><h1>{title}</h1>{search}{export}{add}</div>\n\
         <table class=\"admin-table\"><thead><tr>{headers}<th>Actions</th></tr></thead>\
         <tbody>{rows}</tbody></table>\n{pagination}",
        crumbs = breadcrumbs_html(&view.breadcrumbs),
        title = escape(&view.title),
        search = search,
        export = export,
        add = add,
        headers = headers.join(""),
        rows = rows_html,
        pagination = pagination,
    );
    page(config, &view.title, &body)
}

/// Render a record detail page.
pub fn render_detail(view: &DetailView, config: &AdminConfig) -> String {
    let fields: Vec<String> = view
        .fields
        .iter()
        .map(|f| {
            format!(
                "<div class=\"admin-field\"><span class=\"field-label\">{}</span>\
                 <span class=\"field-value\">{}</span></div>",
                escape(&f.label),
                f.rendered
            )
        })
        .collect();

    let fields_html = if fields.is_empty() {
        "<p class=\"admin-empty\">No data</p>".to_string()
    } else {
        fields.join("")
    };

    let mut actions = format!(
        "<a class=\"admin-btn\" href=\"{}\">Back to list</a>",
        escape(&view.list_url)
    );
    if view.can_edit {
        actions.push_str(&format!(
            "<a class=\"admin-btn admin-btn-primary\" href=\"{}\">Edit</a>",
            escape(&view.edit_url)
        ));
    }
    if view.can_delete {
        actions.push_str(&format!(
            "<form method=\"post\" action=\"{}\" class=\"admin-inline-form\">\
             <button class=\"admin-btn admin-btn-danger\" type=\"submit\">Delete</button></form>",
            escape(&view.delete_url)
        ));
    }

    let body = format!(
        "{crumbs}\n<h1>{title}</h1>\n<div class=\"admin-card\">{fields}</div>\n\
         <div class=\"admin-actions\">{actions}</div>",
        crumbs = breadcrumbs_html(&view.breadcrumbs),
        title = escape(&view.title),
        fields = fields_html,
        actions = actions,
    );
    page(config, &view.title, &body)
}

fn form_input(field: &FormField) -> String {
    let value = match &field.value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => escape(s),
        other => escape(&other.to_string()),
    };
    let required = if field.required { " required" } else { "" };
    let input = if field.widget.contains("textarea") {
        format!(
            "<textarea class=\"admin-input\" name=\"{}\"{}>{}</textarea>",
            escape(&field.name),
            required,
            value
        )
    } else {
        format!(
            "<input class=\"admin-input\" name=\"{}\" value=\"{}\"{}>",
            escape(&field.name),
            value,
            required
        )
    };
    format!(
        "<div class=\"admin-form-field\"><label>{}</label>{}</div>",
        escape(&field.label),
        input
    )
}

/// Render the create form.
pub fn render_create(view: &CreateView, config: &AdminConfig) -> String {
    let fields: Vec<String> = view.fields.iter().map(form_input).collect();
    let body = format!(
        "{crumbs}\n<h1>{title}</h1>\n\
         <form method=\"post\" action=\"{action}\" class=\"admin-card admin-form\">\
         {fields}<div class=\"admin-actions\">\
         <a class=\"admin-btn\" href=\"{cancel}\">Cancel</a>\
         <button class=\"admin-btn admin-btn-primary\" type=\"submit\">Save</button></div></form>",
        crumbs = breadcrumbs_html(&view.breadcrumbs),
        title = escape(&view.title),
        action = escape(&view.submit_url),
        fields = fields.join(""),
        cancel = escape(&view.cancel_url),
    );
    page(config, &view.title, &body)
}

/// Render the edit form.
pub fn render_edit(view: &EditView, config: &AdminConfig) -> String {
    let fields: Vec<String> = view.fields.iter().map(form_input).collect();
    let delete = if view.can_delete {
        format!(
            "<form method=\"post\" action=\"{}\" class=\"admin-inline-form\">\
             <button class=\"admin-btn admin-btn-danger\" type=\"submit\">Delete</button></form>",
            escape(&view.delete_url)
        )
    } else {
        String::new()
    };
    let body = format!(
        "{crumbs}\n<h1>{title}</h1>\n\
         <form method=\"post\" action=\"{action}\" class=\"admin-card admin-form\">\
         {fields}<div class=\"admin-actions\">\
         <a class=\"admin-btn\" href=\"{cancel}\">Cancel</a>\
         <button class=\"admin-btn admin-btn-primary\" type=\"submit\">Save</button></div></form>\n{delete}",
        crumbs = breadcrumbs_html(&view.breadcrumbs),
        title = escape(&view.title),
        action = escape(&view.submit_url),
        fields = fields.join(""),
        cancel = escape(&view.cancel_url),
        delete = delete,
    );
    page(config, &view.title, &body)
}

/// A minimal unauthorized page (used when `require_auth` blocks a request).
pub fn render_unauthorized(config: &AdminConfig) -> String {
    page(
        config,
        "Unauthorized",
        "<h1>401 Unauthorized</h1><p>Authentication is required to access this admin dashboard.</p>",
    )
}

/// A minimal error page for failed mutations.
pub fn render_error(config: &AdminConfig, message: &str) -> String {
    page(
        config,
        "Error",
        &format!("<h1>Request failed</h1><p>{}</p>", escape(message)),
    )
}

/// A minimal not-found page.
pub fn render_not_found(config: &AdminConfig) -> String {
    page(
        config,
        "Not Found",
        "<h1>404 Not Found</h1><p>The requested resource does not exist.</p>",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ListParams;
    use crate::field::{FieldDefinition, FieldType};
    use crate::model::ModelDefinition;

    fn sample_list_view() -> ListView {
        let model = ModelDefinition::builder("user")
            .id_field()
            .field(FieldDefinition::new("name", FieldType::String))
            .list_display(["id", "name"])
            .build();
        ListView::new(&model, ListParams::default(), 25)
    }

    /// The display/config knobs must be genuinely consumed by rendering: when
    /// set they appear in the HTML, when unset they are absent.
    #[test]
    fn test_display_knobs_wired_into_html() {
        let config = AdminConfig {
            logo_url: Some("https://cdn.example.com/logo.png".to_string()),
            favicon_url: Some("https://cdn.example.com/favicon.ico".to_string()),
            custom_css: Some(".admin-header{background:hotpink}".to_string()),
            custom_js: Some("console.log('admin loaded');".to_string()),
            footer_text: Some("© 2026 Example Corp".to_string()),
            enable_search: true,
            enable_export: true,
            ..AdminConfig::default()
        };

        let html = render_list(&sample_list_view(), &config);

        // Favicon + logo.
        assert!(html.contains("<link rel=\"icon\" href=\"https://cdn.example.com/favicon.ico\">"));
        assert!(html.contains("src=\"https://cdn.example.com/logo.png\""));
        assert!(html.contains("admin-logo"));
        // Custom CSS + JS.
        assert!(html.contains(".admin-header{background:hotpink}"));
        assert!(html.contains("console.log('admin loaded');"));
        // Footer.
        assert!(html.contains("admin-footer"));
        assert!(html.contains("© 2026 Example Corp"));
        // Search + export controls on the list view.
        assert!(html.contains("admin-search"));
        assert!(html.contains("name=\"q\""));
        assert!(html.contains("admin-export"));
        assert!(html.contains("?export=csv"));
    }

    /// Unset knobs must not leak markup into the page.
    #[test]
    fn test_display_knobs_absent_when_unset() {
        let config = AdminConfig {
            enable_search: false,
            enable_export: false,
            ..AdminConfig::default()
        };
        // Defaults leave logo/favicon/css/js/footer as None.

        let html = render_list(&sample_list_view(), &config);

        assert!(!html.contains("<link rel=\"icon\""));
        assert!(!html.contains("admin-logo"));
        assert!(!html.contains("admin-footer"));
        assert!(!html.contains("admin-search"));
        assert!(!html.contains("admin-export"));
    }
}
