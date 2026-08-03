//! Metrics endpoint handler
//!
//! Provides the `/metrics` endpoint for Prometheus scraping.

use armature_core::handler::{BoxedHandler, from_legacy_handler};
use armature_core::{Error, HttpRequest, HttpResponse};

/// Handle metrics endpoint request
///
/// Returns all registered metrics in Prometheus text format.
///
/// # Examples
///
/// ```no_run
/// use armature_core::*;
/// use armature_core::handler::from_legacy_handler;
/// use armature_metrics::*;
/// use std::sync::Arc;
///
/// // Add to your router
/// let mut router = Router::new();
/// router.add_route(Route {
///     method: HttpMethod::GET,
///     path: "/metrics".to_string(),
///     handler: from_legacy_handler(Arc::new(|req: HttpRequest| {
///         Box::pin(async move {
///             metrics_handler(req).await
///         })
///     })),
///     constraints: None,
/// });
/// ```
pub async fn metrics_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
    let metrics = crate::export_metrics();

    Ok(HttpResponse::ok()
        .with_header(
            "Content-Type".to_string(),
            "text/plain; version=0.0.4".to_string(),
        )
        .with_body(metrics.into_bytes()))
}

/// Create a metrics handler function
///
/// Returns a handler function that can be used with Armature routing.
///
/// # Examples
///
/// ```no_run
/// use armature_core::*;
/// use armature_metrics::*;
///
/// let handler = create_metrics_handler();
///
/// let mut router = Router::new();
/// router.add_route(Route {
///     method: HttpMethod::GET,
///     path: "/metrics".to_string(),
///     handler,
///     constraints: None,
/// });
/// ```
pub fn create_metrics_handler() -> BoxedHandler {
    from_legacy_handler(std::sync::Arc::new(|req: HttpRequest| {
        Box::pin(async move { metrics_handler(req).await })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_handler() {
        let request = HttpRequest::new("GET", "/metrics".to_string());
        let response = metrics_handler(request).await.unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("Content-Type"),
            Some(&"text/plain; version=0.0.4".to_string())
        );
    }
}
