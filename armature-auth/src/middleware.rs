//! Token-verifying authentication middleware.
//!
//! This module ships [`JwtAuthMiddleware`], the concrete
//! [`armature_core::Middleware`] that turns a raw `Authorization: Bearer …`
//! header into verified, request-scoped identity. It is the layer the
//! fail-closed guards ([`crate::RoleGuard`], [`crate::PermissionGuard`] and
//! `armature_core::RolesGuard`) expect to run *before* them: those guards read
//! the [`UserContext`] / [`armature_core::RequestRoles`] extensions that this
//! middleware inserts, and reject any request where they are absent.
//!
//! Without this middleware (or an equivalent that populates the same
//! extensions), role- and permission-protected routes deny everyone, because
//! a bare bearer token is never treated as proof of any role.
//!
//! # Two modes
//!
//! The middleware supports two policies, selected by the `require_auth` flag:
//!
//! - **Required auth** (default, [`JwtAuthMiddleware::new`]): a missing or
//!   invalid/expired token is rejected immediately with `401 Unauthorized`;
//!   the downstream handler never runs. Use this to protect an entire route
//!   subtree where anonymous access is never allowed.
//! - **Optional auth** ([`JwtAuthMiddleware::optional`]): a valid token is
//!   verified and its identity attached; a missing or invalid token is simply
//!   passed through with **no** identity attached. The request still reaches
//!   the handler chain, so a per-route guard downstream gets to make the
//!   final allow/deny decision (typically returning `403` when it finds no
//!   verified roles). Use this when some routes behind the middleware are
//!   public and others are guarded.
//!
//! In both modes a *valid* token results in a [`UserContext`] and a
//! [`RequestRoles`] extension being inserted before the next handler runs.

use crate::UserContext;
use armature_core::{Error, HttpRequest, HttpResponse, Middleware, Next, RequestRoles};
use armature_jwt::JwtManager;
use armature_log::{debug, trace};
use async_trait::async_trait;

/// Middleware that verifies a JWT bearer token and populates request-scoped
/// identity for the authorization guards.
///
/// On a valid token it inserts two extensions into the request:
///
/// 1. A [`UserContext`] built from the configured `subject`, `roles` and
///    `permissions` claims (with the full claim set preserved in
///    [`UserContext::metadata`]). This drives [`crate::RoleGuard`] and
///    [`crate::PermissionGuard`].
/// 2. An [`armature_core::RequestRoles`] built from the same roles, so the
///    core `armature_core::RolesGuard` works too.
///
/// # Configuration
///
/// The claim names are configurable (defaults: `sub`, `roles`, `permissions`)
/// via [`with_subject_claim`](Self::with_subject_claim),
/// [`with_roles_claim`](Self::with_roles_claim) and
/// [`with_permissions_claim`](Self::with_permissions_claim). The verification
/// key/algorithm live in the [`JwtManager`] the middleware holds.
///
/// # Example: protect a route with `RoleGuard`
///
/// A request carrying a validly-signed token with the `admin` role passes the
/// guard; a token without that role (or no token at all) is denied.
///
/// ```
/// use armature_auth::{JwtAuthMiddleware, RoleGuard, Guard};
/// use armature_core::{HttpRequest, HttpResponse, Middleware, Next};
/// use armature_jwt::{JwtConfig, JwtManager};
/// use serde_json::json;
/// use std::pin::Pin;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let rt = tokio::runtime::Runtime::new()?;
/// rt.block_on(async {
///     let manager = JwtManager::new(JwtConfig::new("secret".to_string()))?;
///     let exp = chrono::Utc::now().timestamp() + 3600;
///
///     // Mint a token that carries the `admin` role.
///     let token = manager.sign(&json!({
///         "sub": "user123",
///         "roles": ["admin"],
///         "exp": exp,
///     }))?;
///
///     // The middleware verifies the token; the guard authorizes the role.
///     let middleware = JwtAuthMiddleware::new(manager);
///     let guard = RoleGuard::any(vec!["admin".to_string()]);
///
///     // `next` stands in for the guarded handler: it applies the guard to
///     // the (now identity-carrying) request.
///     let make_next = |guard: RoleGuard| -> Next {
///         Box::new(move |req: HttpRequest| {
///             Box::pin(async move {
///                 match guard.can_activate(&req).await {
///                     Ok(true) => Ok(HttpResponse::ok()),
///                     _ => Ok(HttpResponse::forbidden()),
///                 }
///             }) as Pin<Box<_>>
///         })
///     };
///
///     // Authenticated request WITH the right role -> 200.
///     let mut req = HttpRequest::new("GET".to_string(), "/admin".to_string());
///     req.headers
///         .insert("authorization".to_string(), format!("Bearer {token}"));
///     let resp = middleware.handle(req, make_next(guard.clone())).await.unwrap();
///     assert_eq!(resp.status, 200);
///
///     // No token, require_auth mode -> 401 before the guard even runs.
///     let req = HttpRequest::new("GET".to_string(), "/admin".to_string());
///     let resp = middleware.handle(req, make_next(guard.clone())).await.unwrap();
///     assert_eq!(resp.status, 401);
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct JwtAuthMiddleware {
    manager: JwtManager,
    require_auth: bool,
    subject_claim: String,
    roles_claim: String,
    permissions_claim: String,
}

impl JwtAuthMiddleware {
    /// Create a middleware in **required-auth** mode.
    ///
    /// Missing or invalid tokens are rejected with `401 Unauthorized` before
    /// the downstream handler runs. Claim names default to `sub`, `roles` and
    /// `permissions`.
    pub fn new(manager: JwtManager) -> Self {
        Self {
            manager,
            require_auth: true,
            subject_claim: "sub".to_string(),
            roles_claim: "roles".to_string(),
            permissions_claim: "permissions".to_string(),
        }
    }

    /// Create a middleware in **optional-auth** mode.
    ///
    /// A valid token is verified and its identity attached; a missing or
    /// invalid token passes through with no identity attached, leaving the
    /// allow/deny decision to a downstream guard.
    pub fn optional(manager: JwtManager) -> Self {
        Self {
            require_auth: false,
            ..Self::new(manager)
        }
    }

    /// Toggle the require-auth policy.
    ///
    /// `true` (the default from [`new`](Self::new)) rejects missing/invalid
    /// tokens with `401`; `false` lets them through for a downstream guard to
    /// decide (see [`optional`](Self::optional)).
    pub fn require_auth(mut self, require_auth: bool) -> Self {
        self.require_auth = require_auth;
        self
    }

    /// Override the claim name used for the user's subject/id (default `sub`).
    pub fn with_subject_claim(mut self, claim: impl Into<String>) -> Self {
        self.subject_claim = claim.into();
        self
    }

    /// Override the claim name used for the user's roles (default `roles`).
    pub fn with_roles_claim(mut self, claim: impl Into<String>) -> Self {
        self.roles_claim = claim.into();
        self
    }

    /// Override the claim name used for the user's permissions
    /// (default `permissions`).
    pub fn with_permissions_claim(mut self, claim: impl Into<String>) -> Self {
        self.permissions_claim = claim.into();
        self
    }

    /// Extract the bearer token from the `Authorization` header, if present.
    fn bearer_token(req: &HttpRequest) -> Option<&str> {
        let header = req
            .headers
            .get("authorization")
            .or_else(|| req.headers.get("Authorization"))?;
        header.strip_prefix("Bearer ").map(str::trim)
    }

    /// Verify `token` and build a [`UserContext`] from its claims.
    fn context_from_token(&self, token: &str) -> Option<UserContext> {
        let claims: serde_json::Value = match self.manager.verify(token) {
            Ok(claims) => claims,
            Err(e) => {
                debug!("JWT verification failed: {}", e);
                return None;
            }
        };

        let subject = claims
            .get(&self.subject_claim)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        let roles = string_list(claims.get(&self.roles_claim));
        let permissions = string_list(claims.get(&self.permissions_claim));

        trace!(
            "JWT verified for subject={subject}, {} role(s), {} permission(s)",
            roles.len(),
            permissions.len()
        );

        Some(
            UserContext::new(subject)
                .with_roles(roles)
                .with_permissions(permissions)
                .with_metadata(claims),
        )
    }

    /// Attach a verified [`UserContext`] (and matching [`RequestRoles`]) to a
    /// request's extensions.
    fn attach(req: &mut HttpRequest, context: UserContext) {
        let roles = RequestRoles::new(context.roles.clone());
        req.extensions.insert(context);
        req.extensions.insert(roles);
    }
}

#[async_trait]
impl Middleware for JwtAuthMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        match Self::bearer_token(&req).map(str::to_string) {
            Some(token) => match self.context_from_token(&token) {
                Some(context) => {
                    Self::attach(&mut req, context);
                    next(req).await
                }
                // Token present but invalid/expired.
                None => {
                    if self.require_auth {
                        debug!("Rejecting request with invalid token (require_auth)");
                        Ok(HttpResponse::unauthorized())
                    } else {
                        // Optional mode: attach nothing, let a guard decide.
                        next(req).await
                    }
                }
            },
            // No bearer token at all.
            None => {
                if self.require_auth {
                    debug!("Rejecting request with no bearer token (require_auth)");
                    Ok(HttpResponse::unauthorized())
                } else {
                    next(req).await
                }
            }
        }
    }
}

/// Coerce a JSON claim value into a list of strings.
///
/// Accepts a JSON array of strings (`["a", "b"]`) or a single string (`"a"`),
/// producing an empty list for anything else (including absence).
fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Guard, RoleGuard};
    use armature_core::guard::{Guard as CoreGuard, GuardContext, RolesGuard};
    use armature_jwt::JwtConfig;
    use serde_json::json;
    use std::pin::Pin;

    fn manager() -> JwtManager {
        JwtManager::new(JwtConfig::new("test-secret".to_string())).unwrap()
    }

    fn token_with(manager: &JwtManager, claims: serde_json::Value) -> String {
        manager.sign(&claims).unwrap()
    }

    fn admin_token(manager: &JwtManager) -> String {
        let exp = chrono::Utc::now().timestamp() + 3600;
        token_with(
            manager,
            json!({ "sub": "user123", "roles": ["admin", "user"], "permissions": ["read"], "exp": exp }),
        )
    }

    fn bearer_request(token: &str) -> HttpRequest {
        let mut req = HttpRequest::new("GET".to_string(), "/admin".to_string());
        req.headers
            .insert("authorization", format!("Bearer {token}"));
        req
    }

    /// A `next` that applies an armature-auth `RoleGuard` to the request.
    fn role_guard_next(guard: RoleGuard) -> Next {
        Box::new(move |req: HttpRequest| {
            Box::pin(async move {
                match guard.can_activate(&req).await {
                    Ok(true) => Ok(HttpResponse::ok()),
                    _ => Ok(HttpResponse::forbidden()),
                }
            }) as Pin<Box<_>>
        })
    }

    #[tokio::test]
    async fn valid_jwt_with_matching_role_passes_role_guard() {
        let manager = manager();
        let token = admin_token(&manager);
        let mw = JwtAuthMiddleware::new(manager);
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let resp = mw.handle(bearer_request(&token), next).await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn valid_jwt_without_role_is_denied() {
        let manager = manager();
        let exp = chrono::Utc::now().timestamp() + 3600;
        let token = token_with(
            &manager,
            json!({ "sub": "u", "roles": ["user"], "exp": exp }),
        );
        let mw = JwtAuthMiddleware::new(manager);
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let resp = mw.handle(bearer_request(&token), next).await.unwrap();
        // Verified identity attached, but the role requirement is not met.
        assert_eq!(resp.status, 403);
    }

    #[tokio::test]
    async fn no_token_in_require_auth_mode_returns_401() {
        let mw = JwtAuthMiddleware::new(manager());
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let req = HttpRequest::new("GET".to_string(), "/admin".to_string());
        let resp = mw.handle(req, next).await.unwrap();
        assert_eq!(resp.status, 401);
    }

    #[tokio::test]
    async fn invalid_token_in_require_auth_mode_returns_401() {
        let mw = JwtAuthMiddleware::new(manager());
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let resp = mw
            .handle(bearer_request("not.a.valid.token"), next)
            .await
            .unwrap();
        assert_eq!(resp.status, 401);
    }

    #[tokio::test]
    async fn token_signed_with_wrong_secret_is_rejected() {
        let signer = JwtManager::new(JwtConfig::new("other-secret".to_string())).unwrap();
        let token = admin_token(&signer);

        // Verifier uses a different secret -> signature check fails.
        let mw = JwtAuthMiddleware::new(manager());
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let resp = mw.handle(bearer_request(&token), next).await.unwrap();
        assert_eq!(resp.status, 401);
    }

    #[tokio::test]
    async fn no_token_in_optional_mode_passes_through_to_guard_which_denies() {
        let mw = JwtAuthMiddleware::optional(manager());
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        // No token: optional mode attaches nothing and forwards to the guard,
        // which fails closed because no verified roles are present.
        let req = HttpRequest::new("GET".to_string(), "/admin".to_string());
        // Give it a bearer header so the guard's auth precheck passes and we
        // exercise the "no verified roles" fail-closed path specifically.
        let mut req_with_header = req;
        req_with_header
            .headers
            .insert("authorization", "Bearer ".to_string());
        let resp = mw.handle(req_with_header, next).await.unwrap();
        assert_eq!(resp.status, 403);
    }

    #[tokio::test]
    async fn optional_mode_attaches_context_when_token_valid() {
        let manager = manager();
        let token = admin_token(&manager);
        let mw = JwtAuthMiddleware::optional(manager);
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let resp = mw.handle(bearer_request(&token), next).await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn populates_core_request_roles_for_core_roles_guard() {
        let manager = manager();
        let token = admin_token(&manager);
        let mw = JwtAuthMiddleware::new(manager);

        // Downstream uses armature_core::RolesGuard, which reads RequestRoles.
        let next: Next = Box::new(move |req: HttpRequest| {
            Box::pin(async move {
                let guard = RolesGuard::new(vec!["admin".to_string()]);
                let ctx = GuardContext::new(req);
                match guard.can_activate(&ctx).await {
                    Ok(true) => Ok(HttpResponse::ok()),
                    _ => Ok(HttpResponse::forbidden()),
                }
            }) as Pin<Box<_>>
        });

        let resp = mw.handle(bearer_request(&token), next).await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn configurable_claim_names_are_honoured() {
        let manager = manager();
        let exp = chrono::Utc::now().timestamp() + 3600;
        let token = token_with(
            &manager,
            json!({ "user_id": "u1", "authorities": ["admin"], "exp": exp }),
        );
        let mw = JwtAuthMiddleware::new(manager)
            .with_subject_claim("user_id")
            .with_roles_claim("authorities");
        let next = role_guard_next(RoleGuard::any(vec!["admin".to_string()]));

        let resp = mw.handle(bearer_request(&token), next).await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn string_list_accepts_array_and_scalar() {
        assert_eq!(
            string_list(Some(&json!(["a", "b"]))),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(string_list(Some(&json!("solo"))), vec!["solo".to_string()]);
        assert!(string_list(Some(&json!(42))).is_empty());
        assert!(string_list(None).is_empty());
    }
}
