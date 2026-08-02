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
        if let Some(should_continue) = scope.get_value::<bool>("continue")
            && !should_continue
        {
            // Middleware wants to return early
            if result.is::<ResponseBinding>() {
                let response: ResponseBinding = result.cast();
                return Ok(Some(response.into_http_response()));
            }
            if let Some(response) = scope.get_value::<ResponseBinding>("response") {
                return Ok(Some(response.into_http_response()));
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

        // Make the *real* outgoing response available, both as `response`
        // (for in-place mutation via method calls, e.g.
        // `response.header("x-served-by", "rhai");`) and as
        // `original_response` (for scripts that build a new response from
        // the original and return it explicitly). Without this, scripts can
        // only replace the response wholesale and lose whatever
        // status/headers/body the handler already produced.
        let resp_binding = ResponseBinding::from_http_response(&response);
        scope.set_value("response", resp_binding.clone());
        scope.push("original_response", resp_binding);

        let script = self.engine.compile_file(script_path)?;
        let result = self.engine.eval(&script, &mut scope)?;

        // A script that returns a ResponseBinding as its final expression
        // replaces the response outright.
        if result.is::<ResponseBinding>() {
            let new_response: ResponseBinding = result.cast();
            return Ok(new_response.into_http_response());
        }

        // Otherwise, read back whatever is bound to `response` in scope.
        // `ResponseBinding` mirrors every field of `HttpResponse` (status,
        // headers, cookies, body), so `into_http_response()` is lossless —
        // which means this single branch correctly covers *both* remaining
        // cases:
        //   - the script mutated `response` in place (e.g.
        //     `response.header("x-served-by", "rhai");` with no explicit
        //     return) — this serves back the amended copy;
        //   - the script never touched `response` at all — `response` still
        //     holds the untouched clone seeded from the real `HttpResponse`
        //     before the script ran, so this serves it back unchanged, with
        //     full fidelity.
        // This is *not* the same as the old belief that the branch below
        // (`Ok(response)`) was the "untouched" passthrough: that branch
        // never fires for the untouched case, because `response` is always
        // present in scope as a `ResponseBinding`. The lookup below can
        // only fail if a script reassigns `response` to a non-`Response`
        // value (e.g. `response = 42;`), which is a script bug, not a
        // normal outcome — see the fallback below.
        if let Some(mutated) = scope.get_value::<ResponseBinding>("response") {
            return Ok(mutated.into_http_response());
        }

        // Defensive fallback: a script stomped `response` with something
        // that isn't a `ResponseBinding`, so it can't be read back. Return
        // the real original response rather than panicking or erroring.
        Ok(response)
    }
}

/// Handler function type for use with routers.
pub type ScriptHandlerFn = Box<dyn Fn(HttpRequest) -> Result<HttpResponse> + Send + Sync>;

/// Create a handler function from a script.
///
/// The returned closure is a plain sync `Fn`, but internally it always has
/// to drive an async [`ScriptHandler::handle`] call to completion. It never
/// panics regardless of the caller's Tokio context:
///
/// - Inside a multi-thread runtime worker, it uses
///   [`tokio::task::block_in_place`] plus [`Handle::block_on`] — safe because
///   `block_in_place` hands this worker's other tasks off elsewhere first.
/// - Inside a current-thread runtime (or with no runtime at all), blocking
///   the calling thread could deadlock the executor (or is simply
///   impossible), so the future is instead driven to completion on a
///   dedicated thread with its own minimal runtime.
///
/// Cost note: that dedicated-thread fallback is not free. Each call on the
/// non-multi-thread path spawns a brand new OS thread and builds a fresh
/// single-threaded Tokio runtime (`Builder::new_current_thread()`) just for
/// that one invocation — neither the thread nor the runtime is pooled or
/// reused across calls. That's fine for occasional use, but it is
/// measurably more expensive per call than the multi-thread branch (which
/// just reuses the existing runtime's worker pool via `block_in_place`).
/// If a `ScriptHandlerFn` built by this function sits on a hot path, prefer
/// running it from a multi-thread Tokio runtime; calling it repeatedly from
/// a current-thread runtime (or with no runtime at all) will show up as
/// real per-call thread-spawn/runtime-build overhead.
pub fn script_handler(engine: Arc<RhaiEngine>, script_path: impl Into<PathBuf>) -> ScriptHandlerFn {
    let handler = Arc::new(ScriptHandler::new(engine, script_path));

    Box::new(move |request| {
        let handler = handler.clone();
        run_blocking(async move { handler.handle(request).await })
    })
}

/// Run `fut` to completion from synchronous code, regardless of whether the
/// calling thread is a Tokio multi-thread runtime worker, a current-thread
/// runtime, or has no Tokio runtime at all. Never panics because of the
/// caller's runtime context.
fn run_blocking<Fut>(fut: Fut) -> Fut::Output
where
    Fut: std::future::Future + Send + 'static,
    Fut::Output: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        // Already inside a multi-thread runtime worker: `block_in_place`
        // moves this worker's other tasks to another thread first, so
        // blocking here to drive `fut` to completion is safe.
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        // No runtime, or a single-threaded `current_thread` runtime: this
        // thread may be the only one driving the executor, so blocking it
        // directly could deadlock (or is outright impossible with no
        // runtime). Run the future on a dedicated thread with its own
        // minimal runtime instead, and block *this* thread on the join.
        _ => std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("failed to start a dedicated runtime for a Rhai script handler");
            rt.block_on(fut)
        })
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _create_test_request() -> HttpRequest {
        HttpRequest::new("GET", "/".to_string())
    }

    #[tokio::test]
    async fn test_script_handler_basic() {
        // This test requires a real script file
        // In a real test, we'd use tempfile to create a test script
    }

    #[tokio::test]
    async fn test_call_after_can_amend_the_real_response() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("after.rhai"),
            r#"
                original_response.header("x-served-by", "script");
                original_response
            "#,
        )
        .unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let middleware = ScriptMiddleware::after(engine, "after.rhai");

        let request = HttpRequest::new("GET", "/".to_string());
        let mut real_response = HttpResponse::new(201);
        real_response
            .headers
            .insert("x-real".to_string(), "yes".to_string());
        let real_response = real_response.with_body(b"hello".to_vec());

        let amended = middleware
            .call_after(&request, real_response)
            .await
            .unwrap();

        assert_eq!(
            amended.status, 201,
            "script must see the real status, not a fresh 200"
        );
        assert_eq!(
            amended.headers.get("x-real"),
            Some(&"yes".to_string()),
            "existing headers must survive"
        );
        assert_eq!(
            amended.headers.get("x-served-by"),
            Some(&"script".to_string())
        );
        assert_eq!(
            amended.body_ref(),
            b"hello",
            "existing body must survive when the script only adds a header"
        );
    }

    #[tokio::test]
    async fn test_call_after_preserves_cookies_on_noop_script() {
        // A completely no-op after-script: it never references `response`
        // or `original_response` at all, so this exercises the "script
        // didn't touch the response" path end-to-end.
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("after.rhai"), "()").unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let middleware = ScriptMiddleware::after(engine, "after.rhai");

        let request = HttpRequest::new("GET", "/".to_string());
        let real_response = HttpResponse::new(200).cookie("session", "abc123; HttpOnly");

        let amended = middleware
            .call_after(&request, real_response)
            .await
            .unwrap();

        assert_eq!(
            amended.cookies,
            vec!["session=abc123; HttpOnly".to_string()],
            "Set-Cookie headers must survive a no-op after-script"
        );
    }

    #[tokio::test]
    async fn test_call_after_mutated_response_in_place_without_explicit_return() {
        // The doc comment on `call_after` advertises mutating `response` in
        // place via method calls (no explicit `return`/final expression) as
        // a supported mode. Exercise it directly: the script statement ends
        // with `;`, so the script's own result is unit, and the mutation
        // must be picked up from the `response` scope variable instead.
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("after.rhai"),
            r#"response.header("x-after", "1");"#,
        )
        .unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let middleware = ScriptMiddleware::after(engine, "after.rhai");

        let request = HttpRequest::new("GET", "/".to_string());
        let real_response = HttpResponse::new(200).with_body(b"unchanged".to_vec());

        let amended = middleware
            .call_after(&request, real_response)
            .await
            .unwrap();

        assert_eq!(
            amended.headers.get("x-after"),
            Some(&"1".to_string()),
            "in-place mutation via response.header(...) must land on the final response"
        );
        assert_eq!(
            amended.status, 200,
            "in-place mutation must not clobber the original status"
        );
        assert_eq!(
            amended.body_ref(),
            b"unchanged",
            "in-place mutation must not clobber the original body"
        );
    }

    #[tokio::test]
    async fn test_call_after_header_only_mutation_preserves_exact_binary_body() {
        // Guards the `ResponseBinding` storage-representation change
        // (`Option<Vec<u8>>` -> `Option<bytes::Bytes>`): a header-only
        // mutation must round-trip an arbitrary binary (non-UTF-8) body
        // through `from_http_response` -> scope clone ->
        // `into_http_response` with byte-for-byte fidelity.
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("after.rhai"),
            r#"response.header("x-after", "binary-ok");"#,
        )
        .unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let middleware = ScriptMiddleware::after(engine, "after.rhai");

        let request = HttpRequest::new("GET", "/".to_string());
        let binary_body: Vec<u8> = vec![0x00, 0xFF, 0x01, 0xFE, 0x80, 0x7F, 0x00, 0x00, 0xDE, 0xAD];
        let real_response = HttpResponse::new(200).with_body(binary_body.clone());

        let amended = middleware
            .call_after(&request, real_response)
            .await
            .unwrap();

        assert_eq!(
            amended.headers.get("x-after"),
            Some(&"binary-ok".to_string())
        );
        assert_eq!(
            amended.body_ref(),
            binary_body.as_slice(),
            "header-only mutation must preserve the exact binary body bytes"
        );
    }

    #[tokio::test]
    async fn test_call_after_falls_back_to_original_response_if_script_stomps_response_variable() {
        // If a script reassigns `response` to something that isn't a
        // ResponseBinding at all (a script bug, not a normal outcome), the
        // `scope.get_value::<ResponseBinding>("response")` lookup fails.
        // `call_after` must fall back to the real original response rather
        // than panicking.
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("after.rhai"), "response = 42;").unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let middleware = ScriptMiddleware::after(engine, "after.rhai");

        let request = HttpRequest::new("GET", "/".to_string());
        let mut real_response = HttpResponse::new(201);
        real_response
            .headers
            .insert("x-real".to_string(), "yes".to_string());
        let real_response = real_response.with_body(b"hello".to_vec());

        let result = middleware
            .call_after(&request, real_response)
            .await
            .unwrap();

        assert_eq!(
            result.status, 201,
            "must fall back to the real original response, not panic"
        );
        assert_eq!(result.headers.get("x-real"), Some(&"yes".to_string()));
        assert_eq!(result.body_ref(), b"hello");
    }

    #[test]
    fn test_script_handler_without_tokio_runtime() {
        // No Tokio runtime is active on this thread at all.
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("handler.rhai"), "").unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let handler = script_handler(engine, "handler.rhai");

        let response = handler(HttpRequest::new("GET", "/".to_string()))
            .expect("script_handler must not panic without a Tokio runtime");
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_script_handler_inside_current_thread_runtime() {
        // The default `#[tokio::test]` flavor is a current-thread runtime.
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("handler.rhai"), "").unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let handler = script_handler(engine, "handler.rhai");

        let response = handler(HttpRequest::new("GET", "/".to_string()))
            .expect("script_handler must not panic inside a current-thread runtime");
        assert_eq!(response.status, 200);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_script_handler_inside_multi_thread_runtime() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("handler.rhai"), "").unwrap();

        let engine = Arc::new(RhaiEngine::new().scripts_dir(temp.path()).build().unwrap());
        let handler = script_handler(engine, "handler.rhai");

        let response = handler(HttpRequest::new("GET", "/".to_string()))
            .expect("script_handler must not panic inside a multi-thread runtime");
        assert_eq!(response.status, 200);
    }
}
