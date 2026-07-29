//! REST-based GCP service clients (Secret Manager, Cloud Run, Cloud Functions).
//!
//! These services have no dedicated gRPC SDK in the pinned dependency set, so
//! they are implemented directly against their public REST APIs using
//! [`reqwest`] for transport and `gcp_auth` for OAuth2 bearer tokens. Each
//! client honors the full [`CredentialsSource`](crate::CredentialsSource) range
//! (including static access tokens) and a per-service endpoint override.
//!
//! The three public clients are thin, typed wrappers over a shared
//! [`RestClient`], which owns the HTTP transport (with request/connect
//! timeouts), the resolved bearer-token auth, the project id, and the resolved
//! endpoint. Each public client keeps only its one distinct request method.

use crate::credentials::{RestAuth, build_rest_auth, resolve_endpoint};
use crate::{GcpConfig, GcpError, Result};

/// Percent-encode a single URL path segment.
///
/// Caller-supplied values (`secret`, `version`, `location`, the `project_id`,
/// ...) are interpolated straight into the request path, so any byte outside
/// the RFC 3986 *unreserved* set is percent-encoded. This keeps the URL
/// well-formed and prevents a stray `/`, `?`, `#`, or `:` in a resource id from
/// silently retargeting the request onto a different path.
fn encode_segment(segment: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(segment.len());
    for &byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
        }
    }
    out
}

/// Map a non-success HTTP status onto a distinct [`GcpError`] variant so callers
/// can branch (e.g. retry `RateLimited`/`ServerError`, surface `Auth`/`NotFound`).
///
/// The response `body` is retained in the message for diagnostics; the bearer
/// token is never part of the response and so never leaks here.
fn status_to_error(status: reqwest::StatusCode, body: String) -> GcpError {
    match status.as_u16() {
        401 | 403 => GcpError::Auth(format!("HTTP {status}: {body}")),
        404 => GcpError::NotFound(format!("HTTP {status}: {body}")),
        429 => GcpError::RateLimited(format!("HTTP {status}: {body}")),
        500..=599 => GcpError::ServerError(format!("HTTP {status}: {body}")),
        _ => GcpError::Service(format!("HTTP {status}: {body}")),
    }
}

/// Resolve the project id or fail with a consistent error.
fn require_project(config: &GcpConfig) -> Result<String> {
    config
        .project_id
        .clone()
        .ok_or(GcpError::ProjectNotSpecified)
}

/// Shared internals for the REST-backed GCP services.
///
/// Owns the HTTP transport (with request and connect timeouts), the resolved
/// bearer-token auth, the project id, and the resolved endpoint. The typed
/// public clients each hold one of these and delegate to it.
#[derive(Clone)]
struct RestClient {
    client: reqwest::Client,
    auth: RestAuth,
    project_id: String,
    endpoint: String,
}

impl RestClient {
    /// Overall per-request timeout (headers + body).
    const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    /// TCP/TLS connection-establishment timeout.
    const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    /// Build the shared client for `service_key`, falling back to
    /// `default_endpoint` when no override is configured.
    async fn new(config: &GcpConfig, service_key: &str, default_endpoint: &str) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Self::REQUEST_TIMEOUT)
                .connect_timeout(Self::CONNECT_TIMEOUT)
                .build()
                .map_err(|e| GcpError::Network(e.to_string()))?,
            auth: build_rest_auth(&config.credentials).await?,
            project_id: require_project(config)?,
            endpoint: resolve_endpoint(config, service_key)
                .unwrap_or_else(|| default_endpoint.to_string()),
        })
    }

    /// The resolved API endpoint.
    fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The project this client is scoped to.
    fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Perform an authorized `GET` against a fully-qualified URL and decode the
    /// JSON body. Non-success statuses are mapped to distinct [`GcpError`]
    /// variants via [`status_to_error`].
    async fn authorized_get(&self, url: &str) -> Result<serde_json::Value> {
        let token = self.auth.bearer().await?;
        let response = self
            .client
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
            return Err(status_to_error(status, body));
        }

        serde_json::from_str(&body).map_err(|e| GcpError::Serialization(e.to_string()))
    }
}

/// Client for the [Secret Manager](https://cloud.google.com/secret-manager) REST API.
#[cfg(feature = "secret-manager")]
#[derive(Clone)]
pub struct SecretManagerClient {
    inner: RestClient,
}

#[cfg(feature = "secret-manager")]
impl SecretManagerClient {
    const DEFAULT_ENDPOINT: &'static str = "https://secretmanager.googleapis.com";

    pub(crate) async fn new(config: &GcpConfig) -> Result<Self> {
        Ok(Self {
            inner: RestClient::new(config, "secret-manager", Self::DEFAULT_ENDPOINT).await?,
        })
    }

    /// The resolved API endpoint.
    pub fn endpoint(&self) -> &str {
        self.inner.endpoint()
    }

    /// The project this client is scoped to.
    pub fn project_id(&self) -> &str {
        self.inner.project_id()
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
            self.inner.endpoint(),
            encode_segment(self.inner.project_id()),
            encode_segment(secret),
            encode_segment(version),
        );
        self.inner.authorized_get(&url).await
    }
}

/// Client for the [Cloud Run](https://cloud.google.com/run) admin REST API (v2).
#[cfg(feature = "cloud-run")]
#[derive(Clone)]
pub struct CloudRunClient {
    inner: RestClient,
}

#[cfg(feature = "cloud-run")]
impl CloudRunClient {
    const DEFAULT_ENDPOINT: &'static str = "https://run.googleapis.com";

    pub(crate) async fn new(config: &GcpConfig) -> Result<Self> {
        Ok(Self {
            inner: RestClient::new(config, "cloud-run", Self::DEFAULT_ENDPOINT).await?,
        })
    }

    /// The resolved API endpoint.
    pub fn endpoint(&self) -> &str {
        self.inner.endpoint()
    }

    /// The project this client is scoped to.
    pub fn project_id(&self) -> &str {
        self.inner.project_id()
    }

    /// List the Cloud Run services in a region, returning the raw
    /// `ListServicesResponse` JSON.
    pub async fn list_services(&self, location: &str) -> Result<serde_json::Value> {
        let url = format!(
            "{}/v2/projects/{}/locations/{}/services",
            self.inner.endpoint(),
            encode_segment(self.inner.project_id()),
            encode_segment(location),
        );
        self.inner.authorized_get(&url).await
    }
}

/// Client for the [Cloud Functions](https://cloud.google.com/functions) REST API (v2).
#[cfg(feature = "cloud-functions")]
#[derive(Clone)]
pub struct CloudFunctionsClient {
    inner: RestClient,
}

#[cfg(feature = "cloud-functions")]
impl CloudFunctionsClient {
    const DEFAULT_ENDPOINT: &'static str = "https://cloudfunctions.googleapis.com";

    pub(crate) async fn new(config: &GcpConfig) -> Result<Self> {
        Ok(Self {
            inner: RestClient::new(config, "cloud-functions", Self::DEFAULT_ENDPOINT).await?,
        })
    }

    /// The resolved API endpoint.
    pub fn endpoint(&self) -> &str {
        self.inner.endpoint()
    }

    /// The project this client is scoped to.
    pub fn project_id(&self) -> &str {
        self.inner.project_id()
    }

    /// List the Cloud Functions in a region, returning the raw
    /// `ListFunctionsResponse` JSON.
    pub async fn list_functions(&self, location: &str) -> Result<serde_json::Value> {
        let url = format!(
            "{}/v2/projects/{}/locations/{}/functions",
            self.inner.endpoint(),
            encode_segment(self.inner.project_id()),
            encode_segment(location),
        );
        self.inner.authorized_get(&url).await
    }
}
