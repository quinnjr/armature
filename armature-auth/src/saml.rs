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
        #[cfg(feature = "saml")]
        {
            self.create_auth_request_from_metadata()
        }

        // Without the `saml` feature there is no way to parse the IdP metadata or (when
        // configured) sign the AuthnRequest, so refuse rather than fabricate a request
        // pointing at a placeholder endpoint.
        #[cfg(not(feature = "saml"))]
        {
            Err(AuthError::SamlValidation(
                "SAML AuthnRequest generation requires the `saml` feature".to_string(),
            ))
        }
    }

    async fn validate_response(&self, saml_response: &str) -> Result<SamlAssertion> {
        #[cfg(feature = "saml")]
        {
            self.validate_response_verified(saml_response)
        }

        // Without the `saml` feature there is no XML-signature verification available.
        // A SAML validator that cannot verify signatures is worse than an explicit
        // unsupported error, so refuse rather than accept an unverified assertion.
        #[cfg(not(feature = "saml"))]
        {
            let _ = saml_response;
            Err(AuthError::SamlValidation(
                "SAML response validation requires the `saml` feature (XML signature \
                 verification); refusing to validate without it"
                    .to_string(),
            ))
        }
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

/// Build the outbound SAML AuthnRequest, resolving the IdP's SingleSignOnService endpoint
/// from the configured IdP metadata (never a hardcoded placeholder), and signing the
/// request when SP credentials (`sp_certificate` + `sp_private_key`) are configured.
#[cfg(feature = "saml")]
impl SamlServiceProvider {
    fn create_auth_request_from_metadata(&self) -> Result<SamlAuthRequest> {
        use samael::crypto::{Crypto, CryptoProvider};
        use samael::metadata::{EntityDescriptor, HTTP_POST_BINDING, HTTP_REDIRECT_BINDING};
        use samael::service_provider::ServiceProviderBuilder;
        use samael::traits::ToXml;

        // 1. Parse the IdP metadata XML into an EntityDescriptor. The SingleSignOnService
        //    endpoint(s) used below live in this metadata.
        let idp_xml = match &self.config.idp_metadata {
            IdpMetadata::Xml(xml) => xml.clone(),
            IdpMetadata::Url(_) => {
                return Err(AuthError::SamlValidation(
                    "IdP metadata must be provided as XML to generate an AuthnRequest; \
                     URL-based metadata fetching is not implemented here"
                        .to_string(),
                ));
            }
        };

        let idp_metadata: EntityDescriptor = idp_xml.parse().map_err(|e| {
            AuthError::SamlValidation(format!("Failed to parse IdP metadata XML: {e}"))
        })?;

        let sp = ServiceProviderBuilder::default()
            .entity_id(Some(self.config.entity_id.clone()))
            .acs_url(Some(self.config.acs_url.clone()))
            .idp_metadata(idp_metadata)
            .allow_idp_initiated(true)
            .build()
            .map_err(|e| {
                AuthError::SamlValidation(format!("Failed to build SAML service provider: {e}"))
            })?;

        // 2. Resolve the real IdP SSO endpoint from metadata (prefer HTTP-Redirect, the
        //    conventional binding for sending an AuthnRequest, falling back to HTTP-POST).
        let idp_sso_url = sp
            .sso_binding_location(HTTP_REDIRECT_BINDING)
            .or_else(|| sp.sso_binding_location(HTTP_POST_BINDING))
            .ok_or_else(|| {
                AuthError::SamlValidation(
                    "IdP metadata does not contain a SingleSignOnService endpoint".to_string(),
                )
            })?;

        // 3. Build the AuthnRequest (issuer = entity_id, ACS = acs_url) addressed to the
        //    resolved IdP endpoint.
        let mut authn_request = sp.make_authentication_request(&idp_sso_url).map_err(|e| {
            AuthError::SamlValidation(format!("Failed to build SAML AuthnRequest: {e}"))
        })?;

        // 4. Sign the AuthnRequest when SP credentials are configured. `sp_private_key` and
        //    `sp_certificate` are otherwise-unused config knobs without this. Signing
        //    requires an enveloped `<ds:Signature>` template (referencing the request's own
        //    ID and carrying the SP certificate) to be present before serialization; xmlsec
        //    fills in the digest/signature values over that template.
        let request_xml = if let (Some(cert_pem), Some(private_key_pem)) =
            (&self.config.sp_certificate, &self.config.sp_private_key)
        {
            use samael::crypto::CertificateDer;
            use samael::signature::Signature;

            let cert_der: CertificateDer = pem_to_der(cert_pem)?.into();
            let private_key_der = pem_to_der(private_key_pem)?;

            authn_request.signature = Some(Signature::template(&authn_request.id, &cert_der));

            let unsigned_xml = authn_request.to_string().map_err(|e| {
                AuthError::SamlValidation(format!("Failed to serialize SAML AuthnRequest: {e:?}"))
            })?;

            Crypto::sign_xml(&unsigned_xml, &private_key_der).map_err(|e| {
                AuthError::SamlValidation(format!("Failed to sign SAML AuthnRequest: {e}"))
            })?
        } else {
            authn_request.to_string().map_err(|e| {
                AuthError::SamlValidation(format!("Failed to serialize SAML AuthnRequest: {e:?}"))
            })?
        };

        let encoded = general_purpose::STANDARD.encode(request_xml.as_bytes());

        Ok(SamlAuthRequest {
            saml_request: encoded,
            relay_state: Some(self.generate_relay_state()),
            redirect_url: idp_sso_url,
        })
    }
}

/// Decode a PEM-encoded key/certificate body into raw DER bytes, ignoring the
/// `-----BEGIN ...-----` / `-----END ...-----` header and footer lines. Used for SP
/// AuthnRequest signing, where samael's `Crypto::sign_xml` expects DER key bytes.
#[cfg(feature = "saml")]
fn pem_to_der(pem: &str) -> Result<Vec<u8>> {
    let mut body = String::new();
    for line in pem.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("-----") {
            continue;
        }
        body.push_str(line);
    }
    general_purpose::STANDARD
        .decode(body)
        .map_err(|e| AuthError::SamlValidation(format!("Failed to decode PEM data: {e}")))
}

/// Real SAML response verification (enveloped XML signature + SAML conditions).
///
/// This is the security-critical path: it parses the base64 SAML Response, verifies
/// the enveloped XML digital signature against the IdP signing certificate taken from
/// the configured IdP metadata, and enforces the SAML conditions (issuer, audience,
/// `NotBefore`/`NotOnOrAfter`, subject confirmation) via samael before returning any
/// assertion data. Any failure results in an `Err` — an assertion is never trusted
/// without a verified signature unless `allow_unsigned_assertions` is explicitly set.
#[cfg(feature = "saml")]
impl SamlServiceProvider {
    fn validate_response_verified(&self, saml_response: &str) -> Result<SamlAssertion> {
        use samael::metadata::EntityDescriptor;
        use samael::service_provider::ServiceProviderBuilder;

        // 1. Obtain IdP metadata XML and parse it into an EntityDescriptor. The signing
        //    certificate(s) used to verify the response signature live in this metadata.
        let idp_xml = match &self.config.idp_metadata {
            IdpMetadata::Xml(xml) => xml.clone(),
            IdpMetadata::Url(_) => {
                return Err(AuthError::SamlValidation(
                    "IdP metadata must be provided as XML for response validation; \
                     URL-based metadata fetching is not implemented here"
                        .to_string(),
                ));
            }
        };

        let mut idp_metadata: EntityDescriptor = idp_xml.parse().map_err(|e| {
            AuthError::SamlValidation(format!("Failed to parse IdP metadata XML: {e}"))
        })?;

        // When unsigned assertions are explicitly allowed, strip the IdP signing
        // certificates so samael skips signature verification. Conditions (issuer,
        // audience, expiry, subject confirmation) are still fully enforced below.
        if self.config.allow_unsigned_assertions
            && let Some(descriptors) = idp_metadata.idp_sso_descriptors.as_mut()
        {
            for descriptor in descriptors {
                descriptor.key_descriptors.clear();
            }
        }

        let sp = ServiceProviderBuilder::default()
            .entity_id(Some(self.config.entity_id.clone()))
            .acs_url(Some(self.config.acs_url.clone()))
            .idp_metadata(idp_metadata)
            // The trait's validate_response does not carry the original AuthnRequest ID,
            // so we accept IdP-initiated flows. Signature, issuer, audience and expiry
            // remain fully enforced; only the InResponseTo correlation is relaxed.
            .allow_idp_initiated(true)
            .build()
            .map_err(|e| {
                AuthError::SamlValidation(format!("Failed to build SAML service provider: {e}"))
            })?;

        // 2. Fail closed: if signatures are required (the default) but the IdP metadata
        //    carries no signing certificate, refuse rather than silently skipping the
        //    signature check (which samael would do when no certs are present).
        if !self.config.allow_unsigned_assertions {
            let signing_certs = sp.idp_signing_certs().map_err(|e| {
                AuthError::SamlValidation(format!("Failed to read IdP signing certificates: {e}"))
            })?;
            if signing_certs.is_none() {
                return Err(AuthError::SamlValidation(
                    "IdP metadata contains no signing certificate; refusing to validate a \
                     SAML response without signature verification"
                        .to_string(),
                ));
            }
        }

        // 3. Verify the enveloped XML signature and enforce SAML conditions. This is the
        //    single call that closes the SSO bypass: it rejects unsigned, tampered,
        //    expired, wrong-audience and wrong-issuer responses.
        let assertion = sp.parse_base64_response(saml_response, None).map_err(|e| {
            AuthError::SamlValidation(format!(
                "SAML response signature/condition check failed: {e}"
            ))
        })?;

        // 4. Extract attributes and enforce required attributes.
        let attributes = extract_saml_attributes(&assertion);
        for required in &self.config.required_attributes {
            if !attributes.contains_key(required) {
                return Err(AuthError::SamlValidation(format!(
                    "Required SAML attribute '{required}' is missing from the assertion"
                )));
            }
        }

        // 5. Build the returned assertion from verified data (real NotOnOrAfter, not now()+1h).
        let subject = assertion.subject.as_ref();
        let name_id = subject
            .and_then(|s| s.name_id.as_ref())
            .map(|n| n.value.clone())
            .ok_or_else(|| {
                AuthError::SamlValidation("SAML assertion is missing a NameID".to_string())
            })?;
        let name_id_format = subject
            .and_then(|s| s.name_id.as_ref())
            .and_then(|n| n.format.clone());

        let session_index = assertion
            .authn_statements
            .as_ref()
            .and_then(|statements| statements.first())
            .and_then(|statement| statement.session_index.clone());

        let not_on_or_after = assertion
            .conditions
            .as_ref()
            .and_then(|conditions| conditions.not_on_or_after);

        Ok(SamlAssertion {
            name_id,
            name_id_format,
            session_index,
            attributes,
            issue_instant: assertion.issue_instant,
            not_on_or_after,
        })
    }
}

/// Collect SAML attributes from a verified assertion into a name -> values map.
#[cfg(feature = "saml")]
fn extract_saml_attributes(assertion: &samael::schema::Assertion) -> HashMap<String, Vec<String>> {
    let mut attributes: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(statements) = &assertion.attribute_statements {
        for statement in statements {
            for attribute in &statement.attributes {
                let Some(name) = attribute
                    .name
                    .clone()
                    .or_else(|| attribute.friendly_name.clone())
                else {
                    continue;
                };
                let values = attribute
                    .values
                    .iter()
                    .filter_map(|v| v.value.clone())
                    .collect::<Vec<_>>();
                attributes.entry(name).or_default().extend(values);
            }
        }
    }
    attributes
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

/// Security tests for real SAML signature + condition verification (closes SSO bypass).
///
/// Fixtures are minted deterministically at runtime: a fresh RSA key/cert acts as the
/// IdP, we build a SAML Response, set its validity windows, and sign it with that key.
/// The matching certificate is embedded in the IdP metadata handed to the validator, so
/// no brittle external fixtures are needed and every reject-case is exercised end to end.
#[cfg(all(test, feature = "saml"))]
mod saml_verification_tests {
    use super::*;
    use base64::engine::general_purpose;
    use chrono::{Duration, Utc};
    use samael::crypto::{Crypto, CryptoProvider};
    use samael::idp::response_builder::{ResponseAttribute, build_response_template};
    use samael::idp::sp_extractor::RequiredAttribute;
    use samael::idp::{CertificateParams, IdentityProvider, KeyType, Rsa};
    use samael::traits::ToXml;

    const IDP_ENTITY_ID: &str = "https://idp.example.test/metadata";
    const SP_ENTITY_ID: &str = "https://sp.example.test/metadata";
    const ACS_URL: &str = "https://sp.example.test/saml/acs";
    const NAME_ID: &str = "user@example.test";

    /// Options controlling how a fixture Response is minted.
    struct MintOptions {
        issuer: String,
        audience: String,
        conditions_not_on_or_after: chrono::DateTime<Utc>,
        subject_not_on_or_after: chrono::DateTime<Utc>,
        /// When false, produce a genuinely unsigned Response (no signature at all).
        sign: bool,
    }

    impl Default for MintOptions {
        fn default() -> Self {
            let future = Utc::now() + Duration::hours(1);
            Self {
                issuer: IDP_ENTITY_ID.to_string(),
                audience: SP_ENTITY_ID.to_string(),
                conditions_not_on_or_after: future,
                subject_not_on_or_after: future,
                sign: true,
            }
        }
    }

    /// A minted fixture: the base64-encoded SAML Response plus the IdP metadata XML
    /// (containing the signing certificate) that a validator must be configured with.
    struct Fixture {
        response_b64: String,
        idp_metadata_xml: String,
    }

    fn mint(opts: MintOptions) -> Fixture {
        let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048))
            .expect("failed to generate IdP key");
        let cert = idp
            .create_certificate(&CertificateParams {
                common_name: IDP_ENTITY_ID,
                issuer_name: IDP_ENTITY_ID,
                days_until_expiration: 3650,
            })
            .expect("failed to mint IdP certificate");

        let attrs = vec![
            ResponseAttribute {
                required_attribute: RequiredAttribute {
                    name: "email".to_string(),
                    format: Some("urn:oasis:names:tc:SAML:2.0:attrname-format:basic".to_string()),
                },
                value: NAME_ID,
            },
            ResponseAttribute {
                required_attribute: RequiredAttribute {
                    name: "role".to_string(),
                    format: Some("urn:oasis:names:tc:SAML:2.0:attrname-format:basic".to_string()),
                },
                value: "admin",
            },
        ];

        let mut response = build_response_template(
            &cert,
            NAME_ID,
            &opts.audience,
            &opts.issuer,
            ACS_URL,
            "_test_request_id",
            &attrs,
        );

        // Set the validity windows on the assertion before signing so that the signature
        // covers them (the template leaves them empty, which SP validation rejects).
        let not_before = Utc::now() - Duration::minutes(5);
        if let Some(assertion) = response.assertion.as_mut() {
            if let Some(conditions) = assertion.conditions.as_mut() {
                conditions.not_before = Some(not_before);
                conditions.not_on_or_after = Some(opts.conditions_not_on_or_after);
            }
            if let Some(confirmations) = assertion
                .subject
                .as_mut()
                .and_then(|s| s.subject_confirmations.as_mut())
            {
                for confirmation in confirmations {
                    if let Some(data) = confirmation.subject_confirmation_data.as_mut() {
                        data.not_before = Some(not_before);
                        data.not_on_or_after = Some(opts.subject_not_on_or_after);
                    }
                }
            }
        }

        if !opts.sign {
            // Genuinely unsigned: drop the signature template entirely.
            response.signature = None;
        }

        let unsigned_xml = response.to_string().expect("failed to serialize Response");
        let response_xml = if opts.sign {
            Crypto::sign_xml(
                unsigned_xml.as_str(),
                idp.export_private_key_der()
                    .expect("failed to export IdP key")
                    .as_slice(),
            )
            .expect("failed to sign Response")
        } else {
            unsigned_xml
        };

        let cert_b64 = general_purpose::STANDARD.encode(cert.der_data());
        let idp_metadata_xml = format!(
            r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY_ID}">
  <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:KeyDescriptor use="signing">
      <ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#">
        <ds:X509Data>
          <ds:X509Certificate>{cert_b64}</ds:X509Certificate>
        </ds:X509Data>
      </ds:KeyInfo>
    </md:KeyDescriptor>
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#
        );

        Fixture {
            response_b64: general_purpose::STANDARD.encode(response_xml.as_bytes()),
            idp_metadata_xml,
        }
    }

    fn provider_from(idp_metadata_xml: &str, allow_unsigned: bool) -> SamlServiceProvider {
        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(idp_metadata_xml.to_string()),
        )
        .allow_unsigned(allow_unsigned)
        .with_required_attributes(vec!["email".to_string()]);
        SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build failed")
    }

    async fn validate(fixture: &Fixture, allow_unsigned: bool) -> Result<SamlAssertion> {
        let provider = provider_from(&fixture.idp_metadata_xml, allow_unsigned);
        provider.validate_response(&fixture.response_b64).await
    }

    // (a) A validly-signed response is accepted, and the returned NameID, attributes and
    //     expiry match what was minted.
    #[tokio::test]
    async fn valid_signed_response_is_accepted() {
        let expiry = Utc::now() + Duration::hours(2);
        let fixture = mint(MintOptions {
            conditions_not_on_or_after: expiry,
            ..Default::default()
        });

        let assertion = validate(&fixture, false)
            .await
            .expect("validly-signed response must be accepted");

        assert_eq!(assertion.name_id, NAME_ID);
        assert_eq!(
            assertion.attributes.get("email"),
            Some(&vec![NAME_ID.to_string()])
        );
        assert_eq!(
            assertion.attributes.get("role"),
            Some(&vec!["admin".to_string()])
        );
        // The real NotOnOrAfter from the assertion is returned (not now()+1h).
        let returned = assertion.not_on_or_after.expect("expiry should be present");
        assert!((returned - expiry).num_seconds().abs() < 2);
    }

    // (b) An unsigned response with allow_unsigned_assertions=false is rejected.
    #[tokio::test]
    async fn unsigned_response_is_rejected_when_signatures_required() {
        let fixture = mint(MintOptions {
            sign: false,
            ..Default::default()
        });
        let err = validate(&fixture, false)
            .await
            .expect_err("unsigned response must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // (c) A response whose signed content was tampered after signing is rejected.
    #[tokio::test]
    async fn tampered_response_is_rejected() {
        let fixture = mint(MintOptions::default());
        // Decode, swap the signed NameID for an attacker identity, re-encode: the
        // enveloped signature no longer matches the (now altered) signed content.
        let xml = String::from_utf8(
            general_purpose::STANDARD
                .decode(&fixture.response_b64)
                .unwrap(),
        )
        .unwrap();
        assert!(xml.contains(NAME_ID), "fixture should contain the NameID");
        let tampered = xml.replace(NAME_ID, "attacker@evil.test");
        let tampered_fixture = Fixture {
            response_b64: general_purpose::STANDARD.encode(tampered.as_bytes()),
            idp_metadata_xml: fixture.idp_metadata_xml,
        };

        let err = validate(&tampered_fixture, false)
            .await
            .expect_err("tampered response must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // (d) A response whose NotOnOrAfter has already passed is rejected.
    #[tokio::test]
    async fn expired_response_is_rejected() {
        let past = Utc::now() - Duration::hours(1);
        let fixture = mint(MintOptions {
            conditions_not_on_or_after: past,
            subject_not_on_or_after: past,
            ..Default::default()
        });
        let err = validate(&fixture, false)
            .await
            .expect_err("expired response must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // (e1) A response addressed to the wrong audience is rejected.
    #[tokio::test]
    async fn wrong_audience_is_rejected() {
        let fixture = mint(MintOptions {
            audience: "https://attacker.example.test/metadata".to_string(),
            ..Default::default()
        });
        let err = validate(&fixture, false)
            .await
            .expect_err("wrong-audience response must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // (e2) A response from the wrong issuer is rejected.
    #[tokio::test]
    async fn wrong_issuer_is_rejected() {
        let fixture = mint(MintOptions {
            issuer: "https://attacker.example.test/metadata".to_string(),
            ..Default::default()
        });
        let err = validate(&fixture, false)
            .await
            .expect_err("wrong-issuer response must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // Guard: signatures required but the IdP metadata has no signing certificate -> refuse.
    #[tokio::test]
    async fn missing_idp_cert_is_rejected_when_signatures_required() {
        let metadata = format!(
            r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY_ID}">
  <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol"/>
</md:EntityDescriptor>"#
        );
        let fixture = mint(MintOptions::default());
        let provider = provider_from(&metadata, false);
        let err = provider
            .validate_response(&fixture.response_b64)
            .await
            .expect_err("must refuse when no signing cert is configured");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // A missing required attribute is rejected even for an otherwise-valid signed response.
    #[tokio::test]
    async fn missing_required_attribute_is_rejected() {
        let fixture = mint(MintOptions::default());
        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(fixture.idp_metadata_xml.clone()),
        )
        .with_required_attributes(vec!["department".to_string()]);
        let provider =
            SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build");
        let err = provider
            .validate_response(&fixture.response_b64)
            .await
            .expect_err("missing required attribute must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }
}

/// Tests for `create_auth_request`: the redirect must point at the IdP's real
/// SingleSignOnService endpoint (resolved from the configured IdP metadata), never a
/// hardcoded placeholder, and the AuthnRequest must be signed when SP credentials are set.
#[cfg(all(test, feature = "saml"))]
mod saml_auth_request_tests {
    use super::*;
    use base64::engine::general_purpose;
    use samael::idp::{CertificateParams, IdentityProvider, KeyType, Rsa};

    const SP_ENTITY_ID: &str = "https://sp.example.test/metadata";
    const ACS_URL: &str = "https://sp.example.test/saml/acs";
    const IDP_ENTITY_ID: &str = "https://idp.example.test/metadata";
    const IDP_SSO_REDIRECT_URL: &str = "https://idp.example.test/sso/redirect";
    const IDP_SSO_POST_URL: &str = "https://idp.example.test/sso/post";

    fn idp_metadata_xml() -> String {
        format!(
            r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY_ID}">
  <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="{IDP_SSO_REDIRECT_URL}"/>
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="{IDP_SSO_POST_URL}"/>
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#
        )
    }

    fn der_to_pem(der: &[u8], label: &str) -> String {
        let b64 = general_purpose::STANDARD.encode(der);
        let mut pem = format!("-----BEGIN {label}-----\n");
        for chunk in b64.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(chunk).unwrap());
            pem.push('\n');
        }
        pem.push_str(&format!("-----END {label}-----\n"));
        pem
    }

    // (a) The generated redirect points at the IdP's metadata SSO endpoint (HTTP-Redirect
    //     preferred), not a hardcoded `idp.example.com` placeholder.
    #[test]
    fn redirect_url_resolves_from_idp_metadata() {
        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(idp_metadata_xml()),
        );
        let provider =
            SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build");

        let auth_request = provider
            .create_auth_request()
            .expect("auth request generation should succeed");

        assert_eq!(auth_request.redirect_url, IDP_SSO_REDIRECT_URL);
        assert_ne!(auth_request.redirect_url, "https://idp.example.com/sso");

        let decoded = String::from_utf8(
            general_purpose::STANDARD
                .decode(&auth_request.saml_request)
                .expect("saml_request should be valid base64"),
        )
        .expect("decoded AuthnRequest should be valid UTF-8");

        assert!(decoded.contains("AuthnRequest"));
        assert!(decoded.contains(SP_ENTITY_ID), "issuer should be entity_id");
        assert!(
            decoded.contains(ACS_URL),
            "AssertionConsumerServiceURL should be acs_url"
        );
    }

    // (b) Falls back to the HTTP-POST SSO binding when no HTTP-Redirect binding is present.
    #[test]
    fn redirect_url_falls_back_to_post_binding() {
        let metadata = format!(
            r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY_ID}">
  <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">
    <md:SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="{IDP_SSO_POST_URL}"/>
  </md:IDPSSODescriptor>
</md:EntityDescriptor>"#
        );
        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(metadata),
        );
        let provider =
            SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build");

        let auth_request = provider
            .create_auth_request()
            .expect("auth request generation should succeed");

        assert_eq!(auth_request.redirect_url, IDP_SSO_POST_URL);
    }

    // (c) Missing SSO endpoint in metadata is a clear error, not a silent placeholder.
    #[test]
    fn missing_sso_endpoint_is_rejected() {
        let metadata = format!(
            r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY_ID}">
  <md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol"/>
</md:EntityDescriptor>"#
        );
        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(metadata),
        );
        let provider =
            SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build");

        let err = provider
            .create_auth_request()
            .expect_err("missing SSO endpoint must be rejected");
        assert!(matches!(err, AuthError::SamlValidation(_)), "got {err:?}");
    }

    // (d) When SP certificate + private key are configured, the AuthnRequest is signed
    //     (an enveloped <Signature> is present in the decoded request XML).
    #[test]
    fn auth_request_is_signed_when_sp_keys_configured() {
        let sp_idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048))
            .expect("failed to generate SP key");
        let cert = sp_idp
            .create_certificate(&CertificateParams {
                common_name: SP_ENTITY_ID,
                issuer_name: SP_ENTITY_ID,
                days_until_expiration: 3650,
            })
            .expect("failed to mint SP certificate");
        let private_key_der = sp_idp
            .export_private_key_der()
            .expect("failed to export SP private key");

        let private_key_pem = der_to_pem(&private_key_der, "RSA PRIVATE KEY");
        let cert_pem = der_to_pem(cert.der_data(), "CERTIFICATE");

        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(idp_metadata_xml()),
        )
        .with_keys(cert_pem, private_key_pem);
        let provider =
            SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build");

        let auth_request = provider
            .create_auth_request()
            .expect("signed auth request generation should succeed");

        let decoded = String::from_utf8(
            general_purpose::STANDARD
                .decode(&auth_request.saml_request)
                .expect("saml_request should be valid base64"),
        )
        .expect("decoded AuthnRequest should be valid UTF-8");

        assert!(
            decoded.contains("Signature"),
            "signed AuthnRequest should contain an enveloped Signature element: {decoded}"
        );
    }

    // (e) Without SP credentials configured, the request is produced unsigned (no dead
    //     config path silently required).
    #[test]
    fn auth_request_is_unsigned_without_sp_keys() {
        let config = SamlConfig::new(
            SP_ENTITY_ID.to_string(),
            ACS_URL.to_string(),
            IdpMetadata::Xml(idp_metadata_xml()),
        );
        let provider =
            SamlServiceProvider::new("test-idp".to_string(), config).expect("provider build");

        let auth_request = provider
            .create_auth_request()
            .expect("auth request generation should succeed");

        let decoded = String::from_utf8(
            general_purpose::STANDARD
                .decode(&auth_request.saml_request)
                .expect("saml_request should be valid base64"),
        )
        .expect("decoded AuthnRequest should be valid UTF-8");

        assert!(!decoded.contains("Signature"));
    }
}
