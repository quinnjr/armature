/// ACME client configuration
use std::path::PathBuf;

/// ACME directory URLs for common providers
pub mod directories {
    /// Let's Encrypt production directory
    pub const LETS_ENCRYPT_PRODUCTION: &str = "https://acme-v02.api.letsencrypt.org/directory";

    /// Let's Encrypt staging directory (for testing)
    pub const LETS_ENCRYPT_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

    /// ZeroSSL production directory
    pub const ZEROSSL: &str = "https://acme.zerossl.com/v2/DV90";

    /// BuyPass production directory
    pub const BUYPASS: &str = "https://api.buypass.com/acme/directory";

    /// Google Trust Services
    pub const GOOGLE: &str = "https://dv.acme-v02.api.pki.goog/directory";
}

/// Challenge types supported by ACME
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeType {
    /// HTTP-01 challenge (port 80)
    Http01,
    /// DNS-01 challenge (DNS TXT record)
    Dns01,
    /// TLS-ALPN-01 challenge (port 443)
    TlsAlpn01,
}

/// ACME client configuration
#[derive(Debug, Clone)]
pub struct AcmeConfig {
    /// ACME directory URL
    pub directory_url: String,

    /// Contact email for account registration
    pub contact_email: Vec<String>,

    /// Domains to obtain certificates for
    pub domains: Vec<String>,

    /// Preferred challenge type
    pub challenge_type: ChallengeType,

    /// Directory to store certificates and keys
    pub cert_dir: PathBuf,

    /// Directory to store account credentials
    pub account_dir: PathBuf,

    /// Renew certificate when this many days remain
    pub renew_before_days: u32,

    /// Accept terms of service automatically
    pub accept_tos: bool,

    /// Use external account binding (EAB)
    pub eab_kid: Option<String>,

    /// EAB HMAC key
    pub eab_hmac_key: Option<String>,

    /// Additional CA root certificate(s) (PEM) to trust for the ACME endpoint.
    ///
    /// Use this to talk to a private or test ACME CA (e.g. Pebble/Boulder)
    /// whose HTTPS interface is signed by a root not in the system trust store.
    pub ca_certificate: Option<Vec<u8>>,
}

impl AcmeConfig {
    /// Create a new ACME configuration
    ///
    /// # Example
    ///
    /// ```
    /// use armature_acme::{AcmeConfig, ChallengeType};
    ///
    /// let config = AcmeConfig::new(
    ///     "https://acme-v02.api.letsencrypt.org/directory",
    ///     vec!["admin@example.com".to_string()],
    ///     vec!["example.com".to_string(), "www.example.com".to_string()],
    /// );
    /// ```
    pub fn new(
        directory_url: impl Into<String>,
        contact_email: Vec<String>,
        domains: Vec<String>,
    ) -> Self {
        Self {
            directory_url: directory_url.into(),
            contact_email,
            domains,
            challenge_type: ChallengeType::Http01,
            cert_dir: PathBuf::from("./certs"),
            account_dir: PathBuf::from("./accounts"),
            renew_before_days: 30,
            accept_tos: false,
            eab_kid: None,
            eab_hmac_key: None,
            ca_certificate: None,
        }
    }

    /// Create configuration for Let's Encrypt production
    ///
    /// # Example
    ///
    /// ```
    /// use armature_acme::AcmeConfig;
    ///
    /// let config = AcmeConfig::lets_encrypt_production(
    ///     vec!["admin@example.com".to_string()],
    ///     vec!["example.com".to_string()],
    /// );
    /// ```
    pub fn lets_encrypt_production(contact_email: Vec<String>, domains: Vec<String>) -> Self {
        Self::new(directories::LETS_ENCRYPT_PRODUCTION, contact_email, domains)
    }

    /// Create configuration for Let's Encrypt staging (testing)
    ///
    /// # Example
    ///
    /// ```
    /// use armature_acme::AcmeConfig;
    ///
    /// let config = AcmeConfig::lets_encrypt_staging(
    ///     vec!["admin@example.com".to_string()],
    ///     vec!["example.com".to_string()],
    /// );
    /// ```
    pub fn lets_encrypt_staging(contact_email: Vec<String>, domains: Vec<String>) -> Self {
        Self::new(directories::LETS_ENCRYPT_STAGING, contact_email, domains)
    }

    /// Create configuration for ZeroSSL
    pub fn zerossl(
        contact_email: Vec<String>,
        domains: Vec<String>,
        eab_kid: String,
        eab_hmac_key: String,
    ) -> Self {
        let mut config = Self::new(directories::ZEROSSL, contact_email, domains);
        config.eab_kid = Some(eab_kid);
        config.eab_hmac_key = Some(eab_hmac_key);
        config
    }

    /// Set the challenge type
    pub fn with_challenge_type(mut self, challenge_type: ChallengeType) -> Self {
        self.challenge_type = challenge_type;
        self
    }

    /// Set the certificate directory
    pub fn with_cert_dir(mut self, cert_dir: PathBuf) -> Self {
        self.cert_dir = cert_dir;
        self
    }

    /// Set the account directory
    pub fn with_account_dir(mut self, account_dir: PathBuf) -> Self {
        self.account_dir = account_dir;
        self
    }

    /// Set renewal threshold (days before expiry)
    pub fn with_renew_before_days(mut self, days: u32) -> Self {
        self.renew_before_days = days;
        self
    }

    /// Accept terms of service automatically
    pub fn with_accept_tos(mut self, accept: bool) -> Self {
        self.accept_tos = accept;
        self
    }

    /// Set external account binding credentials
    pub fn with_eab(mut self, kid: String, hmac_key: String) -> Self {
        self.eab_kid = Some(kid);
        self.eab_hmac_key = Some(hmac_key);
        self
    }

    /// Trust an additional CA root certificate (PEM) for the ACME endpoint.
    ///
    /// Required to talk to a private or test ACME CA whose HTTPS interface is
    /// signed by a root outside the system trust store.
    pub fn with_ca_certificate(mut self, pem: Vec<u8>) -> Self {
        self.ca_certificate = Some(pem);
        self
    }
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self::lets_encrypt_staging(
            vec!["admin@example.com".to_string()],
            vec!["example.com".to_string()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acme_config_new() {
        let config = AcmeConfig::new(
            directories::LETS_ENCRYPT_PRODUCTION,
            vec!["test@example.com".to_string()],
            vec!["example.com".to_string()],
        );

        assert_eq!(config.directory_url, directories::LETS_ENCRYPT_PRODUCTION);
        assert_eq!(config.contact_email, vec!["test@example.com"]);
        assert_eq!(config.domains, vec!["example.com"]);
        assert_eq!(config.challenge_type, ChallengeType::Http01);
    }

    #[test]
    fn test_lets_encrypt_production() {
        let config = AcmeConfig::lets_encrypt_production(
            vec!["admin@example.com".to_string()],
            vec!["example.com".to_string()],
        );

        assert_eq!(config.directory_url, directories::LETS_ENCRYPT_PRODUCTION);
    }

    #[test]
    fn test_lets_encrypt_staging() {
        let config = AcmeConfig::lets_encrypt_staging(
            vec!["admin@example.com".to_string()],
            vec!["example.com".to_string()],
        );

        assert_eq!(config.directory_url, directories::LETS_ENCRYPT_STAGING);
    }

    #[test]
    fn test_builder_pattern() {
        let config = AcmeConfig::lets_encrypt_production(
            vec!["admin@example.com".to_string()],
            vec!["example.com".to_string()],
        )
        .with_challenge_type(ChallengeType::Dns01)
        .with_cert_dir(PathBuf::from("/etc/certs"))
        .with_renew_before_days(14)
        .with_accept_tos(true);

        assert_eq!(config.challenge_type, ChallengeType::Dns01);
        assert_eq!(config.cert_dir, PathBuf::from("/etc/certs"));
        assert_eq!(config.renew_before_days, 14);
        assert!(config.accept_tos);
    }

    #[test]
    fn test_zerossl_config() {
        let config = AcmeConfig::zerossl(
            vec!["admin@example.com".to_string()],
            vec!["example.com".to_string()],
            "kid123".to_string(),
            "hmac456".to_string(),
        );

        assert_eq!(config.directory_url, directories::ZEROSSL);
        assert_eq!(config.eab_kid, Some("kid123".to_string()));
        assert_eq!(config.eab_hmac_key, Some("hmac456".to_string()));
    }
}
