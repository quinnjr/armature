//! Regression tests for two Critical findings:
//! 1. Aggregations must be parsed from the response's top-level "aggregations"
//!    key (not "aggs", which is only used in *request* bodies).
//! 2. When `OpenSearchConfig::aws_region` is set, outgoing requests must be
//!    signed with real AWS SigV4 (not sent unsigned).

#![cfg(feature = "aws-auth")]

use armature_opensearch::{Document, OpenSearchClient, OpenSearchConfig};
use armature_testkit::{StubResponse, StubServer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct TestDoc {
    name: String,
}

impl Document for TestDoc {
    fn index_name() -> &'static str {
        "test_docs"
    }
}

const SEARCH_RESPONSE_WITH_AGGS: &str = r#"{
  "took": 3,
  "hits": { "total": { "value": 0, "relation": "eq" }, "max_score": null, "hits": [] },
  "aggregations": {
    "by_name": {
      "buckets": [ { "key": "alice", "doc_count": 2 } ]
    }
  }
}"#;

#[tokio::test]
async fn aggregations_are_parsed_from_top_level_aggregations_key() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/test_docs/_search",
            StubResponse::json(200, SEARCH_RESPONSE_WITH_AGGS),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    let result = client
        .search()
        .execute_with_meta::<TestDoc>()
        .await
        .expect("search should succeed");

    let aggs = result.aggregations.expect(
        "aggregations should be Some when response contains a top-level \"aggregations\" object",
    );
    assert_eq!(aggs["by_name"]["buckets"][0]["key"].as_str(), Some("alice"));
}

#[tokio::test]
async fn aws_sigv4_signs_outgoing_requests() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/test_docs/_search",
            StubResponse::json(200, SEARCH_RESPONSE_WITH_AGGS),
        )
        .start()
        .await;

    let provider = aws_credential_types::provider::SharedCredentialsProvider::new(
        aws_credential_types::Credentials::for_tests(),
    );

    let config = OpenSearchConfig::new(server.url())
        .with_aws_region("us-east-1")
        .with_aws_credentials_provider(provider);
    let client = OpenSearchClient::new(config).expect("client construction");

    let _ = client
        .search()
        .execute_with_meta::<TestDoc>()
        .await
        .expect("search should succeed");

    let recorded = server.assert_received("POST", "/test_docs/_search");
    let auth = recorded
        .header("authorization")
        .expect("Authorization header must be present for a SigV4-signed request");

    assert!(
        auth.starts_with("AWS4-HMAC-SHA256 Credential=ANOTREAL/"),
        "unexpected Authorization header: {auth}"
    );
    assert!(
        auth.contains("/us-east-1/es/aws4_request"),
        "expected region 'us-east-1' and service 'es' in scope: {auth}"
    );
    assert!(
        auth.contains("SignedHeaders="),
        "missing SignedHeaders: {auth}"
    );
    assert!(auth.contains("Signature="), "missing Signature: {auth}");
}

#[test]
fn aws_region_without_credentials_provider_is_rejected() {
    let config = OpenSearchConfig::new("http://localhost:9200").with_aws_region("us-east-1");
    let err = OpenSearchClient::new(config)
        .expect_err("client construction must fail without a credentials provider");
    let msg = format!("{err}");
    assert!(
        msg.contains("aws_credentials_provider"),
        "expected error to mention aws_credentials_provider, got: {msg}"
    );
}
