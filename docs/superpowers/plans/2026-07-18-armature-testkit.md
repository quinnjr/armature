# armature-testkit (Workflow 0) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `armature-testkit`, a dev-oriented crate providing deterministic, offline test harnesses (a hyper-based HTTP stub server with request recording, plus Docker-gated testcontainer and ACME/Pebble helpers) that every later conformance workflow uses to verify real integrations without live credentials.

**Architecture:** A new `publish = false` workspace crate. The always-available core is an HTTP stub server (hyper 1.x) that scripts responses by method+path and records received requests for assertions — this is what auth/cloud/provider/web workflows need. Heavier helpers (Redis/Postgres/OpenSearch containers, Pebble ACME CA) live behind a `containers` feature and self-skip when Docker is absent.

**Tech Stack:** Rust 2024, tokio, hyper 1.x + hyper-util + http-body-util, bytes, testcontainers (optional).

## Global Constraints

- Rust 2024 edition, MSRV 1.89 (inherit via `edition.workspace = true`, `rust-version.workspace = true`).
- The crate is `publish = false`.
- Default build must NOT require Docker or any native/OpenSSL dependency; container/ACME helpers are behind the `containers` feature and additionally self-skip at runtime when Docker is unavailable.
- `armature-testkit` must NOT depend on any `armature-*` crate (avoid dependency cycles — it will be a dev-dependency of them).
- Pre-commit gate must pass: `cargo fmt -- --check` and `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings` (no lint allowances).
- Commit after every task.

---

### Task 1: Scaffold the crate and register it in the workspace

**Files:**
- Create: `armature-testkit/Cargo.toml`
- Create: `armature-testkit/src/lib.rs`
- Modify: `Cargo.toml` (root, `[workspace].members`, after `"armature-mcp",` on line 63)

**Interfaces:**
- Produces: crate `armature_testkit` with a `pub fn crate_smoke() -> bool` used only to prove the crate compiles and tests run; removed in Task 2.

- [ ] **Step 1: Create the crate manifest**

`armature-testkit/Cargo.toml`:
```toml
[package]
name = "armature-testkit"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
description = "Deterministic test harnesses (HTTP stubs, containers) for the Armature framework"
publish = false

[features]
default = []
# Docker-backed helpers (testcontainers, Pebble ACME). Off by default so the
# standard `cargo test` never requires Docker.
containers = []

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "sync", "time", "io-util"] }
hyper = { version = "1", features = ["server", "http1"] }
hyper-util = { version = "0.1", features = ["tokio"] }
http-body-util = "0.1"
bytes = "1"

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time"] }
```

- [ ] **Step 2: Create the lib with a smoke function and its test**

`armature-testkit/src/lib.rs`:
```rust
//! Deterministic, offline test harnesses for verifying Armature integrations.

/// Temporary smoke marker proving the crate compiles and tests run.
pub fn crate_smoke() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        assert!(crate_smoke());
    }
}
```

- [ ] **Step 3: Register the crate in the workspace**

In root `Cargo.toml`, add `"armature-testkit",` to `[workspace].members` immediately after the `"armature-mcp",` line.

- [ ] **Step 4: Verify it builds and tests pass**

Run: `cargo test -p armature-testkit`
Expected: PASS (1 test, `smoke`).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/Cargo.toml armature-testkit/src/lib.rs Cargo.toml
git commit -m "feat(testkit): scaffold armature-testkit crate"
```

---

### Task 2: HTTP stub server — script a single response

**Files:**
- Create: `armature-testkit/src/http_stub.rs`
- Modify: `armature-testkit/src/lib.rs` (remove `crate_smoke`, add `pub mod http_stub;` and re-exports)

**Interfaces:**
- Produces:
  - `pub struct StubResponse { pub status: u16, pub headers: Vec<(String, String)>, pub body: bytes::Bytes }` with `StubResponse::new(status: u16, body: impl Into<Bytes>) -> Self` and `StubResponse::json(status: u16, body: impl Into<Bytes>) -> Self` (adds `content-type: application/json`).
  - `pub struct StubServer` with `async fn start_single(resp: StubResponse) -> StubServer` and `fn url(&self) -> &str` (e.g. `http://127.0.0.1:PORT`).

- [ ] **Step 1: Write the failing test**

Append to `armature-testkit/src/http_stub.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serves_a_single_scripted_response() {
        let server = StubServer::start_single(StubResponse::json(200, r#"{"ok":true}"#)).await;

        let body = reqwest_get(server.url()).await;
        assert_eq!(body, r#"{"ok":true}"#);
    }

    // Minimal dependency-free HTTP/1.1 GET client for tests.
    async fn reqwest_get(url: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = url.trim_start_matches("http://");
        let (host, port) = addr.split_once(':').unwrap();
        let mut s = tokio::net::TcpStream::connect((host, port.parse::<u16>().unwrap()))
            .await
            .unwrap();
        s.write_all(format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --lib http_stub`
Expected: FAIL to compile — `StubServer`/`StubResponse` not defined.

- [ ] **Step 3: Write minimal implementation**

Prepend to `armature-testkit/src/http_stub.rs` (above the tests module):
```rust
//! A hyper-based HTTP stub server for scripting responses in tests.

use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A scripted HTTP response.
#[derive(Clone, Debug)]
pub struct StubResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl StubResponse {
    /// Response with a raw body and no extra headers.
    pub fn new(status: u16, body: impl Into<Bytes>) -> Self {
        Self { status, headers: Vec::new(), body: body.into() }
    }

    /// JSON response (sets `content-type: application/json`).
    pub fn json(status: u16, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into(),
        }
    }
}

/// A running stub HTTP server. The accept loop is aborted on drop.
pub struct StubServer {
    base_url: String,
    handle: JoinHandle<()>,
}

impl StubServer {
    /// Start a server that returns `resp` for every request.
    pub async fn start_single(resp: StubResponse) -> StubServer {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind stub server");
        let addr: SocketAddr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { break };
                let resp = resp.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |_req: Request<Incoming>| {
                        let resp = resp.clone();
                        async move { Ok::<_, Infallible>(build_response(&resp)) }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
        });

        StubServer { base_url, handle }
    }

    /// The server's base URL, e.g. `http://127.0.0.1:PORT`.
    pub fn url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn build_response(resp: &StubResponse) -> Response<Full<Bytes>> {
    let mut builder = Response::builder().status(resp.status);
    for (k, v) in &resp.headers {
        builder = builder.header(k, v);
    }
    builder.body(Full::new(resp.body.clone())).unwrap()
}
```

Update `armature-testkit/src/lib.rs` to:
```rust
//! Deterministic, offline test harnesses for verifying Armature integrations.

pub mod http_stub;

pub use http_stub::{StubResponse, StubServer};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p armature-testkit --lib http_stub`
Expected: PASS (`serves_a_single_scripted_response`).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/http_stub.rs armature-testkit/src/lib.rs
git commit -m "feat(testkit): hyper-based stub server with single scripted response"
```

---

### Task 3: Route matching by method + path

**Files:**
- Modify: `armature-testkit/src/http_stub.rs`
- Modify: `armature-testkit/src/lib.rs` (export `StubServerBuilder`)

**Interfaces:**
- Consumes: `StubResponse`, `StubServer` from Task 2.
- Produces:
  - `pub struct StubServerBuilder` with `fn route(self, method: &str, path: &str, resp: StubResponse) -> Self`, `fn default_response(self, resp: StubResponse) -> Self`, and `async fn start(self) -> StubServer`.
  - `StubServer::builder() -> StubServerBuilder`.
  - Matching is by uppercase method + exact path; unmatched requests return the default response (a `404` with empty body unless overridden).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `http_stub.rs`:
```rust
    #[tokio::test]
    async fn routes_by_method_and_path() {
        let server = StubServer::builder()
            .route("GET", "/health", StubResponse::new(200, "ok"))
            .route("POST", "/token", StubResponse::json(201, r#"{"id":1}"#))
            .default_response(StubResponse::new(404, "missing"))
            .start()
            .await;

        assert_eq!(raw_request(server.url(), "GET", "/health", "").await, "ok");
        assert_eq!(raw_request(server.url(), "POST", "/token", "").await, r#"{"id":1}"#);
        assert_eq!(raw_request(server.url(), "GET", "/nope", "").await, "missing");
    }

    async fn raw_request(url: &str, method: &str, path: &str, body: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = url.trim_start_matches("http://");
        let (host, port) = addr.split_once(':').unwrap();
        let mut s = tokio::net::TcpStream::connect((host, port.parse::<u16>().unwrap()))
            .await
            .unwrap();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default()
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --lib http_stub::tests::routes_by_method_and_path`
Expected: FAIL to compile — `StubServer::builder` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `http_stub.rs`:
```rust
use std::collections::HashMap;
use std::sync::Arc;

/// Builder for a stub server with per-route responses.
pub struct StubServerBuilder {
    routes: HashMap<(String, String), StubResponse>,
    default: StubResponse,
}

impl StubServerBuilder {
    /// Add a response for an exact method + path.
    pub fn route(mut self, method: &str, path: &str, resp: StubResponse) -> Self {
        self.routes.insert((method.to_ascii_uppercase(), path.to_string()), resp);
        self
    }

    /// Set the response returned when no route matches.
    pub fn default_response(mut self, resp: StubResponse) -> Self {
        self.default = resp;
        self
    }

    /// Start the server.
    pub async fn start(self) -> StubServer {
        let routes = Arc::new(self.routes);
        let default = Arc::new(self.default);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind stub server");
        let addr: SocketAddr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { break };
                let routes = routes.clone();
                let default = default.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |req: Request<Incoming>| {
                        let routes = routes.clone();
                        let default = default.clone();
                        async move {
                            let key = (req.method().as_str().to_ascii_uppercase(), req.uri().path().to_string());
                            let resp = routes.get(&key).cloned().unwrap_or_else(|| (*default).clone());
                            Ok::<_, Infallible>(build_response(&resp))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
        });

        StubServer { base_url, handle }
    }
}
```

Add to `impl StubServer`:
```rust
    /// Start building a multi-route stub server.
    pub fn builder() -> StubServerBuilder {
        StubServerBuilder {
            routes: HashMap::new(),
            default: StubResponse::new(404, Bytes::new()),
        }
    }
```

Add `StubServerBuilder` to the re-export in `lib.rs`:
```rust
pub use http_stub::{StubResponse, StubServer, StubServerBuilder};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p armature-testkit --lib http_stub`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/http_stub.rs armature-testkit/src/lib.rs
git commit -m "feat(testkit): route stub responses by method and path"
```

---

### Task 4: Record received requests and expose assertions

**Files:**
- Modify: `armature-testkit/src/http_stub.rs`
- Modify: `armature-testkit/src/lib.rs` (export `RecordedRequest`)

**Interfaces:**
- Consumes: `StubServer`, `StubServerBuilder` from Task 3.
- Produces:
  - `pub struct RecordedRequest { pub method: String, pub path: String, pub headers: Vec<(String, String)>, pub body: bytes::Bytes }` with `fn header(&self, name: &str) -> Option<&str>` (case-insensitive) and `fn body_string(&self) -> String`.
  - `StubServer::requests(&self) -> Vec<RecordedRequest>` (snapshot of all requests received so far).
  - `StubServer::assert_received(&self, method: &str, path: &str) -> RecordedRequest` (panics with a clear message if none matched).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:
```rust
    #[tokio::test]
    async fn records_requests_for_assertions() {
        let server = StubServer::builder()
            .route("POST", "/introspect", StubResponse::json(200, r#"{"active":true}"#))
            .start()
            .await;

        let _ = raw_request(server.url(), "POST", "/introspect", "token=abc").await;

        let rec = server.assert_received("POST", "/introspect");
        assert_eq!(rec.body_string(), "token=abc");
        assert_eq!(rec.header("content-length"), Some("9"));
        assert_eq!(server.requests().len(), 1);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --lib http_stub::tests::records_requests_for_assertions`
Expected: FAIL to compile — `assert_received`/`requests`/`RecordedRequest` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `http_stub.rs`:
```rust
use std::sync::Mutex;
use http_body_util::BodyExt;

/// A request captured by the stub server.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl RecordedRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The body decoded as UTF-8 (lossy).
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}
```

Change `StubServer` to hold recorded requests:
```rust
pub struct StubServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: JoinHandle<()>,
}
```

Add methods to `impl StubServer`:
```rust
    /// Snapshot of all requests received so far.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Return the first recorded request matching `method` + `path`, or panic.
    pub fn assert_received(&self, method: &str, path: &str) -> RecordedRequest {
        let m = method.to_ascii_uppercase();
        self.requests()
            .into_iter()
            .find(|r| r.method == m && r.path == path)
            .unwrap_or_else(|| panic!("stub server received no {method} {path}; got: {:?}", self.requests()))
    }
```

In `StubServerBuilder::start`, create the shared log and record inside the service. Replace the `start` body's server-spawn with a version that records. The recording block (inside the `service_fn` closure, before matching) is:
```rust
                        async move {
                            let (parts, body) = req.into_parts();
                            let method = parts.method.as_str().to_ascii_uppercase();
                            let path = parts.uri.path().to_string();
                            let headers = parts
                                .headers
                                .iter()
                                .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
                                .collect();
                            let bytes = body.collect().await.map(|c| c.to_bytes()).unwrap_or_default();
                            requests.lock().unwrap().push(RecordedRequest {
                                method: method.clone(),
                                path: path.clone(),
                                headers,
                                body: bytes,
                            });
                            let resp = routes.get(&(method, path)).cloned().unwrap_or_else(|| (*default).clone());
                            Ok::<_, Infallible>(build_response(&resp))
                        }
```
Thread an `Arc<Mutex<Vec<RecordedRequest>>>` named `requests` through the accept loop (clone per connection like `routes`/`default`), store it on the returned `StubServer`, and also update `StubServer::start_single` (Task 2) to construct the `requests` field — simplest is to make `start_single` delegate: `Self::builder().default_response(resp).start().await`.

Also update `start_single` to:
```rust
    pub async fn start_single(resp: StubResponse) -> StubServer {
        Self::builder().default_response(resp).start().await
    }
```
(Remove the old hand-rolled `start_single` body; it no longer records.)

Update `lib.rs` re-export:
```rust
pub use http_stub::{RecordedRequest, StubResponse, StubServer, StubServerBuilder};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p armature-testkit --lib http_stub`
Expected: PASS (all three tests).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/http_stub.rs armature-testkit/src/lib.rs
git commit -m "feat(testkit): record requests and expose assert_received"
```

---

### Task 5: Clean async shutdown on drop

**Files:**
- Modify: `armature-testkit/src/http_stub.rs`

**Interfaces:**
- Consumes: `StubServer` from Task 4.
- Produces: no new API. Guarantees the accept loop task is aborted when `StubServer` is dropped, so the bound port is released and no task leaks.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:
```rust
    #[tokio::test]
    async fn port_is_released_after_drop() {
        let addr = {
            let server = StubServer::start_single(StubResponse::new(200, "x")).await;
            server.url().trim_start_matches("http://").to_string()
        }; // server dropped here

        // Give the aborted accept loop a moment to release the socket.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // We can now bind the same port (proves the listener was released).
        let bound = tokio::net::TcpListener::bind(addr.parse::<std::net::SocketAddr>().unwrap()).await;
        assert!(bound.is_ok(), "port was not released after StubServer drop");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --lib http_stub::tests::port_is_released_after_drop`
Expected: FAIL — port still bound (the `Drop` impl aborts the task but the listener may linger if the loop holds it; if it already passes because Task 2's `Drop` aborts cleanly, keep the test as a regression guard and proceed).

- [ ] **Step 3: Confirm/strengthen the implementation**

The `Drop for StubServer { self.handle.abort(); }` from Task 2 already aborts the accept loop, which drops the `TcpListener`. Ensure the accept loop owns the `TcpListener` by value (it does) so aborting the task frees it. No code change needed beyond confirming the `Drop` impl is present; if the test flakes on timing, raise the sleep to 100ms.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p armature-testkit --lib http_stub`
Expected: PASS (all tests).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/http_stub.rs
git commit -m "test(testkit): assert stub server releases its port on drop"
```

---

### Task 6: Docker gating — `containers` feature and `skip_if_no_docker!`

**Files:**
- Create: `armature-testkit/src/docker.rs`
- Modify: `armature-testkit/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn docker_available() -> bool` — true iff a Docker daemon is reachable (runs `docker info` and checks exit status; false on any error).
  - `#[macro_export] macro_rules! skip_if_no_docker` — expands to: if `!armature_testkit::docker_available()`, `eprintln!` a skip notice and `return;` from the calling test.
  - Both compile unconditionally; the container helpers in later tasks are `#[cfg(feature = "containers")]`.

- [ ] **Step 1: Write the failing test**

Create `armature-testkit/src/docker.rs`:
```rust
//! Docker availability detection and a self-skip macro for container tests.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_available_is_a_bool_and_never_panics() {
        // Must not panic whether or not Docker is installed.
        let _ = docker_available();
    }

    #[test]
    fn skip_macro_runs_without_panicking() {
        // If Docker is absent this returns early; if present it falls through.
        crate::skip_if_no_docker!();
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --lib docker`
Expected: FAIL to compile — `docker_available`/`skip_if_no_docker!` not defined.

- [ ] **Step 3: Write minimal implementation**

Prepend to `armature-testkit/src/docker.rs`:
```rust
/// Returns true if a Docker daemon is reachable (`docker info` succeeds).
pub fn docker_available() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return early from the calling test (with a skip notice) when Docker is
/// unavailable. Use at the top of a `#[cfg(feature = "containers")]` test.
#[macro_export]
macro_rules! skip_if_no_docker {
    () => {
        if !$crate::docker_available() {
            eprintln!("skipping: Docker not available");
            return;
        }
    };
}
```

Add `pub mod docker;` and `pub use docker::docker_available;` to `lib.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p armature-testkit --lib docker`
Expected: PASS (both tests; the skip test returns early or falls through depending on the environment).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/docker.rs armature-testkit/src/lib.rs
git commit -m "feat(testkit): docker detection and skip_if_no_docker! macro"
```

---

### Task 7: Redis testcontainer helper (gated)

**Files:**
- Modify: `armature-testkit/Cargo.toml` (add optional `testcontainers` + `testcontainers-modules` under the `containers` feature)
- Create: `armature-testkit/src/containers.rs`
- Modify: `armature-testkit/src/lib.rs` (`#[cfg(feature = "containers")] pub mod containers;`)

**Interfaces:**
- Consumes: `skip_if_no_docker!` from Task 6.
- Produces (all under `#[cfg(feature = "containers")]`):
  - `pub struct RedisContainer` holding the running container guard, with `async fn start() -> RedisContainer` and `fn url(&self) -> String` (e.g. `redis://127.0.0.1:PORT`). Container stops when dropped (the testcontainers guard handles this).

- [ ] **Step 1: Add the optional dependencies**

Run (resolves current 0.x versions, avoids hardcoding a possibly-stale version):
```bash
cargo add testcontainers --package armature-testkit --optional
cargo add testcontainers-modules --package armature-testkit --optional --features redis,postgres
```
Then edit `armature-testkit/Cargo.toml` so the `containers` feature enables them:
```toml
[features]
default = []
containers = ["dep:testcontainers", "dep:testcontainers-modules"]
```

- [ ] **Step 2: Write the failing (gated, ignored) test**

Create `armature-testkit/src/containers.rs`:
```rust
//! Docker-backed datastore helpers (behind the `containers` feature).

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn redis_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let redis = RedisContainer::start().await;
        let url = redis.url();
        assert!(url.starts_with("redis://"), "unexpected url: {url}");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p armature-testkit --features containers --lib containers`
Expected: FAIL to compile — `RedisContainer` not defined.

- [ ] **Step 4: Write minimal implementation**

Prepend to `containers.rs` (consult the installed `testcontainers-modules` docs for the exact `Redis` image type and the async runner API — the shape below matches testcontainers 0.x's `AsyncRunner`; adjust import paths to the resolved version):
```rust
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::redis::Redis;

/// A running Redis container. Stops when dropped.
pub struct RedisContainer {
    container: ContainerAsync<Redis>,
}

impl RedisContainer {
    /// Start a Redis container and wait until it is ready.
    pub async fn start() -> RedisContainer {
        let container = Redis::default().start().await.expect("start redis container");
        RedisContainer { container }
    }

    /// A `redis://127.0.0.1:PORT` URL for the mapped port.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(6379)
            .await
            .expect("redis mapped port");
        format!("redis://127.0.0.1:{port}")
    }
}
```
Note: if the resolved testcontainers version makes `get_host_port_ipv4` async (it is in 0.x), keep `url(&self)` async and update the test to `redis.url().await`. Make the test match the final signature.

Add to `lib.rs`:
```rust
#[cfg(feature = "containers")]
pub mod containers;
```

- [ ] **Step 5: Run the gated test (requires Docker)**

Run: `cargo test -p armature-testkit --features containers --lib containers -- --ignored`
Expected: PASS when Docker is available; otherwise the `skip_if_no_docker!` returns early. Also confirm the default build is unaffected: `cargo test -p armature-testkit` (Redis code is behind the feature, so it is not compiled).

- [ ] **Step 6: Commit**

```bash
git add armature-testkit/Cargo.toml armature-testkit/src/containers.rs armature-testkit/src/lib.rs
git commit -m "feat(testkit): Redis testcontainer helper behind containers feature"
```

---

### Task 8: Postgres testcontainer helper (gated)

**Files:**
- Modify: `armature-testkit/src/containers.rs`

**Interfaces:**
- Produces (under `#[cfg(feature = "containers")]`): `pub struct PostgresContainer` with `async fn start() -> PostgresContainer` and `async fn url(&self) -> String` (a `postgres://postgres:postgres@127.0.0.1:PORT/postgres` connection string).

- [ ] **Step 1: Write the failing (gated, ignored) test**

Add to the `tests` module in `containers.rs`:
```rust
    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn postgres_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let pg = PostgresContainer::start().await;
        let url = pg.url().await;
        assert!(url.starts_with("postgres://"), "unexpected url: {url}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --features containers --lib containers::tests::postgres_container_starts_and_reports_url`
Expected: FAIL to compile — `PostgresContainer` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `containers.rs` (adjust import to the resolved `testcontainers-modules::postgres::Postgres` type):
```rust
use testcontainers_modules::postgres::Postgres;

/// A running Postgres container. Stops when dropped.
pub struct PostgresContainer {
    container: ContainerAsync<Postgres>,
}

impl PostgresContainer {
    /// Start a Postgres container (default `postgres`/`postgres` credentials).
    pub async fn start() -> PostgresContainer {
        let container = Postgres::default().start().await.expect("start postgres container");
        PostgresContainer { container }
    }

    /// A `postgres://postgres:postgres@127.0.0.1:PORT/postgres` connection string.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(5432)
            .await
            .expect("postgres mapped port");
        format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres")
    }
}
```

- [ ] **Step 4: Run the gated test (requires Docker)**

Run: `cargo test -p armature-testkit --features containers --lib containers -- --ignored`
Expected: PASS when Docker is available.

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/containers.rs
git commit -m "feat(testkit): Postgres testcontainer helper"
```

---

### Task 9: OpenSearch testcontainer helper (gated)

**Files:**
- Modify: `armature-testkit/src/containers.rs`

**Interfaces:**
- Produces (under `#[cfg(feature = "containers")]`): `pub struct OpenSearchContainer` with `async fn start() -> OpenSearchContainer` and `async fn url(&self) -> String` (an `http://127.0.0.1:PORT` base URL for the REST API on 9200).

- [ ] **Step 1: Write the failing (gated, ignored) test**

Add to the `tests` module:
```rust
    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn opensearch_container_starts_and_reports_url() {
        crate::skip_if_no_docker!();
        let os = OpenSearchContainer::start().await;
        let url = os.url().await;
        assert!(url.starts_with("http://"), "unexpected url: {url}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --features containers --lib containers::tests::opensearch_container_starts_and_reports_url`
Expected: FAIL to compile — `OpenSearchContainer` not defined.

- [ ] **Step 3: Write minimal implementation**

`testcontainers-modules` may not ship an OpenSearch module; use a generic image. Add to `containers.rs`:
```rust
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::{GenericImage, ImageExt};

/// A running single-node OpenSearch container (security disabled). Stops when dropped.
pub struct OpenSearchContainer {
    container: ContainerAsync<GenericImage>,
}

impl OpenSearchContainer {
    /// Start OpenSearch 2.x in single-node mode with the security plugin off.
    pub async fn start() -> OpenSearchContainer {
        let image = GenericImage::new("opensearchproject/opensearch", "2.13.0")
            .with_wait_for(WaitFor::message_on_stdout("Node started"))
            .with_env_var("discovery.type", "single-node")
            .with_env_var("DISABLE_SECURITY_PLUGIN", "true")
            .with_env_var("OPENSEARCH_INITIAL_ADMIN_PASSWORD", "Testkit123!");
        let container = image.start().await.expect("start opensearch container");
        OpenSearchContainer { container }
    }

    /// The REST API base URL for the mapped 9200 port.
    pub async fn url(&self) -> String {
        let port = self
            .container
            .get_host_port_ipv4(9200.tcp())
            .await
            .expect("opensearch mapped port");
        format!("http://127.0.0.1:{port}")
    }
}
```
Note: exact `GenericImage` builder method names (`with_env_var`, `with_wait_for`) follow testcontainers 0.x; confirm against the resolved version and fix if renamed.

- [ ] **Step 4: Run the gated test (requires Docker)**

Run: `cargo test -p armature-testkit --features containers --lib containers -- --ignored`
Expected: PASS when Docker is available (OpenSearch takes ~30-60s to become ready).

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/containers.rs
git commit -m "feat(testkit): OpenSearch testcontainer helper"
```

---

### Task 10: ACME/Pebble test-CA harness (gated)

**Files:**
- Create: `armature-testkit/src/acme.rs`
- Modify: `armature-testkit/src/lib.rs` (`#[cfg(feature = "containers")] pub mod acme;`)

**Interfaces:**
- Consumes: `skip_if_no_docker!` from Task 6, testcontainers from Task 7.
- Produces (under `#[cfg(feature = "containers")]`): `pub struct PebbleCa` with `async fn start() -> PebbleCa`, `async fn directory_url(&self) -> String` (the ACME `dir` endpoint, `https://127.0.0.1:PORT/dir`), and `fn ca_note() -> &'static str` documenting that Pebble uses a test root the client must trust (Pebble serves its root at `/roots/0`; the Workflow 4 ACME client passes a rustls config that accepts it).

- [ ] **Step 1: Write the failing (gated, ignored) test**

Create `armature-testkit/src/acme.rs`:
```rust
//! Pebble ACME test-CA harness (behind the `containers` feature).

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
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p armature-testkit --features containers --lib acme`
Expected: FAIL to compile — `PebbleCa` not defined.

- [ ] **Step 3: Write minimal implementation**

Prepend to `acme.rs`:
```rust
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
        let image = GenericImage::new("letsencrypt/pebble", "latest")
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

    /// Explains Pebble's self-signed CA: the ACME client under test must accept
    /// Pebble's test root (served at `/roots/0`) — Workflow 4 wires a rustls
    /// client config that trusts it.
    pub fn ca_note() -> &'static str {
        "Pebble uses a self-signed test CA; fetch its root from /roots/0 and trust it in the ACME client's TLS config."
    }
}
```
Add to `lib.rs`:
```rust
#[cfg(feature = "containers")]
pub mod acme;
```

- [ ] **Step 4: Run the gated test (requires Docker)**

Run: `cargo test -p armature-testkit --features containers --lib acme -- --ignored`
Expected: PASS when Docker is available.

- [ ] **Step 5: Commit**

```bash
git add armature-testkit/src/acme.rs armature-testkit/src/lib.rs
git commit -m "feat(testkit): Pebble ACME test-CA harness"
```

---

### Task 11: Final gate — fmt, strict clippy, workspace check

**Files:** none (verification only).

- [ ] **Step 1: Format**

Run: `cargo fmt`
Then: `cargo fmt -- --check` → Expected: no diff.

- [ ] **Step 2: Strict clippy (matches the pre-commit hook)**

Run: `cargo clippy -p armature-testkit --all-targets -- -D warnings`
And with the feature: `cargo clippy -p armature-testkit --all-targets --features containers -- -D warnings`
Expected: clean, no warnings.

- [ ] **Step 3: Default workspace build unaffected**

Run: `cargo check --workspace`
Expected: clean (the new crate compiles; container deps are optional and off by default).

- [ ] **Step 4: Commit any fmt fixups**

```bash
git add -A
git commit -m "chore(testkit): fmt and clippy clean" || echo "nothing to commit"
```

---

## Self-Review

**Spec coverage (against the roadmap's Workflow 0 requirements):**
1. HTTP stub server (scripted responses + request recording + assertions) → Tasks 2, 3, 4. ✓
2. Testcontainer helpers for Redis/Postgres/OpenSearch behind a `containers`/gate → Tasks 7, 8, 9 (feature `containers`) + Task 6 (`skip_if_no_docker!`). ✓
3. ACME/Pebble harness → Task 10. ✓
4. LocalStack/Azurite → explicitly deferred to Workflow 5 in the spec; not in this plan. ✓ (intentional)
5. Ergonomics: RAII shutdown (Task 5 for the stub server; testcontainers guards handle container drop) + `skip_if_no_docker!` (Task 6). ✓
6. Verification of testkit itself: stub-server unit tests run in the default suite (Tasks 2-5); container/ACME tests are `#[ignore]`d/gated (Tasks 7-10). ✓
7. Boundaries: no `armature-*` dependency; `publish = false`; added to members → Task 1 + manifest. ✓

**Placeholder scan:** No "TBD"/"handle edge cases"/"similar to". Each code step shows complete code. The container-image API notes ("confirm against the resolved version") are explicit adjustment instructions, not placeholders, because exact testcontainers 0.x method names can shift between minor versions.

**Type consistency:** `StubResponse`, `StubServer`, `StubServerBuilder`, `RecordedRequest`, `docker_available`, `skip_if_no_docker!`, `RedisContainer`/`PostgresContainer`/`OpenSearchContainer`/`PebbleCa` are named consistently across tasks and re-exports. `url(&self)` is async for containers (noted) and sync for the stub server (in-process, known at start).
