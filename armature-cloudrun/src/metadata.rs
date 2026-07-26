//! Cloud Run instance metadata.

use serde::{Deserialize, Serialize};

/// Instance metadata from the GCE metadata server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceMetadata {
    /// Instance ID.
    pub instance_id: Option<String>,
    /// Instance zone.
    pub zone: Option<String>,
    /// Project ID.
    pub project_id: Option<String>,
    /// Project number.
    pub project_number: Option<String>,
    /// Service account email.
    pub service_account: Option<String>,
    /// Instance region.
    pub region: Option<String>,
}

/// Base URL of the GCE/Cloud Run metadata server.
const METADATA_BASE: &str = "http://metadata.google.internal/computeMetadata/v1/";

impl InstanceMetadata {
    /// Fetch instance metadata from the GCE metadata server.
    ///
    /// Returns `Err(CloudRunError::Metadata(..))` when the metadata server is
    /// unreachable (network/transport error), so callers can distinguish
    /// "metadata server unavailable" from "field absent". Individual fields
    /// that the server reports as missing (a non-success HTTP status) degrade
    /// to `None` rather than erroring.
    pub async fn fetch() -> Result<Self, crate::CloudRunError> {
        Self::fetch_from(METADATA_BASE).await
    }

    /// Fetch instance metadata from a specific base URL.
    ///
    /// Exposed (crate-internal) so tests can point at an unreachable or stub
    /// address instead of the live metadata server.
    pub(crate) async fn fetch_from(base: &str) -> Result<Self, crate::CloudRunError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| {
                crate::CloudRunError::Metadata(format!("failed to build HTTP client: {e}"))
            })?;

        // A single field lookup. A transport error (server unreachable) is a
        // hard error; a non-success status (field absent) degrades to `None`.
        async fn get_metadata(
            client: &reqwest::Client,
            base: &str,
            path: &str,
        ) -> Result<Option<String>, crate::CloudRunError> {
            let resp = client
                .get(format!("{base}{path}"))
                .header("Metadata-Flavor", "Google")
                .send()
                .await
                .map_err(|e| {
                    crate::CloudRunError::Metadata(format!(
                        "metadata server unreachable ({path}): {e}"
                    ))
                })?;

            let status = resp.status();
            if !status.is_success() {
                // Distinguish "field genuinely absent" from "server degraded".
                // 404 (not found) and 403 (this instance may not read the field)
                // mean the value simply isn't available -> None. Anything else,
                // notably 5xx and 429, is a transient/degraded response: surface
                // it as a retryable error rather than silently returning an
                // incomplete InstanceMetadata that looks complete.
                if status.as_u16() == 404 || status.as_u16() == 403 {
                    return Ok(None);
                }
                return Err(crate::CloudRunError::Metadata(format!(
                    "metadata server returned HTTP {} for {path}",
                    status.as_u16()
                )));
            }

            let text = resp.text().await.map_err(|e| {
                crate::CloudRunError::Metadata(format!(
                    "failed to read metadata response ({path}): {e}"
                ))
            })?;
            Ok(Some(text))
        }

        // Independent lookups run concurrently; the first transport error
        // short-circuits the whole fetch.
        let (instance_id, zone, project_id, project_number, service_account) = tokio::try_join!(
            get_metadata(&client, base, "instance/id"),
            get_metadata(&client, base, "instance/zone"),
            get_metadata(&client, base, "project/project-id"),
            get_metadata(&client, base, "project/numeric-project-id"),
            get_metadata(&client, base, "instance/service-accounts/default/email"),
        )?;

        let region = zone.as_deref().and_then(region_from_zone);

        Ok(Self {
            instance_id,
            zone,
            project_id,
            project_number,
            service_account,
            region,
        })
    }

    /// Create from environment variables (fallback when metadata server unavailable).
    pub fn from_env() -> Self {
        Self {
            instance_id: std::env::var("INSTANCE_ID").ok(),
            zone: std::env::var("CLOUD_RUN_ZONE").ok(),
            project_id: std::env::var("GOOGLE_CLOUD_PROJECT").ok(),
            project_number: std::env::var("GOOGLE_CLOUD_PROJECT_NUMBER").ok(),
            service_account: std::env::var("SERVICE_ACCOUNT").ok(),
            region: std::env::var("GOOGLE_CLOUD_REGION").ok(),
        }
    }
}

/// Derive the Cloud Run region from a GCE `instance/zone` value.
///
/// The metadata server reports zones like
/// `projects/123456789/zones/us-central1-a`; the region is that value with the
/// leading path stripped and the trailing single-letter zone suffix (`-a`)
/// removed, giving `us-central1`.
///
/// Degenerate inputs are handled explicitly rather than silently producing an
/// empty string (the previous behaviour):
/// - a value whose final `-` segment is not a single-letter zone suffix
///   (e.g. a bare `"us-central1"` with no `-a`) is treated as already being a
///   region and returned whole;
/// - a value with no `-` at all (e.g. `"uscentral1"`) is likewise returned
///   whole rather than collapsing to `""`;
/// - an empty value yields `None`.
fn region_from_zone(zone: &str) -> Option<String> {
    let last = zone.rsplit('/').next().unwrap_or(zone);
    if last.is_empty() {
        return None;
    }
    match last.rsplit_once('-') {
        Some((region, suffix)) if suffix.len() == 1 && !region.is_empty() => {
            Some(region.to_string())
        }
        _ => Some(last.to_string()),
    }
}

/// Service metadata for the Cloud Run service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceMetadata {
    /// Service name.
    pub name: String,
    /// Revision name.
    pub revision: Option<String>,
    /// Configuration name.
    pub configuration: Option<String>,
    /// Project ID.
    pub project_id: Option<String>,
    /// Region.
    pub region: Option<String>,
    /// Service URL.
    pub url: Option<String>,
}

impl ServiceMetadata {
    /// Create from environment variables.
    pub fn from_env() -> Option<Self> {
        let name = std::env::var("K_SERVICE").ok()?;

        Some(Self {
            name,
            revision: std::env::var("K_REVISION").ok(),
            configuration: std::env::var("K_CONFIGURATION").ok(),
            project_id: std::env::var("GOOGLE_CLOUD_PROJECT").ok(),
            region: std::env::var("GOOGLE_CLOUD_REGION").ok(),
            url: std::env::var("CLOUD_RUN_URL").ok(),
        })
    }

    /// Check if running on Cloud Run.
    pub fn is_cloud_run() -> bool {
        std::env::var("K_SERVICE").is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fetch_errors_when_metadata_server_unreachable() {
        // Port 1 on loopback refuses connections immediately, standing in for
        // an unreachable metadata server. The old code returned Ok(all-None).
        let err = InstanceMetadata::fetch_from("http://127.0.0.1:1/")
            .await
            .expect_err("unreachable metadata server must surface an error");
        assert!(
            matches!(err, crate::CloudRunError::Metadata(_)),
            "expected CloudRunError::Metadata, got {err:?}"
        );
    }

    #[test]
    fn region_from_zone_handles_normal_and_degenerate_inputs() {
        // Full metadata-server zone path -> region with the "-a" suffix stripped.
        assert_eq!(
            region_from_zone("projects/123456789/zones/us-central1-a").as_deref(),
            Some("us-central1")
        );
        // Bare zone without the leading metadata path.
        assert_eq!(
            region_from_zone("us-central1-a").as_deref(),
            Some("us-central1")
        );
        // Degenerate: no single-letter "-a" zone suffix. Treated as already a
        // region and returned whole (previously this over-stripped to "us").
        assert_eq!(
            region_from_zone("us-central1").as_deref(),
            Some("us-central1")
        );
        // Degenerate: no '-' at all -> returned whole, not an empty string
        // (the old derivation silently produced "").
        assert_eq!(
            region_from_zone("uscentral1").as_deref(),
            Some("uscentral1")
        );
        // Empty value -> None.
        assert_eq!(region_from_zone(""), None);
    }

    #[tokio::test]
    async fn fetch_maps_absent_fields_to_none_and_derives_region() {
        use bytes::Bytes;
        use http_body_util::Full;
        use hyper::service::service_fn;
        use hyper::{Request, Response, StatusCode};
        use hyper_util::rt::TokioIo;
        use tokio::net::TcpListener;

        // Stub metadata server: 200 for instance/zone, 404 for everything else.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(|req: Request<hyper::body::Incoming>| async move {
                        let resp = if req.uri().path().ends_with("/instance/zone") {
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Full::new(Bytes::from_static(
                                    b"projects/123456789/zones/us-central1-a",
                                )))
                                .unwrap()
                        } else {
                            Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Full::new(Bytes::new()))
                                .unwrap()
                        };
                        Ok::<_, std::convert::Infallible>(resp)
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
        });

        let base = format!("http://{addr}/computeMetadata/v1/");
        let md = InstanceMetadata::fetch_from(&base)
            .await
            .expect("reachable server (even with 404 fields) must not error");

        // Present field parsed; region derived from zone.
        assert_eq!(
            md.zone.as_deref(),
            Some("projects/123456789/zones/us-central1-a")
        );
        assert_eq!(md.region.as_deref(), Some("us-central1"));
        // 404 fields degrade to None rather than erroring.
        assert!(md.instance_id.is_none());
        assert!(md.project_id.is_none());
        assert!(md.project_number.is_none());
        assert!(md.service_account.is_none());
    }
}
