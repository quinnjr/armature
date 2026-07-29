//! Regression tests for `GraphQLClient::batch`:
//!
//! - The request body must be serialized as a JSON *array* of the individual
//!   GraphQL requests (one round-trip), not a series of separate requests.
//! - The array response must map positionally into `BatchResponse`, preserving
//!   both order and per-entry `data`/`errors`.

use armature_graphql_client::{BatchRequest, GraphQLClient};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn batch_sends_json_array_and_maps_positionally() {
    let server = MockServer::start().await;

    // The endpoint must be hit exactly once (a single round-trip) with a JSON
    // ARRAY body containing both queries in order. The response is a matching
    // array that must map positionally into the `BatchResponse`.
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_json(json!([
            { "query": "query { a }" },
            { "query": "query { b }" },
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "data": { "a": 1 } },
            { "data": { "b": 2 } },
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = GraphQLClient::new(format!("{}/graphql", server.uri()));

    let batch = BatchRequest::new()
        .query("query { a }")
        .query("query { b }");

    let response = client.batch(batch).await.expect("batch should succeed");

    assert_eq!(response.len(), 2, "one response per request, in order");
    assert!(!response.has_errors(), "no per-entry errors expected");

    let first = response.get(0).expect("first response present");
    assert_eq!(first.data.as_ref().unwrap(), &json!({ "a": 1 }));

    let second = response.get(1).expect("second response present");
    assert_eq!(second.data.as_ref().unwrap(), &json!({ "b": 2 }));

    // Exactly one HTTP round-trip served the whole batch.
    server.verify().await;
}

#[tokio::test]
async fn batch_preserves_per_entry_errors() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_json(json!([
            { "query": "query { ok }" },
            { "query": "query { bad }" },
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "data": { "ok": true } },
            { "data": null, "errors": [{ "message": "boom" }] },
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let client = GraphQLClient::new(format!("{}/graphql", server.uri()));

    let batch = BatchRequest::new()
        .query("query { ok }")
        .query("query { bad }");

    let response = client.batch(batch).await.expect("batch should succeed");

    assert_eq!(response.len(), 2);
    assert!(
        response.has_errors(),
        "the second entry carries a GraphQL error"
    );

    let errors = response.all_errors();
    assert_eq!(errors.len(), 1, "exactly one entry has errors");
    assert_eq!(errors[0].message, "boom");

    server.verify().await;
}

#[tokio::test]
async fn empty_batch_makes_no_request() {
    let server = MockServer::start().await;

    // No mock is mounted: any HTTP call would 404 and surface as an error. An
    // empty batch must short-circuit without touching the network.
    let client = GraphQLClient::new(format!("{}/graphql", server.uri()));

    let response = client
        .batch(BatchRequest::new())
        .await
        .expect("empty batch should succeed without a request");

    assert!(response.is_empty());
}
