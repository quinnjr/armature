//! Deterministic, offline test harnesses for verifying Armature integrations.

/// Temporary smoke marker proving the crate compiles and tests run.
pub fn crate_smoke() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        assert!(crate_smoke());
    }
}
