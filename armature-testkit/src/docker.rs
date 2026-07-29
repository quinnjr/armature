//! Docker availability detection and a self-skip macro for container tests.
//!
//! Container-gated tests normally self-skip when no Docker daemon is
//! reachable, so a developer without Docker still gets a green `cargo test`.
//! That is exactly the wrong behaviour in CI, where Docker *is* present and a
//! silent skip turns a broken suite into a green pass. Setting
//! [`REQUIRE_DOCKER_ENV`] (`ARMATURE_REQUIRE_DOCKER=1`) flips the skip into a
//! hard failure.

use std::io::Write as _;

/// Environment variable that turns a Docker skip into a hard failure.
///
/// Set it to `1` (CI does this) to assert that container-gated suites really
/// execute instead of self-skipping.
pub const REQUIRE_DOCKER_ENV: &str = "ARMATURE_REQUIRE_DOCKER";

/// Returns true if a Docker daemon is reachable (`docker info` succeeds).
pub fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Returns true when `ARMATURE_REQUIRE_DOCKER=1` is set in the environment.
///
/// Any other value (including unset, empty, or `0`) leaves the default
/// self-skip behaviour in place — the panic never fires unless the variable is
/// explicitly opted into.
pub fn require_docker_env() -> bool {
    std::env::var(REQUIRE_DOCKER_ENV).as_deref() == Ok("1")
}

/// Decide whether a container-gated test should proceed.
///
/// * Docker available → `true` (run the test).
/// * Docker unavailable and `require` unset → `false` (skip), after printing a
///   notice straight to the process stderr so it survives libtest's output
///   capture and is visible without `--nocapture`.
/// * Docker unavailable and `require` set → panic naming
///   [`REQUIRE_DOCKER_ENV`].
///
/// Taking both inputs as parameters keeps the decision pure and testable
/// without depending on the ambient Docker state or mutating the environment.
///
/// # Panics
///
/// Panics when `available` is false and `require` is true.
#[track_caller]
pub fn docker_gate(available: bool, require: bool) -> bool {
    if available {
        return true;
    }
    if require {
        panic!(
            "{REQUIRE_DOCKER_ENV}=1 is set but no Docker daemon is reachable: \
             this container-gated test must run, not skip. Start Docker, or \
             unset {REQUIRE_DOCKER_ENV} to allow self-skipping."
        );
    }
    // Bypass libtest's stdout/stderr capture (which only intercepts the
    // `print!`/`eprint!` macros) so the skip is visible without `--nocapture`.
    let _ = writeln!(
        std::io::stderr(),
        "skipping: Docker not available (set {REQUIRE_DOCKER_ENV}=1 to make this a failure)"
    );
    false
}

/// Return early from the calling test (with a visible skip notice) when Docker
/// is unavailable, or panic when `ARMATURE_REQUIRE_DOCKER=1` demands that
/// container-gated tests actually run. Use at the top of a
/// `#[cfg(feature = "containers")]` test.
#[macro_export]
macro_rules! skip_if_no_docker {
    () => {
        if !$crate::docker::docker_gate(
            $crate::docker::docker_available(),
            $crate::docker::require_docker_env(),
        ) {
            return;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_available_is_a_bool_and_never_panics() {
        // Must not panic whether or not Docker is installed.
        let _ = docker_available();
    }

    #[test]
    fn skip_macro_runs_without_panicking() {
        // If Docker is absent this returns early; if present it falls through.
        // Guarded so the suite still passes on a CI runner that sets
        // ARMATURE_REQUIRE_DOCKER without a reachable daemon.
        if docker_available() || !require_docker_env() {
            crate::skip_if_no_docker!();
        }
    }

    #[test]
    fn gate_proceeds_when_docker_is_available() {
        assert!(docker_gate(true, false));
        assert!(docker_gate(true, true));
    }

    #[test]
    fn gate_skips_when_docker_missing_and_not_required() {
        assert!(!docker_gate(false, false));
    }

    #[test]
    fn gate_panics_when_docker_missing_and_required() {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(|| docker_gate(false, true));
        std::panic::set_hook(hook);

        let payload =
            result.expect_err("docker_gate must panic when Docker is required but absent");
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(
            message.contains(REQUIRE_DOCKER_ENV),
            "panic message must name the env var, got: {message}"
        );
    }

    #[test]
    fn require_docker_env_only_opts_in_on_exactly_one() {
        // Only the exact value "1" opts in; unset/empty/"0"/"true" all leave
        // self-skipping in place so the panic can never fire by default.
        let ambient = std::env::var(REQUIRE_DOCKER_ENV);
        assert_eq!(require_docker_env(), ambient.as_deref() == Ok("1"));
        if ambient.is_err() {
            assert!(
                !require_docker_env(),
                "an unset {REQUIRE_DOCKER_ENV} must never require Docker"
            );
        }
    }
}
