//! Rhai bindings for Armature HTTP types.

use armature_core::{HttpRequest, HttpResponse};
use bytes::Bytes;
use rhai::{Dynamic, Engine, EvalAltResult, Map};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Request binding for Rhai scripts.
///
/// Cloning is cheap in the body — it is `Bytes`, so a clone is a refcount bump
/// — but not in the maps, so build one per request and share it rather than
/// rebuilding it per guard/middleware hop.
#[derive(Debug, Clone)]
pub struct RequestBinding {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    /// Query pairs in wire order, duplicates included.
    ///
    /// A `Vec` rather than a `HashMap` so repeated keys resolve the same way
    /// they do in Rust: `armature_core::QueryView::get` returns the FIRST value
    /// for a key, and a map would have made scripts see the LAST — `?tag=a&tag=b`
    /// reading `"b"` in a script and `"a"` in Rust for the same request.
    query: Vec<(String, String)>,
    params: HashMap<String, String>,
    /// Names of every path param the router captured, including any whose bytes
    /// are not valid UTF-8 and therefore have no entry in `params`.
    param_names: Vec<String>,
    body: Bytes,
}

impl RequestBinding {
    /// Create a new request binding from an HttpRequest.
    pub fn from_request(req: &HttpRequest) -> Self {
        let mut headers = HashMap::new();
        for (name, value) in req.headers.iter() {
            // Rhai bindings hand owned strings to the script engine, so this is
            // one of the places a copy is the point rather than an oversight.
            headers.insert(name.to_owned(), value.to_owned());
        }

        let query: Vec<(String, String)> = req
            .query()
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();

        let mut params = HashMap::new();
        let mut param_names = Vec::new();
        for (key, value) in req.path_params.iter() {
            param_names.push((*key).to_owned());
            if let Ok(value) = std::str::from_utf8(value) {
                params.insert((*key).to_owned(), value.to_owned());
            }
        }

        Self {
            method: req.method_str().to_owned(),
            // The routing path, not the raw request target: a script reading
            // `request.path` for `/users/42?a=1` gets `/users/42`, matching
            // what the router matched against. The query is reachable through
            // `request.query(name)`/`request.query_params`.
            path: req.path_only().to_owned(),
            headers,
            query,
            params,
            param_names,
            body: req.body_bytes(),
        }
    }

    /// Get the HTTP method.
    pub fn get_method(&mut self) -> String {
        self.method.clone()
    }

    /// Get the request path.
    pub fn get_path(&mut self) -> String {
        self.path.clone()
    }

    /// Get a header value.
    pub fn header(&mut self, name: &str) -> Dynamic {
        // `self.headers` is the binding's own owned HashMap, not a HeaderMap,
        // and the script engine needs an owned value.
        self.headers
            .get(name)
            .cloned()
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    }

    /// Get all headers as a map.
    pub fn get_headers(&mut self) -> Map {
        let mut map = Map::new();
        for (k, v) in &self.headers {
            map.insert(k.clone().into(), Dynamic::from(v.clone()));
        }
        map
    }

    /// Get a query parameter.
    ///
    /// For a repeated key this is the FIRST value, matching
    /// `armature_core::QueryView::get` and `HttpRequest::query_param`. Use
    /// [`RequestBinding::query_all`] to see every value.
    pub fn query(&mut self, name: &str) -> Dynamic {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| Dynamic::from(v.clone()))
            .unwrap_or(Dynamic::UNIT)
    }

    /// Get every value for a repeated query parameter, in wire order.
    pub fn query_all(&mut self, name: &str) -> rhai::Array {
        self.query
            .iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| Dynamic::from(v.clone()))
            .collect()
    }

    /// Get all query parameters as a map.
    ///
    /// A map cannot represent a repeated key, so each key maps to its FIRST
    /// value — the same value `query(name)` returns. Reach for `query_all` when
    /// repeats matter.
    pub fn get_query_params(&mut self) -> Map {
        let mut map = Map::new();
        for (k, v) in &self.query {
            map.entry(k.clone().into())
                .or_insert_with(|| Dynamic::from(v.clone()));
        }
        map
    }

    /// Get a path parameter.
    ///
    /// Values are the raw path segment text captured by the router (see
    /// `ScriptRouter::match_pattern`) — they are **not** percent-decoded.
    /// A route pattern like `/users/:id` matched against
    /// `/users/john%20doe` yields `"john%20doe"`, not `"john doe"`; decode
    /// it in the script yourself (e.g. via a registered helper) if you need
    /// the decoded value.
    ///
    /// Returns `()` both when no such param was captured and when the captured
    /// bytes are not valid UTF-8 (Rhai strings are UTF-8, so there is nothing
    /// to hand back). [`RequestBinding::has_param`] distinguishes the two: it
    /// is `true` for a captured-but-undecodable param and `false` for an
    /// absent one.
    pub fn param(&mut self, name: &str) -> Dynamic {
        self.params
            .get(name)
            .cloned()
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    }

    /// Whether the router captured a path param called `name`.
    ///
    /// True even when `param(name)` returns `()` because the captured bytes
    /// were not valid UTF-8.
    pub fn has_param(&mut self, name: &str) -> bool {
        self.param_names.iter().any(|n| n == name)
    }

    /// Get all path parameters as a map.
    pub fn get_params(&mut self) -> Map {
        let mut map = Map::new();
        for (k, v) in &self.params {
            map.insert(k.clone().into(), Dynamic::from(v.clone()));
        }
        map
    }

    /// Get raw body bytes.
    ///
    /// A `rhai::Blob` is a `Vec<u8>`, so this copies; the binding itself holds
    /// the body as `Bytes` and only pays the copy when a script asks for it.
    pub fn get_body_bytes(&mut self) -> rhai::Blob {
        self.body.to_vec()
    }

    /// Get body as string.
    pub fn body_text(&mut self) -> Result<String, Box<EvalAltResult>> {
        std::str::from_utf8(&self.body)
            .map(str::to_owned)
            .map_err(|e| Box::new(EvalAltResult::from(e.to_string())))
    }

    /// Get body as JSON (parsed to Rhai Dynamic).
    pub fn body_json(&mut self) -> Result<Dynamic, Box<EvalAltResult>> {
        let text = self.body_text()?;
        if text.is_empty() {
            return Ok(Dynamic::UNIT);
        }
        let value: JsonValue = serde_json::from_str(&text)
            .map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
        json_to_dynamic(value)
    }

    /// Check if request has a specific content type.
    pub fn get_is_json(&mut self) -> bool {
        self.headers
            .get("content-type")
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false)
    }

    /// Check if request has form data.
    pub fn get_is_form(&mut self) -> bool {
        self.headers
            .get("content-type")
            .map(|ct| ct.contains("application/x-www-form-urlencoded"))
            .unwrap_or(false)
    }
}

/// Response builder for Rhai scripts.
#[derive(Debug, Clone)]
pub struct ResponseBinding {
    status: u16,
    headers: HashMap<String, String>,
    /// Set-Cookie header values, mirroring `HttpResponse::cookies`. Carried
    /// through `from_http_response`/`into_http_response` so that
    /// round-tripping a response through a script (e.g. `call_after`) never
    /// silently drops cookies.
    cookies: Vec<String>,
    /// Stored as `Bytes` (not `Vec<u8>`) so that `.clone()`-ing a
    /// `ResponseBinding` — e.g. to seed both the `response` and
    /// `original_response` script scope variables in `call_after` — is an
    /// O(1) refcount bump instead of a full body memcpy. `from_http_response`
    /// obtains its `Bytes` via `HttpResponse::body_bytes()`, which is itself
    /// O(1) when the source response is already `Bytes`-backed and only copies
    /// when it still holds a `Vec<u8>` body.
    body: Option<Bytes>,
}

impl Default for ResponseBinding {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseBinding {
    /// Create a new response binding.
    pub fn new() -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            cookies: Vec::new(),
            body: None,
        }
    }

    /// Build a `ResponseBinding` that mirrors an existing `HttpResponse`.
    ///
    /// Used to hand middleware scripts the *real* outgoing response
    /// (status/headers/body) instead of a blank slate, so a script can
    /// inspect it and amend it (e.g. add a header) without discarding
    /// whatever the handler already produced.
    pub fn from_http_response(response: &HttpResponse) -> Self {
        let mut headers = HashMap::new();
        for (name, value) in response.headers.iter() {
            headers.insert(name.clone(), value.clone());
        }

        Self {
            status: response.status,
            headers,
            cookies: response.cookies.clone(),
            body: Some(response.body_bytes()),
        }
    }

    /// Set status code.
    pub fn status(&mut self, code: i64) -> Self {
        self.status = code as u16;
        self.clone()
    }

    /// Set a header.
    pub fn header(&mut self, name: String, value: String) -> Self {
        self.headers.insert(name, value);
        self.clone()
    }

    /// Set body as text.
    pub fn body(&mut self, content: String) -> Self {
        self.body = Some(Bytes::from(content.into_bytes()));
        self.clone()
    }

    /// Set body as JSON from a Rhai Dynamic.
    pub fn json(&mut self, data: Dynamic) -> Result<Self, Box<EvalAltResult>> {
        let value = dynamic_to_json(data)?;
        let json = serde_json::to_string(&value)
            .map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
        self.headers
            .insert("content-type".to_string(), "application/json".to_string());
        self.body = Some(Bytes::from(json.into_bytes()));
        Ok(self.clone())
    }

    /// Create 200 OK response.
    pub fn ok() -> Self {
        Self::new()
    }

    /// Create 201 Created response.
    pub fn created() -> Self {
        let mut r = Self::new();
        r.status = 201;
        r
    }

    /// Create 204 No Content response.
    pub fn no_content() -> Self {
        let mut r = Self::new();
        r.status = 204;
        r
    }

    /// Create 400 Bad Request response.
    pub fn bad_request() -> Self {
        let mut r = Self::new();
        r.status = 400;
        r
    }

    /// Create 401 Unauthorized response.
    pub fn unauthorized() -> Self {
        let mut r = Self::new();
        r.status = 401;
        r
    }

    /// Create 403 Forbidden response.
    pub fn forbidden() -> Self {
        let mut r = Self::new();
        r.status = 403;
        r
    }

    /// Create 404 Not Found response.
    pub fn not_found() -> Self {
        let mut r = Self::new();
        r.status = 404;
        r
    }

    /// Create 405 Method Not Allowed response.
    pub fn method_not_allowed() -> Self {
        let mut r = Self::new();
        r.status = 405;
        r
    }

    /// Create 500 Internal Server Error response.
    pub fn internal_error() -> Self {
        let mut r = Self::new();
        r.status = 500;
        r
    }

    /// Create redirect response.
    pub fn redirect(url: String) -> Self {
        let mut r = Self::new();
        r.status = 302;
        r.headers.insert("location".to_string(), url);
        r
    }

    /// Convert to HttpResponse.
    pub fn into_http_response(self) -> HttpResponse {
        let mut response = HttpResponse::new(self.status);

        for (name, value) in self.headers {
            response.headers.insert(name, value);
        }

        response.cookies = self.cookies;

        if let Some(body) = self.body {
            // `with_bytes_body` stores the `Bytes` directly on
            // `HttpResponse` (its `body_bytes` field), so this is O(1) —
            // no `.to_vec()` copy back into a `Vec<u8>` is needed.
            response = response.with_bytes_body(body);
        }

        response
    }
}

/// Register all Armature API bindings with the Rhai engine.
pub fn register_armature_api(engine: &mut Engine) {
    // Register RequestBinding
    engine
        .register_type_with_name::<RequestBinding>("Request")
        .register_get("method", RequestBinding::get_method)
        .register_get("path", RequestBinding::get_path)
        .register_fn("header", RequestBinding::header)
        .register_get("headers", RequestBinding::get_headers)
        .register_fn("query", RequestBinding::query)
        .register_fn("query_all", RequestBinding::query_all)
        .register_get("query_params", RequestBinding::get_query_params)
        .register_fn("param", RequestBinding::param)
        .register_fn("has_param", RequestBinding::has_param)
        .register_get("params", RequestBinding::get_params)
        .register_get("body_bytes", RequestBinding::get_body_bytes)
        .register_fn("body_text", RequestBinding::body_text)
        .register_fn("body_json", RequestBinding::body_json)
        .register_fn("json", RequestBinding::body_json)
        .register_get("is_json", RequestBinding::get_is_json)
        .register_get("is_form", RequestBinding::get_is_form);

    // Register ResponseBinding
    engine
        .register_type_with_name::<ResponseBinding>("Response")
        .register_fn("new_response", ResponseBinding::new)
        .register_fn("status", ResponseBinding::status)
        .register_fn("header", ResponseBinding::header)
        .register_fn("body", ResponseBinding::body)
        .register_fn("json", ResponseBinding::json)
        .register_fn("ok", ResponseBinding::ok)
        .register_fn("created", ResponseBinding::created)
        .register_fn("no_content", ResponseBinding::no_content)
        .register_fn("bad_request", ResponseBinding::bad_request)
        .register_fn("unauthorized", ResponseBinding::unauthorized)
        .register_fn("forbidden", ResponseBinding::forbidden)
        .register_fn("not_found", ResponseBinding::not_found)
        .register_fn("method_not_allowed", ResponseBinding::method_not_allowed)
        .register_fn("internal_error", ResponseBinding::internal_error)
        .register_fn("redirect", ResponseBinding::redirect);

    // Register helper functions
    register_utility_functions(engine);
}

/// Register utility functions.
fn register_utility_functions(engine: &mut Engine) {
    // JSON helpers
    engine.register_fn(
        "to_json",
        |data: Dynamic| -> Result<String, Box<EvalAltResult>> {
            let value = dynamic_to_json(data)?;
            serde_json::to_string(&value).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))
        },
    );

    engine.register_fn(
        "to_json_pretty",
        |data: Dynamic| -> Result<String, Box<EvalAltResult>> {
            let value = dynamic_to_json(data)?;
            serde_json::to_string_pretty(&value)
                .map_err(|e| Box::new(EvalAltResult::from(e.to_string())))
        },
    );

    engine.register_fn(
        "from_json",
        |text: String| -> Result<Dynamic, Box<EvalAltResult>> {
            let value: JsonValue = serde_json::from_str(&text)
                .map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
            json_to_dynamic(value)
        },
    );

    // Logging helpers
    engine.register_fn("log_info", |msg: &str| {
        tracing::info!("[script] {}", msg);
    });

    engine.register_fn("log_warn", |msg: &str| {
        tracing::warn!("[script] {}", msg);
    });

    engine.register_fn("log_error", |msg: &str| {
        tracing::error!("[script] {}", msg);
    });

    engine.register_fn("log_debug", |msg: &str| {
        tracing::debug!("[script] {}", msg);
    });

    // Environment access (read-only)
    engine.register_fn("env", |name: &str| -> Dynamic {
        std::env::var(name)
            .ok()
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });

    engine.register_fn("env_or", |name: &str, default: &str| -> String {
        std::env::var(name).unwrap_or_else(|_| default.to_string())
    });
}

/// Convert JSON value to Rhai Dynamic.
fn json_to_dynamic(value: JsonValue) -> Result<Dynamic, Box<EvalAltResult>> {
    match value {
        JsonValue::Null => Ok(Dynamic::UNIT),
        JsonValue::Bool(b) => Ok(Dynamic::from(b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Dynamic::from(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Dynamic::from(f))
            } else {
                Err(Box::new(EvalAltResult::from("Invalid number")))
            }
        }
        JsonValue::String(s) => Ok(Dynamic::from(s)),
        JsonValue::Array(arr) => {
            let mut rhai_arr = rhai::Array::new();
            for item in arr {
                rhai_arr.push(json_to_dynamic(item)?);
            }
            Ok(Dynamic::from(rhai_arr))
        }
        JsonValue::Object(obj) => {
            let mut map = Map::new();
            for (key, val) in obj {
                map.insert(key.into(), json_to_dynamic(val)?);
            }
            Ok(Dynamic::from(map))
        }
    }
}

/// Convert Rhai Dynamic to JSON value.
fn dynamic_to_json(value: Dynamic) -> Result<JsonValue, Box<EvalAltResult>> {
    if value.is_unit() {
        Ok(JsonValue::Null)
    } else if value.is_bool() {
        Ok(JsonValue::Bool(value.as_bool().unwrap()))
    } else if value.is_int() {
        Ok(JsonValue::Number(value.as_int().unwrap().into()))
    } else if value.is_float() {
        let f = value.as_float().unwrap();
        Ok(JsonValue::Number(
            serde_json::Number::from_f64(f)
                .ok_or_else(|| Box::new(EvalAltResult::from("Invalid float")))?,
        ))
    } else if value.is_string() {
        Ok(JsonValue::String(value.into_string().unwrap()))
    } else if value.is_array() {
        let arr: rhai::Array = value.cast();
        let mut json_arr = Vec::new();
        for item in arr {
            json_arr.push(dynamic_to_json(item)?);
        }
        Ok(JsonValue::Array(json_arr))
    } else if value.is_map() {
        let map: Map = value.cast();
        let mut json_obj = serde_json::Map::new();
        for (key, val) in map {
            json_obj.insert(key.to_string(), dynamic_to_json(val)?);
        }
        Ok(JsonValue::Object(json_obj))
    } else {
        // Try to convert via debug string
        Ok(JsonValue::String(format!("{:?}", value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_roundtrip() {
        let json = r#"{"name": "Alice", "age": 30, "active": true}"#;
        let value: JsonValue = serde_json::from_str(json).unwrap();
        let dynamic = json_to_dynamic(value.clone()).unwrap();
        let back = dynamic_to_json(dynamic).unwrap();
        assert_eq!(value, back);
    }

    /// Regression: `path` used to be the raw request target, so a script
    /// reading `request.path` for `/users/42?a=1` saw `/users/42?a=1` while the
    /// router had matched `/users/42`.
    #[test]
    fn test_path_excludes_the_query_string() {
        let req = HttpRequest::new("GET", "/users/42?a=1&b=2");
        let mut binding = RequestBinding::from_request(&req);
        assert_eq!(binding.get_path(), "/users/42");
    }

    /// Regression: the query was a `HashMap`, so a repeated key was LAST-wins
    /// in scripts while `QueryView::get` is FIRST-wins in Rust — the same
    /// request read differently from the two languages.
    #[test]
    fn test_repeated_query_key_is_first_wins_like_core() {
        let req = HttpRequest::new("GET", "/search?tag=a&tag=b");
        assert_eq!(req.query_param("tag"), Some("a"));

        let mut binding = RequestBinding::from_request(&req);
        assert_eq!(binding.query("tag").into_string().unwrap(), "a");
        assert_eq!(
            binding
                .get_query_params()
                .get("tag")
                .unwrap()
                .clone()
                .into_string()
                .unwrap(),
            "a"
        );

        // Every value is still reachable, in wire order.
        let all: Vec<String> = binding
            .query_all("tag")
            .into_iter()
            .map(|v| v.into_string().unwrap())
            .collect();
        assert_eq!(all, vec!["a".to_string(), "b".to_string()]);
    }

    /// A missing query param is `()`, and `query_all` is empty rather than an
    /// error.
    #[test]
    fn test_absent_query_param() {
        let req = HttpRequest::new("GET", "/search?tag=a");
        let mut binding = RequestBinding::from_request(&req);
        assert!(binding.query("missing").is_unit());
        assert!(binding.query_all("missing").is_empty());
    }

    /// `param` cannot represent a non-UTF-8 capture, so `has_param` is what
    /// separates "captured but undecodable" from "not captured at all".
    #[test]
    fn test_has_param_distinguishes_undecodable_from_absent() {
        let mut req = HttpRequest::new("GET", "/files/x");
        req.push_param("name", Bytes::from_static(&[0xff, 0xfe]));

        let mut binding = RequestBinding::from_request(&req);
        assert!(binding.param("name").is_unit());
        assert!(binding.has_param("name"));

        assert!(binding.param("nope").is_unit());
        assert!(!binding.has_param("nope"));
    }

    #[test]
    fn test_response_builder() {
        let mut response = ResponseBinding::new();
        response = response.status(201);
        response = response.header("x-custom".to_string(), "value".to_string());
        response = response.body("Hello".to_string());

        let http = response.into_http_response();
        assert_eq!(http.status, 201);
    }
}
