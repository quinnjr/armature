// Guards for route protection

use crate::{Error, HttpRequest};
use async_trait::async_trait;

/// Execution context for guards
pub struct GuardContext {
    pub request: HttpRequest,
}

impl GuardContext {
    pub fn new(request: HttpRequest) -> Self {
        Self { request }
    }

    pub fn get_header(&self, name: &str) -> Option<&String> {
        self.request.headers.get(name)
    }

    pub fn get_param(&self, name: &str) -> Option<&String> {
        self.request.path_params.get(name)
    }
}

/// Guard trait for protecting routes
#[async_trait]
pub trait Guard: Send + Sync {
    /// Determine if the request can proceed
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error>;
}

/// Authentication guard.
///
/// **Presence/format check only.** This verifies that an `Authorization` header
/// exists and begins with `Bearer `; it performs **no** token validation
/// (signature, expiry, issuer, or audience are not checked). Use `armature-auth`
/// for real credential validation. This guard is a lightweight gate for routes
/// that only need to reject obviously-unauthenticated requests.
pub struct AuthenticationGuard;

#[async_trait]
impl Guard for AuthenticationGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        // Check for Authorization header
        match context.get_header("authorization") {
            Some(header) if header.starts_with("Bearer ") => Ok(true),
            _ => Err(Error::Forbidden(
                "Missing or invalid authorization header".to_string(),
            )),
        }
    }
}

/// Roles attached to a request by an authentication layer.
///
/// An upstream authentication middleware (e.g. a JWT validator) is expected
/// to insert this into [`HttpRequest::extensions`] after verifying the
/// caller's identity. [`RolesGuard`] reads it to authorize the request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestRoles(pub Vec<String>);

impl RequestRoles {
    pub fn new(roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self(roles.into_iter().map(Into::into).collect())
    }

    pub fn contains(&self, role: &str) -> bool {
        self.0.iter().any(|r| r == role)
    }
}

/// Role-based guard.
///
/// Requires a `Bearer` authorization header and a [`RequestRoles`] extension
/// containing at least one of the required roles. Fails closed: requests
/// without verified roles are rejected.
pub struct RolesGuard {
    required_roles: Vec<String>,
}

impl RolesGuard {
    pub fn new(roles: Vec<String>) -> Self {
        Self {
            required_roles: roles,
        }
    }
}

#[async_trait]
impl Guard for RolesGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        // First check authentication
        let auth_header = context
            .get_header("authorization")
            .ok_or_else(|| Error::Forbidden("Missing authorization header".to_string()))?;

        if !auth_header.starts_with("Bearer ") {
            return Err(Error::Forbidden("Invalid authorization header".to_string()));
        }

        if self.required_roles.is_empty() {
            return Ok(true);
        }

        // Roles must have been attached by an authentication layer that
        // verified the token (e.g. armature-auth / armature-jwt).
        let roles = context
            .request
            .extensions
            .get::<RequestRoles>()
            .ok_or_else(|| {
                Error::Forbidden("No verified roles associated with this request".to_string())
            })?;

        if self.required_roles.iter().any(|role| roles.contains(role)) {
            Ok(true)
        } else {
            Err(Error::Forbidden("Insufficient role".to_string()))
        }
    }
}

/// Custom guard builder
pub struct CustomGuard<F>
where
    F: Fn(&GuardContext) -> Result<bool, Error> + Send + Sync,
{
    predicate: F,
}

impl<F> CustomGuard<F>
where
    F: Fn(&GuardContext) -> Result<bool, Error> + Send + Sync,
{
    pub fn new(predicate: F) -> Self {
        Self { predicate }
    }
}

#[async_trait]
impl<F> Guard for CustomGuard<F>
where
    F: Fn(&GuardContext) -> Result<bool, Error> + Send + Sync,
{
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        (self.predicate)(context)
    }
}

/// API key guard
pub struct ApiKeyGuard {
    valid_keys: Vec<String>,
}

impl ApiKeyGuard {
    pub fn new(keys: Vec<String>) -> Self {
        Self { valid_keys: keys }
    }
}

#[async_trait]
impl Guard for ApiKeyGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        let api_key = context
            .get_header("x-api-key")
            .ok_or_else(|| Error::Forbidden("Missing API key".to_string()))?;

        if self.valid_keys.contains(api_key) {
            Ok(true)
        } else {
            Err(Error::Forbidden("Invalid API key".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_authentication_guard() {
        let guard = AuthenticationGuard;

        // Test with valid token
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer token123".to_string());
        let request = HttpRequest::from_parts(
            "GET".to_string(),
            "/test".to_string(),
            headers,
            vec![],
            HashMap::new(),
            HashMap::new(),
        );
        let context = GuardContext::new(request);

        assert!(guard.can_activate(&context).await.is_ok());
    }

    #[tokio::test]
    async fn test_authentication_guard_missing_header() {
        let guard = AuthenticationGuard;

        let request = HttpRequest::new("GET".to_string(), "/test".to_string());
        let context = GuardContext::new(request);

        assert!(guard.can_activate(&context).await.is_err());
    }

    #[tokio::test]
    async fn test_api_key_guard() {
        let guard = ApiKeyGuard::new(vec!["valid-key".to_string()]);

        // Valid key
        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), "valid-key".to_string());
        let request = HttpRequest::from_parts(
            "GET".to_string(),
            "/test".to_string(),
            headers,
            vec![],
            HashMap::new(),
            HashMap::new(),
        );
        let context = GuardContext::new(request);

        assert!(guard.can_activate(&context).await.is_ok());
    }

    #[tokio::test]
    async fn test_api_key_guard_invalid() {
        let guard = ApiKeyGuard::new(vec!["valid-key".to_string()]);

        let mut headers = HashMap::new();
        headers.insert("x-api-key".to_string(), "invalid-key".to_string());
        let request = HttpRequest::from_parts(
            "GET".to_string(),
            "/test".to_string(),
            headers,
            vec![],
            HashMap::new(),
            HashMap::new(),
        );
        let context = GuardContext::new(request);

        assert!(guard.can_activate(&context).await.is_err());
    }

    #[tokio::test]
    async fn test_api_key_guard_missing() {
        let guard = ApiKeyGuard::new(vec!["valid-key".to_string()]);

        let request = HttpRequest::new("GET".to_string(), "/test".to_string());
        let context = GuardContext::new(request);

        assert!(guard.can_activate(&context).await.is_err());
    }

    fn bearer_request() -> HttpRequest {
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer token123".to_string());
        HttpRequest::from_parts(
            "GET".to_string(),
            "/admin".to_string(),
            headers,
            vec![],
            HashMap::new(),
            HashMap::new(),
        )
    }

    #[tokio::test]
    async fn test_roles_guard_without_verified_roles_is_rejected() {
        let guard = RolesGuard::new(vec!["admin".to_string()]);

        // Authenticated (bearer token present) but no RequestRoles extension:
        // must fail closed.
        let context = GuardContext::new(bearer_request());
        assert!(guard.can_activate(&context).await.is_err());
    }

    #[tokio::test]
    async fn test_roles_guard_with_matching_role() {
        let guard = RolesGuard::new(vec!["admin".to_string()]);

        let mut request = bearer_request();
        request
            .extensions
            .insert(RequestRoles::new(["user", "admin"]));
        let context = GuardContext::new(request);

        assert!(matches!(guard.can_activate(&context).await, Ok(true)));
    }

    #[tokio::test]
    async fn test_roles_guard_with_insufficient_role() {
        let guard = RolesGuard::new(vec!["admin".to_string()]);

        let mut request = bearer_request();
        request.extensions.insert(RequestRoles::new(["user"]));
        let context = GuardContext::new(request);

        assert!(guard.can_activate(&context).await.is_err());
    }

    #[tokio::test]
    async fn test_roles_guard_no_required_roles_allows_authenticated() {
        let guard = RolesGuard::new(vec![]);

        let context = GuardContext::new(bearer_request());
        assert!(matches!(guard.can_activate(&context).await, Ok(true)));
    }

    #[test]
    fn test_guard_context_creation() {
        let mut request = HttpRequest::new("POST".to_string(), "/api/test".to_string());
        request.body = vec![1, 2, 3];
        let context = GuardContext::new(request.clone());

        assert_eq!(context.request.method, "POST");
        assert_eq!(context.request.path, "/api/test");
    }

    #[tokio::test]
    async fn test_authentication_guard_bearer_format() {
        let guard = AuthenticationGuard;

        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer abc123xyz".to_string());
        let request = HttpRequest::from_parts(
            "GET".to_string(),
            "/secure".to_string(),
            headers,
            vec![],
            HashMap::new(),
            HashMap::new(),
        );
        let context = GuardContext::new(request);

        let result = guard.can_activate(&context).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_authentication_guard_wrong_scheme() {
        let guard = AuthenticationGuard;

        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Basic abc123".to_string());
        let request = HttpRequest::from_parts(
            "GET".to_string(),
            "/secure".to_string(),
            headers,
            vec![],
            HashMap::new(),
            HashMap::new(),
        );
        let context = GuardContext::new(request);

        let result = guard.can_activate(&context).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_api_key_guard_multiple_valid_keys() {
        let guard = ApiKeyGuard::new(vec![
            "key1".to_string(),
            "key2".to_string(),
            "key3".to_string(),
        ]);

        for key in &["key1", "key2", "key3"] {
            let mut headers = HashMap::new();
            headers.insert("x-api-key".to_string(), key.to_string());
            let request = HttpRequest::from_parts(
                "GET".to_string(),
                "/test".to_string(),
                headers,
                vec![],
                HashMap::new(),
                HashMap::new(),
            );
            let context = GuardContext::new(request);

            assert!(guard.can_activate(&context).await.is_ok());
        }
    }

    #[test]
    fn test_api_key_guard_creation() {
        let keys = vec!["key1".to_string(), "key2".to_string()];
        let guard = ApiKeyGuard::new(keys.clone());
        assert_eq!(guard.valid_keys, keys);
    }

    #[test]
    fn test_roles_guard_creation() {
        let roles = vec!["admin".to_string(), "user".to_string()];
        let _guard = RolesGuard::new(roles);
        // Just test creation
    }

    #[tokio::test]
    async fn test_guard_context_with_params() {
        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "123".to_string());

        let mut query_params = HashMap::new();
        query_params.insert("sort".to_string(), "asc".to_string());

        let request = HttpRequest::from_parts(
            "GET".to_string(),
            "/users/123".to_string(),
            HashMap::new(),
            vec![],
            path_params,
            query_params,
        );
        let context = GuardContext::new(request);

        assert_eq!(
            context.request.path_params.get("id"),
            Some(&"123".to_string())
        );
        assert_eq!(
            context.request.query_params.get("sort"),
            Some(&"asc".to_string())
        );
    }
}
