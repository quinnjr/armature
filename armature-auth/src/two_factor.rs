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

            if expected_code == code {
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
    /// Generate backup codes
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
                let bytes: Vec<u8> = (0..8).map(|_| rng.random()).collect();
                let hex = hex::encode(bytes);
                format!("{}-{}", &hex[..4], &hex[4..8])
            })
            .collect();

        Self { codes }
    }

    /// Verify and consume a backup code
    ///
    /// Returns true if code was valid and removes it from the list.
    pub fn verify_and_consume(&mut self, code: &str) -> bool {
        if let Some(pos) = self.codes.iter().position(|c| c == code) {
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
    fn test_qr_url() {
        let secret = TotpSecret::generate();
        let url = secret.to_qr_url("user@example.com", "MyApp").unwrap();
        assert!(url.starts_with("otpauth://totp/"));
        assert!(url.contains("user@example.com"));
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
    fn test_backup_code_consumption() {
        let mut codes = BackupCodes::generate(5);
        let first_code = codes.codes[0].clone();

        assert_eq!(codes.remaining(), 5);
        assert!(codes.verify_and_consume(&first_code));
        assert_eq!(codes.remaining(), 4);
        assert!(!codes.verify_and_consume(&first_code)); // Already used
    }
}
