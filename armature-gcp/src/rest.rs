//! REST-based GCP service clients (Secret Manager, Cloud Run, Cloud Functions).
//!
//! These services have no dedicated gRPC SDK in the pinned dependency set, so
//! they are implemented directly against their public REST APIs using
//! [`reqwest`] for transport and `gcp_auth` for OAuth2 bearer tokens. Each
//! client honors the full [`CredentialsSource`](crate::CredentialsSource) range
//! (including static access tokens) and a per-service endpoint override.

use crate::credentials::{RestAuth, build_rest_auth, resolve_endpoint};
use crate::{GcpConfig, GcpError, Result};

/// Perform an authorized `GET` against a fully-qualified URL and decode the JSON body.
async fn authorized_get(
    client: &reqwest::Client,
    auth: &RestAuth,
    url: &str,
) -> Result<serde_json::Value> {
    let token = auth.bearer().await?;
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| GcpError::Network(e.to_string()))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| GcpError::Network(e.to_string()))?;

    if !status.is_success() {
        return Err(GcpError::Service(format!("HTTP {status}: {body}")));
    }

    serde_json::from_str(&body).map_err(|e| GcpError::Serialization(e.to_string()))
}

/// Resolve the project id or fail with a consistent error.
fn require_project(config: &GcpConfig) -> Result<String> {
    config
        .project_id
        .clone()
        .ok_or(GcpError::ProjectNotSpecified)
}

/// Client for the [Secret Manager](https://cloud.google.com/secret-manager) REST API.
#[cfg(feature = "secret-manager")]
#[derive(Clone)]
pub struct SecretManagerClient {
    client: reqwest::Client,
    auth: RestAuth,
    project_id: String,
    endpoint: String,
}

#[cfg(feature = "secret-manager")]
impl SecretManagerClient {
    const DEFAULT_ENDPOINT: &'static str = "https://secretmanager.googleapis.com";

    pub(crate) async fn new(config: &GcpConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            auth: build_rest_auth(&config.credentials).await?,
            project_id: require_project(config)?,
            endpoint: resolve_endpoint(config, "secret-manager")
                .unwrap_or_else(|| Self::DEFAULT_ENDPOINT.to_string()),
        })
    }

    /// The resolved API endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The project this client is scoped to.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Access a secret version payload, returning the raw
    /// `AccessSecretVersionResponse` JSON (the payload data is base64-encoded).
    pub async fn access_secret_version(
        &self,
        secret: &str,
        version: &str,
    ) -> Result<serde_json::Value> {
        let url = format!(
            "{}/v1/projects/{}/secrets/{}/versions/{}:access",
            self.endpoint, self.project_id, secret, version
        );
        authorized_get(&self.client, &self.auth, &url).await
    }
}

/// Client for the [Cloud Run](https://cloud.google.com/run) admin REST API (v2).
#[cfg(feature = "cloud-run")]
#[derive(Clone)]
pub struct CloudRunClient {
    client: reqwest::Client,
    auth: RestAuth,
    project_id: String,
    endpoint: String,
}

#[cfg(feature = "cloud-run")]
impl CloudRunClient {
    const DEFAULT_ENDPOINT: &'static str = "https://run.googleapis.com";

    pub(crate) async fn new(config: &GcpConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            auth: build_rest_auth(&config.credentials).await?,
            project_id: require_project(config)?,
            endpoint: resolve_endpoint(config, "cloud-run")
                .unwrap_or_else(|| Self::DEFAULT_ENDPOINT.to_string()),
        })
    }

    /// The resolved API endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The project this client is scoped to.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// List the Cloud Run services in a region, returning the raw
    /// `ListServicesResponse` JSON.
    pub async fn list_services(&self, location: &str) -> Result<serde_json::Value> {
        let url = format!(
            "{}/v2/projects/{}/locations/{}/services",
            self.endpoint, self.project_id, location
        );
        authorized_get(&self.client, &self.auth, &url).await
    }
}

/// Client for the [Cloud Functions](https://cloud.google.com/functions) REST API (v2).
#[cfg(feature = "cloud-functions")]
#[derive(Clone)]
pub struct CloudFunctionsClient {
    client: reqwest::Client,
    auth: RestAuth,
    project_id: String,
    endpoint: String,
}

#[cfg(feature = "cloud-functions")]
impl CloudFunctionsClient {
    const DEFAULT_ENDPOINT: &'static str = "https://cloudfunctions.googleapis.com";

    pub(crate) async fn new(config: &GcpConfig) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            auth: build_rest_auth(&config.credentials).await?,
            project_id: require_project(config)?,
            endpoint: resolve_endpoint(config, "cloud-functions")
                .unwrap_or_else(|| Self::DEFAULT_ENDPOINT.to_string()),
        })
    }

    /// The resolved API endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The project this client is scoped to.
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// List the Cloud Functions in a region, returning the raw
    /// `ListFunctionsResponse` JSON.
    pub async fn list_functions(&self, location: &str) -> Result<serde_json::Value> {
        let url = format!(
            "{}/v2/projects/{}/locations/{}/functions",
            self.endpoint, self.project_id, location
        );
        authorized_get(&self.client, &self.auth, &url).await
    }
}
