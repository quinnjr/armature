//! Route groups for organizing routes with shared configuration
//!
//! Route groups allow you to organize routes with shared:
//! - Path prefixes
//! - Middleware
//! - Guards
//! - Configuration
//!
//! # Examples
//!
//! ```
//! use armature_core::RouteGroup;
//!
//! let api_group = RouteGroup::new()
//!     .prefix("/api/v1");
//!
//! // Add routes to the group
//! // These routes will inherit the prefix and middleware
//! ```

use crate::{Error, Guard, GuardContext, Middleware};
use std::sync::Arc;

/// A guard that checks all guards in a list
pub struct MultiGuard {
    guards: Vec<Box<dyn Guard>>,
}

impl MultiGuard {
    pub fn new(guards: Vec<Box<dyn Guard>>) -> Self {
        Self { guards }
    }
}

#[async_trait::async_trait]
impl Guard for MultiGuard {
    async fn can_activate(&self, context: &GuardContext) -> Result<bool, Error> {
        for guard in &self.guards {
            if !guard.can_activate(context).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Holds an ordered list of middleware.
///
/// Note: `MultiMiddleware` does not implement the `Middleware` trait and
/// therefore does not itself chain or dispatch anything. It is a simple
/// container exposing `get_middleware()` so callers can retrieve the list;
/// actual sequential middleware execution is handled elsewhere (see
/// `MiddlewareChain` in `middleware.rs`).
pub struct MultiMiddleware {
    middleware: Vec<Arc<dyn Middleware>>,
}

impl MultiMiddleware {
    pub fn new(middleware: Vec<Arc<dyn Middleware>>) -> Self {
        Self { middleware }
    }

    pub fn get_middleware(&self) -> &[Arc<dyn Middleware>] {
        &self.middleware
    }
}

/// Route group configuration
///
/// A route group allows you to organize routes with shared configuration
/// including path prefixes, middleware, and guards.
///
/// # Examples
///
/// ```
/// use armature_core::RouteGroup;
///
/// // Create an API group
/// let api = RouteGroup::new()
///     .prefix("/api/v1");
///
/// // Create a nested admin group
/// let admin = RouteGroup::new()
///     .prefix("/api/v1/admin");
/// ```
#[derive(Clone, Default)]
pub struct RouteGroup {
    /// Path prefix for all routes in this group
    prefix: String,

    /// Middleware to apply to all routes
    middleware: Vec<Arc<dyn Middleware>>,

    /// Guards to apply to all routes
    ///
    /// Stored as `Arc<dyn Guard>` so that cloned and nested groups share
    /// (rather than silently lose) their guards.
    guards: Vec<Arc<dyn Guard>>,
}

impl RouteGroup {
    /// Create a new route group
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the path prefix for this group
    ///
    /// All routes added to this group will have this prefix prepended.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::RouteGroup;
    ///
    /// let group = RouteGroup::new().prefix("/api/v1");
    /// // Routes like "/users" will become "/api/v1/users"
    /// ```
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        // Ensure prefix starts with / and doesn't end with /
        let prefix = if !prefix.starts_with('/') {
            format!("/{}", prefix)
        } else {
            prefix
        };
        let prefix = prefix.trim_end_matches('/').to_string();

        self.prefix = prefix;
        self
    }

    /// Add middleware to this group
    ///
    /// Middleware will be applied to all routes in this group.
    pub fn middleware(mut self, middleware: Arc<dyn Middleware>) -> Self {
        self.middleware.push(middleware);
        self
    }

    /// Add multiple middleware to this group
    pub fn with_middleware(mut self, middleware: Vec<Arc<dyn Middleware>>) -> Self {
        self.middleware.extend(middleware);
        self
    }

    /// Add a guard to this group
    ///
    /// Guards will be checked for all routes in this group.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::{RouteGroup, AuthenticationGuard};
    ///
    /// let group = RouteGroup::new()
    ///     .guard(Box::new(AuthenticationGuard));
    /// ```
    pub fn guard(mut self, guard: Box<dyn Guard>) -> Self {
        self.guards.push(Arc::from(guard));
        self
    }

    /// Add multiple guards to this group
    ///
    /// All guards must pass for the route to be accessed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::{RouteGroup, AuthenticationGuard, RolesGuard};
    ///
    /// let group = RouteGroup::new()
    ///     .with_guards(vec![
    ///         Box::new(AuthenticationGuard),
    ///         Box::new(RolesGuard::new(vec!["admin".to_string()])),
    ///     ]);
    /// ```
    pub fn with_guards(mut self, guards: Vec<Box<dyn Guard>>) -> Self {
        self.guards.extend(guards.into_iter().map(Arc::from));
        self
    }

    /// Get the prefix for this group
    pub fn get_prefix(&self) -> &str {
        &self.prefix
    }

    /// Apply the group's prefix to a path
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_core::RouteGroup;
    ///
    /// let group = RouteGroup::new().prefix("/api/v1");
    /// assert_eq!(group.apply_prefix("/users"), "/api/v1/users");
    /// ```
    pub fn apply_prefix(&self, path: &str) -> String {
        if self.prefix.is_empty() {
            path.to_string()
        } else {
            let path = path.trim_start_matches('/');
            if path.is_empty() {
                self.prefix.clone()
            } else {
                format!("{}/{}", self.prefix, path)
            }
        }
    }

    /// Get all middleware for this group
    pub fn get_middleware(&self) -> &[Arc<dyn Middleware>] {
        &self.middleware
    }

    /// Get all guards for this group
    pub fn get_guards(&self) -> &[Arc<dyn Guard>] {
        &self.guards
    }

    /// Combine this group with a parent group
    ///
    /// Creates a new group that inherits configuration from both.
    pub fn with_parent(self, parent: &RouteGroup) -> Self {
        let mut new_group = RouteGroup::new();

        // Combine prefixes
        if !parent.prefix.is_empty() {
            new_group.prefix = if !self.prefix.is_empty() {
                format!("{}{}", parent.prefix, self.prefix)
            } else {
                parent.prefix.clone()
            };
        } else {
            new_group.prefix = self.prefix;
        }

        // Combine middleware (parent first, then child)
        new_group
            .middleware
            .extend(parent.middleware.iter().cloned());
        new_group.middleware.extend(self.middleware);

        // Combine guards (parent first, then child)
        new_group.guards.extend(parent.guards.iter().cloned());
        new_group.guards.extend(self.guards);

        new_group
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestGuard;

    #[async_trait::async_trait]
    impl Guard for TestGuard {
        async fn can_activate(&self, _context: &GuardContext) -> Result<bool, Error> {
            Ok(true)
        }
    }

    #[test]
    fn test_route_group_clone_preserves_guards() {
        let group = RouteGroup::new()
            .prefix("/api")
            .guard(Box::new(TestGuard))
            .guard(Box::new(TestGuard));

        let cloned = group.clone();
        assert_eq!(cloned.get_guards().len(), 2);
        assert_eq!(group.get_guards().len(), 2);
    }

    #[test]
    fn test_route_group_with_parent_inherits_guards() {
        let parent = RouteGroup::new().prefix("/api").guard(Box::new(TestGuard));
        let child = RouteGroup::new()
            .prefix("/v1")
            .guard(Box::new(TestGuard))
            .with_parent(&parent);

        // Parent guard + child guard
        assert_eq!(child.get_guards().len(), 2);
    }

    #[test]
    fn test_route_group_prefix() {
        let group = RouteGroup::new().prefix("/api/v1");
        assert_eq!(group.get_prefix(), "/api/v1");
        assert_eq!(group.apply_prefix("/users"), "/api/v1/users");
        assert_eq!(group.apply_prefix("users"), "/api/v1/users");
        assert_eq!(group.apply_prefix("/"), "/api/v1");
        assert_eq!(group.apply_prefix(""), "/api/v1");
    }

    #[test]
    fn test_route_group_prefix_normalization() {
        let group = RouteGroup::new().prefix("api/v1/");
        assert_eq!(group.get_prefix(), "/api/v1");
    }

    #[test]
    fn test_route_group_no_prefix() {
        let group = RouteGroup::new();
        assert_eq!(group.get_prefix(), "");
        assert_eq!(group.apply_prefix("/users"), "/users");
    }

    #[test]
    fn test_route_group_with_parent() {
        let parent = RouteGroup::new().prefix("/api");
        let child = RouteGroup::new().prefix("/v1").with_parent(&parent);

        assert_eq!(child.get_prefix(), "/api/v1");
        assert_eq!(child.apply_prefix("/users"), "/api/v1/users");
    }
}
