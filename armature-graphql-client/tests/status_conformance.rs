//! Regression test: a non-success HTTP status that reqwest surfaces instead of
//! following (e.g. `304 Not Modified` or any 3xx from a broken proxy/cache)
//! must return an `Err`, never panic inside the request loop.
//!
//! `response.status().is_success()` is false for 1xx/3xx as well as 4xx/5xx, but
//! reqwest's `error_for_status()` only returns `Err` for 400-599. The old code
//! called `.unwrap_err()` on that `Result`, so a surfaced 3xx panicked on
//! server-controlled input. This test would panic against that code and passes
//! once the `Ok` case is handled explicitly.

use armature_graphql_client::{GraphQLClient, GraphQLClientConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn non_success_3xx_status_returns_err_not_panic() {
    let server = MockServer::start().await;

    // 304 Not Modified: not a success, and reqwest does not treat it as an
    // error status, so `error_for_status()` returns `Ok`. Retries are disabled
    // so the single response is what surfaces.
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&server)
        .await;

    let config = GraphQLClientConfig::builder()
        .endpoint(format!("{}/graphql", server.uri()))
        .retry(false)
        .build();
    let client = GraphQLClient::with_config(config);

    let result: armature_graphql_client::Result<serde_json::Value> =
        client.query("query { ok }").send().await;

    assert!(
        result.is_err(),
        "a surfaced 3xx must return an Err, not panic"
    );

    server.verify().await;
}
