//! Regression test: `ARMATURE_*` env-var configuration must take effect
//! from the first use of a log macro alone, without an explicit call to
//! `armature_log::init()`.
//!
//! Before the fix, `LogConfig::from_env()` (which reads `ARMATURE_LOG_LEVEL`
//! etc.) and the runtime atomic sync only ran on an explicit `init()`/
//! `config()` call. The log macros only ever read the (never-updated)
//! default atomics, so env vars were silently ignored despite the crate
//! docs claiming init "is called automatically when first log macro is
//! used."
//!
//! This lives in its own integration-test binary (a fresh process) on
//! purpose: `armature_log`'s config is a process-global `Once`/`Lazy` pair
//! that is initialized at most once per process. Sharing this test with
//! others in the same binary would make the observed outcome depend on
//! unpredictable test execution order (whichever test happens to touch the
//! config first "wins" for the whole process).

#[test]
fn log_macro_auto_initializes_from_env_without_explicit_init() {
    // SAFETY: this is the sole test in this process, and it runs before
    // anything else has touched `armature_log`'s global state or process
    // env, so there is no concurrent access to race with.
    unsafe {
        std::env::set_var("ARMATURE_LOG_LEVEL", "trace");
    }

    // Intentionally NOT calling `armature_log::init()` here — that's
    // exactly the bug under test.
    armature_log::trace!("trigger auto-init via the trace! macro alone");

    assert_eq!(
        armature_log::current_level(),
        armature_log::Level::Trace,
        "ARMATURE_LOG_LEVEL=trace should take effect via macro use alone, \
         without an explicit init() call"
    );
}
