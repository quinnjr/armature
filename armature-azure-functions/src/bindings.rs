//! Azure Functions bindings scaffolding.
//!
//! Only the [`InputBinding`]/[`OutputBinding`] traits are defined here. They
//! are scaffolding for a future binding layer — the runtime does not yet read
//! inputs from, or flush outputs to, Azure services. No concrete binding
//! implementations are provided; do not advertise binding-based Azure I/O as
//! working functionality.

/// Input binding trait.
pub trait InputBinding: Send + Sync {
    /// Get the binding name.
    fn name(&self) -> &str;

    /// Get the binding type.
    fn binding_type(&self) -> &str;
}

/// Output binding trait.
pub trait OutputBinding: Send + Sync {
    /// Get the binding name.
    fn name(&self) -> &str;

    /// Get the binding type.
    fn binding_type(&self) -> &str;
}
