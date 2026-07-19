//! Pebble ACME test-CA harness (behind the `containers` feature).

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// A running Pebble ACME test CA. Stops when dropped.
pub struct PebbleCa {
    container: ContainerAsync<GenericImage>,
}

impl PebbleCa {
    /// Start Pebble. It listens for the ACME directory on container port 14000.
    pub async fn start() -> PebbleCa {
        let image = GenericImage::new("ghcr.io/letsencrypt/pebble", "latest")
            .with_exposed_port(14000.tcp())
            .with_wait_for(WaitFor::message_on_stdout("Listening on"))
            .with_env_var("PEBBLE_VA_ALWAYS_VALID", "1"); // skip real challenge validation in tests
        let container = image.start().await.expect("start pebble container");
        PebbleCa { container }
    }

    /// The ACME directory endpoint, e.g. `https://127.0.0.1:PORT/dir`.
    pub async fn directory_url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(14000.tcp())
            .await
            .expect("pebble mapped port");
        format!("https://127.0.0.1:{port}/dir")
    }

    /// The trust-anchor root certificate URL, e.g. `https://127.0.0.1:PORT/roots/0`.
    pub async fn roots_url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(14000.tcp())
            .await
            .expect("pebble mapped port");
        format!("https://127.0.0.1:{port}/roots/0")
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
        let dir = ca.directory_url().await;
        assert!(dir.ends_with("/dir"), "unexpected directory url: {dir}");
        let roots = ca.roots_url().await;
        assert!(roots.ends_with("/roots/0"), "unexpected roots url: {roots}");
    }
}
