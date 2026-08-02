//! Shared helpers for the controller-struct guard/middleware integration tests.
//!
//! These drive the real `#[module]`-generated route registrar against a real
//! [`Router`], so the tests exercise the emitted codegen rather than a mock.

use armature_core::{Container, HttpRequest, Module, Router};
use std::collections::HashMap;

/// Build a [`Router`] by running every controller registration of `M` through
/// the macro-generated factory + route registrar.
pub fn build_router<M: Module + Default>() -> Router {
    let container = Container::new();
    let mut router = Router::new();
    let module = M::default();
    for reg in module.controllers() {
        let instance = (reg.factory)(&container).expect("controller factory");
        (reg.route_registrar)(&container, &mut router, instance).expect("route registrar");
    }
    router
}

/// A bare `GET` request for `path`.
pub fn get_request(path: &str) -> HttpRequest {
    HttpRequest::from_parts(
        "GET",
        path.to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        HashMap::new(),
    )
}
