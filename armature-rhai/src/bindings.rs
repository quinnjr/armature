//! Rhai bindings for Armature HTTP types.

use armature_core::{HttpRequest, HttpResponse};
use rhai::{Dynamic, Engine, EvalAltResult, Map};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Request binding for Rhai scripts.
#[derive(Debug, Clone)]
pub struct RequestBinding {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    query: HashMap<String, String>,
    params: HashMap<String, String>,
    body: Vec<u8>,
}

impl RequestBinding {
    /// Create a new request binding from an HttpRequest.
    pub fn from_request(req: &HttpRequest) -> Self {
        let mut headers = HashMap::new();
        for (name, value) in req.headers.iter() {
            headers.insert(name.clone(), value.clone());
        }

        let mut query = HashMap::new();
        for (key, value) in req.query_params.iter() {
            query.insert(key.clone(), value.clone());
        }

        let mut params = HashMap::new();
        for (key, value) in req.path_params.iter() {
            params.insert(key.clone(), value.clone());
        }

        Self {
            method: req.method.clone(),
            path: req.path.clone(),
            headers,
            query,
            params,
            body: req.body_ref().to_vec(),
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
    pub fn query(&mut self, name: &str) -> Dynamic {
        self.query
            .get(name)
            .cloned()
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    }

    /// Get all query parameters as a map.
    pub fn get_query_params(&mut self) -> Map {
        let mut map = Map::new();
        for (k, v) in &self.query {
            map.insert(k.clone().into(), Dynamic::from(v.clone()));
        }
        map
    }

    /// Get a path parameter.
    pub fn param(&mut self, name: &str) -> Dynamic {
        self.params
            .get(name)
            .cloned()
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
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
    pub fn get_body_bytes(&mut self) -> rhai::Blob {
        self.body.clone()
    }

    /// Get body as string.
    pub fn body_text(&mut self) -> Result<String, Box<EvalAltResult>> {
        String::from_utf8(self.body.clone())
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
    body: Option<Vec<u8>>,
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
            body: None,
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
        self.body = Some(content.into_bytes());
        self.clone()
    }

    /// Set body as JSON from a Rhai Dynamic.
    pub fn json(&mut self, data: Dynamic) -> Result<Self, Box<EvalAltResult>> {
        let value = dynamic_to_json(data)?;
        let json = serde_json::to_string(&value)
            .map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
        self.headers
            .insert("content-type".to_string(), "application/json".to_string());
        self.body = Some(json.into_bytes());
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

        if let Some(body) = self.body {
            response = response.with_body(body);
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
        .register_get("query_params", RequestBinding::get_query_params)
        .register_fn("param", RequestBinding::param)
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
