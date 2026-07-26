//! Credential and endpoint resolution shared by the service initializers.
//!
//! These helpers translate the crate-level [`GcpConfig`] knobs
//! (`credentials`, `emulator_host`, and per-service `service_configs`) into the
//! concrete values the upstream SDK client builders expect. They are kept
//! side-effect free and independently testable so the wiring can be verified
//! without touching the network.

use crate::config::GcpConfig;

/// Normalize an endpoint/host string into a URL the SDK builders accept.
///
/// Emulator environment variables (e.g. `PUBSUB_EMULATOR_HOST`) are commonly
/// bare `host:port` pairs; the SDK `with_endpoint` methods expect a URL, so a
/// scheme is prepended when one is missing.
///
/// A bare host defaults to **`https://`**: these endpoints carry a bearer
/// token, so plaintext must never be inferred. Emulator / loopback users who
/// genuinely want cleartext must write an explicit `http://` prefix, which is
/// preserved verbatim (as is an explicit `https://`).
pub(crate) fn normalize_endpoint(host: &str) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.to_string()
    } else {
        format!("https://{host}")
    }
}

/// Resolve the endpoint override for a given service, if any.
///
/// Precedence:
/// 1. A per-service `endpoint` string under `service_configs[service]`.
/// 2. The global `emulator_host`.
///
/// Returns `None` when neither is set, i.e. the SDK default endpoint is used.
pub(crate) fn resolve_endpoint(config: &GcpConfig, service: &str) -> Option<String> {
    if let Some(ep) = config
        .service_configs
        .get(service)
        .and_then(|cfg| cfg.get("endpoint"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(normalize_endpoint(ep));
    }
    config.emulator_host.as_deref().map(normalize_endpoint)
}

/// Build explicit credentials for the gRPC-based SDKs (Cloud Storage, Pub/Sub)
/// from the configured [`CredentialsSource`].
///
/// Returns `Ok(None)` for [`CredentialsSource::ApplicationDefault`], meaning the
/// SDK's own Application Default Credentials discovery is left in place.
///
/// `AccessToken` credentials are not representable in the `google-cloud-auth`
/// credential model used by these SDKs, so that variant is surfaced as an error
/// rather than being silently ignored (it *is* honored by the REST services).
#[cfg(any(feature = "storage", feature = "pubsub"))]
pub(crate) fn build_gcloud_credentials(
    source: &crate::config::CredentialsSource,
) -> crate::Result<Option<google_cloud_auth::credentials::Credentials>> {
    use crate::GcpError;
    use crate::config::CredentialsSource;
    use google_cloud_auth::credentials::{mds, service_account};

    match source {
        CredentialsSource::ApplicationDefault => Ok(None),
        CredentialsSource::ServiceAccountJson(json) => {
            let value: serde_json::Value = serde_json::from_str(json)
                .map_err(|e| GcpError::Config(format!("invalid service account JSON: {e}")))?;
            let cred = service_account::Builder::new(value)
                .build()
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(Some(cred))
        }
        CredentialsSource::ServiceAccountFile(path) => {
            let contents = std::fs::read_to_string(path)?;
            let value: serde_json::Value = serde_json::from_str(&contents).map_err(|e| {
                GcpError::Config(format!("invalid service account file {path}: {e}"))
            })?;
            let cred = service_account::Builder::new(value)
                .build()
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(Some(cred))
        }
        CredentialsSource::MetadataServer => {
            let cred = mds::Builder::default()
                .build()
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(Some(cred))
        }
        CredentialsSource::AccessToken(_) => Err(GcpError::Config(
            "AccessToken credentials are not supported by the gRPC SDK services \
             (storage/pubsub); use ServiceAccountJson, ServiceAccountFile, \
             MetadataServer, or ApplicationDefault"
                .to_string(),
        )),
    }
}

/// Resolved authentication handle for the REST-based services.
#[cfg(any(
    feature = "secret-manager",
    feature = "cloud-run",
    feature = "cloud-functions"
))]
#[derive(Clone)]
pub(crate) enum RestAuth {
    /// A token provider that mints (and refreshes) OAuth2 access tokens.
    Provider(std::sync::Arc<dyn gcp_auth::TokenProvider>),
    /// A caller-supplied static access token, used verbatim as a bearer token.
    Static(String),
}

#[cfg(any(
    feature = "secret-manager",
    feature = "cloud-run",
    feature = "cloud-functions"
))]
impl RestAuth {
    /// Default OAuth2 scope for Google Cloud REST APIs.
    const SCOPES: &'static [&'static str] = &["https://www.googleapis.com/auth/cloud-platform"];

    /// Obtain a bearer token string for an `Authorization: Bearer` header.
    pub(crate) async fn bearer(&self) -> crate::Result<String> {
        match self {
            RestAuth::Provider(provider) => {
                let token = provider
                    .token(Self::SCOPES)
                    .await
                    .map_err(|e| crate::GcpError::Auth(e.to_string()))?;
                Ok(token.as_str().to_string())
            }
            RestAuth::Static(token) => Ok(token.clone()),
        }
    }
}

/// Build a [`RestAuth`] from the configured [`CredentialsSource`].
///
/// Every variant is honored: static access tokens are used directly, while the
/// service-account / ADC / metadata variants are turned into a `gcp_auth`
/// [`TokenProvider`](gcp_auth::TokenProvider).
#[cfg(any(
    feature = "secret-manager",
    feature = "cloud-run",
    feature = "cloud-functions"
))]
pub(crate) async fn build_rest_auth(
    source: &crate::config::CredentialsSource,
) -> crate::Result<RestAuth> {
    use crate::GcpError;
    use crate::config::CredentialsSource;
    use std::sync::Arc;

    match source {
        CredentialsSource::ApplicationDefault => {
            let provider = gcp_auth::provider()
                .await
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(RestAuth::Provider(provider))
        }
        CredentialsSource::ServiceAccountFile(path) => {
            let sa = gcp_auth::CustomServiceAccount::from_file(path)
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(RestAuth::Provider(Arc::new(sa)))
        }
        CredentialsSource::ServiceAccountJson(json) => {
            let sa = gcp_auth::CustomServiceAccount::from_json(json)
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(RestAuth::Provider(Arc::new(sa)))
        }
        CredentialsSource::MetadataServer => {
            let sa = gcp_auth::MetadataServiceAccount::new()
                .await
                .map_err(|e| GcpError::Auth(e.to_string()))?;
            Ok(RestAuth::Provider(Arc::new(sa)))
        }
        CredentialsSource::AccessToken(token) => Ok(RestAuth::Static(token.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CredentialsSource;

    #[test]
    fn normalize_endpoint_prepends_scheme_when_missing() {
        // A bare host defaults to https (never plaintext, since a bearer token
        // is attached downstream).
        assert_eq!(
            normalize_endpoint("localhost:8085"),
            "https://localhost:8085"
        );
        // Explicit schemes are preserved verbatim, including http for emulators.
        assert_eq!(
            normalize_endpoint("http://localhost:8085"),
            "http://localhost:8085"
        );
        assert_eq!(
            normalize_endpoint("https://storage.example.com"),
            "https://storage.example.com"
        );
    }

    #[test]
    fn resolve_endpoint_none_by_default() {
        let config = GcpConfig::default();
        assert_eq!(resolve_endpoint(&config, "storage"), None);
    }

    #[test]
    fn resolve_endpoint_uses_emulator_host() {
        let config = GcpConfig::builder().emulator_host("localhost:9000").build();
        assert_eq!(
            resolve_endpoint(&config, "storage"),
            Some("https://localhost:9000".to_string())
        );
    }

    #[test]
    fn resolve_endpoint_service_config_overrides_emulator() {
        let config = GcpConfig::builder()
            .emulator_host("localhost:9000")
            .service_config(
                "storage",
                serde_json::json!({ "endpoint": "https://storage.internal:443" }),
            )
            .build();
        assert_eq!(
            resolve_endpoint(&config, "storage"),
            Some("https://storage.internal:443".to_string())
        );
        // A service without an override still falls back to the emulator host
        // (a bare host normalizes to https).
        assert_eq!(
            resolve_endpoint(&config, "pubsub"),
            Some("https://localhost:9000".to_string())
        );
    }

    #[cfg(any(feature = "storage", feature = "pubsub"))]
    #[test]
    fn gcloud_credentials_application_default_is_none() {
        let cred = build_gcloud_credentials(&CredentialsSource::ApplicationDefault).unwrap();
        assert!(cred.is_none());
    }

    #[cfg(any(feature = "storage", feature = "pubsub"))]
    #[test]
    fn gcloud_credentials_access_token_is_rejected() {
        let err =
            build_gcloud_credentials(&CredentialsSource::AccessToken("tok".into())).unwrap_err();
        assert!(matches!(err, crate::GcpError::Config(_)));
    }

    // `service_account::Builder::build()` spins up a token cache that requires a
    // Tokio reactor, matching the async context of `GcpServices::new`.
    #[cfg(any(feature = "storage", feature = "pubsub"))]
    #[tokio::test]
    async fn gcloud_credentials_service_account_json_is_built() {
        // A well-formed (but non-functional) service account key. `build()` only
        // deserializes the JSON; the private key is not parsed until signing.
        let key = serde_json::json!({
            "type": "service_account",
            "project_id": "test-project",
            "private_key_id": "abc123",
            "private_key": "-----BEGIN PRIVATE KEY-----\nMIIfake\n-----END PRIVATE KEY-----\n",
            "client_email": "test@test-project.iam.gserviceaccount.com",
            "client_id": "1234567890"
        })
        .to_string();
        let cred = build_gcloud_credentials(&CredentialsSource::ServiceAccountJson(key)).unwrap();
        assert!(cred.is_some());
    }

    #[cfg(any(feature = "storage", feature = "pubsub"))]
    #[test]
    fn gcloud_credentials_malformed_json_errors() {
        let err =
            build_gcloud_credentials(&CredentialsSource::ServiceAccountJson("{ not json".into()))
                .unwrap_err();
        assert!(matches!(err, crate::GcpError::Config(_)));
    }

    #[cfg(any(
        feature = "secret-manager",
        feature = "cloud-run",
        feature = "cloud-functions"
    ))]
    #[tokio::test]
    async fn rest_auth_access_token_is_used_verbatim() {
        let auth = build_rest_auth(&CredentialsSource::AccessToken("static-tok".into()))
            .await
            .unwrap();
        assert_eq!(auth.bearer().await.unwrap(), "static-tok");
    }
}
