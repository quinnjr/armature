//! Script-based HTTP handlers and middleware.

use crate::bindings::{RequestBinding, ResponseBinding};
use crate::context::ScriptContext;
use crate::engine::RhaiEngine;
use crate::error::Result;
use armature_core::{HttpRequest, HttpResponse};
use rhai::{Dynamic, Scope};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, instrument};

/// A script-based HTTP handler.
///
/// Executes a Rhai script to handle HTTP requests.
pub struct ScriptHandler {
    engine: Arc<RhaiEngine>,
    script_path: PathBuf,
}

impl ScriptHandler {
    /// Create a new script handler.
    pub fn new(engine: Arc<RhaiEngine>, script_path: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            script_path: script_path.into(),
        }
    }

    /// Handle an HTTP request.
    #[instrument(skip(self, request), fields(path = %self.script_path.display()))]
    pub async fn handle(&self, request: HttpRequest) -> Result<HttpResponse> {
        // Create request binding
        let req_binding = RequestBinding::from_request(&request);

        // Create script context
        let context = ScriptContext::new(req_binding);
        let mut scope = context.into_scope();

        // Compile and run script
        let script = self.engine.compile_file(&self.script_path)?;

        debug!("Executing script handler");

        let result = self.engine.eval(&script, &mut scope)?;

        // Convert result to HttpResponse
        self.result_to_response(result, &scope)
    }

    /// Convert script result to HttpResponse.
    fn result_to_response(&self, result: Dynamic, scope: &Scope) -> Result<HttpResponse> {
        // If result is a ResponseBinding, use it directly
        if result.is::<ResponseBinding>() {
            let response: ResponseBinding = result.cast();
            return Ok(response.into_http_response());
        }

        // Check if response was modified in scope
        if let Some(response) = scope.get_value::<ResponseBinding>("response") {
            return Ok(response.into_http_response());
        }

        // If result is a string, return as text body
        if result.is_string() {
            let text: String = result.cast();
            let mut resp = HttpResponse::new(200);
            resp.headers
                .insert("content-type".to_string(), "text/plain".to_string());
            return Ok(resp.with_body(text.into_bytes()));
        }

        // If result is a map or array, return as JSON
        if result.is_map() || result.is_array() {
            let mut binding = ResponseBinding::new();
            let json_resp = binding.json(result)?.into_http_response();
            return Ok(json_resp);
        }

        // Default: empty 200 response
        Ok(HttpResponse::new(200))
    }

    /// Get the script path.
    pub fn script_path(&self) -> &Path {
        &self.script_path
    }
}

/// A script-based middleware.
///
/// Can modify requests before handlers and responses after handlers.
pub struct ScriptMiddleware {
    engine: Arc<RhaiEngine>,
    before_script: Option<PathBuf>,
    after_script: Option<PathBuf>,
}

impl ScriptMiddleware {
    /// Create a new middleware with before script.
    pub fn before(engine: Arc<RhaiEngine>, script_path: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            before_script: Some(script_path.into()),
            after_script: None,
        }
    }

    /// Create a new middleware with after script.
    pub fn after(engine: Arc<RhaiEngine>, script_path: impl Into<PathBuf>) -> Self {
        Self {
            engine,
            before_script: None,
            after_script: Some(script_path.into()),
        }
    }

    /// Create a new middleware with both before and after scripts.
    pub fn both(
        engine: Arc<RhaiEngine>,
        before: impl Into<PathBuf>,
        after: impl Into<PathBuf>,
    ) -> Self {
        Self {
            engine,
            before_script: Some(before.into()),
            after_script: Some(after.into()),
        }
    }

    /// Execute the before script.
    ///
    /// Returns `Some(response)` if the middleware wants to short-circuit,
    /// or `None` to continue to the handler.
    #[instrument(skip(self, request), fields(script = ?self.before_script))]
    pub async fn call_before(&self, request: &HttpRequest) -> Result<Option<HttpResponse>> {
        let Some(script_path) = &self.before_script else {
            return Ok(None);
        };

        let req_binding = RequestBinding::from_request(request);
        let context = ScriptContext::new(req_binding);
        let mut scope = context.into_scope();

        // Add a flag to indicate middleware should continue
        scope.push("continue", true);

        let script = self.engine.compile_file(script_path)?;
        let result = self.engine.eval(&script, &mut scope)?;

        // Check if middleware wants to short-circuit
        if let Some(should_continue) = scope.get_value::<bool>("continue") {
            if !should_continue {
                // Middleware wants to return early
                if result.is::<ResponseBinding>() {
                    let response: ResponseBinding = result.cast();
                    return Ok(Some(response.into_http_response()));
                }
                if let Some(response) = scope.get_value::<ResponseBinding>("response") {
                    return Ok(Some(response.into_http_response()));
                }
            }
        }

        Ok(None)
    }

    /// Execute the after script.
    #[instrument(skip(self, request, response), fields(script = ?self.after_script))]
    pub async fn call_after(
        &self,
        request: &HttpRequest,
        response: HttpResponse,
    ) -> Result<HttpResponse> {
        let Some(script_path) = &self.after_script else {
            return Ok(response);
        };

        let req_binding = RequestBinding::from_request(request);
        let mut context = ScriptContext::new(req_binding);

        // Add response info to context
        context.set_local("status", Dynamic::from(response.status as i64));

        let mut scope = context.into_scope();

        // Make the response available for modification
        let resp_binding = ResponseBinding::new();
        scope.push("original_response", resp_binding);

        let script = self.engine.compile_file(script_path)?;
        let result = self.engine.eval(&script, &mut scope)?;

        // Check if a new response was returned
        if result.is::<ResponseBinding>() {
            let response: ResponseBinding = result.cast();
            return Ok(response.into_http_response());
        }

        // Return original response if not modified
        Ok(response)
    }
}

/// Handler function type for use with routers.
pub type ScriptHandlerFn = Box<dyn Fn(HttpRequest) -> Result<HttpResponse> + Send + Sync>;

/// Create a handler function from a script.
pub fn script_handler(engine: Arc<RhaiEngine>, script_path: impl Into<PathBuf>) -> ScriptHandlerFn {
    let handler = Arc::new(ScriptHandler::new(engine, script_path));

    Box::new(move |request| {
        // For sync context, we need to block on the async handler
        // In practice, this would be called from an async context
        let handler = handler.clone();
        let rt = tokio::runtime::Handle::try_current().expect("must be called from async context");

        rt.block_on(handler.handle(request))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _create_test_request() -> HttpRequest {
        HttpRequest::new("GET".to_string(), "/".to_string())
    }

    #[tokio::test]
    async fn test_script_handler_basic() {
        // This test requires a real script file
        // In a real test, we'd use tempfile to create a test script
    }
}
