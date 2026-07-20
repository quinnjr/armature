//! End-to-end ACME tests against a real Pebble CA (Docker-gated).
//!
//! Pebble runs with `PEBBLE_VA_ALWAYS_VALID=1`, so it auto-validates challenges
//! without the client having to serve HTTP-01 tokens. Pebble's ACME HTTPS
//! interface is signed by the image's `pebble.minica.pem`, which the test takes
//! from the container and hands to the client via `with_ca_certificate` — the
//! client always verifies TLS and has no option to disable verification.
//!
//! These tests self-skip when Docker is unavailable, unless
//! `ARMATURE_REQUIRE_DOCKER=1` is set (CI), which turns the skip into a
//! failure via `armature_testkit::skip_if_no_docker!`.

use armature_acme::{AcmeClient, AcmeConfig};
use armature_testkit::acme::PebbleCa;

/// Pebble's interface root certificate (PEM), taken from the image itself.
///
/// This is the root that signs Pebble's HTTPS ACME endpoint, so the client can
/// verify TLS normally. (`/roots/0` is the *issuance* root and would yield
/// `UnknownIssuer` here.) No TLS verification is disabled anywhere in these
/// tests — the library under test has no option to do so.
fn pebble_root_pem(ca: &PebbleCa) -> Vec<u8> {
    ca.management_ca_pem()
}

/// Build a config pointed at a running Pebble, trusting its test root.
fn pebble_config(directory_url: String, root_pem: Vec<u8>, tmp: &std::path::Path) -> AcmeConfig {
    AcmeConfig::new(
        directory_url,
        vec!["admin@example.com".to_string()],
        vec![
            "e2e.example.com".to_string(),
            "www.e2e.example.com".to_string(),
        ],
    )
    .with_accept_tos(true)
    .with_account_dir(tmp.join("accounts"))
    .with_cert_dir(tmp.join("certs"))
    .with_ca_certificate(root_pem)
}

fn tempdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "acme-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn order_certificate_end_to_end() {
    armature_testkit::skip_if_no_docker!();
    {
        let ca = PebbleCa::start().await;
        let tmp = tempdir("orch");
        let root_pem = pebble_root_pem(&ca);
        let config = pebble_config(ca.directory_url(), root_pem.clone(), &tmp);

        let mut client = AcmeClient::new(config)
            .await
            .expect("create client + fetch directory");

        client.register_account().await.expect("register account");
        assert!(
            client.account_url().is_some(),
            "account URL must be set after registration"
        );

        let (cert_pem, key_pem) = client
            .order_certificate()
            .await
            .expect("order_certificate must obtain a real cert");

        // The certificate chain must be non-empty and parse as X.509.
        assert!(
            cert_pem.contains("-----BEGIN CERTIFICATE-----"),
            "expected a PEM certificate chain, got: {cert_pem:?}"
        );
        assert!(
            key_pem.contains("-----BEGIN") && key_pem.contains("PRIVATE KEY"),
            "expected a PEM private key"
        );

        // Parse the leaf to prove it is a real certificate.
        let mut reader = std::io::BufReader::new(cert_pem.as_bytes());
        let leaf = rustls_pemfile::certs(&mut reader)
            .next()
            .expect("at least one cert in chain")
            .expect("valid cert DER");
        let (_, parsed) =
            x509_parser::parse_x509_certificate(leaf.as_ref()).expect("parse leaf certificate");
        // SAN must include our domains.
        let san = parsed
            .subject_alternative_name()
            .expect("SAN ext parse")
            .expect("SAN present");
        let names: Vec<String> = san
            .value
            .general_names
            .iter()
            .filter_map(|gn| match gn {
                x509_parser::extensions::GeneralName::DNSName(n) => Some(n.to_string()),
                _ => None,
            })
            .collect();
        assert!(
            names.iter().any(|n| n == "e2e.example.com"),
            "SAN must contain e2e.example.com, got {names:?}"
        );

        // save_certificate must write real files.
        let (cert_path, key_path) = client
            .save_certificate(&cert_pem, &key_pem)
            .await
            .expect("save real certificate");
        assert!(std::path::Path::new(&cert_path).exists());
        assert!(std::path::Path::new(&key_path).exists());
        let saved = std::fs::read_to_string(&cert_path).unwrap();
        assert!(saved.contains("-----BEGIN CERTIFICATE-----"));

        // should_renew must parse the real issued certificate's validity.
        // Use an enormous renewal window so the result is deterministic
        // regardless of the CA's issuance policy: any real, not-yet-expired
        // cert is "within" a ~274-year window, so renewal is due.
        let renew_cfg = AcmeConfig::new(
            ca.directory_url(),
            vec!["admin@example.com".to_string()],
            vec!["e2e.example.com".to_string()],
        )
        .with_ca_certificate(root_pem.clone())
        .with_renew_before_days(100_000);
        let renew_client = AcmeClient::new(renew_cfg)
            .await
            .expect("renew-check client");
        assert!(
            renew_client.should_renew(&cert_path).await.unwrap(),
            "should_renew must parse the issued cert and report due within a huge window"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[tokio::test]
async fn manual_flow_get_challenges_notify_finalize() {
    armature_testkit::skip_if_no_docker!();
    {
        let ca = PebbleCa::start().await;
        let tmp = tempdir("manual");
        let root_pem = pebble_root_pem(&ca);
        let config = pebble_config(ca.directory_url(), root_pem.clone(), &tmp);

        let mut client = AcmeClient::new(config).await.expect("create client");
        client.register_account().await.expect("register account");

        let order_url = client.create_order().await.expect("create order");

        let challenges = client
            .get_challenges(&order_url)
            .await
            .expect("get challenges");
        assert!(!challenges.is_empty(), "expected at least one challenge");
        for ch in &challenges {
            assert!(!ch.token.is_empty());
            assert!(ch.key_authorization.starts_with(&format!("{}.", ch.token)));
            // In a non-PEBBLE_VA_ALWAYS_VALID deployment we'd serve this now.
            client
                .notify_challenge_ready(&ch.url)
                .await
                .expect("challenge should validate");
        }

        let (cert_pem, key_pem) = client.finalize_order(&order_url).await.expect("finalize");
        assert!(cert_pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(key_pem.contains("PRIVATE KEY"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
