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

/// The emulator host is honored as an endpoint override too.
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
    assert_eq!(client.endpoint(), "http://localhost:7000");
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
