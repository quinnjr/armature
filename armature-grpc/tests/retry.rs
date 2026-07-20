//! Regression test for Finding 8: `GrpcClientConfig::retry_enabled` /
//! `max_retry_attempts` were dead config — nothing on the client call path
//! ever retried. `GrpcChannel::call_with_retry` now wires them to a real
//! retry loop.

use armature_grpc::{GrpcClient, GrpcClientConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tonic::service::interceptor::InterceptedService;
use tonic_health::pb::HealthCheckRequest;
use tonic_health::pb::health_client::HealthClient;

/// A server that fails the first `fail_first_n` requests with `Unavailable`,
/// then succeeds, used to prove retry actually recovers.
async fn spawn_flaky_health_server(fail_first_n: usize) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (_reporter, health_service) = tonic_health::server::health_reporter();
    let attempts = Arc::new(AtomicUsize::new(0));
    let interceptor = move |req: tonic::Request<()>| -> Result<tonic::Request<()>, tonic::Status> {
        if attempts.fetch_add(1, Ordering::SeqCst) < fail_first_n {
            Err(tonic::Status::unavailable("temporarily unavailable"))
        } else {
            Ok(req)
        }
    };
    let service = InterceptedService::new(health_service, interceptor);

    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    addr
}

#[tokio::test]
async fn client_with_retry_enabled_recovers_from_transient_failures() {
    let addr = spawn_flaky_health_server(2).await;

    let config = GrpcClientConfig::builder()
        .endpoint(format!("http://{addr}"))
        .retry(true)
        .max_retry_attempts(5)
        .build();
    let channel = GrpcClient::connect(config).await.unwrap();

    let result = channel
        .call_with_retry(|| {
            let mut client = HealthClient::new(channel.inner().clone());
            async move {
                client
                    .check(HealthCheckRequest {
                        service: String::new(),
                    })
                    .await
            }
        })
        .await;

    assert!(
        result.is_ok(),
        "retrying client should eventually succeed: {result:?}"
    );
}

#[tokio::test]
async fn client_without_retry_fails_on_first_transient_failure() {
    let addr = spawn_flaky_health_server(2).await;

    let config = GrpcClientConfig::builder()
        .endpoint(format!("http://{addr}"))
        .retry(false)
        .build();
    let channel = GrpcClient::connect(config).await.unwrap();

    let result = channel
        .call_with_retry(|| {
            let mut client = HealthClient::new(channel.inner().clone());
            async move {
                client
                    .check(HealthCheckRequest {
                        service: String::new(),
                    })
                    .await
            }
        })
        .await;

    assert!(
        result.is_err(),
        "non-retrying client should fail on the first transient error"
    );
}
