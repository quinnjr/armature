//! Core traits for the Armature framework

use async_trait::async_trait;
use std::any::TypeId;

// Re-export DI traits from dependency-injector
pub use dependency_injector::{Injectable, Provider};

/// Trait for HTTP controllers
#[async_trait]
pub trait Controller: Send + Sync + 'static {
    /// Returns the base path for this controller
    fn base_path(&self) -> &'static str;

    /// Returns the routes registered on this controller
    fn routes(&self) -> Vec<RouteDefinition>;
}

/// Trait for modules that organize components
pub trait Module: Send + Sync + 'static {
    /// Returns the `TypeId` of the concrete type implementing this trait.
    ///
    /// Do not override this: the default implementation is monomorphized
    /// per concrete `Self` (the same technique `std::any::Any::type_id`
    /// uses), so its vtable entry always reports the *concrete* module
    /// type, even when called through a `&dyn Module` trait object.
    ///
    /// This exists so callers that only hold a `&dyn Module` (e.g.
    /// [`crate::Application`]'s module-tree walk) can still deduplicate
    /// modules by concrete identity. `std::any::type_name_of_val`/
    /// `TypeId::of` cannot do this from outside the trait: both resolve
    /// their type parameter from the *static* type of the reference
    /// (`dyn Module`), not the concrete type behind the vtable, so every
    /// module would compare equal to every other module.
    fn module_type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }

    /// Returns the type name of the concrete type implementing this trait.
    ///
    /// Like [`Module::module_type_id`], this is monomorphized per concrete
    /// `Self`, so it reports the real module type name (useful for
    /// diagnostics/logging) even through a `&dyn Module` reference — unlike
    /// `std::any::type_name_of_val(&dyn Module)`, which always returns the
    /// trait object's own type name.
    fn module_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Returns the list of provider types to register
    fn providers(&self) -> Vec<ProviderRegistration>;

    /// Returns the list of controller types to register
    fn controllers(&self) -> Vec<ControllerRegistration>;

    /// Returns the list of guard types to register
    fn guards(&self) -> Vec<crate::module::GuardRegistration> {
        vec![]
    }

    /// Returns the list of imported modules
    fn imports(&self) -> Vec<Box<dyn Module>>;

    /// Returns the list of exported provider types
    fn exports(&self) -> Vec<TypeId>;

    /// Returns the list of re-exported modules
    ///
    /// Re-exported modules have their exports forwarded to any module
    /// that imports this module.
    fn re_exports(&self) -> Vec<Box<dyn Module>> {
        vec![]
    }
}

/// Trait for request handlers (route methods)
#[async_trait]
pub trait RequestHandler: Send + Sync {
    /// Handle an HTTP request and return a response
    async fn handle(
        &self,
        request: crate::HttpRequest,
    ) -> Result<crate::HttpResponse, crate::Error>;
}

/// Trait for validators
pub trait Validator: Send + Sync {
    /// Validate a value
    fn validate(&self, value: &str) -> Result<(), String>;
}

/// Definition of a route
#[derive(Clone, Debug)]
pub struct RouteDefinition {
    pub method: HttpMethod,
    pub path: String,
    pub handler_name: String,
}

/// HTTP methods
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpMethod {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
    HEAD,
    OPTIONS,
    /// Safe, idempotent query with a request body
    /// (draft-ietf-httpbis-safe-method-w-body).
    QUERY,
}

impl HttpMethod {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Some(HttpMethod::GET),
            "POST" => Some(HttpMethod::POST),
            "PUT" => Some(HttpMethod::PUT),
            "DELETE" => Some(HttpMethod::DELETE),
            "PATCH" => Some(HttpMethod::PATCH),
            "HEAD" => Some(HttpMethod::HEAD),
            "OPTIONS" => Some(HttpMethod::OPTIONS),
            "QUERY" => Some(HttpMethod::QUERY),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::GET => "GET",
            HttpMethod::POST => "POST",
            HttpMethod::PUT => "PUT",
            HttpMethod::DELETE => "DELETE",
            HttpMethod::PATCH => "PATCH",
            HttpMethod::HEAD => "HEAD",
            HttpMethod::OPTIONS => "OPTIONS",
            HttpMethod::QUERY => "QUERY",
        }
    }
}

/// Registration information for a provider
#[derive(Clone)]
pub struct ProviderRegistration {
    pub type_id: TypeId,
    pub type_name: &'static str,
    /// Function that registers the provider in the container.
    /// Uses the Container wrapper type from armature-core.
    pub register_fn: fn(&crate::container::Container),
}

impl std::fmt::Debug for ProviderRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistration")
            .field("type_id", &self.type_id)
            .field("type_name", &self.type_name)
            .finish()
    }
}

/// Registration information for a controller
#[derive(Clone)]
pub struct ControllerRegistration {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub base_path: &'static str,
    pub factory:
        fn(&crate::Container) -> Result<Box<dyn std::any::Any + Send + Sync>, crate::Error>,
    #[allow(clippy::type_complexity)]
    pub route_registrar: fn(
        &crate::Container,
        &mut crate::Router,
        Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), crate::Error>,
}

impl std::fmt::Debug for ControllerRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerRegistration")
            .field("type_id", &self.type_id)
            .field("type_name", &self.type_name)
            .field("base_path", &self.base_path)
            .finish()
    }
}

impl From<HttpMethod> for crate::Method {
    #[inline]
    fn from(m: HttpMethod) -> Self {
        match m {
            HttpMethod::GET => crate::Method::Get,
            HttpMethod::POST => crate::Method::Post,
            HttpMethod::PUT => crate::Method::Put,
            HttpMethod::DELETE => crate::Method::Delete,
            HttpMethod::PATCH => crate::Method::Patch,
            HttpMethod::HEAD => crate::Method::Head,
            HttpMethod::OPTIONS => crate::Method::Options,
            HttpMethod::QUERY => crate::Method::Query,
            // Deliberately exhaustive with no catch-all. `HttpMethod` is
            // #[non_exhaustive] for downstream crates, but this match is inside
            // the defining crate, so a variant added later fails to compile here
            // rather than silently mapping onto something wrong.
        }
    }
}

/// The method has no `HttpMethod` counterpart, so it is not routable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnroutableMethod(pub String);

impl std::fmt::Display for UnroutableMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "method `{}` has no HttpMethod counterpart", self.0)
    }
}

impl std::error::Error for UnroutableMethod {}

impl TryFrom<&crate::Method> for HttpMethod {
    type Error = UnroutableMethod;

    #[inline]
    fn try_from(m: &crate::Method) -> Result<Self, Self::Error> {
        match m {
            crate::Method::Get => Ok(HttpMethod::GET),
            crate::Method::Post => Ok(HttpMethod::POST),
            crate::Method::Put => Ok(HttpMethod::PUT),
            crate::Method::Delete => Ok(HttpMethod::DELETE),
            crate::Method::Patch => Ok(HttpMethod::PATCH),
            crate::Method::Head => Ok(HttpMethod::HEAD),
            crate::Method::Options => Ok(HttpMethod::OPTIONS),
            crate::Method::Query => Ok(HttpMethod::QUERY),
            crate::Method::Connect => Err(UnroutableMethod("CONNECT".into())),
            crate::Method::Trace => Err(UnroutableMethod("TRACE".into())),
            crate::Method::Other(t) => Err(UnroutableMethod(t.into_owned())),
        }
    }
}

#[cfg(test)]
mod method_conversion_tests {
    use super::HttpMethod;
    use crate::Method;

    #[test]
    fn every_http_method_round_trips_through_method() {
        for m in [
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
            HttpMethod::QUERY,
        ] {
            let converted = Method::from(m.clone());
            assert_eq!(
                HttpMethod::try_from(&converted).ok(),
                Some(m.clone()),
                "{m:?} did not round-trip"
            );
        }
    }

    #[test]
    fn methods_with_no_http_method_counterpart_fail_conversion() {
        // CONNECT, TRACE, and Other exist in armature-h1 because the wire has
        // them; HttpMethod is the *routable* set, which is deliberately smaller.
        // A router that silently mapped CONNECT onto GET would be a security bug.
        assert!(HttpMethod::try_from(&Method::Connect).is_err());
        assert!(HttpMethod::try_from(&Method::Trace).is_err());
        assert!(HttpMethod::try_from(&Method::from("PURGE")).is_err());
    }
}
