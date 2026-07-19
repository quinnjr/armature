//! Pebble ACME test-CA harness (behind the `containers` feature).

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// A running Pebble ACME test CA. Stops when dropped.
pub struct PebbleCa {
    // Never read directly after construction, but must be kept alive: dropping it
    // stops the container (RAII lifecycle).
    #[allow(dead_code)]
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
    pub fn roots_url(&self) -> String {
        format!("https://127.0.0.1:{}/roots/0", self.port)
    }

    /// Explains Pebble's self-signed CA: the ACME client under test must accept
    /// Pebble's test root (served at `/roots/0`) — Workflow 4 wires a rustls
    /// client config that trusts it.
    pub fn ca_note() -> &'static str {
        "Pebble uses a self-signed test CA; fetch its root from /roots/0 and trust it in the ACME client's TLS config."
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
