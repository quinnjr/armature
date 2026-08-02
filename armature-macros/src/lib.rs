//! Declarative macros for the Armature framework
//!
//! This crate provides convenient declarative macros (macro_rules!) for
//! common patterns in Armature applications.
//!
//! # Response Macros
//!
//! Quick HTTP responses:
//! ```ignore
//! json_response!({ "message": "Success" })
//! ok_json!({ "id": 123 })
//! not_found!("User not found")
//! ```
//!
//! # Route Grouping
//!
//! ```ignore
//! routes! {
//!     GET "/users" => list_users,
//!     POST "/users" => create_user,
//!     GET "/users/:id" => get_user,
//! }
//! ```

/// Create a JSON HTTP response with status code
///
/// # Examples
///
/// ```ignore
/// // With explicit status
/// json_response!(200, { "message": "Success", "id": 123 })
///
/// // With status constant
/// json_response!(StatusCode::OK, { "data": data })
/// ```
#[macro_export]
macro_rules! json_response {
    ($status:expr, $value:expr) => {{
        use armature_core::HttpResponse;
        HttpResponse::new($status).with_json(&$value)
    }};
}

/// Create a 200 OK JSON response
///
/// # Examples
///
/// ```ignore
/// ok_json!({ "message": "Success" })
/// ok_json!({ "users": users_vec })
/// ```
#[macro_export]
macro_rules! ok_json {
    ($value:expr) => {{
        use armature_core::HttpResponse;
        HttpResponse::ok().with_json(&$value)
    }};
}

/// Create a 201 Created JSON response
///
/// # Examples
///
/// ```ignore
/// created_json!({ "id": new_id, "message": "User created" })
/// ```
#[macro_export]
macro_rules! created_json {
    ($value:expr) => {{
        use armature_core::HttpResponse;
        HttpResponse::created().with_json(&$value)
    }};
}

/// Create a 400 Bad Request JSON error response
///
/// # Examples
///
/// ```ignore
/// bad_request!("Invalid input")
/// bad_request!("Field '{}' is required", field_name)
/// ```
#[macro_export]
macro_rules! bad_request {
    ($msg:expr) => {
        {
            use armature_core::HttpResponse;
            HttpResponse::bad_request().with_json(&serde_json::json!({
                "error": $msg,
                "status": 400
            }))
        }
    };
    ($fmt:expr, $($arg:tt)*) => {
        bad_request!(format!($fmt, $($arg)*))
    };
}

/// Create a 404 Not Found JSON error response
///
/// # Examples
///
/// ```ignore
/// not_found!("User not found")
/// not_found!("Resource '{}' not found", resource_id)
/// ```
#[macro_export]
macro_rules! not_found {
    ($msg:expr) => {
        {
            use armature_core::HttpResponse;
            HttpResponse::not_found().with_json(&serde_json::json!({
                "error": $msg,
                "status": 404
            }))
        }
    };
    ($fmt:expr, $($arg:tt)*) => {
        not_found!(format!($fmt, $($arg)*))
    };
}

/// Create a 500 Internal Server Error JSON response
///
/// # Examples
///
/// ```ignore
/// internal_error!("Database connection failed")
/// ```
#[macro_export]
macro_rules! internal_error {
    ($msg:expr) => {
        {
            use armature_core::HttpResponse;
            HttpResponse::internal_server_error().with_json(&serde_json::json!({
                "error": $msg,
                "status": 500
            }))
        }
    };
}

/// Extract and validate path parameters
///
/// # Examples
///
/// ```ignore
/// let id: i64 = path_param!(request, "id")?;
/// let slug: String = path_param!(request, "slug")?;
/// ```
#[macro_export]
macro_rules! path_param {
    ($req:expr, $name:expr) => {{
        $req.param($name)
            .ok_or_else(|| {
                armature_core::Error::BadRequest(format!("Missing path parameter: {}", $name))
            })?
            .parse()
            .map_err(|e| {
                armature_core::Error::BadRequest(format!(
                    "Invalid path parameter '{}': {}",
                    $name, e
                ))
            })
    }};
}

/// Extract and validate query parameters
///
/// # Examples
///
/// ```ignore
/// let page: u32 = query_param!(request, "page").unwrap_or(1);
/// let limit: u32 = query_param!(request, "limit").unwrap_or(20);
/// ```
#[macro_export]
macro_rules! query_param {
    ($req:expr, $name:expr) => {{ $req.query_param($name).and_then(|v| v.parse().ok()) }};
}

/// Extract header value
///
/// # Examples
///
/// ```ignore
/// let auth = header!(request, "Authorization")?;
/// let content_type = header!(request, "Content-Type").unwrap_or(&"text/plain".to_string());
/// ```
#[macro_export]
macro_rules! header {
    ($req:expr, $name:expr) => {{
        $req.headers
            .get($name)
            .ok_or_else(|| armature_core::Error::BadRequest(format!("Missing header: {}", $name)))
    }};
}

/// Create a validation error
///
/// # Examples
///
/// ```ignore
/// return validation_error!("Email is required");
/// return validation_error!("Age must be at least {}", min_age);
/// ```
#[macro_export]
macro_rules! validation_error {
    ($msg:expr) => {
        Err(armature_core::Error::Validation($msg.to_string()))
    };
    ($fmt:expr, $($arg:tt)*) => {
        validation_error!(format!($fmt, $($arg)*))
    };
}

/// Guard a condition, returning an error if false
///
/// # Examples
///
/// ```ignore
/// guard!(user.is_admin(), "Admin access required");
/// guard!(age >= 18, "Must be 18 or older");
/// ```
#[macro_export]
macro_rules! guard {
    ($cond:expr, $msg:expr) => {
        if !($cond) {
            return Err(armature_core::Error::Forbidden($msg.to_string()));
        }
    };
}

/// Define multiple routes concisely
///
/// # Examples
///
/// ```ignore
/// routes! {
///     GET "/users" => list_users,
///     POST "/users" => create_user,
///     GET "/users/:id" => get_user,
///     PUT "/users/:id" => update_user,
///     DELETE "/users/:id" => delete_user,
/// }
/// ```
#[macro_export]
macro_rules! routes {
    ($($method:ident $path:literal => $handler:expr),* $(,)?) => {
        vec![
            $(
                armature_core::Route::new(
                    armature_core::HttpMethod::from_str(stringify!($method)).unwrap(),
                    $path,
                    $handler,
                ),
            )*
        ]
    };
}

/// Build JSON objects with type safety
///
/// # Examples
///
/// ```ignore
/// let response = json_object! {
///     "id" => user.id,
///     "name" => user.name,
///     "email" => user.email,
/// };
/// ```
#[macro_export]
macro_rules! json_object {
    ($($key:literal => $value:expr),* $(,)?) => {
        {
            let mut map = serde_json::Map::new();
            $(
                map.insert($key.to_string(), serde_json::json!($value));
            )*
            serde_json::Value::Object(map)
        }
    };
}

/// Extract multiple path params at once
///
/// # Examples
///
/// ```ignore
/// let (user_id, post_id) = path_params!(request, "user_id": i64, "post_id": i64);
/// ```
#[macro_export]
macro_rules! path_params {
    ($req:expr, $($name:literal : $type:ty),+ $(,)?) => {
        {
            (
                $(
                    {
                        $req.param($name)
                            .ok_or_else(|| armature_core::Error::BadRequest(
                                format!("Missing path parameter: {}", $name)
                            ))?
                            .parse::<$type>()
                            .map_err(|e| armature_core::Error::BadRequest(
                                format!("Invalid path parameter '{}': {}", $name, e)
                            ))?
                    }
                ),+
            )
        }
    };
}

/// Log and return an error
///
/// Expands to a call to `tracing::error!`, so callers must depend on
/// `tracing` directly (the path is unprefixed and resolves at the call
/// site, not in this crate).
///
/// # Examples
///
/// ```ignore
/// log_error!("Failed to connect to database: {}", err);
/// ```
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        {
            tracing::error!($($arg)*);
            Err(armature_core::Error::Internal(
                format!($($arg)*)
            ))
        }
    };
}

/// Create a paginated response
///
/// # Examples
///
/// ```ignore
/// paginated_response!(users, page, total_count)
/// ```
#[macro_export]
macro_rules! paginated_response {
    ($data:expr, $page:expr, $total:expr) => {
        {
            use armature_core::HttpResponse;
            let __armature_paginated_data = $data;
            let __armature_paginated_per_page = __armature_paginated_data.len();
            HttpResponse::ok().with_json(&serde_json::json!({
                "data": __armature_paginated_data,
                "pagination": {
                    "page": $page,
                    "total": $total,
                    "per_page": __armature_paginated_per_page,
                }
            }))
        }
    };
}

pub mod prelude;

// The macro test suite lives in `tests/macros.rs` as an integration test:
// `#[macro_export]`-ed `macro_rules!` macros are meant to be consumed from
// external crates, so exercising them the same way downstream users would
// (via `armature_macros::prelude::*` / `armature_macros::routes!`) is the
// most faithful regression coverage. See tests/macros.rs.
