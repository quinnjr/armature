//! Script execution context.

use crate::bindings::{RequestBinding, ResponseBinding};
use rhai::{Dynamic, Map, Scope};
use std::collections::HashMap;
use std::sync::Arc;

/// Context for script execution.
///
/// Provides access to request data, shared state, and utilities.
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// Request binding.
    pub request: RequestBinding,
    /// Shared state accessible to scripts.
    pub state: Arc<HashMap<String, Dynamic>>,
    /// Request-scoped data.
    pub locals: HashMap<String, Dynamic>,
}

impl ScriptContext {
    /// Create a new script context.
    pub fn new(request: RequestBinding) -> Self {
        Self {
            request,
            state: Arc::new(HashMap::new()),
            locals: HashMap::new(),
        }
    }

    /// Create context with shared state.
    pub fn with_state(request: RequestBinding, state: Arc<HashMap<String, Dynamic>>) -> Self {
        Self {
            request,
            state,
            locals: HashMap::new(),
        }
    }

    /// Set a local value.
    pub fn set_local(&mut self, key: impl Into<String>, value: Dynamic) {
        self.locals.insert(key.into(), value);
    }

    /// Get a local value.
    pub fn get_local(&self, key: &str) -> Option<&Dynamic> {
        self.locals.get(key)
    }

    /// Get a state value.
    pub fn get_state(&self, key: &str) -> Option<&Dynamic> {
        self.state.get(key)
    }

    /// Build a Rhai scope from this context.
    pub fn into_scope(self) -> Scope<'static> {
        let mut scope = Scope::new();

        // Add request
        scope.push_constant("request", self.request);

        // Add response builder (for chaining)
        scope.push("response", ResponseBinding::new());

        // Add state as a map
        let mut state_map = Map::new();
        for (key, value) in self.state.iter() {
            state_map.insert(key.clone().into(), value.clone());
        }
        scope.push_constant("state", state_map);

        // Add locals
        for (key, value) in self.locals {
            scope.push(&key, value);
        }

        scope
    }
}

/// Builder for creating script contexts.
pub struct ScriptContextBuilder {
    request: Option<RequestBinding>,
    state: HashMap<String, Dynamic>,
    locals: HashMap<String, Dynamic>,
}

impl Default for ScriptContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptContextBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            request: None,
            state: HashMap::new(),
            locals: HashMap::new(),
        }
    }

    /// Set the request.
    pub fn request(mut self, request: RequestBinding) -> Self {
        self.request = Some(request);
        self
    }

    /// Add a state value.
    pub fn state(mut self, key: impl Into<String>, value: impl Into<Dynamic>) -> Self {
        self.state.insert(key.into(), value.into());
        self
    }

    /// Add a local value.
    pub fn local(mut self, key: impl Into<String>, value: impl Into<Dynamic>) -> Self {
        self.locals.insert(key.into(), value.into());
        self
    }

    /// Build the context.
    pub fn build(self) -> Option<ScriptContext> {
        let request = self.request?;
        Some(ScriptContext {
            request,
            state: Arc::new(self.state),
            locals: self.locals,
        })
    }
}
