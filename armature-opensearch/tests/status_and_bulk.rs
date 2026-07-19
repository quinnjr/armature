//! Regression tests for two Warning findings:
//! 1. Several index/client operations sent `.send().await?` without checking
//!    the response status code, silently treating 4xx/5xx as success (and, for
//!    `delete_by_query`/`count`, returning `0` instead of an error).
//! 2. `BulkOperation`/`to_bulk_lines` had no caller; `OpenSearchClient::bulk_execute`
//!    wires them up to actually send a mixed batch of bulk operations.

use armature_opensearch::{BulkOperation, Document, OpenSearchClient, OpenSearchConfig};
use armature_testkit::{StubResponse, StubServer};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
struct TestDoc {
    name: String,
}

impl Document for TestDoc {
    fn index_name() -> &'static str {
        "test_docs"
    }
}

const ERROR_BODY: &str = r#"{"error": {"type": "some_exception", "reason": "boom"}}"#;

#[tokio::test]
async fn index_refresh_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/test_docs/_refresh",
            StubResponse::json(500, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    let err = client
        .indices()
        .refresh("test_docs")
        .await
        .expect_err("a 500 response must surface as an error, not Ok(())");
    assert!(format!("{err}").contains("boom"), "unexpected error: {err}");
}

#[tokio::test]
async fn index_open_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/test_docs/_open",
            StubResponse::json(404, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .indices()
        .open("test_docs")
        .await
        .expect_err("a 404 response must surface as an error, not Ok(())");
}

#[tokio::test]
async fn index_close_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/test_docs/_close",
            StubResponse::json(409, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .indices()
        .close("test_docs")
        .await
        .expect_err("a 409 response must surface as an error, not Ok(())");
}

#[tokio::test]
async fn index_flush_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/test_docs/_flush",
            StubResponse::json(500, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    let err = client
        .indices()
        .flush("test_docs")
        .await
        .expect_err("a 500 response must surface as an error, not Ok(())");
    assert!(format!("{err}").contains("boom"), "unexpected error: {err}");
}

#[tokio::test]
async fn index_create_alias_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "PUT",
            "/test_docs/_alias/my_alias",
            StubResponse::json(404, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .indices()
        .create_alias("test_docs", "my_alias")
        .await
        .expect_err("a 404 response must surface as an error, not Ok(())");
}

#[tokio::test]
async fn index_delete_alias_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "DELETE",
            "/test_docs/_alias/my_alias",
            StubResponse::json(404, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .indices()
        .delete_alias("test_docs", "my_alias")
        .await
        .expect_err("a 404 response must surface as an error, not Ok(())");
}

#[tokio::test]
async fn client_delete_by_query_surfaces_failure_instead_of_zero() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/test_docs/_delete_by_query",
            StubResponse::json(400, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    let err = client
        .delete_by_query("test_docs", json!({ "match_all": {} }))
        .await
        .expect_err("a 400 response must surface as an error, not Ok(0)");
    assert!(format!("{err}").contains("boom"), "unexpected error: {err}");
}

#[tokio::test]
async fn client_count_surfaces_failure_instead_of_zero() {
    let server = StubServer::builder()
        .route(
            "POST",
            "/test_docs/_count",
            StubResponse::json(500, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .search()
        .index("test_docs")
        .count()
        .await
        .expect_err("a 500 response must surface as an error, not Ok(0)");
}

#[tokio::test]
async fn client_refresh_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/test_docs/_refresh",
            StubResponse::json(500, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .refresh("test_docs")
        .await
        .expect_err("a 500 response must surface as an error, not Ok(())");
}

#[tokio::test]
async fn client_health_surfaces_failure_status() {
    let server = StubServer::builder()
        .route(
            "GET",
            "/_cluster/health",
            StubResponse::json(503, ERROR_BODY),
        )
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    client
        .health()
        .await
        .expect_err("a 503 response must surface as an error, not Ok(body)");
}

#[tokio::test]
async fn bulk_execute_sends_ndjson_and_parses_response() {
    let bulk_response = json!({
        "took": 5,
        "errors": false,
        "items": [
            {
                "index": {
                    "_index": "test_docs",
                    "_id": "1",
                    "_version": 1,
                    "result": "created",
                    "status": 201
                }
            },
            {
                "delete": {
                    "_index": "test_docs",
                    "_id": "2",
                    "_version": 1,
                    "result": "deleted",
                    "status": 200
                }
            }
        ]
    })
    .to_string();

    let server = StubServer::builder()
        .route("POST", "/_bulk", StubResponse::json(200, bulk_response))
        .start()
        .await;

    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    let ops = vec![
        BulkOperation::Index {
            id: "1".to_string(),
            doc: TestDoc {
                name: "alice".to_string(),
            },
        },
        BulkOperation::Delete {
            id: "2".to_string(),
        },
    ];

    let response = client
        .bulk_execute(ops)
        .await
        .expect("bulk_execute should succeed");

    assert_eq!(response.items.len(), 2);
    assert!(!response.errors);

    let recorded = server.assert_received("POST", "/_bulk");
    let body = recorded.body_string();
    let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 3, "expected 3 NDJSON lines, got: {body}");
    assert!(lines[0].contains("\"index\""));
    assert!(lines[1].contains("\"alice\""));
    assert!(lines[2].contains("\"delete\""));
}

#[tokio::test]
async fn bulk_execute_with_no_operations_short_circuits() {
    let server = StubServer::start_single(StubResponse::json(500, ERROR_BODY)).await;
    let config = OpenSearchConfig::new(server.url());
    let client = OpenSearchClient::new(config).expect("client construction");

    let response = client
        .bulk_execute::<TestDoc>(vec![])
        .await
        .expect("empty batch should short-circuit to Ok without a request");

    assert_eq!(response.items.len(), 0);
    assert!(server.requests().is_empty());
}
