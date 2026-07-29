//! Shared clock helper for the JWT-signing providers.
//!
//! Lives here rather than in `apns.rs` because FCM signs a JWT too and needs
//! exactly the same treatment: the two modules previously diverged, with APNS
//! returning a structured error where FCM panicked on identical input.

use crate::{PushError, Result};

/// Seconds since the UNIX epoch, or a `Config` error if the system clock is
/// set before it.
///
/// `SystemTime::duration_since(UNIX_EPOCH).unwrap()` is a real panic on a VM
/// resumed from a snapshot or a container with an unsynced RTC. On APNS that
/// panic used to fire while the `refresh_lock` was held, poisoning the mutex
/// and making every later send panic forever; on FCM it unwound straight
/// through the caller's task from inside `send()`. Both now surface a
/// [`PushError::Config`].
pub(crate) fn unix_now_secs() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| {
            PushError::Config(
                "system clock is before the UNIX epoch; cannot sign a JWT".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_now_secs_reports_a_plausible_clock() {
        // Guards the replacement of `duration_since(UNIX_EPOCH).unwrap()`:
        // the happy path must still yield real epoch seconds, and the error
        // path is a `Config` error rather than a panic inside a held lock.
        let now = unix_now_secs().expect("a sane clock is after the epoch");
        assert!(now > 1_600_000_000, "got {now}");
    }
}
