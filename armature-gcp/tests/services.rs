//! Integration tests for the `GcpServices` container.
//!
//! These exercise the public accessor error contract and the credential-free
//! construction path for the REST-based services (which build entirely offline
//! when a static access token is supplied). The gRPC-backed services
//! (storage/pubsub) have their credential/endpoint wiring covered by the
//! in-crate unit tests on the pure `credentials` helpers, since constructing
//! their clients would require reaching Google's auth/metadata endpoints.

use armature_gcp::GcpConfig;
// `CredentialsSource` / `GcpServices` are only referenced by the REST
// feature-gated tests below, so bring them in there to keep single-feature
// builds warning-clean.
#[cfg(any(
    feature = "secret-manager",
    feature = "cloud-run",
    feature = "cloud-functions"
))]
use armature_gcp::{CredentialsSource, GcpServices};

/// enable_*/is_enabled must round-trip through the config.
#[test]
fn enable_flags_round_trip_through_is_enabled() {
    let config = GcpConfig::builder()
        .enable_secret_manager()
        .enable_cloud_run()
        .build();

    assert!(config.is_enabled("secret-manager"));
    assert!(config.is_enabled("cloud-run"));
    assert!(!config.is_enabled("cloud-functions"));
    assert!(!config.is_enabled("storage"));
}

/// enable_data no longer references the removed firestore service.
#[test]
fn enable_data_covers_storage_spanner_bigquery_only() {
    let config = GcpConfig::builder().enable_data().build();
    assert!(config.is_enabled("storage"));
    assert!(config.is_enabled("spanner"));
    assert!(config.is_enabled("bigquery"));
    assert!(!config.is_enabled("firestore"));
}

/// A feature-compiled-but-not-enabled accessor returns `ServiceNotConfigured`.
#[cfg(feature = "secret-manager")]
#[tokio::test]
async fn accessor_not_configured_when_service_disabled() {
    // Nothing enabled -> new() initializes nothing.
    let services = GcpServices::new(GcpConfig::default()).await.unwrap();
    let Err(err) = services.secret_manager() else {
        panic!("expected an error for a disabled service");
    };
    assert!(
        matches!(
            err,
            armature_gcp::GcpError::ServiceNotConfigured("secret-manager")
        ),
        "unexpected error: {err:?}"
    );
}

/// Enabling a REST service constructs the client eagerly (offline, via a static
/// access token) and the accessor returns it.
#[cfg(feature = "secret-manager")]
#[tokio::test]
async fn secret_manager_constructs_and_accessor_returns_ok() {
    let config = GcpConfig::builder()
        .project_id("test-project")
        .credentials(CredentialsSource::AccessToken("dummy-token".into()))
        .enable_secret_manager()
        .build();

    let services = GcpServices::new(config).await.unwrap();
    let client = services
        .secret_manager()
        .expect("client should be initialized");
    assert_eq!(client.project_id(), "test-project");
    assert_eq!(client.endpoint(), "https://secretmanager.googleapis.com");
}

/// A per-service endpoint override (service_configs) is threaded into the client.
#[cfg(feature = "cloud-run")]
#[tokio::test]
async fn cloud_run_honors_endpoint_override() {
    let config = GcpConfig::builder()
        .project_id("test-project")
        .credentials(CredentialsSource::AccessToken("dummy-token".into()))
        .service_config(
            "cloud-run",
            serde_json::json!({ "endpoint": "http://localhost:8088" }),
        )
        .enable_cloud_run()
        .build();

    let services = GcpServices::new(config).await.unwrap();
    let client = services.cloud_run().expect("client should be initialized");
    assert_eq!(client.endpoint(), "http://localhost:8088");
}

/// The emulator host is honored as an endpoint override too. A bare host
/// normalizes to https (a bearer token is attached, so plaintext is never
/// inferred; explicit `http://` would be required for cleartext).
#[cfg(feature = "cloud-functions")]
#[tokio::test]
async fn cloud_functions_honors_emulator_host() {
    let config = GcpConfig::builder()
        .project_id("test-project")
        .credentials(CredentialsSource::AccessToken("dummy-token".into()))
        .emulator_host("localhost:7000")
        .enable_cloud_functions()
        .build();

    let services = GcpServices::new(config).await.unwrap();
    let client = services
        .cloud_functions()
        .expect("client should be initialized");
    assert_eq!(client.endpoint(), "https://localhost:7000");
}

/// The Secret Manager client emits the expected request path and
/// `Authorization: Bearer <token>` header. A local single-shot stub server on an
/// ephemeral loopback port captures the raw request; the client's endpoint is
/// pointed at it via an explicit `http://` `service_config` override.
#[cfg(feature = "secret-manager")]
#[tokio::test]
async fn secret_manager_targets_expected_path_and_auth_header() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // The stub hands the captured (path, authorization-header) back to the test.
    let (tx, rx) = tokio::sync::oneshot::channel::<(String, Option<String>)>();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();

        // Read the request head (a GET has no body; it ends at the blank line).
        let mut raw = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&raw);

        // Request line: "GET <path> HTTP/1.1".
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .to_string();

        // Authorization header (case-insensitive name).
        let auth = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
            .and_then(|line| line.split_once(':').map(|(_, v)| v))
            .map(|value| value.trim().to_string());

        // Minimal well-formed JSON response so the client's decode succeeds.
        let body = r#"{"name":"ok"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        let _ = tx.send((path, auth));
    });

    let config = GcpConfig::builder()
        .project_id("test-project")
        .credentials(CredentialsSource::AccessToken("dummy-token".into()))
        .service_config(
            "secret-manager",
            serde_json::json!({ "endpoint": format!("http://{addr}") }),
        )
        .enable_secret_manager()
        .build();

    let services = GcpServices::new(config).await.unwrap();
    let client = services
        .secret_manager()
        .expect("client should be initialized");

    let value = client
        .access_secret_version("my-secret", "latest")
        .await
        .expect("request against the local stub should succeed");
    assert_eq!(value, serde_json::json!({ "name": "ok" }));

    let (path, auth) = rx.await.expect("stub should capture one request");
    assert_eq!(
        path,
        "/v1/projects/test-project/secrets/my-secret/versions/latest:access"
    );
    assert_eq!(auth.as_deref(), Some("Bearer dummy-token"));
}

/// A REST service enabled without a project id fails construction with a clear error.
#[cfg(feature = "secret-manager")]
#[tokio::test]
async fn rest_service_requires_project_id() {
    let config = GcpConfig::builder()
        .credentials(CredentialsSource::AccessToken("dummy-token".into()))
        .enable_secret_manager()
        .build();

    let Err(err) = GcpServices::new(config).await else {
        panic!("expected construction to fail without a project id");
    };
    assert!(
        matches!(err, armature_gcp::GcpError::ProjectNotSpecified),
        "unexpected error: {err:?}"
    );
}
