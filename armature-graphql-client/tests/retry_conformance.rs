//! Regression tests: `GraphQLClientConfig::retry_enabled` /
//! `max_retries` must actually be honored by `execute_request` (and `batch`),
//! and a well-formed GraphQL error response must never be retried.

use armature_graphql_client::{GraphQLClient, GraphQLClientConfig};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn retries_on_5xx_then_succeeds() {
    let server = MockServer::start().await;

    // First response: 500. Second response: 200 with data.
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "ok": true }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let config = GraphQLClientConfig::builder()
        .endpoint(format!("{}/graphql", server.uri()))
        .retry(true)
        .max_retries(3)
        .build();
    let client = GraphQLClient::with_config(config);

    let response: serde_json::Value = client
        .query("query { ok }")
        .send()
        .await
        .expect("client should retry past the 500 and succeed");

    assert_eq!(response, json!({ "ok": true }));

    server.verify().await;
}

#[tokio::test]
async fn does_not_retry_graphql_error_response() {
    let server = MockServer::start().await;

    // A well-formed GraphQL error response (200 + errors) should be seen
    // exactly once — it must never be retried.
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": null,
            "errors": [{ "message": "boom" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let config = GraphQLClientConfig::builder()
        .endpoint(format!("{}/graphql", server.uri()))
        .retry(true)
        .max_retries(3)
        .build();
    let client = GraphQLClient::with_config(config);

    let result: armature_graphql_client::Result<serde_json::Value> =
        client.query("query { ok }").send().await;

    assert!(result.is_err(), "expected a GraphQL error result");

    server.verify().await;
}

#[tokio::test]
async fn no_retry_when_disabled() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(500))
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
        "expected the single 500 to surface as an error"
    );

    server.verify().await;
}
