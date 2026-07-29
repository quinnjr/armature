// Authentication guards

use crate::{AuthError, AuthUser, Result, UserContext};
use armature_core::{HttpRequest, RequestRoles};
use async_trait::async_trait;

/// Guard trait for protecting routes
#[async_trait]
pub trait Guard: Send + Sync {
    /// Check if the request can proceed
    async fn can_activate(&self, request: &HttpRequest) -> Result<bool>;
}

/// Authentication guard - ensures a request carries a bearer token.
///
/// This guard only checks that an `Authorization: Bearer …` header is present;
/// it does **not** verify the token itself. Token verification and identity
/// extraction are the job of the shipped [`crate::JwtAuthMiddleware`], which
/// must run *before* this guard so that verified [`UserContext`] /
/// [`armature_core::RequestRoles`] extensions are available to the role- and
/// permission-aware guards below.
#[derive(Clone)]
pub struct AuthGuard;

impl AuthGuard {
    pub fn new() -> Self {
        Self
    }

    /// Extract the verified user identity from the request.
    ///
    /// Reads the `T` extension attached by an authentication layer that has
    /// already verified the caller's token — in practice
    /// [`crate::JwtAuthMiddleware`], which inserts a [`UserContext`] built
    /// from the token's verified claims. Returns
    /// [`AuthError::Unauthorized`] if no such extension is present, e.g.
    /// because `JwtAuthMiddleware` (or an equivalent) did not run first.
    pub fn extract_user<T: AuthUser + Clone + 'static>(&self, request: &HttpRequest) -> Result<T> {
        request
            .extensions
            .get::<T>()
            .cloned()
            .ok_or(AuthError::Unauthorized)
    }
}

impl Default for AuthGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Guard for AuthGuard {
    async fn can_activate(&self, request: &HttpRequest) -> Result<bool> {
        // Check for Authorization header
        let auth_header = request
            .headers
            .get("authorization")
            .ok_or(AuthError::Unauthorized)?;

        // Check for Bearer token
        if !auth_header.starts_with("Bearer ") {
            return Err(AuthError::InvalidToken(
                "Invalid authorization format".to_string(),
            ));
        }

        // Token exists - authentication check passed
        // In production, you'd verify the token here
        Ok(true)
    }
}

/// Role-based authorization guard.
///
/// Requires a `Bearer` authorization header (via [`AuthGuard`]) plus verified
/// role information attached to the request by an authentication layer that
/// has already validated the caller's token. The intended layer is the
/// shipped [`crate::JwtAuthMiddleware`]: register it ahead of the guarded
/// route so it verifies the JWT and populates the extensions this guard reads.
/// The guard looks for, in order:
///
/// 1. A [`UserContext`] in [`HttpRequest::extensions`], as produced by
///    [`crate::JwtAuthMiddleware`] from the token's claims after signature
///    verification.
/// 2. An [`armature_core::RequestRoles`] extension (the same mechanism used
///    by `armature_core::RolesGuard`, also populated by
///    [`crate::JwtAuthMiddleware`]).
///
/// The guard fails closed: when `required_roles` is non-empty and neither
/// extension is present, the request is rejected — a bearer token alone is
/// never sufficient to satisfy a role requirement. Without
/// [`crate::JwtAuthMiddleware`] (or an equivalent that inserts these
/// extensions) in front, role-protected routes deny every request.
#[derive(Clone)]
pub struct RoleGuard {
    required_roles: Vec<String>,
    require_all: bool,
}

impl RoleGuard {
    /// Create a guard that requires ANY of the roles
    pub fn any(roles: Vec<String>) -> Self {
        Self {
            required_roles: roles,
            require_all: false,
        }
    }

    /// Create a guard that requires ALL of the roles
    pub fn all(roles: Vec<String>) -> Self {
        Self {
            required_roles: roles,
            require_all: true,
        }
    }

    /// Check if user has required roles
    pub fn check_roles<T: AuthUser>(&self, user: &T) -> bool {
        let role_refs: Vec<&str> = self.required_roles.iter().map(|s| s.as_str()).collect();

        if self.require_all {
            user.has_all_roles(&role_refs)
        } else {
            user.has_any_role(&role_refs)
        }
    }
}

impl RoleGuard {
    /// Check the required roles against a plain role list (e.g. from a
    /// [`RequestRoles`] extension).
    fn check_role_list(&self, roles: &RequestRoles) -> bool {
        if self.require_all {
            self.required_roles.iter().all(|role| roles.contains(role))
        } else {
            self.required_roles.iter().any(|role| roles.contains(role))
        }
    }
}

#[async_trait]
impl Guard for RoleGuard {
    async fn can_activate(&self, request: &HttpRequest) -> Result<bool> {
        // First check authentication
        let auth_guard = AuthGuard::new();
        auth_guard.can_activate(request).await?;

        // No role requirement: any authenticated request may proceed.
        if self.required_roles.is_empty() {
            return Ok(true);
        }

        // Prefer the richer UserContext attached by an authentication layer
        // after it verified the caller's token.
        if let Some(user) = request.extensions.get::<UserContext>() {
            return if self.check_roles(user) {
                Ok(true)
            } else {
                Err(AuthError::Forbidden("Insufficient role".to_string()))
            };
        }

        // Fall back to armature-core's RequestRoles extension.
        if let Some(roles) = request.extensions.get::<RequestRoles>() {
            return if self.check_role_list(roles) {
                Ok(true)
            } else {
                Err(AuthError::Forbidden("Insufficient role".to_string()))
            };
        }

        // Fail closed: authenticated, but no verified roles were attached to
        // the request, so a non-empty role requirement cannot be satisfied.
        Err(AuthError::Forbidden(
            "No verified roles associated with this request".to_string(),
        ))
    }
}

/// Permission-based authorization guard
#[derive(Clone)]
pub struct PermissionGuard {
    required_permissions: Vec<String>,
    require_all: bool,
}

impl PermissionGuard {
    /// Create a guard that requires ANY of the permissions
    pub fn any(permissions: Vec<String>) -> Self {
        Self {
            required_permissions: permissions,
            require_all: false,
        }
    }

    /// Create a guard that requires ALL of the permissions
    pub fn all(permissions: Vec<String>) -> Self {
        Self {
            required_permissions: permissions,
            require_all: true,
        }
    }

    /// Check if user has required permissions
    pub fn check_permissions<T: AuthUser>(&self, user: &T) -> bool {
        if self.require_all {
            self.required_permissions
                .iter()
                .all(|perm| user.has_permission(perm))
        } else {
            self.required_permissions
                .iter()
                .any(|perm| user.has_permission(perm))
        }
    }
}

#[async_trait]
impl Guard for PermissionGuard {
    async fn can_activate(&self, request: &HttpRequest) -> Result<bool> {
        // First check authentication
        let auth_guard = AuthGuard::new();
        auth_guard.can_activate(request).await?;

        // No permission requirement: any authenticated request may proceed.
        if self.required_permissions.is_empty() {
            return Ok(true);
        }

        // Permissions are only carried by the richer UserContext that an
        // authentication layer attaches after verifying the caller's token.
        // Fail closed if it is absent.
        let user = request.extensions.get::<UserContext>().ok_or_else(|| {
            AuthError::Forbidden("No verified user associated with this request".to_string())
        })?;

        if self.check_permissions(user) {
            Ok(true)
        } else {
            Err(AuthError::Forbidden("Insufficient permissions".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserContext;

    #[test]
    fn test_role_guard() {
        let user = UserContext::new("user123".to_string())
            .with_roles(vec!["admin".to_string(), "user".to_string()]);

        // Test ANY
        let guard = RoleGuard::any(vec!["admin".to_string()]);
        assert!(guard.check_roles(&user));

        let guard = RoleGuard::any(vec!["guest".to_string()]);
        assert!(!guard.check_roles(&user));

        // Test ALL
        let guard = RoleGuard::all(vec!["admin".to_string(), "user".to_string()]);
        assert!(guard.check_roles(&user));

        let guard = RoleGuard::all(vec!["admin".to_string(), "guest".to_string()]);
        assert!(!guard.check_roles(&user));
    }

    #[test]
    fn extract_user_returns_user_context_from_extension() {
        let guard = AuthGuard::new();
        let mut request = HttpRequest::new("GET".to_string(), "/me".to_string());
        request
            .extensions
            .insert(UserContext::new("user123".to_string()).with_roles(vec!["admin".to_string()]));

        let user = guard.extract_user::<UserContext>(&request).unwrap();
        assert_eq!(user.user_id, "user123");
        assert!(user.has_role("admin"));
    }

    #[test]
    fn extract_user_errors_without_extension() {
        let guard = AuthGuard::new();
        let request = HttpRequest::new("GET".to_string(), "/me".to_string());

        let result = guard.extract_user::<UserContext>(&request);
        assert!(matches!(result, Err(AuthError::Unauthorized)));
    }

    fn authenticated_request() -> HttpRequest {
        let mut request = HttpRequest::new("GET".to_string(), "/admin".to_string());
        request
            .headers
            .insert("authorization".to_string(), "Bearer token123".to_string());
        request
    }

    #[tokio::test]
    async fn test_role_guard_matching_role_passes() {
        let guard = RoleGuard::any(vec!["admin".to_string()]);

        // Via UserContext extension
        let mut request = authenticated_request();
        request
            .extensions
            .insert(UserContext::new("user123".to_string()).with_roles(vec!["admin".to_string()]));
        assert!(matches!(guard.can_activate(&request).await, Ok(true)));

        // Via RequestRoles extension
        let mut request = authenticated_request();
        request
            .extensions
            .insert(RequestRoles::new(["user", "admin"]));
        assert!(matches!(guard.can_activate(&request).await, Ok(true)));
    }

    #[tokio::test]
    async fn test_role_guard_missing_role_denied() {
        let guard = RoleGuard::any(vec!["admin".to_string()]);

        // UserContext without the required role
        let mut request = authenticated_request();
        request
            .extensions
            .insert(UserContext::new("user123".to_string()).with_roles(vec!["user".to_string()]));
        assert!(guard.can_activate(&request).await.is_err());

        // RequestRoles without the required role
        let mut request = authenticated_request();
        request.extensions.insert(RequestRoles::new(["user"]));
        assert!(guard.can_activate(&request).await.is_err());
    }

    #[tokio::test]
    async fn test_role_guard_require_all_via_request_roles() {
        let guard = RoleGuard::all(vec!["admin".to_string(), "user".to_string()]);

        let mut request = authenticated_request();
        request.extensions.insert(RequestRoles::new(["admin"]));
        assert!(guard.can_activate(&request).await.is_err());

        let mut request = authenticated_request();
        request
            .extensions
            .insert(RequestRoles::new(["admin", "user"]));
        assert!(matches!(guard.can_activate(&request).await, Ok(true)));
    }

    #[tokio::test]
    async fn test_role_guard_no_roles_attached_denied() {
        let guard = RoleGuard::any(vec!["admin".to_string()]);

        // Authenticated (bearer token present) but no roles attached:
        // must fail closed.
        let request = authenticated_request();
        assert!(guard.can_activate(&request).await.is_err());
    }

    #[tokio::test]
    async fn test_role_guard_empty_required_roles_allows_authenticated() {
        let guard = RoleGuard::any(vec![]);

        let request = authenticated_request();
        assert!(matches!(guard.can_activate(&request).await, Ok(true)));
    }

    #[tokio::test]
    async fn test_role_guard_unauthenticated_denied() {
        let guard = RoleGuard::any(vec!["admin".to_string()]);

        let request = HttpRequest::new("GET".to_string(), "/admin".to_string());
        assert!(guard.can_activate(&request).await.is_err());
    }

    #[test]
    fn test_permission_guard() {
        let user = UserContext::new("user123".to_string())
            .with_permissions(vec!["read".to_string(), "write".to_string()]);

        // Test ANY
        let guard = PermissionGuard::any(vec!["read".to_string()]);
        assert!(guard.check_permissions(&user));

        let guard = PermissionGuard::any(vec!["delete".to_string()]);
        assert!(!guard.check_permissions(&user));

        // Test ALL
        let guard = PermissionGuard::all(vec!["read".to_string(), "write".to_string()]);
        assert!(guard.check_permissions(&user));

        let guard = PermissionGuard::all(vec!["read".to_string(), "delete".to_string()]);
        assert!(!guard.check_permissions(&user));
    }

    #[tokio::test]
    async fn test_permission_guard_matching_permission_passes() {
        let guard = PermissionGuard::any(vec!["write".to_string()]);
        let mut request = authenticated_request();
        request.extensions.insert(
            UserContext::new("user123".to_string())
                .with_permissions(vec!["read".to_string(), "write".to_string()]),
        );
        assert!(matches!(guard.can_activate(&request).await, Ok(true)));
    }

    #[tokio::test]
    async fn test_permission_guard_missing_permission_denied() {
        let guard = PermissionGuard::any(vec!["delete".to_string()]);
        let mut request = authenticated_request();
        request.extensions.insert(
            UserContext::new("user123".to_string()).with_permissions(vec!["read".to_string()]),
        );
        assert!(guard.can_activate(&request).await.is_err());
    }

    #[tokio::test]
    async fn test_permission_guard_no_user_attached_denied() {
        let guard = PermissionGuard::any(vec!["read".to_string()]);
        // Authenticated but no UserContext attached: must fail closed.
        let request = authenticated_request();
        assert!(guard.can_activate(&request).await.is_err());
    }

    #[tokio::test]
    async fn test_permission_guard_empty_required_allows_authenticated() {
        let guard = PermissionGuard::any(vec![]);
        let request = authenticated_request();
        assert!(matches!(guard.can_activate(&request).await, Ok(true)));
    }
}
