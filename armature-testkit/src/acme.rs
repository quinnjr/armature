//! Pebble ACME test-CA harness (behind the `containers` feature).

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// A running Pebble ACME test CA. Stops when dropped.
pub struct PebbleCa {
    // Must be kept alive: dropping it stops the container (RAII lifecycle).
    // Also read by `management_ca_pem` to address the container by id.
    container: ContainerAsync<GenericImage>,
    port: u16,
}

impl PebbleCa {
    /// Start Pebble. It listens for the ACME directory on container port 14000.
    pub async fn start() -> PebbleCa {
        let image = GenericImage::new("ghcr.io/letsencrypt/pebble", "2.10.1")
            .with_exposed_port(14000.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Listening on"))
            .with_env_var("PEBBLE_VA_ALWAYS_VALID", "1"); // skip real challenge validation in tests
        let container = tokio::time::timeout(std::time::Duration::from_secs(60), image.start())
            .await
            .expect("pebble container did not become ready within 60s")
            .expect("start pebble container");
        let port = container
            .get_host_port_ipv4(14000.tcp())
            .await
            .expect("pebble mapped port");
        PebbleCa { container, port }
    }

    /// The ACME directory endpoint, e.g. `https://127.0.0.1:PORT/dir`.
    pub fn directory_url(&self) -> String {
        format!("https://127.0.0.1:{}/dir", self.port)
    }

    /// The trust-anchor root certificate URL, e.g. `https://127.0.0.1:PORT/roots/0`.
    ///
    /// Note this is the root of the *issuance* chain (the CA that signs
    /// certificates Pebble issues). It is **not** the root that signs Pebble's
    /// own HTTPS interface — for that, use [`PebbleCa::management_ca_pem`].
    pub fn roots_url(&self) -> String {
        format!("https://127.0.0.1:{}/roots/0", self.port)
    }

    /// The PEM root certificate that signs Pebble's own HTTPS ACME interface.
    ///
    /// Pebble serves its ACME API with a static test certificate signed by
    /// `pebble.minica.pem`, which is baked into the image. A client that
    /// verifies TLS (as it should) must trust *this* root to reach the
    /// directory — fetching `/roots/0` instead yields `UnknownIssuer`, because
    /// that endpoint returns the issuance root, not the interface root.
    ///
    /// The image is distroless, so the file is extracted with `docker cp`
    /// rather than an in-container shell.
    pub fn management_ca_pem(&self) -> Vec<u8> {
        let dest = std::env::temp_dir().join(format!("pebble-minica-{}.pem", std::process::id()));
        let status = std::process::Command::new("docker")
            .arg("cp")
            .arg(format!(
                "{}:/test/certs/pebble.minica.pem",
                self.container.id()
            ))
            .arg(&dest)
            .status()
            .expect("run docker cp for pebble minica");
        assert!(status.success(), "docker cp of pebble.minica.pem failed");
        let pem = std::fs::read(&dest).expect("read pebble minica pem");
        let _ = std::fs::remove_file(&dest);
        pem
    }

    /// Explains Pebble's self-signed CA: the ACME client under test must trust
    /// Pebble's interface root ([`PebbleCa::management_ca_pem`]) to reach the
    /// directory over verified TLS.
    pub fn ca_note() -> &'static str {
        "Pebble serves its ACME API with a cert signed by the image's pebble.minica.pem; trust that root (management_ca_pem) in the ACME client's TLS config. /roots/0 is the issuance root, not the interface root."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn pebble_reports_a_directory_url() {
        crate::skip_if_no_docker!();
        let ca = PebbleCa::start().await;
        let dir = ca.directory_url();
        assert!(dir.ends_with("/dir"), "unexpected directory url: {dir}");
        let roots = ca.roots_url();
        assert!(roots.ends_with("/roots/0"), "unexpected roots url: {roots}");
    }
}
