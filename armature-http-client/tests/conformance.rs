//! Regression tests for conformance findings against a real HTTP stub server
//! (wiremock). These exercise `HttpClient` end-to-end so that gaps between
//! documented behavior and actual behavior (empty retried bodies, a circuit
//! breaker that never opens on status failures, unwired interceptors, etc.)
//! are caught by an actual network round trip rather than unit-level mocks.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use armature_http_client::{
    CircuitBreakerConfig, HttpClient, HttpClientConfig, HttpClientError, Interceptor, Response,
    Result as HttpResult, RetryConfig,
};
use async_trait::async_trait;
use reqwest::Request;
use wiremock::{Mock, MockServer, Request as WmRequest, Respond, ResponseTemplate, matchers};

/// Responder that fails with 500 for the first `fail_times` requests it
/// sees, then returns 200. Records every request body it observes (via a
/// shared handle the test keeps) so tests can assert on what was actually
/// sent over the wire.
struct FailThenSucceed {
    fail_times: u32,
    seen: AtomicU32,
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FailThenSucceed {
    /// Returns the responder (to hand to wiremock) and a shared handle to
    /// the recorded request bodies (for the test to inspect afterwards).
    fn new(fail_times: u32) -> (Self, Arc<Mutex<Vec<Vec<u8>>>>) {
        let bodies = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                fail_times,
                seen: AtomicU32::new(0),
                bodies: bodies.clone(),
            },
            bodies,
        )
    }
}

impl Respond for FailThenSucceed {
    fn respond(&self, request: &WmRequest) -> ResponseTemplate {
        self.bodies.lock().unwrap().push(request.body.clone());
        let n = self.seen.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_times {
            ResponseTemplate::new(500)
        } else {
            ResponseTemplate::new(200).set_body_string("ok")
        }
    }
}

/// Retried requests must carry the original body (and the
/// original per-request timeout is likewise preserved, exercised indirectly
/// since a lost timeout would surface as spurious client-side timeouts).
#[tokio::test]
async fn retried_request_carries_original_body() {
    let server = MockServer::start().await;
    let (responder, bodies) = FailThenSucceed::new(2); // fail twice, succeed on 3rd

    Mock::given(matchers::method("POST"))
        .respond_with(responder)
        .expect(3)
        .mount(&server)
        .await;

    let config = HttpClientConfig::builder()
        .retry(RetryConfig::immediate(5))
        .build();
    let client = HttpClient::new(config);

    #[derive(serde::Serialize)]
    struct Payload {
        item: &'static str,
        quantity: u32,
    }

    let response = client
        .post(server.uri())
        .json(&Payload {
            item: "widget",
            quantity: 5,
        })
        .expect("serialize")
        .send()
        .await
        .expect("request should eventually succeed");

    assert_eq!(response.status(), 200);

    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(
        bodies.len(),
        3,
        "expected exactly 3 attempts (2 failures + success)"
    );
    let expected =
        serde_json::to_vec(&serde_json::json!({"item": "widget", "quantity": 5})).unwrap();
    for (i, body) in bodies.iter().enumerate() {
        assert_eq!(
            body,
            &expected,
            "attempt {i} was sent with the wrong body (found {:?})",
            String::from_utf8_lossy(body)
        );
        assert!(!body.is_empty(), "attempt {i} had an empty body");
    }
}

/// The circuit breaker must open on a run of failing HTTP status
/// responses (5xx), not just transport-level errors.
#[tokio::test]
async fn circuit_breaker_opens_on_repeated_5xx_status() {
    let server = MockServer::start().await;

    Mock::given(matchers::method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let config = HttpClientConfig::builder()
        .circuit_breaker(CircuitBreakerConfig::new(3, Duration::from_secs(30)))
        .build();
    let client = HttpClient::new(config);

    // First 3 requests should go through (and fail with a 503 response, not
    // an error - the client does not treat a plain 503 as Err without retry
    // configured).
    for i in 0..3 {
        let resp = client
            .get(server.uri())
            .send()
            .await
            .unwrap_or_else(|e| panic!("request {i} should not error: {e}"));
        assert_eq!(resp.status(), 503);
    }

    // The breaker should now be open: the next call must short-circuit with
    // CircuitOpen rather than hitting the network.
    let result = client.get(server.uri()).send().await;
    match result {
        Err(HttpClientError::CircuitOpen) => {}
        other => panic!("expected CircuitOpen after 3 failing statuses, got {other:?}"),
    }
}

/// Registered interceptors must actually run around execute().
#[derive(Default)]
struct CountingInterceptor {
    request_count: Arc<AtomicU32>,
    response_count: Arc<AtomicU32>,
}

#[async_trait]
impl Interceptor for CountingInterceptor {
    async fn intercept_request(&self, request: Request) -> HttpResult<Request> {
        self.request_count.fetch_add(1, Ordering::SeqCst);
        Ok(request)
    }

    async fn intercept_response(&self, response: Response) -> HttpResult<Response> {
        self.response_count.fetch_add(1, Ordering::SeqCst);
        Ok(response)
    }
}

#[tokio::test]
async fn registered_interceptor_runs_on_execute() {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let request_count = Arc::new(AtomicU32::new(0));
    let response_count = Arc::new(AtomicU32::new(0));

    let interceptor = CountingInterceptor {
        request_count: request_count.clone(),
        response_count: response_count.clone(),
    };

    let client = HttpClient::new(HttpClientConfig::default()).with_interceptor(interceptor);

    let response = client.get(server.uri()).send().await.unwrap();
    assert_eq!(response.status(), 200);

    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "request interceptor did not run"
    );
    assert_eq!(
        response_count.load(Ordering::SeqCst),
        1,
        "response interceptor did not run"
    );
}

/// `RateLimitInterceptor` must actually delay for (approximately)
/// the parsed `Retry-After` duration instead of only logging.
#[tokio::test]
async fn rate_limit_interceptor_actually_delays() {
    use armature_http_client::RateLimitInterceptor;

    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .mount(&server)
        .await;

    let client = HttpClient::new(HttpClientConfig::default())
        .with_response_interceptor(RateLimitInterceptor::new());

    let start = std::time::Instant::now();
    let response = client.get(server.uri()).send().await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(response.status(), 429);
    assert!(
        elapsed >= Duration::from_millis(900),
        "expected the interceptor to delay ~1s for Retry-After, only waited {elapsed:?}"
    );
}

/// A body-read failure (connection closed mid-body) must surface
/// as an error, not a silently empty 2xx response.
#[tokio::test]
async fn body_read_failure_is_not_a_silent_empty_success() {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            // Send headers that promise a much longer body than we actually
            // deliver, then close the connection - simulating a network
            // error mid-body.
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nshort";
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
            // Drop the socket, forcing a connection reset while the client
            // is still expecting more body bytes.
        }
    });

    let client = HttpClient::new(HttpClientConfig::default());
    let result = client.get(format!("http://{addr}/")).send().await;

    assert!(
        result.is_err(),
        "expected a body-read failure to surface as an error, got {result:?}"
    );
}

/// `RequestBuilder::json`/`form` must surface serialization
/// failures to the caller instead of silently sending a bodyless request.
#[tokio::test]
async fn json_serialization_failure_is_surfaced() {
    use serde::Serialize;

    // A type whose Serialize impl always errors.
    struct AlwaysFails;
    impl Serialize for AlwaysFails {
        fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom(
                "intentional serialization failure",
            ))
        }
    }

    let client = HttpClient::new(HttpClientConfig::default());
    let result = client
        .post("http://127.0.0.1:1/unreachable")
        .json(&AlwaysFails);

    assert!(
        result.is_err(),
        "expected .json() to return an error on serialization failure"
    );
}

/// Responder that fails with 500 for the first `fail_times` requests it
/// sees, then returns 200, recording every request body it observes (like
/// `FailThenSucceed`, but not tied to a specific fail count at construction
/// via a closure-friendly shape - kept separate since its record includes
/// no body-content-specific behavior of its own).
struct RecordBodyFailThenSucceed {
    fail_times: u32,
    seen: AtomicU32,
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Respond for RecordBodyFailThenSucceed {
    fn respond(&self, request: &WmRequest) -> ResponseTemplate {
        self.bodies.lock().unwrap().push(request.body.clone());
        let n = self.seen.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_times {
            ResponseTemplate::new(500)
        } else {
            ResponseTemplate::new(200)
        }
    }
}

/// An interceptor that rewrites the request body.
struct BodyRewritingInterceptor;

#[async_trait]
impl Interceptor for BodyRewritingInterceptor {
    async fn intercept_request(&self, mut request: Request) -> HttpResult<Request> {
        *request.body_mut() = Some(reqwest::Body::from(b"rewritten".to_vec()));
        Ok(request)
    }
}

/// An interceptor-mutated body must actually be sent, not silently
/// discarded when the interceptor-processed request is folded back into the
/// `RequestSpec` used for the real send - and this must hold for *every*
/// retry attempt, not just the first (a naive fix that only reuses the
/// already-mutated request object for attempt 1 but never copies the
/// mutation into `RequestSpec.body` would still lose it on attempt 2+,
/// since retries rebuild the request from the spec). This test forces a
/// retry (first attempt 500, second attempt 200) and asserts both attempts
/// carried the rewritten body.
#[tokio::test]
async fn interceptor_body_mutation_is_sent_on_every_retry_attempt() {
    let server = MockServer::start().await;
    let bodies = Arc::new(Mutex::new(Vec::new()));

    Mock::given(matchers::method("POST"))
        .respond_with(RecordBodyFailThenSucceed {
            fail_times: 1,
            seen: AtomicU32::new(0),
            bodies: bodies.clone(),
        })
        .expect(2)
        .mount(&server)
        .await;

    let config = HttpClientConfig::builder()
        .retry(RetryConfig::immediate(3))
        .build();
    let client = HttpClient::new(config).with_interceptor(BodyRewritingInterceptor);

    let response = client
        .post(server.uri())
        .body(b"original".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2, "expected exactly 2 attempts");
    for (i, body) in bodies.iter().enumerate() {
        assert_eq!(
            body,
            &b"rewritten".to_vec(),
            "attempt {i} did not carry the interceptor's rewritten body; got {:?}",
            String::from_utf8_lossy(body)
        );
    }
}

#[tokio::test]
async fn form_encoding_failure_is_surfaced() {
    // f64::NAN is not valid in url-encoded form output via serde_urlencoded
    // for a struct with a float field is actually accepted as "NaN"; use a
    // map with a non-serializable key type instead to force a real error.
    use std::collections::BTreeMap;

    #[derive(serde::Serialize)]
    struct BadForm {
        #[serde(serialize_with = "fail_serialize")]
        _field: (),
    }

    fn fail_serialize<S: serde::Serializer>(_: &(), _s: S) -> std::result::Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("intentional form failure"))
    }

    let client = HttpClient::new(HttpClientConfig::default());
    let result = client
        .post("http://127.0.0.1:1/unreachable")
        .form(&BadForm { _field: () });

    assert!(
        result.is_err(),
        "expected .form() to return an error on encoding failure"
    );

    // Sanity: a well-formed map still succeeds.
    let mut ok_form = BTreeMap::new();
    ok_form.insert("a", "b");
    let ok = client.post("http://127.0.0.1:1/unreachable").form(&ok_form);
    assert!(ok.is_ok());
}
