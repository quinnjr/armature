#![allow(clippy::needless_question_mark)]
//! Macro Utilities Example
//!
//! Demonstrates the utility macros available in Armature by actually
//! *invoking* every named macro inside real HTTP handlers wired into a
//! running server — not just leaving a comment next to plain
//! `HttpResponse` calls.
//!
//! Run with: `cargo run --example macro_utils_example`, then curl the
//! endpoints printed at startup.

use armature_core::{Application, Container, Error, HttpRequest, HttpResponse, Router};
use armature_macros::{
    bad_request, created_json, guard, header, internal_error, json_object, json_response,
    log_error, not_found, ok_json, paginated_response, path_param, path_params, query_param,
    routes, validation_error,
};
use armature_macros_utils::{html, json as proc_json, redirect, text};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

const PORT: u16 = 3091;

// =============================================================================
// In-memory "database" the handlers below operate on.
// =============================================================================

#[derive(Debug, Clone)]
struct User {
    id: i64,
    name: String,
    email: String,
}

#[derive(Debug, Deserialize)]
struct CreateUserPayload {
    name: String,
    email: String,
}

type UserStore = Arc<RwLock<HashMap<i64, User>>>;

fn seed_users() -> UserStore {
    let mut users = HashMap::new();
    users.insert(
        1,
        User {
            id: 1,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
        },
    );
    users.insert(
        2,
        User {
            id: 2,
            name: "Bob".to_string(),
            email: "bob@example.com".to_string(),
        },
    );
    Arc::new(RwLock::new(users))
}

// Recovers from lock poisoning rather than propagating it, so one panicking
// request doesn't permanently break every future request — acceptable for
// this demo; production code should decide deliberately whether poisoning
// should be fatal or recoverable.
static USERS: LazyLock<UserStore> = LazyLock::new(seed_users);

// =============================================================================
// Handlers — each one genuinely invokes the macro(s) named in its comment.
// =============================================================================

/// `GET /users` — demonstrates `query_param!` + `paginated_response!`.
async fn list_users(req: HttpRequest) -> Result<HttpResponse, Error> {
    let page: Option<u32> = query_param!(req, "page");
    let page = page.unwrap_or(1);

    let store = USERS.read().unwrap_or_else(|e| e.into_inner());
    let mut users: Vec<User> = store.values().cloned().collect();
    users.sort_by_key(|u| u.id);
    let total = users.len();

    let per_page = 10usize;
    let page_items: Vec<serde_json::Value> = users
        .into_iter()
        .skip((page.saturating_sub(1) as usize) * per_page)
        .take(per_page)
        .map(|u| json!({ "id": u.id, "name": u.name, "email": u.email }))
        .collect();

    paginated_response!(page_items, page, total)
}

/// `POST /users` — demonstrates `bad_request!`, `validation_error!` and
/// `created_json!`.
async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let payload: CreateUserPayload = match req.json() {
        Ok(p) => p,
        Err(e) => return bad_request!("Invalid request body: {}", e),
    };

    if payload.name.trim().is_empty() {
        return validation_error!("name must not be empty");
    }
    if !payload.email.contains('@') {
        return validation_error!("email '{}' is not a valid email address", payload.email);
    }

    let mut store = USERS.write().unwrap_or_else(|e| e.into_inner());
    let id = store.keys().copied().max().unwrap_or(0) + 1;
    let user = User {
        id,
        name: payload.name,
        email: payload.email,
    };
    store.insert(id, user.clone());

    created_json!(json!({ "id": user.id, "name": user.name, "email": user.email }))
}

/// `GET /users/:id` — demonstrates `header!`, `path_param!`, `guard!`,
/// `not_found!`, `json_object!` and `ok_json!`.
async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Requires callers to send an (unchecked, demo-only) API key header.
    let _api_key: &String = header!(req, "x-api-key")?;

    let id: i64 = path_param!(req, "id")?;
    guard!(id > 0, "id must be a positive integer");

    let store = USERS.read().unwrap_or_else(|e| e.into_inner());
    let user = match store.get(&id) {
        Some(u) => u.clone(),
        None => return not_found!("User {} not found", id),
    };
    drop(store);

    let payload = json_object! {
        "id" => user.id,
        "name" => user.name,
        "email" => user.email,
    };
    ok_json!(payload)
}

/// `GET /orgs/:org_id/teams/:team_id/report` — demonstrates `path_params!`
/// and `json_response!` (custom, non-200 status).
async fn queue_team_report(req: HttpRequest) -> Result<HttpResponse, Error> {
    let (org_id, team_id) = path_params!(req, "org_id": i64, "team_id": i64);
    json_response!(
        202,
        json!({
            "org_id": org_id,
            "team_id": team_id,
            "status": "report queued",
        })
    )
}

/// `GET /reports/summary` — demonstrates `armature-macros-utils`' procedural
/// `json!` macro (aliased to `proc_json!` to avoid clashing with
/// `serde_json::json!`).
async fn report_summary(_req: HttpRequest) -> Result<HttpResponse, Error> {
    proc_json!(ok, json!({ "report": "summary", "rows": 42 }))
}

/// `GET /reports/failing[?mode=logged]` — demonstrates `internal_error!` and
/// `log_error!` (reusing `query_param!` to pick which one runs).
async fn report_failing(req: HttpRequest) -> Result<HttpResponse, Error> {
    let mode: Option<String> = query_param!(req, "mode");
    if mode.as_deref() == Some("logged") {
        let cause = "disk full while writing report";
        return log_error!("Failed to generate report: {}", cause);
    }
    internal_error!("Report generation is temporarily unavailable")
}

/// `GET /pages/home` — demonstrates `armature-macros-utils`' `html!` macro.
async fn page_home(_req: HttpRequest) -> Result<HttpResponse, Error> {
    html!(
        ok,
        "<html><body><h1>Armature Macro Utilities</h1>\
         <p>This page was rendered by the html! macro.</p></body></html>"
    )
}

/// `GET /pages/legacy` — demonstrates `armature-macros-utils`' `redirect!`
/// macro (using its `permanent` status alias).
async fn legacy_redirect(_req: HttpRequest) -> Result<HttpResponse, Error> {
    redirect!(permanent, "/pages/home")
}

/// `GET /status/text` — demonstrates `armature-macros-utils`' `text!` macro.
async fn status_text(_req: HttpRequest) -> Result<HttpResponse, Error> {
    text!(ok, "Armature macro utilities example is running")
}

// =============================================================================
// Wiring & startup
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // So `log_error!`'s `tracing::error!` call is actually visible on stdout
    // when `/reports/failing?mode=logged` is hit.
    tracing_subscriber::fmt::init();

    // `routes!` (armature-macros) builds the `Vec<Route>` concisely.
    let built_routes = routes! {
        GET "/users" => list_users,
        POST "/users" => create_user,
        GET "/users/:id" => get_user,
        GET "/orgs/:org_id/teams/:team_id/report" => queue_team_report,
        GET "/reports/summary" => report_summary,
        GET "/reports/failing" => report_failing,
        GET "/pages/home" => page_home,
        GET "/pages/legacy" => legacy_redirect,
        GET "/status/text" => status_text,
    };

    let mut router = Router::new();
    for route in built_routes {
        router.add_route(route);
    }

    println!("=== Armature Macro Utilities Example ===\n");
    println!("Every endpoint below genuinely invokes the macro(s) listed next to it.\n");
    println!(
        "Declarative response macros   (armature-macros):       ok_json!, created_json!, json_response!, bad_request!, not_found!, internal_error!"
    );
    println!(
        "Parameter extraction          (armature-macros):       path_param!, path_params!, query_param!, header!"
    );
    println!("Validation & guards           (armature-macros):       validation_error!, guard!");
    println!(
        "Utilities                     (armature-macros):       json_object!, paginated_response!, log_error!"
    );
    println!("Routing                       (armature-macros):       routes!");
    println!(
        "Procedural response macros    (armature-macros-utils): json!, html!, text!, redirect!"
    );
    println!();
    println!("Server listening on http://127.0.0.1:{PORT}\n");
    println!("Endpoints:");
    println!(
        "  GET  /users                                     -> query_param!, paginated_response!"
    );
    println!(
        "  POST /users                                     -> bad_request!, validation_error!, created_json!"
    );
    println!(
        "  GET  /users/:id                                 -> header!, path_param!, guard!, not_found!, json_object!, ok_json!"
    );
    println!("  GET  /orgs/:org_id/teams/:team_id/report        -> path_params!, json_response!");
    println!("  GET  /reports/summary                           -> json! (armature-macros-utils)");
    println!("  GET  /reports/failing[?mode=logged]             -> internal_error! / log_error!");
    println!("  GET  /pages/home                                -> html!");
    println!("  GET  /pages/legacy                              -> redirect!");
    println!("  GET  /status/text                               -> text!");
    println!();
    println!("Try:");
    println!("  curl http://127.0.0.1:{PORT}/users");
    println!(
        "  curl http://127.0.0.1:{PORT}/users/1                                    # missing header! -> 400"
    );
    println!("  curl http://127.0.0.1:{PORT}/users/1 -H 'x-api-key: demo'");
    println!(
        "  curl http://127.0.0.1:{PORT}/users/999 -H 'x-api-key: demo'             # not_found! -> 404"
    );
    println!(
        "  curl -X POST http://127.0.0.1:{PORT}/users -H 'Content-Type: application/json' \\\n       -d '{{\"name\":\"Dana\",\"email\":\"dana@example.com\"}}'"
    );
    println!(
        "  curl -X POST http://127.0.0.1:{PORT}/users -H 'Content-Type: application/json' \\\n       -d '{{\"name\":\"\",\"email\":\"nope\"}}'      # validation_error! -> 400"
    );
    println!("  curl http://127.0.0.1:{PORT}/orgs/7/teams/3/report");
    println!("  curl http://127.0.0.1:{PORT}/reports/summary");
    println!(
        "  curl http://127.0.0.1:{PORT}/reports/failing                            # internal_error! -> 500"
    );
    println!(
        "  curl http://127.0.0.1:{PORT}/reports/failing?mode=logged                # log_error! -> 500"
    );
    println!("  curl http://127.0.0.1:{PORT}/pages/home");
    println!(
        "  curl -i http://127.0.0.1:{PORT}/pages/legacy                            # redirect! -> 301"
    );
    println!("  curl http://127.0.0.1:{PORT}/status/text");
    println!();

    let app = Application::new(Container::new(), router);
    app.listen(PORT).await?;
    Ok(())
}
