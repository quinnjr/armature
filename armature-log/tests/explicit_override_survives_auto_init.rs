//! Safety-net regression test for the auto-init mechanism itself (T7 item
//! 1): an explicit `set_level()` call made *before* anything else has
//! touched `armature_log` must not later be clobbered by the env-derived
//! auto-init that fires on first log-macro use.
//!
//! This isn't a literal audit finding, but it guards against a real risk
//! introduced by wiring auto-init into every entry point: the crate's own
//! docs recommend calling `armature_log::preset_development()` /
//! `configure()...apply()` (which use `set_level`/`set_format`/etc.) as
//! the very first line of `main()`. If those setters didn't themselves run
//! `ensure_init()` before applying their explicit store, a *later* first
//! use of a log macro would force `CONFIG` from env and overwrite the
//! user's explicit choice — silently discarding it.
//!
//! Own process/binary for the same isolation reason as the other T7
//! integration tests: this only proves anything if it's the sole thing
//! that ever forces the global config in this process.

#[test]
fn explicit_set_level_before_any_other_call_is_not_overwritten_by_later_auto_init() {
    // SAFETY: sole test in this process; runs before anything else touches
    // armature_log's global state or process env.
    unsafe {
        std::env::set_var("ARMATURE_LOG_LEVEL", "error");
    }

    // Explicit override happens first, before anything else has touched
    // the global config -- exactly the documented "Option 2: Programmatic
    // Configuration" quick-start pattern.
    armature_log::set_level(armature_log::Level::Debug);

    // A later macro use is still the first-ever CONFIG force in this
    // process (set_level() itself doesn't force `CONFIG`'s Lazy through to
    // completion beyond ensure_init's one-time seed, which already ran as
    // part of set_level above) -- it must not clobber the explicit value.
    armature_log::info!("later log use must not reset the explicit level");

    assert_eq!(
        armature_log::current_level(),
        armature_log::Level::Debug,
        "an explicit set_level() call must not be overwritten by the \
         env-derived auto-init that fires on first log-macro use"
    );
}
