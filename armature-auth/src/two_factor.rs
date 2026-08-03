//! Two-Factor Authentication (2FA)
//!
//! Provides TOTP (Time-based One-Time Password) and HOTP (HMAC-based One-Time Password)
//! support for two-factor authentication.
//!
//! # Features
//!
//! - TOTP generation and validation
//! - QR code generation for authenticator apps
//! - Backup codes generation
//! - Recovery codes
//!
//! # Usage
//!
//! ```no_run
//! use armature_auth::two_factor::*;
//!
//! # async fn example() -> Result<(), TwoFactorError> {
//! // Generate TOTP secret for user
//! let secret = TotpSecret::generate();
//! println!("Secret: {}", secret.to_base32());
//!
//! // Generate QR code URL for authenticator apps
//! let qr_url = secret.to_qr_url("user@example.com", "MyApp")?;
//! println!("Scan this QR: {}", qr_url);
//!
//! // Verify TOTP code from user
//! let code = "123456"; // From authenticator app
//! if secret.verify(code, 30)? {
//!     println!("2FA code valid!");
//! }
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "two-factor")]
use data_encoding::BASE32_NOPAD;
#[cfg(feature = "two-factor")]
use qrcode::{QrCode, render::svg};
#[cfg(feature = "two-factor")]
use totp_lite::{Sha1, totp_custom};

use rand::RngExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Constant-time string comparison (prevents timing attacks on secret-derived
/// values such as TOTP codes and backup codes).
///
/// Thin wrapper over the shared [`armature_core::crypto::constant_time_eq`]
/// helper so all crates that compare secret-derived values use the same,
/// single verified implementation.
fn constant_time_eq(a: &str, b: &str) -> bool {
    armature_core::crypto::constant_time_eq(a.as_bytes(), b.as_bytes())
}

/// Two-Factor Authentication errors
#[derive(Debug, Error)]
pub enum TwoFactorError {
    #[error("Invalid TOTP code")]
    InvalidCode,

    #[error("Invalid secret")]
    InvalidSecret,

    #[error("QR code generation failed: {0}")]
    QrCodeError(String),

    #[error("Feature not enabled: {0}")]
    FeatureNotEnabled(&'static str),
}

/// TOTP Secret
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpSecret {
    /// Base32-encoded secret
    secret: String,
}

impl TotpSecret {
    /// Generate a new random TOTP secret
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::two_factor::*;
    ///
    /// let secret = TotpSecret::generate();
    /// println!("Secret: {}", secret.to_base32());
    /// ```
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        let bytes: Vec<u8> = (0..20).map(|_| rng.random()).collect();

        #[cfg(feature = "two-factor")]
        let secret = BASE32_NOPAD.encode(&bytes);

        #[cfg(not(feature = "two-factor"))]
        let secret = base64::encode(&bytes);

        Self { secret }
    }

    /// Create TOTP secret from base32 string
    pub fn from_base32(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    /// Get base32-encoded secret
    pub fn to_base32(&self) -> &str {
        &self.secret
    }

    /// Generate TOTP code for current time
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::two_factor::*;
    ///
    /// let secret = TotpSecret::generate();
    /// # #[cfg(feature = "two-factor")]
    /// let code = secret.generate_code(30).unwrap();
    /// # #[cfg(feature = "two-factor")]
    /// println!("Current TOTP: {}", code);
    /// ```
    #[cfg(feature = "two-factor")]
    pub fn generate_code(&self, time_step: u64) -> Result<String, TwoFactorError> {
        let secret_bytes = BASE32_NOPAD
            .decode(self.secret.as_bytes())
            .map_err(|_| TwoFactorError::InvalidSecret)?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // totp-lite returns the code already zero-padded to the requested digit count.
        let code = totp_custom::<Sha1>(time_step, 6, &secret_bytes, timestamp);
        Ok(code)
    }

    #[cfg(not(feature = "two-factor"))]
    pub fn generate_code(&self, _time_step: u64) -> Result<String, TwoFactorError> {
        Err(TwoFactorError::FeatureNotEnabled("two-factor"))
    }

    /// Verify TOTP code
    ///
    /// Checks code against current time ± window (default: 1 time step = 30s before/after).
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::two_factor::*;
    ///
    /// # #[cfg(feature = "two-factor")]
    /// # fn example() -> Result<(), TwoFactorError> {
    /// let secret = TotpSecret::generate();
    /// let code = secret.generate_code(30)?;
    ///
    /// // Verify the code
    /// assert!(secret.verify(&code, 30)?);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "two-factor")]
    pub fn verify(&self, code: &str, time_step: u64) -> Result<bool, TwoFactorError> {
        let secret_bytes = BASE32_NOPAD
            .decode(self.secret.as_bytes())
            .map_err(|_| TwoFactorError::InvalidSecret)?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Check current time and ±1 time window
        for offset in [-1, 0, 1] {
            let check_time = ((timestamp as i64) + (offset * time_step as i64)) as u64;
            let expected_code = totp_custom::<Sha1>(time_step, 6, &secret_bytes, check_time);

            if constant_time_eq(&expected_code, code) {
                return Ok(true);
            }
        }

        Ok(false)
    }

    #[cfg(not(feature = "two-factor"))]
    pub fn verify(&self, _code: &str, _time_step: u64) -> Result<bool, TwoFactorError> {
        Err(TwoFactorError::FeatureNotEnabled("two-factor"))
    }

    /// Generate QR code URL for authenticator apps
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::two_factor::*;
    ///
    /// # fn example() -> Result<(), TwoFactorError> {
    /// let secret = TotpSecret::generate();
    /// let url = secret.to_qr_url("user@example.com", "MyApp")?;
    /// println!("otpauth:// URL: {}", url);
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_qr_url(&self, account: &str, issuer: &str) -> Result<String, TwoFactorError> {
        let url = format!(
            "otpauth://totp/{}:{}?secret={}&issuer={}",
            urlencoding::encode(issuer),
            urlencoding::encode(account),
            self.secret,
            urlencoding::encode(issuer)
        );
        Ok(url)
    }

    /// Generate QR code SVG
    ///
    /// Returns SVG string that can be rendered in HTML.
    #[cfg(feature = "two-factor")]
    pub fn to_qr_svg(&self, account: &str, issuer: &str) -> Result<String, TwoFactorError> {
        let url = self.to_qr_url(account, issuer)?;

        let code =
            QrCode::new(url.as_bytes()).map_err(|e| TwoFactorError::QrCodeError(e.to_string()))?;

        let svg = code.render::<svg::Color>().min_dimensions(200, 200).build();

        Ok(svg)
    }

    #[cfg(not(feature = "two-factor"))]
    pub fn to_qr_svg(&self, _account: &str, _issuer: &str) -> Result<String, TwoFactorError> {
        Err(TwoFactorError::FeatureNotEnabled("two-factor"))
    }
}

/// Backup/Recovery codes
///
/// Generate one-time use backup codes for account recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupCodes {
    /// List of backup codes
    pub codes: Vec<String>,
}

impl BackupCodes {
    /// Number of random bytes behind each backup code. Backup codes are 2FA
    /// bypass credentials: presenting one substitutes for the second factor
    /// entirely, so the code must be as hard to guess as the factor it replaces.
    /// Eight bytes are drawn and all eight are rendered — an earlier version
    /// dropped half of them, leaving 32 bits, which is within reach of an online
    /// guessing campaign against a known account.
    const CODE_BYTES: usize = 8;

    /// Generate backup codes
    ///
    /// Each code carries `CODE_BYTES` bytes (64 bits) of entropy, rendered as
    /// four hyphen-separated groups of four hex digits for transcription.
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_auth::two_factor::*;
    ///
    /// let codes = BackupCodes::generate(10);
    /// for code in &codes.codes {
    ///     println!("Backup code: {}", code);
    /// }
    /// ```
    pub fn generate(count: usize) -> Self {
        let mut rng = rand::rng();
        let codes = (0..count)
            .map(|_| {
                let bytes: Vec<u8> = (0..Self::CODE_BYTES).map(|_| rng.random()).collect();
                let hex = hex::encode(bytes);
                hex.as_bytes()
                    .chunks(4)
                    .map(|c| std::str::from_utf8(c).expect("hex is ASCII"))
                    .collect::<Vec<_>>()
                    .join("-")
            })
            .collect();

        Self { codes }
    }

    /// Verify and consume a backup code
    ///
    /// Returns true if code was valid and removes it from the list.
    ///
    /// Every stored code is compared against the candidate in constant time
    /// and the scan always runs to completion (no early exit on match), so
    /// neither which byte differs nor which position matches is observable
    /// via timing.
    pub fn verify_and_consume(&mut self, code: &str) -> bool {
        let mut found: Option<usize> = None;
        for (i, c) in self.codes.iter().enumerate() {
            if constant_time_eq(c, code) {
                found = found.or(Some(i));
            }
        }

        if let Some(pos) = found {
            self.codes.remove(pos);
            true
        } else {
            false
        }
    }

    /// Check remaining codes
    pub fn remaining(&self) -> usize {
        self.codes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_totp_secret() {
        let secret = TotpSecret::generate();
        assert!(!secret.to_base32().is_empty());
    }

    #[test]
    #[cfg(feature = "two-factor")]
    fn test_generate_and_verify_totp() {
        let secret = TotpSecret::generate();
        let code = secret.generate_code(30).unwrap();
        assert!(secret.verify(&code, 30).unwrap());
    }

    #[test]
    #[cfg(feature = "two-factor")]
    fn test_verify_totp_rejects_wrong_code() {
        // Functional-correctness check for the constant-time comparison in
        // `TotpSecret::verify`: an incorrect code must still be rejected.
        let secret = TotpSecret::generate();
        let code = secret.generate_code(30).unwrap();

        // Flip the last digit to produce a guaranteed-wrong, same-length code.
        let mut wrong = code.clone();
        let last = wrong.pop().unwrap();
        let flipped = if last == '0' { '1' } else { '0' };
        wrong.push(flipped);

        assert!(!secret.verify(&wrong, 30).unwrap());
        // Different-length input must also be rejected without panicking.
        assert!(!secret.verify(&format!("{}9", code), 30).unwrap());
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("123456", "123456"));
        assert!(!constant_time_eq("123456", "123457"));
        assert!(!constant_time_eq("123456", "1234567"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn test_qr_url() {
        let secret = TotpSecret::generate();
        let url = secret.to_qr_url("user@example.com", "MyApp").unwrap();
        assert!(url.starts_with("otpauth://totp/"));
        // The account name is percent-encoded (per RFC 3986) before being
        // embedded in the URL, so "@" becomes "%40" rather than appearing
        // literally — this is correct, safe encoding, not a bug.
        assert!(url.contains("user%40example.com"));
        assert!(url.contains("MyApp"));
    }

    #[test]
    fn test_backup_codes() {
        let codes = BackupCodes::generate(10);
        assert_eq!(codes.codes.len(), 10);

        for code in &codes.codes {
            assert!(code.contains('-'));
        }
    }

    #[test]
    fn test_backup_codes_use_full_entropy() {
        // A backup code bypasses the second factor, so its guessability is the
        // security of 2FA for that account. The generator previously rendered
        // only 4 of the 16 hex digits it drew, leaving 32 bits.
        let codes = BackupCodes::generate(64);

        for code in &codes.codes {
            let hex: String = code.chars().filter(|c| *c != '-').collect();
            assert_eq!(
                hex.len(),
                BackupCodes::CODE_BYTES * 2,
                "every drawn byte must reach the code: {code}"
            );
            assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        }

        // With 64 bits per code, 64 codes colliding is not something that
        // happens by chance — it happens when entropy is being discarded.
        let unique: std::collections::HashSet<&String> = codes.codes.iter().collect();
        assert_eq!(unique.len(), codes.codes.len());
    }

    #[test]
    fn test_backup_code_consumption() {
        let mut codes = BackupCodes::generate(5);
        let first_code = codes.codes[0].clone();

        assert_eq!(codes.remaining(), 5);
        assert!(codes.verify_and_consume(&first_code));
        assert_eq!(codes.remaining(), 4);
        assert!(!codes.verify_and_consume(&first_code)); // Already used
    }

    #[test]
    fn test_backup_code_rejects_unknown_code() {
        // Functional-correctness check for the constant-time scan in
        // `BackupCodes::verify_and_consume`: a code that was never issued
        // must be rejected and the list left untouched.
        let mut codes = BackupCodes::generate(5);
        assert!(!codes.verify_and_consume("0000-0000"));
        assert_eq!(codes.remaining(), 5);
    }

    #[test]
    fn test_backup_code_consumes_any_matching_position() {
        // The scan no longer short-circuits on the first match; make sure a
        // code in the middle/end of the list is still found and consumed
        // correctly, and that only that one code is removed.
        let mut codes = BackupCodes::generate(5);
        let last_code = codes.codes[4].clone();

        assert!(codes.verify_and_consume(&last_code));
        assert_eq!(codes.remaining(), 4);
        assert!(!codes.codes.contains(&last_code));
    }
}
