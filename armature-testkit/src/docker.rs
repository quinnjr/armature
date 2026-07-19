//! Docker availability detection and a self-skip macro for container tests.

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

/// Return early from the calling test (with a skip notice) when Docker is
/// unavailable. Use at the top of a `#[cfg(feature = "containers")]` test.
#[macro_export]
macro_rules! skip_if_no_docker {
    () => {
        if !$crate::docker::docker_available() {
            eprintln!("skipping: Docker not available");
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
        crate::skip_if_no_docker!();
    }
}
