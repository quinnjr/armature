/// ACME challenge types and handling
use serde::{Deserialize, Serialize};

/// ACME authorization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Authorization {
    /// Authorization identifier
    pub identifier: crate::order::Identifier,

    /// Authorization status
    pub status: AuthorizationStatus,

    /// Expiration timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,

    /// List of challenges
    pub challenges: Vec<Challenge>,

    /// Wildcard indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wildcard: Option<bool>,
}

/// Authorization status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationStatus {
    /// Authorization is pending
    Pending,
    /// Authorization is valid
    Valid,
    /// Authorization is invalid
    Invalid,
    /// Authorization is deactivated
    Deactivated,
    /// Authorization has expired
    Expired,
    /// Authorization is revoked
    Revoked,
}

/// ACME challenge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Challenge {
    /// Challenge type
    #[serde(rename = "type")]
    pub challenge_type: String,

    /// Challenge URL
    pub url: String,

    /// Challenge status
    pub status: ChallengeStatus,

    /// Challenge token.
    ///
    /// Defaults to empty when absent: some newer challenge types advertised by
    /// a CA (e.g. `dns-persist-01`) carry no token, and must not break parsing
    /// of the surrounding authorization.
    #[serde(default)]
    pub token: String,

    /// Validation record (for DNS challenges)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validated: Option<String>,

    /// Error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

/// Challenge status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChallengeStatus {
    /// Challenge is pending
    Pending,
    /// Challenge is processing
    Processing,
    /// Challenge is valid
    Valid,
    /// Challenge is invalid
    Invalid,
}

/// HTTP-01 challenge data
#[derive(Debug, Clone)]
pub struct Http01Challenge {
    /// Challenge token
    pub token: String,

    /// Key authorization
    pub key_authorization: String,

    /// Challenge URL
    pub url: String,
}

impl Http01Challenge {
    /// Get the path where the challenge should be served
    pub fn path(&self) -> String {
        format!("/.well-known/acme-challenge/{}", self.token)
    }

    /// Get the content that should be served at the challenge path
    pub fn content(&self) -> &str {
        &self.key_authorization
    }
}

/// DNS-01 challenge data
#[derive(Debug, Clone)]
pub struct Dns01Challenge {
    /// Challenge token
    pub token: String,

    /// DNS record value (base64url encoded SHA-256 hash)
    pub dns_value: String,

    /// Challenge URL
    pub url: String,
}

impl Dns01Challenge {
    /// Get the DNS record name
    pub fn record_name(&self, domain: &str) -> String {
        format!("_acme-challenge.{}", domain)
    }

    /// Get the DNS record value
    pub fn record_value(&self) -> &str {
        &self.dns_value
    }
}

/// TLS-ALPN-01 challenge data
#[derive(Debug, Clone)]
pub struct TlsAlpn01Challenge {
    /// Challenge token
    pub token: String,

    /// Key authorization
    pub key_authorization: String,

    /// Challenge URL
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http01_challenge_path() {
        let challenge = Http01Challenge {
            token: "test_token".to_string(),
            key_authorization: "test_key_auth".to_string(),
            url: "https://example.com/challenge".to_string(),
        };

        assert_eq!(challenge.path(), "/.well-known/acme-challenge/test_token");
        assert_eq!(challenge.content(), "test_key_auth");
    }

    #[test]
    fn test_dns01_challenge_record() {
        let challenge = Dns01Challenge {
            token: "test_token".to_string(),
            dns_value: "test_dns_value".to_string(),
            url: "https://example.com/challenge".to_string(),
        };

        assert_eq!(
            challenge.record_name("example.com"),
            "_acme-challenge.example.com"
        );
        assert_eq!(challenge.record_value(), "test_dns_value");
    }

    #[test]
    fn test_challenge_status_serialization() {
        let status = ChallengeStatus::Valid;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"valid\"");
    }
}
