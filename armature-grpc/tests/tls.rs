//! Regression test for Finding 3: the README advertises "TLS - Secure
//! connections with rustls" but neither the client nor the server actually
//! wired any TLS configuration. This asserts a plaintext connection to a
//! TLS-only server fails, while a properly-configured TLS client succeeds.

use armature_grpc::{
    GrpcClient, GrpcClientConfig, GrpcClientTlsConfig, GrpcServer, GrpcServerConfig,
    GrpcServerTlsConfig,
};
use std::time::Duration;

fn generate_self_signed_localhost_cert() -> (String, String) {
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
    let cert = params.self_signed(&key_pair).unwrap();
    (cert.pem(), key_pair.serialize_pem())
}

#[tokio::test]
async fn tls_configured_client_connects_while_plaintext_client_fails() {
    let (cert_pem, key_pem) = generate_self_signed_localhost_cert();

    // Grab a free ephemeral port, then hand it to the gRPC server config.
    let addr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };

    let mut server_config = GrpcServerConfig::builder()
        .bind_socket_addr(addr)
        .tls(GrpcServerTlsConfig::new(
            cert_pem.clone().into_bytes(),
            key_pem.into_bytes(),
        ))
        .build()
        .unwrap();
    // The service under test *is* the health service; avoid double-registering
    // a second, builder-managed one under the same route.
    server_config.enable_health_check = false;

    let (_reporter, health_service) = tonic_health::server::health_reporter();
    tokio::spawn(async move {
        let _ = GrpcServer::builder(server_config)
            .serve(health_service)
            .await;
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    // A plaintext client speaking to a TLS-only server must fail — either at
    // `connect()` (transport-level failure) or on the first RPC (since some
    // transports lazily complete the HTTP/2 handshake on first use).
    let plaintext = GrpcClient::connect_default(format!("http://{addr}")).await;
    match plaintext {
        Err(_) => {}
        Ok(channel) => {
            let mut health_client =
                tonic_health::pb::health_client::HealthClient::new(channel.inner().clone());
            let resp = health_client
                .check(tonic_health::pb::HealthCheckRequest {
                    service: String::new(),
                })
                .await;
            assert!(
                resp.is_err(),
                "expected a plaintext RPC against a TLS-only server to fail"
            );
        }
    }

    // A TLS-configured client trusting the server's self-signed CA succeeds.
    let client_tls = GrpcClientTlsConfig::new()
        .ca_certificate(cert_pem.into_bytes())
        .domain_name("localhost");
    let client_config = GrpcClientConfig::builder()
        .endpoint(format!("https://{addr}"))
        .tls(client_tls)
        .build();

    let channel = GrpcClient::connect(client_config)
        .await
        .expect("TLS client should connect to the TLS-configured server");

    let mut health_client =
        tonic_health::pb::health_client::HealthClient::new(channel.inner().clone());
    let resp = health_client
        .check(tonic_health::pb::HealthCheckRequest {
            service: String::new(),
        })
        .await;
    assert!(resp.is_ok(), "TLS RPC should succeed: {resp:?}");
}
