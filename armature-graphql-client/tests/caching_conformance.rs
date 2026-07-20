//! Regression tests for Finding 3: `GraphQLClientConfig::caching` /
//! `cache_ttl` must actually be honored for query operations.

use std::time::Duration;

use armature_graphql_client::{GraphQLClient, GraphQLClientConfig};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn identical_query_within_ttl_hits_cache() {
    let server = MockServer::start().await;

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
        .caching(true)
        .cache_ttl(Duration::from_secs(60))
        .build();
    let client = GraphQLClient::with_config(config);

    let first: serde_json::Value = client
        .query("query { ok }")
        .send()
        .await
        .expect("first request should succeed");
    let second: serde_json::Value = client
        .query("query { ok }")
        .send()
        .await
        .expect("second identical request should be served from cache");

    assert_eq!(first, json!({ "ok": true }));
    assert_eq!(second, json!({ "ok": true }));

    // Exactly one HTTP call should have reached the server.
    server.verify().await;
}

#[tokio::test]
async fn mutations_are_never_cached() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "ok": true }
        })))
        .expect(2)
        .mount(&server)
        .await;

    let config = GraphQLClientConfig::builder()
        .endpoint(format!("{}/graphql", server.uri()))
        .caching(true)
        .cache_ttl(Duration::from_secs(60))
        .build();
    let client = GraphQLClient::with_config(config);

    let _: serde_json::Value = client
        .mutation("mutation { ok }")
        .send()
        .await
        .expect("first mutation should succeed");
    let _: serde_json::Value = client
        .mutation("mutation { ok }")
        .send()
        .await
        .expect("second mutation should succeed");

    // Both mutation calls must reach the server — mutations are never cached.
    server.verify().await;
}

#[tokio::test]
async fn cache_expires_after_ttl() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "ok": true }
        })))
        .expect(2)
        .mount(&server)
        .await;

    let config = GraphQLClientConfig::builder()
        .endpoint(format!("{}/graphql", server.uri()))
        .caching(true)
        .cache_ttl(Duration::from_millis(50))
        .build();
    let client = GraphQLClient::with_config(config);

    let _: serde_json::Value = client
        .query("query { ok }")
        .send()
        .await
        .expect("first request should succeed");

    tokio::time::sleep(Duration::from_millis(150)).await;

    let _: serde_json::Value = client
        .query("query { ok }")
        .send()
        .await
        .expect("request after TTL expiry should re-fetch");

    server.verify().await;
}
