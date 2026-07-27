//! Regression test: calling `armature_log::config()` (a getter) alone must
//! perform the *same full* atomic initialization as `init()` — level,
//! debug, format, color, timestamps, and module path — not just a partial
//! subset.
//!
//! Before the fix, `LogConfig::from_env()` (forced by `config()` via the
//! `CONFIG` lazy) only synced the `DEBUG_ENABLED`/`LOG_LEVEL` atomics as a
//! side effect; `LOG_FORMAT`/`LOG_COLOR`/`LOG_TIMESTAMPS`/`LOG_MODULE_PATH`
//! were only ever synced inside `init()`. So calling `config()` without
//! also calling `init()` left the runtime format/color/etc. atomics at
//! their hardcoded defaults regardless of env vars — an inconsistent
//! partial init depending on call order.
//!
//! This must observe the effect through the runtime atomics (via
//! `current_format()`), not through `config()`'s returned `&LogConfig`
//! struct fields: `from_env()` always computed `LogConfig.format` correctly
//! from env, struct-field-wise — the bug was specifically that the
//! separately-tracked runtime atomics (which `log()` actually reads to
//! decide real output formatting) went unsynced.
//!
//! Own process/binary, for the same reason as `auto_init_via_macro.rs`:
//! the global config is initialized at most once per process, so this test
//! must be the sole thing that ever touches it here.

#[test]
fn config_getter_performs_full_atomic_init_not_just_level_and_debug() {
    // SAFETY: sole test in this binary; runs before anything else touches
    // armature_log's global state.
    unsafe {
        std::env::set_var("ARMATURE_LOG_FORMAT", "compact");
    }

    // The getter alone -- not init() -- must be enough to sync the format
    // atomic from env.
    let _ = armature_log::config();

    assert_eq!(
        armature_log::current_format(),
        armature_log::Format::Compact,
        "config() must sync LOG_FORMAT (and color/timestamps/module_path) \
         from env, not just level/debug"
    );
}
