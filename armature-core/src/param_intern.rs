//! Leak-once interning for route parameter names.
//!
//! A request's path params are `(&'static str, Bytes)`: the name comes from the
//! compiled route pattern and the value is a slice of the request target. The
//! name has to outlive the request without being cloned per request, and route
//! registration happens at startup, so leaking one `Box<str>` per distinct
//! parameter name is the whole cost — bounded by the number of route parameters
//! an application declares, not by traffic.
//!
//! This is deliberately not a general-purpose interner, and callers should
//! still treat it as registration-time machinery. But `intern` is reachable
//! from constructors that take a caller-supplied name ([`crate::HttpRequest::push_param`],
//! `from_parts`, `ArenaRequest::to_http_request`), and a caller can feed those a
//! request-derived string. So the table is hard-capped: past [`MAX_INTERNED`]
//! distinct names it stops leaking and returns [`OVERFLOW_NAME`]. Memory is
//! bounded by the cap regardless of what reaches it.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Ceiling on distinct interned names.
///
/// A real application declares route parameters in the tens; four thousand is
/// far above any legitimate route table while capping the leak at a few hundred
/// kilobytes if something request-derived ever reaches [`intern`].
pub const MAX_INTERNED: usize = 4096;

/// Returned by [`intern`] once [`MAX_INTERNED`] distinct names have been
/// interned.
///
/// The leading NUL makes it unrepresentable as a route pattern's parameter
/// name, so a lookup for a real name can never accidentally match a parameter
/// that landed here.
pub const OVERFLOW_NAME: &str = "\0param-intern-overflow";

fn table() -> &'static Mutex<HashSet<&'static str>> {
    static TABLE: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Intern a route parameter name, returning a `&'static str`.
///
/// Call this at route-registration time. The table holds at most
/// [`MAX_INTERNED`] distinct names; beyond that this leaks nothing further and
/// returns [`OVERFLOW_NAME`], so no caller — including one handed a
/// request-derived name — can grow the process without bound.
pub fn intern(name: &str) -> &'static str {
    let mut table = table().lock().expect("param intern table poisoned");
    intern_in(&mut table, name)
}

/// The interning itself, over an explicit table.
///
/// Split out from [`intern`] so the cap can be exercised against a scratch
/// table — a test that filled the process-global one would starve every other
/// caller in the same test binary.
fn intern_in(table: &mut HashSet<&'static str>, name: &str) -> &'static str {
    if let Some(existing) = table.get(name) {
        return existing;
    }
    if table.len() >= MAX_INTERNED {
        return OVERFLOW_NAME;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    table.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::{MAX_INTERNED, OVERFLOW_NAME, intern, intern_in};
    use std::collections::HashSet;

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

    #[test]
    fn the_table_stops_growing_at_the_cap() {
        let mut table = HashSet::new();
        for i in 0..MAX_INTERNED {
            assert_ne!(intern_in(&mut table, &format!("p{i}")), OVERFLOW_NAME);
        }
        assert_eq!(table.len(), MAX_INTERNED);

        // A name already in the table still resolves; only *new* ones are
        // refused, so a legitimate route registered before the cap was hit is
        // unaffected by whatever filled it.
        assert_eq!(intern_in(&mut table, "p0"), "p0");
        assert_eq!(intern_in(&mut table, "one-too-many"), OVERFLOW_NAME);
        assert_eq!(table.len(), MAX_INTERNED);
    }
}
