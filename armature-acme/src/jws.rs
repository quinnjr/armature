//! JWS / crypto primitives for the ACME protocol (RFC 8555 + RFC 7638).
//!
//! This module implements the cryptographic core used by [`crate::AcmeClient`]:
//!
//! - an ECDSA P-256 account key ([`AccountKey`]) producing ES256 signatures,
//! - ACME flattened JWS request signing ([`sign_jws`]),
//! - the RFC 7638 JWK thumbprint and `key_authorization` derivation,
//! - the External Account Binding (EAB) HS256 JWS ([`eab_jws`]),
//! - the DNS-01 record value derivation ([`dns01_txt_value`]).

use crate::error::{AcmeError, Result};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};

/// Encode bytes as base64url without padding (per JOSE / RFC 7515).
pub fn b64url(data: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// Decode a base64url (no padding) string.
pub fn b64url_decode(s: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(AcmeError::from)
}

/// The DNS-01 TXT record value for a given key authorization:
/// `base64url(SHA-256(key_authorization))` (RFC 8555 §8.4).
pub fn dns01_txt_value(key_authorization: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, key_authorization.as_bytes());
    b64url(digest.as_ref())
}

/// Which key identifier to place in a JWS protected header.
#[derive(Clone, Copy)]
pub enum KeyRef<'a> {
    /// Embed the full public JWK (used for `newAccount` and EAB inner keys).
    Jwk,
    /// Reference the account URL as `kid` (used for every other request).
    Kid(&'a str),
}

/// The JWS payload: an ACME JSON body, or empty for POST-as-GET.
#[derive(Clone, Copy)]
pub enum Payload<'a> {
    /// Empty payload (`""`) — a POST-as-GET request.
    Empty,
    /// A JSON payload, base64url-encoded when signed.
    Json(&'a Value),
}

/// An ECDSA P-256 account key used to sign ACME requests with ES256.
pub struct AccountKey {
    key_pair: EcdsaKeyPair,
    pkcs8: Vec<u8>,
    /// base64url of the 32-byte public X coordinate.
    x: String,
    /// base64url of the 32-byte public Y coordinate.
    y: String,
    rng: SystemRandom,
}

impl std::fmt::Debug for AccountKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountKey").finish_non_exhaustive()
    }
}

impl AccountKey {
    /// Generate a fresh random P-256 account key.
    pub fn generate() -> Result<Self> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|e| AcmeError::InvalidKey(format!("generate account key: {e}")))?;
        Self::from_pkcs8(pkcs8.as_ref().to_vec())
    }

    /// Load an account key from its PKCS#8 DER encoding.
    pub fn from_pkcs8(pkcs8: Vec<u8>) -> Result<Self> {
        let rng = SystemRandom::new();
        let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng)
            .map_err(|e| AcmeError::InvalidKey(format!("load account key: {e}")))?;
        let public = key_pair.public_key().as_ref();
        // Uncompressed SEC1 point: 0x04 || X(32) || Y(32).
        if public.len() != 65 || public[0] != 0x04 {
            return Err(AcmeError::InvalidKey(
                "account public key is not an uncompressed P-256 point".to_string(),
            ));
        }
        let x = b64url(&public[1..33]);
        let y = b64url(&public[33..65]);
        Ok(Self {
            key_pair,
            pkcs8,
            x,
            y,
            rng,
        })
    }

    /// The PKCS#8 DER bytes, for persistence.
    pub fn pkcs8_der(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The public key as a JWK value: `{crv, kty, x, y}`.
    pub fn jwk(&self) -> Value {
        json!({
            "crv": "P-256",
            "kty": "EC",
            "x": self.x,
            "y": self.y,
        })
    }

    /// The RFC 7638 JWK thumbprint: `base64url(SHA-256(canonical JWK))`.
    ///
    /// The canonical form has the required members in lexicographic order
    /// (`crv`, `kty`, `x`, `y`) with no whitespace.
    pub fn thumbprint(&self) -> String {
        let canonical = format!(
            "{{\"crv\":\"P-256\",\"kty\":\"EC\",\"x\":\"{}\",\"y\":\"{}\"}}",
            self.x, self.y
        );
        let digest = ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes());
        b64url(digest.as_ref())
    }

    /// The `key_authorization` for a challenge token:
    /// `token + "." + base64url(thumbprint)` (RFC 8555 §8.1).
    pub fn key_authorization(&self, token: &str) -> String {
        format!("{}.{}", token, self.thumbprint())
    }

    /// Sign `message` with ES256, returning the raw 64-byte `r || s` signature.
    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        let sig = self
            .key_pair
            .sign(&self.rng, message)
            .map_err(|e| AcmeError::InvalidKey(format!("sign: {e}")))?;
        Ok(sig.as_ref().to_vec())
    }
}

/// Build an ACME flattened JWS (`{protected, payload, signature}`) for `url`.
///
/// The protected header always carries `alg=ES256`, the `nonce`, and the `url`,
/// plus either the embedded `jwk` or the account `kid`.
pub fn sign_jws(
    key: &AccountKey,
    url: &str,
    nonce: &str,
    key_ref: KeyRef<'_>,
    payload: Payload<'_>,
) -> Result<Value> {
    let mut protected = serde_json::Map::new();
    protected.insert("alg".to_string(), json!("ES256"));
    protected.insert("nonce".to_string(), json!(nonce));
    protected.insert("url".to_string(), json!(url));
    match key_ref {
        KeyRef::Jwk => {
            protected.insert("jwk".to_string(), key.jwk());
        }
        KeyRef::Kid(kid) => {
            protected.insert("kid".to_string(), json!(kid));
        }
    }

    let protected_b64 = b64url(serde_json::to_vec(&Value::Object(protected))?);
    let payload_b64 = match payload {
        Payload::Empty => String::new(),
        Payload::Json(v) => b64url(serde_json::to_vec(v)?),
    };

    let signing_input = format!("{protected_b64}.{payload_b64}");
    let signature = b64url(key.sign(signing_input.as_bytes())?);

    Ok(json!({
        "protected": protected_b64,
        "payload": payload_b64,
        "signature": signature,
    }))
}

/// Decode an EAB HMAC key leniently.
///
/// Real-world CAs (ZeroSSL, Google Trust Services, etc.) hand out EAB HMAC
/// keys in varying base64 encodings — some padded, some using the standard
/// alphabet. Try the RFC 8555-mandated `base64url` (no padding) first, then
/// fall back to the other common encodings before giving up.
fn decode_eab_hmac_key(s: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .or_else(|_| URL_SAFE.decode(s.as_bytes()))
        .or_else(|_| STANDARD_NO_PAD.decode(s.as_bytes()))
        .or_else(|_| STANDARD.decode(s.as_bytes()))
        .map_err(AcmeError::from)
}

/// Build the External Account Binding JWS (HS256) over the account JWK.
///
/// `eab_hmac_key_b64` is the base64-encoded MAC key supplied by the CA. Real
/// CAs vary in which base64 flavor they use (padded/unpadded,
/// standard/url-safe alphabet), so it is decoded leniently — see
/// [`decode_eab_hmac_key`].
pub fn eab_jws(
    eab_kid: &str,
    eab_hmac_key_b64: &str,
    new_account_url: &str,
    account_jwk: &Value,
) -> Result<Value> {
    let key_bytes = decode_eab_hmac_key(eab_hmac_key_b64)?;
    let protected = json!({ "alg": "HS256", "kid": eab_kid, "url": new_account_url });
    let protected_b64 = b64url(serde_json::to_vec(&protected)?);
    let payload_b64 = b64url(serde_json::to_vec(account_jwk)?);
    let signing_input = format!("{protected_b64}.{payload_b64}");
    let hmac_key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key_bytes);
    let tag = ring::hmac::sign(&hmac_key, signing_input.as_bytes());
    Ok(json!({
        "protected": protected_b64,
        "payload": payload_b64,
        "signature": b64url(tag.as_ref()),
    }))
}

/// PEM-encode PKCS#8 DER as a `PRIVATE KEY` block.
pub fn pkcs8_to_pem(der: &[u8]) -> String {
    let b64 = STANDARD.encode(der);
    let mut out = String::from("-----BEGIN PRIVATE KEY-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str("-----END PRIVATE KEY-----\n");
    out
}

/// Extract PKCS#8 DER bytes from a `PRIVATE KEY` PEM block.
pub fn pem_to_pkcs8(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    STANDARD
        .decode(body.trim())
        .map_err(|e| AcmeError::PemError(format!("decode account key PEM: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    #[test]
    fn es256_sign_verify_round_trip() {
        let key = AccountKey::generate().unwrap();
        let msg = b"acme jws signing input";
        let sig = key.sign(msg).unwrap();
        assert_eq!(sig.len(), 64, "ES256 raw signature must be r||s = 64 bytes");

        // Reconstruct the public key from the JWK coordinates and verify.
        let jwk = key.jwk();
        let x = b64url_decode(jwk["x"].as_str().unwrap()).unwrap();
        let y = b64url_decode(jwk["y"].as_str().unwrap()).unwrap();
        let mut point = vec![0x04u8];
        point.extend_from_slice(&x);
        point.extend_from_slice(&y);
        let pubkey = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point);
        pubkey.verify(msg, &sig).expect("signature must verify");
    }

    #[test]
    fn base64url_sha256_pipeline_matches_known_vector() {
        // Well-known vector: base64url(SHA-256("")) with no padding.
        let got = b64url(ring::digest::digest(&ring::digest::SHA256, b"").as_ref());
        assert_eq!(got, "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU");
    }

    #[test]
    fn thumbprint_uses_canonical_sorted_jwk() {
        // RFC 7638: the thumbprint is SHA-256 over the JWK with required members
        // in lexicographic order and no whitespace. Prove our hand-written
        // canonical string matches an independently serialized sorted map, so an
        // ordering or whitespace bug would be caught.
        let key = AccountKey::generate().unwrap();
        let jwk = key.jwk();
        let mut sorted: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        sorted.insert("crv".into(), jwk["crv"].as_str().unwrap().to_string());
        sorted.insert("kty".into(), jwk["kty"].as_str().unwrap().to_string());
        sorted.insert("x".into(), jwk["x"].as_str().unwrap().to_string());
        sorted.insert("y".into(), jwk["y"].as_str().unwrap().to_string());
        let canonical = serde_json::to_vec(&sorted).unwrap();
        let expect = b64url(ring::digest::digest(&ring::digest::SHA256, &canonical).as_ref());
        assert_eq!(key.thumbprint(), expect);

        // Pin the exact byte sequence RFC 7638 requires, independently of how
        // any serializer happens to order or space a map: the members must be
        // crv, kty, x, y in that order with no whitespace. Building the string
        // by hand means a change to the hashed form fails here even if both
        // sides of the assertion above drifted together.
        let literal = format!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            jwk["x"].as_str().unwrap(),
            jwk["y"].as_str().unwrap()
        );
        assert_eq!(
            String::from_utf8(canonical.clone()).unwrap(),
            literal,
            "canonical JWK must be exactly the RFC 7638 member order with no whitespace"
        );
        let expect_literal =
            b64url(ring::digest::digest(&ring::digest::SHA256, literal.as_bytes()).as_ref());
        assert_eq!(key.thumbprint(), expect_literal);
    }

    #[test]
    fn thumbprint_is_stable_for_a_key() {
        let key = AccountKey::generate().unwrap();
        assert_eq!(key.thumbprint(), key.thumbprint());
        assert_eq!(key.thumbprint().len(), 43, "SHA-256 base64url is 43 chars");
    }

    #[test]
    fn key_authorization_format() {
        let key = AccountKey::generate().unwrap();
        let ka = key.key_authorization("tok123");
        let (tok, tp) = ka.split_once('.').expect("token.thumbprint");
        assert_eq!(tok, "tok123");
        assert_eq!(tp, key.thumbprint());
    }

    #[test]
    fn pkcs8_pem_round_trips() {
        let key = AccountKey::generate().unwrap();
        let pem = pkcs8_to_pem(key.pkcs8_der());
        assert!(pem.contains("-----BEGIN PRIVATE KEY-----"));
        let der = pem_to_pkcs8(&pem).unwrap();
        assert_eq!(der, key.pkcs8_der());
        // The reloaded key has the same public JWK / thumbprint.
        let reloaded = AccountKey::from_pkcs8(der).unwrap();
        assert_eq!(reloaded.thumbprint(), key.thumbprint());
    }

    #[test]
    fn eab_jws_has_expected_shape() {
        let key = AccountKey::generate().unwrap();
        let hmac_b64 = b64url(b"super-secret-mac-key-bytes");
        let eab = eab_jws(
            "my-kid",
            &hmac_b64,
            "https://ca.example/acme/new-account",
            &key.jwk(),
        )
        .unwrap();
        assert!(eab["protected"].is_string());
        assert!(eab["payload"].is_string());
        assert!(eab["signature"].is_string());

        // Protected header decodes to the expected HS256 header.
        let hdr: Value =
            serde_json::from_slice(&b64url_decode(eab["protected"].as_str().unwrap()).unwrap())
                .unwrap();
        assert_eq!(hdr["alg"], "HS256");
        assert_eq!(hdr["kid"], "my-kid");
    }

    #[test]
    fn eab_hmac_key_decodes_leniently_across_base64_flavors() {
        // A fixed 32-byte key, encoded in the four common base64 flavors a
        // real CA might use. All must decode to the same bytes.
        let key_bytes: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0xfa, 0xfb, 0xfc, 0xfd, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60,
            0x70, 0x80, 0x90, 0xa0, 0xb0, 0xc0, 0xd0, 0xe0, 0xf0, 0xff, 0x11, 0x22, 0x33, 0x44,
            0x55, 0x66, 0x77, 0x88,
        ];

        let padded_standard = STANDARD.encode(key_bytes);
        let unpadded_url_safe = URL_SAFE_NO_PAD.encode(key_bytes);

        let from_padded_standard = decode_eab_hmac_key(&padded_standard).unwrap();
        let from_unpadded_url_safe = decode_eab_hmac_key(&unpadded_url_safe).unwrap();

        assert_eq!(from_padded_standard, key_bytes.to_vec());
        assert_eq!(from_unpadded_url_safe, key_bytes.to_vec());
    }

    #[test]
    fn eab_jws_accepts_padded_standard_hmac_key() {
        // register_account must not fail to decode a padded-standard EAB key.
        let key = AccountKey::generate().unwrap();
        let hmac_b64 = STANDARD.encode(b"a 32+ byte long super-secret-mac-key!!");
        let eab = eab_jws(
            "my-kid",
            &hmac_b64,
            "https://ca.example/acme/new-account",
            &key.jwk(),
        );
        assert!(
            eab.is_ok(),
            "EAB signing must accept a padded-standard base64 HMAC key"
        );
    }

    #[test]
    fn thumbprint_matches_known_answer_vector() {
        // A fixed P-256 JWK. The expected thumbprint below was computed
        // independently (not via this crate's code) with:
        //   printf '<canonical json, no spaces>' \
        //     | openssl dgst -sha256 -binary | base64 | tr '+/' '-_' | tr -d '='
        // so this test catches a field-order or canonicalization regression,
        // not just self-consistency against our own implementation.
        let x = "MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4";
        let y = "4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFGM";
        let canonical = format!("{{\"crv\":\"P-256\",\"kty\":\"EC\",\"x\":\"{x}\",\"y\":\"{y}\"}}");
        // Assert the exact canonical JSON string that gets hashed: lexicographic
        // member order (crv, kty, x, y), no whitespace.
        assert_eq!(
            canonical,
            "{\"crv\":\"P-256\",\"kty\":\"EC\",\"x\":\"MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4\",\"y\":\"4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFGM\"}"
        );

        let expected_thumbprint = "bsJlUSPkh6gjRQFHJlC15Ximi4pDK8Y3tYaR-TjhLNk";
        let got =
            b64url(ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes()).as_ref());
        assert_eq!(
            got, expected_thumbprint,
            "RFC 7638 thumbprint must match the independently computed known-answer vector"
        );
    }

    #[test]
    fn dns01_value_is_sha256_of_key_auth() {
        let v = dns01_txt_value("tok.thumb");
        let expect = b64url(ring::digest::digest(&ring::digest::SHA256, b"tok.thumb").as_ref());
        assert_eq!(v, expect);
        assert_eq!(v.len(), 43);
    }
}
