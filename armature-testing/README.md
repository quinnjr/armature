# armature-testing

Testing utilities for the Armature framework.

## Features

- **Test Client** - HTTP client for testing handlers
- **Mock Services** - Mock external dependencies
- **Fixtures** - Database and state fixtures
- **Assertions** - HTTP response assertions
- **Integration Tests** - Full application testing

## Installation

```toml
[dev-dependencies]
armature-testing = "0.1"
```

## Quick Start

```rust
use armature_testing::prelude::*;
use armature_core::HttpResponse;

#[tokio::test]
async fn test_hello_endpoint() {
    let app = TestAppBuilder::new()
        .with_route("/hello", |_req| async {
            Ok(HttpResponse::ok().with_body(b"Hello, World!".to_vec()))
        })
        .build();

    let client = app.client();
    let response = client.get("/hello").await;

    assert_status(&response, 200);
    assert_eq!(response.body_string(), Some("Hello, World!".to_string()));
}
```

## Test Client

`TestClient` is built from a running `TestApp` (via `app.client()`) or directly
from an `Arc<Router>`. Its methods run the request through the router and
resolve immediately — there's no `.send()` step.

```rust
use armature_testing::prelude::*;

let client = app.client();

// GET request
let resp = client.get("/users").await;

// POST with a JSON body
let body = serde_json::to_vec(&user).unwrap();
let resp = client.post("/users", body).await;

// PUT and PATCH take a body the same way; DELETE takes none
let resp = client.put("/users/1", body).await;
let resp = client.delete("/users/1").await;

// Inspect the response
assert_eq!(resp.status(), Some(200));
let text: Option<String> = resp.body_string();
let json: serde_json::Value = resp.body_json().unwrap();
```

## Assertions

```rust
use armature_testing::prelude::*;

assert_status(&response, 200);
assert_json(&response, &serde_json::json!({"id": 1, "name": "Test"}));
assert_header(&response, "Content-Type", "application/json");
```

## License

MIT OR Apache-2.0

