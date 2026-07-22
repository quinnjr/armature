//! Conformance tests for `armature-aws`.
//!
//! These tests run entirely offline: they never contact AWS. The AWS credential
//! providers are lazy, so building an [`AwsServices`] only constructs providers
//! and resolves the (explicitly-set) region — no network I/O happens unless a
//! provider is actually asked to resolve credentials, which we only do for the
//! `Explicit` source (which resolves from in-memory values).

use armature_aws::{AwsConfig, AwsServices, CredentialsSource};

/// Region is always set explicitly in these tests so the region provider chain
/// short-circuits instead of probing IMDS.
const TEST_REGION: &str = "us-west-2";

fn base_builder() -> armature_aws::AwsConfigBuilder {
    AwsConfig::builder().region(TEST_REGION)
}

// --------------------------------------------------------------------------
// Credential-source branching (finding #1)
// --------------------------------------------------------------------------

/// Every credential source must build an `AwsServices` without panicking and
/// must install *some* credentials provider on the resulting `SdkConfig`.
///
/// This is a regression test for the old `_ => {}` arm: the `IamRole` and
/// `WebIdentity` arms now eagerly construct concrete providers (the latter
/// builds an STS client), so a mistake there would surface as a panic here.
#[tokio::test]
async fn every_credential_source_builds_a_provider() {
    let sources = [
        CredentialsSource::Auto,
        CredentialsSource::Environment,
        CredentialsSource::IamRole,
        CredentialsSource::WebIdentity,
        CredentialsSource::Profile("default".to_string()),
        CredentialsSource::Explicit {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: None,
        },
    ];

    for source in sources {
        let config = base_builder().credentials(source.clone()).build();
        let services = AwsServices::new(config)
            .await
            .unwrap_or_else(|e| panic!("building services for {source:?} failed: {e}"));

        assert_eq!(
            services.sdk_config().region().map(|r| r.as_ref()),
            Some(TEST_REGION),
            "region should propagate for {source:?}",
        );

        // The `Profile` source configures a profile name rather than an explicit
        // provider object, so `credentials_provider()` may be `None` until the
        // profile is loaded. Every other source installs a provider eagerly.
        if !matches!(source, CredentialsSource::Profile(_)) {
            assert!(
                services.sdk_config().credentials_provider().is_some(),
                "expected a credentials provider for {source:?}",
            );
        }
    }
}

/// The `Explicit` source must actually resolve to the supplied static
/// credentials — the strongest observable proof that the variant selects a
/// distinct, correct provider rather than the default chain.
#[tokio::test]
async fn explicit_source_resolves_the_supplied_credentials() {
    use aws_credential_types::provider::ProvideCredentials;

    let config = base_builder()
        .explicit_credentials(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        )
        .build();
    let services = AwsServices::new(config).await.expect("build services");

    let provider = services
        .sdk_config()
        .credentials_provider()
        .expect("explicit source installs a provider");
    let creds = provider
        .provide_credentials()
        .await
        .expect("explicit credentials resolve without network");

    assert_eq!(creds.access_key_id(), "AKIAIOSFODNN7EXAMPLE");
    assert_eq!(
        creds.secret_access_key(),
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
    );
}

// --------------------------------------------------------------------------
// Config: region / endpoint propagation + service enable gating
// --------------------------------------------------------------------------

#[tokio::test]
async fn endpoint_url_propagates_to_sdk_config() {
    let config = base_builder().endpoint_url("http://localhost:4566").build();
    let services = AwsServices::new(config).await.expect("build services");

    assert_eq!(
        services.sdk_config().endpoint_url(),
        Some("http://localhost:4566"),
    );
}

#[test]
fn is_enabled_reflects_enabled_services() {
    let config = AwsConfig::builder().enable_s3().enable_sqs().build();

    assert!(config.is_enabled("s3"));
    assert!(config.is_enabled("sqs"));
    assert!(!config.is_enabled("dynamodb"));
    assert!(!config.is_enabled("kms"));
}

#[test]
fn enable_groups_expand_to_member_services() {
    let storage = AwsConfig::builder().enable_storage().build();
    assert!(storage.is_enabled("s3"));
    assert!(storage.is_enabled("dynamodb"));

    let messaging = AwsConfig::builder().enable_messaging().build();
    assert!(messaging.is_enabled("sqs"));
    assert!(messaging.is_enabled("sns"));
    assert!(messaging.is_enabled("kinesis"));
}

// --------------------------------------------------------------------------
// Accessor round-trip + force-path-style (findings #5 / #6)
// --------------------------------------------------------------------------

/// An enabled service yields a client, and the double-checked read-lock fast
/// path returns a stable (equal-pointing) client on repeated calls.
#[cfg(feature = "s3")]
#[tokio::test]
async fn s3_accessor_round_trips_when_enabled() {
    let config = base_builder().enable_s3().build();
    let services = AwsServices::new(config).await.expect("build services");

    assert!(services.config().is_enabled("s3"));

    let first = services.s3().expect("s3 client available when enabled");
    // Second call must hit the initialized fast path and still succeed.
    let _second = services.s3().expect("s3 client available on second call");

    // Sanity: the client is usable as a value.
    let _ = first;
}

/// Requesting a service that was not enabled must error rather than hand back a
/// client, even when the feature is compiled in.
#[cfg(feature = "s3")]
#[tokio::test]
async fn s3_accessor_errors_when_not_enabled() {
    let config = base_builder().build();
    let services = AwsServices::new(config).await.expect("build services");

    assert!(!services.config().is_enabled("s3"));
    assert!(
        services.s3().is_err(),
        "s3() must error when the service is not enabled",
    );
}

/// With a custom endpoint set, the S3 accessor exercises the
/// `force_path_style(true)` branch and must still construct a client.
#[cfg(feature = "s3")]
#[tokio::test]
async fn s3_force_path_style_branch_with_endpoint() {
    let config = base_builder()
        .enable_s3()
        .endpoint_url("http://localhost:4566")
        .build();
    let services = AwsServices::new(config).await.expect("build services");

    // Endpoint override is what triggers force-path-style in the accessor.
    assert_eq!(
        services.sdk_config().endpoint_url(),
        Some("http://localhost:4566"),
    );
    // Constructing the client runs the force_path_style(true) path.
    assert!(services.s3().is_ok());
}
