//! Regression tests for GraphQL subscription conformance:
//!
//! - Headers set via `SubscriptionBuilder::header` (and default client
//!   headers) must actually reach the server on the WebSocket handshake
//!   request instead of being silently dropped.
//! - A server `Ping` frame must be answered with a `Pong` frame.
//! - The client must offer the `graphql-transport-ws` sub-protocol on the
//!   handshake. Every stub server below rejects an upgrade that lacks it, the
//!   same way Armature's own GraphQL server (and Apollo, and `graphql-ws`) do,
//!   so a client that stopped sending the header would fail these tests rather
//!   than pass against a permissive stub.

use std::sync::{Arc, Mutex};

use armature_graphql_client::{GraphQLClient, GraphQLClientConfig};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::Request as HandshakeRequest;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse as HandshakeErrorResponse, Response as HandshakeResponse,
};

/// The sub-protocol the client is required to offer and the server echoes back.
const GRAPHQL_TRANSPORT_WS: &str = "graphql-transport-ws";

/// Whether the handshake request offers [`GRAPHQL_TRANSPORT_WS`].
fn offers_graphql_transport_ws(req: &HandshakeRequest) -> bool {
    req.headers()
        .get_all("Sec-WebSocket-Protocol")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|candidate| candidate.trim() == GRAPHQL_TRANSPORT_WS)
}

/// Fail the handshake with `400` when the sub-protocol is missing, otherwise
/// echo it back as RFC 6455 requires of a server that accepts one.
///
/// `tungstenite`'s client fails the connection if it offered a sub-protocol and
/// the server answers without one, so echoing is not optional here.
// The `Result<Response, ErrorResponse>` return shape is dictated by
// tungstenite's `Callback` trait, not chosen here.
#[allow(clippy::result_large_err)]
fn negotiate_sub_protocol(
    mut resp: HandshakeResponse,
    offered: bool,
) -> Result<HandshakeResponse, HandshakeErrorResponse> {
    if !offered {
        let mut rejection = HandshakeErrorResponse::new(Some(format!(
            "handshake must offer the {GRAPHQL_TRANSPORT_WS} sub-protocol"
        )));
        *rejection.status_mut() = http::StatusCode::BAD_REQUEST;
        return Err(rejection);
    }

    resp.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        http::HeaderValue::from_static(GRAPHQL_TRANSPORT_WS),
    );
    Ok(resp)
}

/// Spawn a minimal graphql-ws test server on an ephemeral port.
///
/// `on_headers` is invoked with the handshake request headers as
/// `(name, value)` pairs. After the handshake, the server performs the
/// `connection_init` / `connection_ack` exchange. If `send_ping` is true, it
/// sends a `ping` frame right after the ack and records whether it receives
/// a `pong` frame back into `pong_received`.
async fn spawn_test_server(
    headers_out: Arc<Mutex<Vec<(String, String)>>>,
    send_ping: bool,
    pong_received: Arc<Mutex<bool>>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let headers_out = headers_out.clone();
            // The `Result<Response, ErrorResponse>` return shape is dictated by
            // tungstenite's `Callback` trait, not chosen here — there's nothing
            // to box or shrink on our side.
            #[allow(clippy::result_large_err)]
            let callback = move |req: &HandshakeRequest, resp: HandshakeResponse| {
                let mut captured = headers_out.lock().unwrap();
                for (name, value) in req.headers() {
                    captured.push((
                        name.as_str().to_string(),
                        value.to_str().unwrap_or_default().to_string(),
                    ));
                }
                drop(captured);
                negotiate_sub_protocol(resp, offers_graphql_transport_ws(req))
            };

            let ws_stream = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
                Ok(s) => s,
                Err(_) => return,
            };

            let (mut write, mut read) = ws_stream.split();

            // Expect connection_init, reply connection_ack.
            if let Some(Ok(Message::Text(_))) = read.next().await {
                let ack = serde_json::json!({"type": "connection_ack"}).to_string();
                let _ = write.send(Message::Text(ack.into())).await;
            } else {
                return;
            }

            if send_ping {
                // Give the client a moment to send its `subscribe` message
                // before pinging (not required by protocol, just tidy).
                let _ = read.next().await;

                let ping = serde_json::json!({"type": "ping"}).to_string();
                let _ = write.send(Message::Text(ping.into())).await;

                // Expect a pong in response.
                if let Some(Ok(Message::Text(text))) = read.next().await
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                    && value.get("type").and_then(|t| t.as_str()) == Some("pong")
                {
                    *pong_received.lock().unwrap() = true;
                }

                let complete = serde_json::json!({"type": "complete", "id": "1"}).to_string();
                let _ = write.send(Message::Text(complete.into())).await;
            }
        }
    });

    format!("ws://{addr}")
}

/// Spawn a server that completes the WebSocket handshake but never sends
/// `connection_ack` — it closes right after reading `connection_init`.
async fn spawn_no_ack_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            #[allow(clippy::result_large_err)]
            let callback = |req: &HandshakeRequest, resp: HandshakeResponse| {
                negotiate_sub_protocol(resp, offers_graphql_transport_ws(req))
            };
            let ws_stream = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
                Ok(s) => s,
                Err(_) => return,
            };
            let (mut write, mut read) = ws_stream.split();

            let _ = read.next().await; // connection_init
            let _ = write.send(Message::Close(None)).await;
        }
    });

    format!("ws://{addr}")
}

/// Spawn a minimal graphql-ws test server that performs the
/// `connection_init`/`connection_ack` exchange, reads the client's `subscribe`
/// message, and then records whether the *next* client message is a
/// client-initiated `complete` (an unsubscribe). It never sends its own
/// `complete`, so any `complete` observed must have come from the client.
async fn spawn_unsubscribe_server(complete_id: Arc<Mutex<Option<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            #[allow(clippy::result_large_err)]
            let callback = |req: &HandshakeRequest, resp: HandshakeResponse| {
                negotiate_sub_protocol(resp, offers_graphql_transport_ws(req))
            };
            let ws_stream = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
                Ok(s) => s,
                Err(_) => return,
            };
            let (mut write, mut read) = ws_stream.split();

            // connection_init -> connection_ack
            if let Some(Ok(Message::Text(_))) = read.next().await {
                let ack = serde_json::json!({"type": "connection_ack"}).to_string();
                let _ = write.send(Message::Text(ack.into())).await;
            } else {
                return;
            }

            // Read the `subscribe` message.
            let _ = read.next().await;

            // The next client message should be a client-initiated `complete`.
            while let Some(Ok(msg)) = read.next().await {
                if let Message::Text(text) = msg
                    && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                    && value.get("type").and_then(|t| t.as_str()) == Some("complete")
                {
                    let id = value
                        .get("id")
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string());
                    *complete_id.lock().unwrap() = id;
                    break;
                }
            }
        }
    });

    format!("ws://{addr}")
}

#[tokio::test]
async fn explicit_unsubscribe_sends_complete() {
    let complete_id = Arc::new(Mutex::new(None));
    let ws_url = spawn_unsubscribe_server(complete_id.clone()).await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .build();
    let client = GraphQLClient::with_config(config);

    let mut subscription = client
        .subscribe("subscription { messageAdded { id } }")
        .send()
        .await
        .expect("subscription should connect");

    let sub_id = subscription
        .subscription()
        .map(|s| s.id.clone())
        .expect("a live subscription carries a handle with its id");
    assert!(subscription.is_active());

    subscription.unsubscribe();
    assert!(
        !subscription.is_active(),
        "handle should be inactive after unsubscribe"
    );

    // Keep the stream alive so the write half is not torn down before the
    // spawned `complete`-send task runs.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let received = complete_id.lock().unwrap().clone();
    assert_eq!(
        received.as_deref(),
        Some(sub_id.as_str()),
        "client should have sent a `complete` carrying the subscription id"
    );

    drop(subscription);
}

#[tokio::test]
async fn dropping_stream_sends_complete() {
    let complete_id = Arc::new(Mutex::new(None));
    let ws_url = spawn_unsubscribe_server(complete_id.clone()).await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .build();
    let client = GraphQLClient::with_config(config);

    let subscription = client
        .subscribe("subscription { messageAdded { id } }")
        .send()
        .await
        .expect("subscription should connect");

    let sub_id = subscription
        .subscription()
        .map(|s| s.id.clone())
        .expect("a live subscription carries a handle with its id");

    // Dropping the stream is an implicit client-initiated unsubscribe.
    drop(subscription);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let received = complete_id.lock().unwrap().clone();
    assert_eq!(
        received.as_deref(),
        Some(sub_id.as_str()),
        "dropping the stream should send a `complete` carrying the subscription id"
    );
}

#[tokio::test]
async fn subscription_header_reaches_server() {
    let captured_headers = Arc::new(Mutex::new(Vec::new()));
    let pong_received = Arc::new(Mutex::new(false));
    let ws_url = spawn_test_server(captured_headers.clone(), false, pong_received).await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .build();
    let client = GraphQLClient::with_config(config);

    let _subscription = client
        .subscribe("subscription { messageAdded { id } }")
        .header("Authorization", "Bearer test-token")
        .send()
        .await
        .expect("subscription should connect");

    // Give the server task a moment to finish recording headers.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let headers = captured_headers.lock().unwrap();
    let has_auth_header = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer test-token"
    });

    assert!(
        has_auth_header,
        "expected Authorization header to reach the WebSocket handshake request, got: {headers:?}"
    );
}

#[tokio::test]
async fn subscription_offers_graphql_transport_ws_sub_protocol() {
    let captured_headers = Arc::new(Mutex::new(Vec::new()));
    let pong_received = Arc::new(Mutex::new(false));
    let ws_url = spawn_test_server(captured_headers.clone(), false, pong_received).await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .build();
    let client = GraphQLClient::with_config(config);

    let _subscription = client
        .subscribe("subscription { messageAdded { id } }")
        .send()
        .await
        .expect("subscription should connect");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let headers = captured_headers.lock().unwrap();
    let offered = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("sec-websocket-protocol")
            && value
                .split(',')
                .any(|candidate| candidate.trim() == GRAPHQL_TRANSPORT_WS)
    });

    assert!(
        offered,
        "expected the client to offer the {GRAPHQL_TRANSPORT_WS} sub-protocol on the handshake, got: {headers:?}"
    );
}

#[tokio::test]
async fn missing_connection_ack_is_an_error() {
    let ws_url = spawn_no_ack_server().await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .build();
    let client = GraphQLClient::with_config(config);

    // A server that closes without acknowledging must not yield a subscription
    // that silently never produces events.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client
            .subscribe("subscription { messageAdded { id } }")
            .send(),
    )
    .await
    .expect("connecting should not hang when the server never acks");

    assert!(
        result.is_err(),
        "expected an error when the server closes without sending connection_ack"
    );
}

#[tokio::test]
async fn subscription_default_header_reaches_server() {
    let captured_headers = Arc::new(Mutex::new(Vec::new()));
    let pong_received = Arc::new(Mutex::new(false));
    let ws_url = spawn_test_server(captured_headers.clone(), false, pong_received).await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .header("Authorization", "Bearer default-token")
        .build();
    let client = GraphQLClient::with_config(config);

    // No per-call `.header(...)` here — only the client's `default_headers`
    // should be responsible for getting the header to the server.
    let _subscription = client
        .subscribe("subscription { messageAdded { id } }")
        .send()
        .await
        .expect("subscription should connect");

    // Give the server task a moment to finish recording headers.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let headers = captured_headers.lock().unwrap();
    let has_auth_header = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("authorization") && value == "Bearer default-token"
    });

    assert!(
        has_auth_header,
        "expected default_headers Authorization header to reach the WebSocket handshake request, got: {headers:?}"
    );
}

#[tokio::test]
async fn subscription_responds_to_ping_with_pong() {
    let captured_headers = Arc::new(Mutex::new(Vec::new()));
    let pong_received = Arc::new(Mutex::new(false));
    let ws_url = spawn_test_server(captured_headers, true, pong_received.clone()).await;

    let config = GraphQLClientConfig::builder()
        .endpoint("http://127.0.0.1:1/graphql")
        .ws_endpoint(ws_url)
        .build();
    let client = GraphQLClient::with_config(config);

    let mut subscription = client
        .subscribe("subscription { messageAdded { id } }")
        .send()
        .await
        .expect("subscription should connect");

    // Drive the stream until the server's `complete` message ends it; this
    // is what causes the client to observe (and respond to) the `ping`. Bound
    // this with a timeout: a client that never sends a pong will cause the
    // test server to block forever waiting for one, so a hang here is itself
    // evidence of the regression rather than a flaky test.
    let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while subscription.next().await.is_some() {}
    })
    .await;
    assert!(
        drained.is_ok(),
        "subscription stream never completed — client likely never answered the server's ping"
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(
        *pong_received.lock().unwrap(),
        "expected client to respond to server ping with a pong frame"
    );
}
