//! Constant-time cryptographic comparison helpers.
//!
//! Provides a single, shared implementation of constant-time equality for
//! comparing secret-derived values (API tokens, HMAC signatures, TOTP/HOTP
//! codes, backup codes, etc.) without leaking timing information about
//! *where* two byte strings first differ.
//!
//! Consumers that only have `&str` values (tokens, codes, signatures) should
//! call [`constant_time_eq`] with `.as_bytes()`.

/// Compare two byte slices for equality in constant time (with respect to
/// their content, not their length).
///
/// # Why this matters
///
/// A naive `a == b` comparison (or one built from `==` on `str`/`[u8]`)
/// short-circuits as soon as a mismatching byte is found. For secret-derived
/// values such as API tokens, HMAC signatures, or TOTP codes, the resulting
/// timing difference can leak how many leading bytes of a submitted value
/// matched the expected value, letting an attacker recover the secret one
/// byte at a time via repeated timing measurements.
///
/// # Implementation
///
/// The length check is performed first and is allowed to short-circuit:
/// lengths are not secret-dependent information worth protecting (callers
/// generally know the expected length of a token/signature/code already).
/// The byte-by-byte comparison, however, never branches on the data — it
/// XOR-accumulates the difference across the *entire* length of the inputs
/// regardless of where (or whether) a mismatch occurs, so the number of
/// CPU cycles spent does not depend on the content being compared.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (byte_a, byte_b) in a.iter().zip(b.iter()) {
        result |= byte_a ^ byte_b;
    }

    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }
}
