//! Tenant Middleware
//!
//! Automatic tenant resolution middleware.

use crate::resolver::TenantResolver;
use crate::tenant::TenantContext;
use armature_core::{Error, HttpRequest, HttpResponse, Middleware};
use async_trait::async_trait;
use std::sync::Arc;

/// Tenant middleware
///
/// Automatically resolves tenant from request and stores in context.
pub struct TenantMiddleware {
    resolver: Arc<dyn TenantResolver>,
    optional: bool,
}

impl TenantMiddleware {
    /// Create new tenant middleware
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_tenancy::{TenantMiddleware, HeaderTenantResolver};
    /// use std::sync::Arc;
    ///
    /// let resolver = Arc::new(HeaderTenantResolver::new(store, "X-Tenant-ID"));
    /// let middleware = TenantMiddleware::new(resolver);
    /// ```
    pub fn new(resolver: Arc<dyn TenantResolver>) -> Self {
        Self {
            resolver,
            optional: false,
        }
    }

    /// Make tenant resolution optional
    ///
    /// If true, requests without valid tenant will proceed.
    /// If false, requests without valid tenant will return 401.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let middleware = TenantMiddleware::new(resolver)
    ///     .with_optional(true);
    /// ```
    pub fn with_optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }
}

/// Internal header-name prefix that once carried the resolved tenant.
///
/// Tenant identity is now stored in [`HttpRequest::extensions`], never in
/// headers. Any incoming header whose name starts with this prefix is
/// attacker-controlled noise and is stripped before resolution so a client
/// can never seed the request-local tenant identity.
const INTERNAL_TENANT_HEADER_PREFIX: &str = "__tenant";

/// Remove every client-supplied `__tenant*` header from the request.
///
/// Header lookup in [`HttpRequest`] is case-insensitive, so we match the
/// prefix case-insensitively to avoid a `__Tenant_Id`-style bypass.
fn strip_incoming_tenant_headers(request: &mut HttpRequest) {
    let to_remove: Vec<String> = request
        .headers
        .names()
        .filter(|name| {
            name.as_bytes()
                .get(..INTERNAL_TENANT_HEADER_PREFIX.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(INTERNAL_TENANT_HEADER_PREFIX.as_bytes()))
        })
        // Owned, because the loop below mutates the same map it is iterating
        // the names of.
        .map(str::to_owned)
        .collect();

    for name in to_remove {
        request.headers.remove_all(&name);
    }
}

#[async_trait]
impl Middleware for TenantMiddleware {
    async fn handle(
        &self,
        mut request: HttpRequest,
        next: armature_core::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        // Defense in depth: strip any client-supplied `__tenant*` headers
        // BEFORE resolution. Tenant identity lives only in request-local
        // extensions, so a spoofed header can never be mistaken for a
        // resolved tenant (closes the isolation bypass).
        strip_incoming_tenant_headers(&mut request);

        // Resolve tenant
        match self.resolver.resolve(&request).await {
            Ok(tenant) => {
                // Store the resolved tenant identity in request-local,
                // type-safe storage that clients cannot influence.
                request
                    .extensions
                    .insert(TenantContext::with_tenant(tenant));

                // Continue with request
                next(request).await
            }
            Err(e) => {
                if self.optional {
                    // Optional mode: proceed with NO tenant identity present.
                    // Headers were stripped above, and no TenantContext was
                    // inserted here, but defense-in-depth: remove any
                    // TenantContext that may have been preset on the request
                    // (e.g. by a server/framework default) before continuing,
                    // so a resolution failure can never let a stale/preset
                    // tenant context leak through.
                    request.extensions.remove::<TenantContext>();
                    next(request).await
                } else {
                    // Return error
                    Err(Error::Unauthorized(format!(
                        "Tenant resolution failed: {}",
                        e
                    )))
                }
            }
        }
    }
}

/// Helper to extract the resolved tenant id from a request.
///
/// Reads the [`TenantContext`] that [`TenantMiddleware`] stores in
/// [`HttpRequest::extensions`] on successful resolution. Returns `None` when
/// no tenant was resolved (anonymous). Client-supplied headers are never
/// consulted, so they cannot spoof the tenant.
pub fn get_tenant_id(request: &HttpRequest) -> Option<String> {
    request
        .extensions
        .get::<TenantContext>()
        .and_then(|ctx| ctx.tenant_id().map(str::to_string))
}

/// Helper to extract the resolved tenant name from a request.
///
/// Reads from [`HttpRequest::extensions`] (see [`get_tenant_id`]); never from
/// headers.
pub fn get_tenant_name(request: &HttpRequest) -> Option<String> {
    request
        .extensions
        .get::<TenantContext>()
        .and_then(|ctx| ctx.tenant().map(|tenant| tenant.name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TenantError;
    use crate::tenant::Tenant;

    struct MockResolver {
        tenant: Option<Tenant>,
    }

    #[async_trait]
    impl TenantResolver for MockResolver {
        async fn resolve(&self, _request: &HttpRequest) -> Result<Tenant, TenantError> {
            self.tenant
                .clone()
                .ok_or_else(|| TenantError::NotFound("No tenant".to_string()))
        }
    }

    fn create_request() -> HttpRequest {
        HttpRequest::new("GET", "/api/users".to_string())
    }

    #[tokio::test]
    async fn test_middleware_with_tenant() {
        let tenant = Tenant::new("tenant-1", "acme");
        let resolver = Arc::new(MockResolver {
            tenant: Some(tenant.clone()),
        });
        let middleware = TenantMiddleware::new(resolver);

        let request = create_request();

        let result = middleware
            .handle(
                request,
                Box::new(|req| {
                    Box::pin(async move {
                        // Check tenant was stored
                        assert_eq!(get_tenant_id(&req), Some("tenant-1".to_string()));
                        assert_eq!(get_tenant_name(&req), Some("acme".to_string()));
                        Ok(HttpResponse::ok())
                    })
                }),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_middleware_without_tenant_required() {
        let resolver = Arc::new(MockResolver { tenant: None });
        let middleware = TenantMiddleware::new(resolver).with_optional(false);

        let request = create_request();

        let result = middleware
            .handle(
                request,
                Box::new(|_req| Box::pin(async move { Ok(HttpResponse::ok()) })),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn enforcing_mode_rejects_unresolved() {
        let resolver = Arc::new(MockResolver { tenant: None });
        let middleware = TenantMiddleware::new(resolver).with_optional(false);

        // Even a spoofed header must not rescue an unresolved request.
        let mut request = create_request();
        request.headers.insert("__tenant_id", "victim".to_string());

        let result = middleware
            .handle(
                request,
                Box::new(|_req| Box::pin(async move { Ok(HttpResponse::ok()) })),
            )
            .await;

        assert!(matches!(result, Err(Error::Unauthorized(_))));
    }

    #[tokio::test]
    async fn successful_resolution_exposes_tenant_via_extensions() {
        let tenant = Tenant::new("tenant-1", "acme");
        let resolver = Arc::new(MockResolver {
            tenant: Some(tenant),
        });
        let middleware = TenantMiddleware::new(resolver);

        // Client tries to spoof a different tenant; the resolved one must win.
        let mut request = create_request();
        request.headers.insert("__tenant_id", "victim".to_string());
        request
            .headers
            .insert("__tenant_name", "victim-corp".to_string());

        let result = middleware
            .handle(
                request,
                Box::new(|req| {
                    Box::pin(async move {
                        assert_eq!(get_tenant_id(&req), Some("tenant-1".to_string()));
                        assert_eq!(get_tenant_name(&req), Some("acme".to_string()));
                        Ok(HttpResponse::ok())
                    })
                }),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn optional_mode_ignores_client_supplied_tenant_header() {
        // A resolver that FAILS, in optional mode, with a client trying to
        // seed the internal tenant header. The spoof must NOT survive.
        let resolver = Arc::new(MockResolver { tenant: None });
        let middleware = TenantMiddleware::new(resolver).with_optional(true);

        let mut request = create_request();
        request.headers.insert("__tenant_id", "victim".to_string());
        request
            .headers
            .insert("__tenant_name", "victim-corp".to_string());

        let result = middleware
            .handle(
                request,
                Box::new(|req| {
                    Box::pin(async move {
                        assert_eq!(get_tenant_id(&req), None, "spoofed tenant id survived");
                        assert_eq!(get_tenant_name(&req), None, "spoofed tenant name survived");
                        Ok(HttpResponse::ok())
                    })
                }),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_middleware_without_tenant_optional() {
        let resolver = Arc::new(MockResolver { tenant: None });
        let middleware = TenantMiddleware::new(resolver).with_optional(true);

        let request = create_request();

        let result = middleware
            .handle(
                request,
                Box::new(|req| {
                    Box::pin(async move {
                        // No tenant should be stored
                        assert_eq!(get_tenant_id(&req), None);
                        Ok(HttpResponse::ok())
                    })
                }),
            )
            .await;

        assert!(result.is_ok());
    }
}
