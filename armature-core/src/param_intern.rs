//! Leak-once interning for route parameter names.
//!
//! A request's path params are `(&'static str, Bytes)`: the name comes from the
//! compiled route pattern and the value is a slice of the request target. The
//! name has to outlive the request without being cloned per request, and route
//! registration happens at startup, so leaking one `Box<str>` per distinct
//! parameter name is the whole cost — bounded by the number of route parameters
//! an application declares, not by traffic.
//!
//! This is deliberately not a general-purpose interner. Do not call it with
//! request-derived strings: that would let a client grow the process without
//! bound.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn table() -> &'static Mutex<HashSet<&'static str>> {
    static TABLE: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Intern a route parameter name, returning a `&'static str`.
///
/// Call this at route-registration time only.
pub fn intern(name: &str) -> &'static str {
    let mut table = table().lock().expect("param intern table poisoned");
    if let Some(existing) = table.get(name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    table.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::intern;

    #[test]
    fn interning_the_same_name_twice_yields_the_same_pointer() {
        let a = intern("user_id");
        let b = intern("user_id");
        // Pointer equality, not just string equality: the point of interning is
        // that a route's param names are allocated once at startup, never per
        // request.
        assert!(std::ptr::eq(a.as_ptr(), b.as_ptr()));
        assert_eq!(a, "user_id");
    }

    #[test]
    fn distinct_names_are_distinct() {
        assert_eq!(intern("a"), "a");
        assert_eq!(intern("b"), "b");
        assert!(!std::ptr::eq(intern("a").as_ptr(), intern("b").as_ptr()));
    }
}
