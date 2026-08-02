// Optimized handler dispatch system for maximum performance
//
// This module provides a handler system that enables:
// - Monomorphization: Handlers are specialized at compile time for each unique handler type
// - Inline dispatch: Hot paths use #[inline(always)] to eliminate function call overhead
// - Zero-cost abstractions: No runtime overhead compared to hand-written handlers
//
// The design follows Axum's approach: use traits with associated types for the Future,
// then type-erase at storage time while keeping invocation monomorphized.

use crate::logging::trace;
use crate::{Error, HttpRequest, HttpResponse};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

/// A handler that can process HTTP requests.
///
/// This trait is designed to be monomorphized: each implementation gets its own
/// specialized code path, allowing the compiler to inline the handler body.
///
/// # Example
///
/// ```ignore
/// async fn my_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// // The handler is automatically converted via IntoHandler
/// let handler = my_handler.into_handler();
/// ```
pub trait Handler: Clone + Send + Sync + 'static {
    /// The future returned by `call`.
    ///
    /// Using an associated type instead of `Box<dyn Future>` allows the compiler
    /// to monomorphize this future type, enabling inlining.
    type Future: Future<Output = Result<HttpResponse, Error>> + Send + 'static;

    /// Handle an HTTP request.
    ///
    /// This method should be inlined by the compiler due to monomorphization.
    /// Note: #[inline] hints are applied in implementations, not trait definitions.
    fn call(&self, req: HttpRequest) -> Self::Future;
}

/// Trait for converting various function types into handlers.
///
/// This enables ergonomic handler registration:
/// ```ignore
/// router.get("/", my_handler);  // Works for any IntoHandler
/// ```
pub trait IntoHandler<Args>: Clone + Send + Sync + 'static {
    /// The handler type this converts into.
    type Handler: Handler;

    /// Convert into a handler.
    fn into_handler(self) -> Self::Handler;
}

/// A function handler that wraps an async function.
///
/// This is the primary handler type, wrapping `async fn(HttpRequest) -> Result<HttpResponse, Error>`.
#[derive(Clone)]
pub struct FnHandler<F> {
    f: F,
}

impl<F> FnHandler<F> {
    /// Create a new function handler.
    #[inline(always)]
    pub fn new(f: F) -> Self {
        Self { f }
    }
}

impl<F, Fut> Handler for FnHandler<F>
where
    F: Fn(HttpRequest) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    type Future = Fut;

    #[inline(always)]
    fn call(&self, req: HttpRequest) -> Self::Future {
        (self.f)(req)
    }
}

/// Implement IntoHandler for async functions.
impl<F, Fut> IntoHandler<(HttpRequest,)> for F
where
    F: Fn(HttpRequest) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    type Handler = FnHandler<F>;

    #[inline(always)]
    fn into_handler(self) -> Self::Handler {
        FnHandler::new(self)
    }
}

/// Type-erased handler for storing in collections.
///
/// While `Handler` uses associated types for zero-cost abstraction,
/// we need type erasure to store different handlers in the same `Vec`.
///
/// This type performs the erasure while keeping the actual handler call
/// as optimized as possible via vtable dispatch with inlined inner handlers.
pub struct BoxedHandler {
    inner: Arc<dyn ErasedHandler>,
}

impl BoxedHandler {
    /// Create a new boxed handler from any Handler.
    ///
    /// The handler is wrapped in an Arc for cheap cloning.
    #[inline]
    pub fn new<H: Handler>(handler: H) -> Self {
        Self {
            inner: Arc::new(HandlerWrapper {
                handler,
                _marker: PhantomData,
            }),
        }
    }

    /// Call the handler.
    ///
    /// This goes through a vtable but the actual handler implementation
    /// is monomorphized and can be inlined within the wrapper.
    #[inline(always)]
    pub fn call(
        &self,
        req: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        trace!(path = %req.path, method = %req.method, "Handler dispatch");
        self.inner.call(req)
    }
}

impl Clone for BoxedHandler {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Internal trait for type-erased handler dispatch.
///
/// This trait enables storing different handler types together while
/// keeping the vtable overhead minimal.
trait ErasedHandler: Send + Sync {
    fn call(
        &self,
        req: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;
}

/// Wrapper that implements ErasedHandler for any Handler.
///
/// The key optimization is that `handler.call(req)` is monomorphized
/// for each concrete handler type, so the compiler can inline the
/// handler body even though we're going through a vtable.
struct HandlerWrapper<H: Handler> {
    handler: H,
    _marker: PhantomData<fn() -> H::Future>,
}

// Safety: HandlerWrapper is Send + Sync if H is Send + Sync
unsafe impl<H: Handler> Send for HandlerWrapper<H> {}
unsafe impl<H: Handler> Sync for HandlerWrapper<H> {}

impl<H: Handler> ErasedHandler for HandlerWrapper<H> {
    #[inline(always)]
    fn call(
        &self,
        req: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        // This call to handler.call() is monomorphized for the specific H type.
        // The compiler sees the concrete handler type and can inline its body.
        Box::pin(self.handler.call(req))
    }
}

/// Optimized handler type alias for the routing system.
///
/// This replaces the old `HandlerFn` type with a more optimized version
/// that enables monomorphization of the inner handler.
pub type OptimizedHandlerFn = BoxedHandler;

/// Create an optimized handler from a function.
///
/// # Example
///
/// ```ignore
/// use armature_core::handler::handler;
///
/// async fn my_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
///     Ok(HttpResponse::ok())
/// }
///
/// let h = handler(my_handler);
/// ```
#[inline]
pub fn handler<H, Args>(h: H) -> BoxedHandler
where
    H: IntoHandler<Args>,
{
    BoxedHandler::new(h.into_handler())
}

/// Legacy handler function type alias.
///
/// This type uses double dynamic dispatch (dyn Fn + Box<dyn Future>) which
/// prevents inlining. Prefer using the optimized handler system.
#[allow(clippy::type_complexity)]
pub type LegacyHandlerFn = Arc<
    dyn Fn(HttpRequest) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
        + Send
        + Sync,
>;

/// Convert the legacy HandlerFn type to a BoxedHandler.
///
/// This provides backwards compatibility while encouraging migration
/// to the new handler system.
#[inline]
pub fn from_legacy_handler(f: LegacyHandlerFn) -> BoxedHandler {
    BoxedHandler::new(LegacyHandler { f })
}

/// Wrapper for legacy handler functions.
#[derive(Clone)]
struct LegacyHandler {
    f: LegacyHandlerFn,
}

impl Handler for LegacyHandler {
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>;

    #[inline(always)]
    fn call(&self, req: HttpRequest) -> Self::Future {
        (self.f)(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[tokio::test]
    async fn test_fn_handler() {
        let handler = FnHandler::new(test_handler);
        let req = HttpRequest::new("GET", "/test".to_string());
        let response = handler.call(req).await.unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_into_handler() {
        let handler = test_handler.into_handler();
        let req = HttpRequest::new("GET", "/test".to_string());
        let response = handler.call(req).await.unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_boxed_handler() {
        let boxed = BoxedHandler::new(test_handler.into_handler());
        let req = HttpRequest::new("GET", "/test".to_string());
        let response = boxed.call(req).await.unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_handler_fn() {
        let h = handler(test_handler);
        let req = HttpRequest::new("GET", "/test".to_string());
        let response = h.call(req).await.unwrap();
        assert_eq!(response.status, 200);
    }

    #[tokio::test]
    async fn test_clone_boxed_handler() {
        let h1 = handler(test_handler);
        let h2 = h1.clone();

        let req1 = HttpRequest::new("GET", "/test".to_string());
        let req2 = HttpRequest::new("GET", "/test".to_string());

        let r1 = h1.call(req1).await.unwrap();
        let r2 = h2.call(req2).await.unwrap();

        assert_eq!(r1.status, 200);
        assert_eq!(r2.status, 200);
    }

    #[test]
    fn test_handler_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BoxedHandler>();
    }
}
