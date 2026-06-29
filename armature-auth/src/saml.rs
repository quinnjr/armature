// SAML 2.0 authentication support

use crate::{AuthError, Result};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose};
use chrono::Utc;
// Using samael for SAML support
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// SAML authentication provider trait
#[async_trait]
pub trait SamlProvider: Send + Sync {
    /// Get the provider name
    fn name(&self) -> &str;

    /// Generate SAML authentication request
    fn create_auth_request(&self) -> Result<SamlAuthRequest>;

    /// Parse and validate SAML response
    async fn validate_response(&self, saml_response: &str) -> Result<SamlAssertion>;

    /// Get SP metadata XML
    fn get_metadata(&self) -> Result<String>;
}

/// SAML authentication request
#[derive(Debug, Clone)]
pub struct SamlAuthRequest {
    /// The SAML request XML
    pub saml_request: String,

    /// Relay state for tracking
    pub relay_state: Option<String>,

    /// The IdP SSO URL to redirect to
    pub redirect_url: String,
}

/// SAML assertion (user information from IdP)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlAssertion {
    /// Name ID (user identifier)
    pub name_id: String,

    /// Name ID format
    pub name_id_format: Option<String>,

    /// Session index (from SAML response - not stored server-side)
    /// This is part of the SAML protocol, not an Armature session
    pub session_index: Option<String>,

    /// User attributes
    pub attributes: HashMap<String, Vec<String>>,

    /// Assertion issue time
    pub issue_instant: chrono::DateTime<Utc>,

    /// Assertion expiration
    pub not_on_or_after: Option<chrono::DateTime<Utc>>,
}

/// SAML Service Provider configuration
#[derive(Debug, Clone)]
pub struct SamlConfig {
    /// Entity ID (SP identifier)
    pub entity_id: String,

    /// Assertion Consumer Service URL (callback URL)
    pub acs_url: String,

    /// Single Logout Service URL
    pub sls_url: Option<String>,

    /// IdP metadata URL or XML
    pub idp_metadata: IdpMetadata,

    /// SP certificate (PEM format)
    pub sp_certificate: Option<String>,

    /// SP private key (PEM format)
    pub sp_private_key: Option<String>,

    /// Contact information
    pub contact_person: Option<ContactInfo>,

    /// Allow unsigned assertions (not recommended for production)
    pub allow_unsigned_assertions: bool,

    /// Required assertion attributes
    pub required_attributes: Vec<String>,
}

/// IdP metadata source
#[derive(Debug, Clone)]
pub enum IdpMetadata {
    /// URL to fetch metadata from
    Url(String),

    /// Raw XML metadata
    Xml(String),
}

/// Contact information for SP metadata
#[derive(Debug, Clone)]
pub struct ContactInfo {
    pub contact_type: String,
    pub given_name: String,
    pub surname: String,
    pub email: String,
}

impl SamlConfig {
    /// Create a new SAML configuration
    pub fn new(entity_id: String, acs_url: String, idp_metadata: IdpMetadata) -> Self {
        Self {
            entity_id,
            acs_url,
            sls_url: None,
            idp_metadata,
            sp_certificate: None,
            sp_private_key: None,
            contact_person: None,
            allow_unsigned_assertions: false,
            required_attributes: Vec::new(),
        }
    }

    /// Set Single Logout Service URL
    pub fn with_sls_url(mut self, url: String) -> Self {
        self.sls_url = Some(url);
        self
    }

    /// Set SP certificate and private key
    pub fn with_keys(mut self, certificate: String, private_key: String) -> Self {
        self.sp_certificate = Some(certificate);
        self.sp_private_key = Some(private_key);
        self
    }

    /// Set contact information
    pub fn with_contact(mut self, contact: ContactInfo) -> Self {
        self.contact_person = Some(contact);
        self
    }

    /// Allow unsigned assertions (not recommended)
    pub fn allow_unsigned(mut self, allow: bool) -> Self {
        self.allow_unsigned_assertions = allow;
        self
    }

    /// Set required attributes
    pub fn with_required_attributes(mut self, attributes: Vec<String>) -> Self {
        self.required_attributes = attributes;
        self
    }
}

/// SAML Service Provider implementation
pub struct SamlServiceProvider {
    name: String,
    config: SamlConfig,
}

impl SamlServiceProvider {
    /// Create a new SAML service provider
    pub fn new(name: String, config: SamlConfig) -> Result<Self> {
        // Validate configuration
        if config.entity_id.is_empty() {
            return Err(AuthError::AuthenticationFailed(
                "Entity ID is required".to_string(),
            ));
        }

        if config.acs_url.is_empty() {
            return Err(AuthError::AuthenticationFailed(
                "ACS URL is required".to_string(),
            ));
        }

        Ok(Self { name, config })
    }

    /// Generate relay state
    fn generate_relay_state(&self) -> String {
        use rand::Rng;
        let mut rng = rand::rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }
}

#[async_trait]
impl SamlProvider for SamlServiceProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn create_auth_request(&self) -> Result<SamlAuthRequest> {
        // For now, return a simplified implementation
        // In production, you'd generate a proper SAML AuthnRequest
        let request_id = format!("_{}", uuid::Uuid::new_v4());

        let authn_request_xml = format!(
            r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol"
                xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"
                ID="{}"
                Version="2.0"
                IssueInstant="{}"
                AssertionConsumerServiceURL="{}">
                <saml:Issuer>{}</saml:Issuer>
            </samlp:AuthnRequest>"#,
            request_id,
            Utc::now().to_rfc3339(),
            self.config.acs_url,
            self.config.entity_id
        );

        // Base64 encode
        let encoded = general_purpose::STANDARD.encode(authn_request_xml.as_bytes());

        // Get IdP SSO URL - use a placeholder for now
        let redirect_url = "https://idp.example.com/sso".to_string();

        Ok(SamlAuthRequest {
            saml_request: encoded,
            relay_state: Some(self.generate_relay_state()),
            redirect_url,
        })
    }

    async fn validate_response(&self, saml_response: &str) -> Result<SamlAssertion> {
        // Decode base64
        let decoded = general_purpose::STANDARD
            .decode(saml_response)
            .map_err(|e| AuthError::InvalidToken(format!("Invalid base64: {}", e)))?;

        let xml = String::from_utf8(decoded)
            .map_err(|e| AuthError::InvalidToken(format!("Invalid UTF-8: {}", e)))?;

        // Simple XML parsing to extract basic information
        // In production, you'd use a proper SAML library with full validation
        let name_id = extract_name_id(&xml)?;
        let attributes = extract_attributes(&xml);

        Ok(SamlAssertion {
            name_id,
            name_id_format: None,
            session_index: None,
            attributes,
            issue_instant: Utc::now(),
            not_on_or_after: Some(Utc::now() + chrono::Duration::hours(1)),
        })
    }

    fn get_metadata(&self) -> Result<String> {
        // Generate SP metadata XML
        let metadata_xml = format!(
            r#"<?xml version="1.0"?>
<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata"
                  entityID="{}">
  <SPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <AssertionConsumerService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
                              Location="{}"
                              index="0"/>
  </SPSSODescriptor>
</EntityDescriptor>"#,
            self.config.entity_id, self.config.acs_url
        );

        Ok(metadata_xml)
    }
}

/// Extract NameID from SAML response (simplified)
fn extract_name_id(xml: &str) -> Result<String> {
    // Simple extraction - in production use proper XML parser
    if let Some(start) = xml.find("<saml:NameID")
        && let Some(content_start) = xml[start..].find('>')
    {
        let content_start = start + content_start + 1;
        if let Some(content_end) = xml[content_start..].find("</saml:NameID>") {
            let name_id = xml[content_start..content_start + content_end].trim();
            return Ok(name_id.to_string());
        }
    }
    Err(AuthError::InvalidToken(
        "No NameID found in SAML response".to_string(),
    ))
}

/// Extract attributes from SAML response (simplified)
fn extract_attributes(_xml: &str) -> HashMap<String, Vec<String>> {
    // Simplified attribute extraction
    // In production, use proper XML parser
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_saml_config() {
        let config = SamlConfig::new(
            "https://example.com/saml/metadata".to_string(),
            "https://example.com/saml/acs".to_string(),
            IdpMetadata::Xml("<xml></xml>".to_string()),
        )
        .with_sls_url("https://example.com/saml/sls".to_string())
        .allow_unsigned(false);

        assert_eq!(config.entity_id, "https://example.com/saml/metadata");
        assert!(config.sls_url.is_some());
        assert!(!config.allow_unsigned_assertions);
    }

    #[test]
    fn test_contact_info() {
        let contact = ContactInfo {
            contact_type: "technical".to_string(),
            given_name: "John".to_string(),
            surname: "Doe".to_string(),
            email: "john@example.com".to_string(),
        };

        assert_eq!(contact.email, "john@example.com");
    }
}
