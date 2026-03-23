//! Script-based router.

use crate::engine::RhaiEngine;
use crate::error::{Result, RhaiError};
use crate::handler::{ScriptHandler, ScriptMiddleware};
use armature_core::{HttpMethod, HttpRequest, HttpResponse};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, instrument};

/// Route configuration.
#[derive(Debug, Clone)]
struct Route {
    /// HTTP method (or None for any method).
    method: Option<HttpMethod>,
    /// Pattern to match.
    pattern: String,
    /// Script path for this route.
    script_path: PathBuf,
}

/// A script-based router.
///
/// Maps URL patterns to Rhai script handlers.
pub struct ScriptRouter {
    engine: Arc<RhaiEngine>,
    routes: Vec<Route>,
    middleware: Vec<ScriptMiddleware>,
    not_found_script: Option<PathBuf>,
    error_script: Option<PathBuf>,
}

impl ScriptRouter {
    /// Create a new script router.
    pub fn new(engine: RhaiEngine) -> Self {
        Self {
            engine: Arc::new(engine),
            routes: Vec::new(),
            middleware: Vec::new(),
            not_found_script: None,
            error_script: None,
        }
    }

    /// Create a new script router with a shared engine.
    pub fn with_engine(engine: Arc<RhaiEngine>) -> Self {
        Self {
            engine,
            routes: Vec::new(),
            middleware: Vec::new(),
            not_found_script: None,
            error_script: None,
        }
    }

    /// Add a route for any HTTP method.
    pub fn route(mut self, pattern: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        self.routes.push(Route {
            method: None,
            pattern: pattern.into(),
            script_path: script.into(),
        });
        self
    }

    /// Add a GET route.
    pub fn get(mut self, pattern: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        self.routes.push(Route {
            method: Some(HttpMethod::GET),
            pattern: pattern.into(),
            script_path: script.into(),
        });
        self
    }

    /// Add a POST route.
    pub fn post(mut self, pattern: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        self.routes.push(Route {
            method: Some(HttpMethod::POST),
            pattern: pattern.into(),
            script_path: script.into(),
        });
        self
    }

    /// Add a PUT route.
    pub fn put(mut self, pattern: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        self.routes.push(Route {
            method: Some(HttpMethod::PUT),
            pattern: pattern.into(),
            script_path: script.into(),
        });
        self
    }

    /// Add a DELETE route.
    pub fn delete(mut self, pattern: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        self.routes.push(Route {
            method: Some(HttpMethod::DELETE),
            pattern: pattern.into(),
            script_path: script.into(),
        });
        self
    }

    /// Add a PATCH route.
    pub fn patch(mut self, pattern: impl Into<String>, script: impl Into<PathBuf>) -> Self {
        self.routes.push(Route {
            method: Some(HttpMethod::PATCH),
            pattern: pattern.into(),
            script_path: script.into(),
        });
        self
    }

    /// Add middleware that runs before handlers.
    pub fn before(mut self, script: impl Into<PathBuf>) -> Self {
        self.middleware
            .push(ScriptMiddleware::before(self.engine.clone(), script));
        self
    }

    /// Add middleware that runs after handlers.
    pub fn after(mut self, script: impl Into<PathBuf>) -> Self {
        self.middleware
            .push(ScriptMiddleware::after(self.engine.clone(), script));
        self
    }

    /// Set the 404 Not Found handler script.
    pub fn not_found(mut self, script: impl Into<PathBuf>) -> Self {
        self.not_found_script = Some(script.into());
        self
    }

    /// Set the error handler script.
    pub fn error_handler(mut self, script: impl Into<PathBuf>) -> Self {
        self.error_script = Some(script.into());
        self
    }

    /// Handle a request.
    #[instrument(skip(self, request), fields(method = %request.method, path = %request.path))]
    pub async fn handle(&self, request: HttpRequest) -> HttpResponse {
        // Run before middleware
        for middleware in &self.middleware {
            match middleware.call_before(&request).await {
                Ok(Some(response)) => {
                    debug!("Middleware short-circuited");
                    return response;
                }
                Ok(None) => continue,
                Err(e) => {
                    return self.handle_error(e, &request).await;
                }
            }
        }

        // Find matching route
        let route = self.find_route(&request);

        let response = match route {
            Some(route) => {
                let handler = ScriptHandler::new(self.engine.clone(), &route.script_path);
                match handler.handle(request.clone()).await {
                    Ok(resp) => resp,
                    Err(e) => self.handle_error(e, &request).await,
                }
            }
            None => self.handle_not_found(&request).await,
        };

        // Run after middleware
        let mut final_response = response;
        for middleware in &self.middleware {
            match middleware.call_after(&request, final_response).await {
                Ok(resp) => final_response = resp,
                Err(e) => {
                    return self.handle_error(e, &request).await;
                }
            }
        }

        final_response
    }

    /// Find a matching route for the request.
    fn find_route(&self, request: &HttpRequest) -> Option<&Route> {
        let method_str = &request.method;
        let path = &request.path;

        for route in &self.routes {
            // Check method if specified
            if let Some(route_method) = &route.method {
                if route_method.as_str() != method_str {
                    continue;
                }
            }

            // Check pattern
            if self.pattern_matches(&route.pattern, path) {
                return Some(route);
            }
        }

        None
    }

    /// Check if a pattern matches a path.
    fn pattern_matches(&self, pattern: &str, path: &str) -> bool {
        let pattern_parts: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
        let path_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        // Handle catch-all pattern
        if pattern.ends_with("/*") {
            let prefix = pattern.trim_end_matches("/*");
            return path.starts_with(prefix);
        }

        // Check exact match or parameter match
        if pattern_parts.len() != path_parts.len() {
            return false;
        }

        for (pattern_part, path_part) in pattern_parts.iter().zip(path_parts.iter()) {
            if pattern_part.starts_with(':') {
                // Parameter - matches anything
                continue;
            }
            if pattern_part != path_part {
                return false;
            }
        }

        true
    }

    /// Handle 404 Not Found.
    async fn handle_not_found(&self, request: &HttpRequest) -> HttpResponse {
        if let Some(script_path) = &self.not_found_script {
            let handler = ScriptHandler::new(self.engine.clone(), script_path);
            match handler.handle(request.clone()).await {
                Ok(resp) => return resp,
                Err(e) => {
                    tracing::error!("Error in not_found handler: {}", e);
                }
            }
        }

        let mut resp = HttpResponse::new(404);
        resp.headers
            .insert("content-type".to_string(), "text/plain".to_string());
        resp.with_body(b"Not Found".to_vec())
    }

    /// Handle errors.
    async fn handle_error(&self, error: RhaiError, request: &HttpRequest) -> HttpResponse {
        tracing::error!("Script error: {}", error);

        if let Some(script_path) = &self.error_script {
            let handler = ScriptHandler::new(self.engine.clone(), script_path);

            // Create a modified request with error info
            match handler.handle(request.clone()).await {
                Ok(resp) => return resp,
                Err(e) => {
                    tracing::error!("Error in error handler: {}", e);
                }
            }
        }

        let mut resp = HttpResponse::new(500);
        resp.headers
            .insert("content-type".to_string(), "text/plain".to_string());
        resp.with_body(format!("Internal Server Error: {}", error).into_bytes())
    }

    /// Get the engine.
    pub fn engine(&self) -> &RhaiEngine {
        &self.engine
    }

    /// Precompile all route scripts.
    pub async fn precompile(&self) -> Result<()> {
        info!("Precompiling {} route scripts", self.routes.len());

        for route in &self.routes {
            self.engine.compile_file(&route.script_path)?;
            debug!("Compiled: {}", route.script_path.display());
        }

        // Compile special handlers
        if let Some(path) = &self.not_found_script {
            self.engine.compile_file(path)?;
        }
        if let Some(path) = &self.error_script {
            self.engine.compile_file(path)?;
        }

        info!("All scripts precompiled");
        Ok(())
    }
}

/// Builder for ScriptRouter with automatic script discovery.
pub struct ScriptRouterBuilder {
    engine: Arc<RhaiEngine>,
    auto_routes: bool,
    routes_dir: PathBuf,
}

impl ScriptRouterBuilder {
    /// Create a new builder.
    pub fn new(engine: RhaiEngine) -> Self {
        Self {
            engine: Arc::new(engine),
            auto_routes: false,
            routes_dir: PathBuf::from("routes"),
        }
    }

    /// Enable automatic route discovery from directory structure.
    ///
    /// Scripts in `routes/` directory will be mapped to URLs based on their path:
    /// - `routes/index.rhai` -> `/`
    /// - `routes/users.rhai` -> `/users`
    /// - `routes/users/[id].rhai` -> `/users/:id`
    /// - `routes/api/v1/items.rhai` -> `/api/v1/items`
    pub fn auto_routes(mut self, dir: impl Into<PathBuf>) -> Self {
        self.auto_routes = true;
        self.routes_dir = dir.into();
        self
    }

    /// Build the router.
    pub fn build(self) -> Result<ScriptRouter> {
        let mut router = ScriptRouter::with_engine(self.engine.clone());

        if self.auto_routes {
            router = self.discover_routes(router)?;
        }

        Ok(router)
    }

    /// Discover routes from directory structure.
    fn discover_routes(&self, mut router: ScriptRouter) -> Result<ScriptRouter> {
        let loader = crate::script::ScriptLoader::new(self.engine.config().scripts_dir.clone());

        let scripts = loader.list_scripts(&self.routes_dir, "rhai")?;

        for script_path in scripts {
            let route = self.path_to_route(&script_path)?;
            info!("Discovered route: {} -> {}", route, script_path.display());
            router = router.route(route, script_path);
        }

        Ok(router)
    }

    /// Convert a file path to a route pattern.
    fn path_to_route(&self, path: &Path) -> Result<String> {
        let relative = path
            .strip_prefix(&self.engine.config().scripts_dir)
            .unwrap_or(path)
            .strip_prefix(&self.routes_dir)
            .unwrap_or(path);

        let mut route = String::from("/");

        for component in relative.components() {
            if let std::path::Component::Normal(name) = component {
                let name = name.to_string_lossy();
                let name = name.trim_end_matches(".rhai");

                if name == "index" {
                    continue;
                }

                // Convert [param] to :param
                let segment = if name.starts_with('[') && name.ends_with(']') {
                    format!(":{}", &name[1..name.len() - 1])
                } else {
                    name.to_string()
                };

                if !route.ends_with('/') {
                    route.push('/');
                }
                route.push_str(&segment);
            }
        }

        if route.is_empty() {
            route = "/".to_string();
        }

        Ok(route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_matching() {
        let router = ScriptRouter::new(RhaiEngine::default());

        assert!(router.pattern_matches("/users", "/users"));
        assert!(router.pattern_matches("/users/:id", "/users/123"));
        assert!(!router.pattern_matches("/users/:id", "/users/123/extra"));
        assert!(router.pattern_matches("/api/*", "/api/v1/users"));
    }
}
