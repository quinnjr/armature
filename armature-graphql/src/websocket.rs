//! GraphQL subscriptions over WebSocket.
//!
//! Implements both the legacy `graphql-ws` (subscriptions-transport-ws) and
//! the current `graphql-transport-ws` sub-protocols by driving
//! [`async_graphql::http::WebSocket`] against a caller-supplied
//! [`tokio_tungstenite::WebSocketStream`].
//!
//! This module is transport-agnostic on purpose: it does not perform the
//! HTTP `Upgrade` handshake or accept a TCP connection itself. The caller
//! (typically an `armature-core` route handler) is responsible for:
//!
//! 1. Negotiating the sub-protocol from the client's `Sec-WebSocket-Protocol`
//!    request header via [`select_protocol`].
//! 2. Completing the WebSocket upgrade and producing a
//!    [`tokio_tungstenite::WebSocketStream`].
//! 3. Calling [`serve_graphql_websocket`] with that stream, the built
//!    [`async_graphql::Schema`], and a [`WebSocketConfig`] describing the
//!    connection's keep-alive timeout, subscription cap, and (optionally) a
//!    `connection_init` authentication hook.
//!
//! Only text frames are treated as GraphQL-over-WebSocket protocol messages;
//! binary frames are ignored, matching the text-only `graphql-ws` /
//! `graphql-transport-ws` specs. Ping/Pong at the WebSocket-frame level are
//! left to `tokio-tungstenite`'s own automatic handling, consistent with
//! [`armature_core::websocket::handle_websocket`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use armature_core::Error as ArmatureError;
use armature_log::warn;
use async_graphql::Executor;
use async_graphql::futures_util::future::BoxFuture;
use async_graphql::futures_util::{SinkExt, StreamExt};
use async_graphql::http::{self, ClientMessage};
use async_graphql::{Data, Result as GqlResult};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsFrame;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;

pub use async_graphql::http::ALL_WEBSOCKET_PROTOCOLS;
pub use async_graphql::http::WebSocketProtocols as Protocols;

/// Hard upper bound on how many operation IDs a single connection may have
/// in flight, applied even when [`WebSocketConfig::max_subscriptions`] is
/// `None`.
///
/// Without it, a `None` cap would let a client grow the per-connection
/// tracking set forever by sending a fresh id each time, turning "no
/// configured cap" into an unbounded memory leak for the life of the
/// connection.
pub const MAX_TRACKED_OPERATIONS: usize = 10_000;

/// Select a `graphql-ws`/`graphql-transport-ws` sub-protocol from the value
/// of an incoming `Sec-WebSocket-Protocol` request header.
///
/// The header may list multiple client-offered protocols separated by
/// commas (per RFC 6455); the first one this server understands is chosen.
/// Returns an error if the header is absent or none of the offered
/// protocols are supported, in which case the caller should reject the
/// upgrade (e.g. with `426 Upgrade Required`) instead of proceeding.
pub fn select_protocol(header_value: Option<&str>) -> Result<Protocols, ArmatureError> {
    let header_value = header_value
        .ok_or_else(|| ArmatureError::BadRequest("Missing Sec-WebSocket-Protocol".to_string()))?;

    header_value
        .split(',')
        .map(str::trim)
        .find_map(|candidate| candidate.parse::<Protocols>().ok())
        .ok_or_else(|| {
            ArmatureError::BadRequest(format!(
                "Unsupported Sec-WebSocket-Protocol: {header_value}"
            ))
        })
}

/// A boxed `connection_init` validation/authentication callback.
///
/// Mirrors the closure shape accepted by
/// [`async_graphql::http::WebSocket::on_connection_init`], boxed so it can be
/// stored in [`WebSocketConfig`] without leaking a generic parameter into
/// [`serve_graphql_websocket`]'s signature.
pub type OnConnectionInit =
    Box<dyn FnOnce(serde_json::Value) -> BoxFuture<'static, GqlResult<Data>> + Send>;

/// Per-connection tuning knobs for [`serve_graphql_websocket`].
///
/// Construct with [`WebSocketConfig::default`] and override individual
/// fields, or use the `with_*` builder methods.
pub struct WebSocketConfig {
    /// Disconnect the client if no message (including a keep-alive ping ack)
    /// is seen within this duration. `None` disables the timeout entirely,
    /// which is not recommended for internet-facing servers since an
    /// unresponsive client with an open TCP connection would never be
    /// dropped. Applies to both sub-protocols.
    pub keepalive_timeout: Option<Duration>,

    /// Reject `subscribe`/`start` messages once this many operations are open
    /// on this connection. `None` means "no configured cap", which is not
    /// recommended for internet-facing servers: a single client could
    /// otherwise open unbounded concurrent subscriptions and exhaust server
    /// memory, CPU, or task capacity. Even with `None`,
    /// [`MAX_TRACKED_OPERATIONS`] still applies as a hard backstop.
    ///
    /// This counts *every* operation the transport delivers, not just
    /// subscriptions: the `graphql-transport-ws` protocol carries queries and
    /// mutations as `subscribe` messages too, so they consume a slot for as
    /// long as they run. Slots are released when the client sends `stop`, and
    /// when the server emits `complete` for the id — which is what returns
    /// the slot for a query or mutation, since those finish on their own
    /// without the client ever sending `stop`.
    ///
    /// When the cap is hit, an over-limit `start` message is silently
    /// dropped: the client simply never receives `next`/`error`/`complete`
    /// for that id. This is a deliberate, non-chatty mitigation, not a
    /// protocol error.
    pub max_subscriptions: Option<usize>,

    /// Called with the client's `connection_init` payload before any
    /// operation is allowed to run; return `Err` to reject the connection
    /// (e.g. invalid or missing auth token). This is the standard place
    /// browser WebSocket clients put auth tokens, since custom headers
    /// aren't available on a WS upgrade in browsers.
    ///
    /// `None` (the default) accepts every `connection_init`, matching
    /// `async_graphql::http::WebSocket`'s own default.
    pub on_connection_init: Option<OnConnectionInit>,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            keepalive_timeout: Some(Duration::from_secs(30)),
            max_subscriptions: Some(100),
            on_connection_init: None,
        }
    }
}

impl std::fmt::Debug for WebSocketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketConfig")
            .field("keepalive_timeout", &self.keepalive_timeout)
            .field("max_subscriptions", &self.max_subscriptions)
            .field(
                "on_connection_init",
                &self.on_connection_init.as_ref().map(|_| "<callback>"),
            )
            .finish()
    }
}

impl WebSocketConfig {
    /// Set [`Self::keepalive_timeout`].
    #[must_use]
    pub fn with_keepalive_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.keepalive_timeout = timeout.into();
        self
    }

    /// Set [`Self::max_subscriptions`].
    #[must_use]
    pub fn with_max_subscriptions(mut self, max: impl Into<Option<usize>>) -> Self {
        self.max_subscriptions = max.into();
        self
    }

    /// Set [`Self::on_connection_init`].
    #[must_use]
    pub fn with_on_connection_init<F, R>(mut self, callback: F) -> Self
    where
        F: FnOnce(serde_json::Value) -> R + Send + 'static,
        R: std::future::Future<Output = GqlResult<Data>> + Send + 'static,
    {
        self.on_connection_init = Some(Box::new(|payload| Box::pin(callback(payload))));
        self
    }
}

/// Lock the per-connection operation set, recovering from a poisoned mutex.
///
/// Nothing under this lock can panic, so poisoning can only be collateral
/// damage from an unrelated panic; dropping the whole connection over it would
/// be a worse outcome than continuing with the (still consistent) set.
fn lock_operations(
    operations: &Mutex<HashSet<String>>,
) -> std::sync::MutexGuard<'_, HashSet<String>> {
    operations.lock().unwrap_or_else(|err| err.into_inner())
}

/// Extract the operation id from an outgoing `complete` message.
///
/// Returns `None` for any other message type or for a payload that does not
/// parse. Both sub-protocols spell the server-side termination message
/// `complete`, so this covers `graphql-ws` and `graphql-transport-ws` alike.
/// A `None` from a parse failure is safe: the id simply stays tracked until
/// the client's `stop` or the end of the connection.
fn completed_operation_id(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("type")?.as_str()? != "complete" {
        return None;
    }
    Some(value.get("id")?.as_str()?.to_string())
}

/// Drive a GraphQL-over-WebSocket session to completion on an already
/// upgraded connection.
///
/// Runs until the client disconnects, sends `ConnectionTerminate`, the
/// `connection_init` callback in `config` rejects the connection, the
/// configured [`WebSocketConfig::keepalive_timeout`] fires because the
/// client went silent, or a transport-level read/write error occurs. All
/// queries, mutations, and subscriptions sent over the connection are
/// executed against `executor` (typically an [`async_graphql::Schema`]);
/// subscription result streams are pushed back to the client as they yield
/// items, subject to `config`'s [`WebSocketConfig::max_subscriptions`] cap.
///
/// This always returns `Ok(())` on a clean protocol-level shutdown (client
/// disconnect, `ConnectionTerminate`, keep-alive timeout, or a rejected
/// `connection_init`); those are normal end-of-session outcomes reported to
/// the client over the WebSocket itself, not out-of-band errors. Transport
/// read errors and send failures are logged via `armature_log::warn!` and
/// end the loop the same way a clean disconnect would, since by that point
/// there is no live connection left to report an `Err` to.
pub async fn serve_graphql_websocket<S, E>(
    ws_stream: WebSocketStream<S>,
    executor: E,
    protocol: Protocols,
    config: WebSocketConfig,
) -> Result<(), ArmatureError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    E: Executor,
{
    let (mut sink, stream) = ws_stream.split();

    // An unconfigured cap still gets the hard backstop, so the tracking set is
    // bounded on every configuration.
    let cap = config.max_subscriptions.unwrap_or(MAX_TRACKED_OPERATIONS);

    // Shared because both halves of the session mutate it: the inbound filter
    // claims a slot on `start` and releases it on `stop`, while the outbound
    // loop releases it when the server emits `complete` for an id.
    let open_operations: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    let inbound_operations = Arc::clone(&open_operations);
    let input = stream.filter_map(move |frame| {
        let result = match frame {
            Ok(WsFrame::Text(text)) => match ClientMessage::from_bytes(text.as_bytes()) {
                Ok(ClientMessage::Start { id, payload }) => {
                    let mut open = lock_operations(&inbound_operations);
                    if !open.contains(&id) && open.len() >= cap {
                        drop(open);
                        warn!(
                            "graphql-ws: dropping subscribe id={id} - max_subscriptions cap reached"
                        );
                        None
                    } else {
                        open.insert(id.clone());
                        drop(open);
                        Some(Ok(ClientMessage::Start { id, payload }))
                    }
                }
                Ok(ClientMessage::Stop { id }) => {
                    lock_operations(&inbound_operations).remove(&id);
                    Some(Ok(ClientMessage::Stop { id }))
                }
                Ok(other) => Some(Ok(other)),
                Err(err) => Some(Err(err)),
            },
            // Binary/Ping/Pong/Close frames carry no graphql-ws protocol
            // message; skip them and keep reading. tokio-tungstenite
            // answers protocol-level Pings automatically, and a Close
            // frame ends the underlying stream on its own on the next poll.
            Ok(_) => None,
            // A transport-level error also ends the underlying stream on
            // the next poll; log it since there is no graphql-ws message to
            // report it as.
            Err(err) => {
                warn!("graphql-ws: transport read error: {err}");
                None
            }
        };
        async move { result }
    });

    // Always route through `on_connection_init` with a boxed callback (even
    // when the caller supplied none) so both branches share one concrete
    // `WebSocket<..>` type; a `None` config falls back to async-graphql's
    // own default (always-accept) initializer.
    let on_connection_init: OnConnectionInit = config.on_connection_init.unwrap_or_else(|| {
        Box::new(|payload| Box::pin(async move { http::default_on_connection_init(payload).await }))
    });

    let mut gql_ws = Box::pin(
        http::WebSocket::from_message_stream(executor, input, protocol)
            .keepalive_timeout(config.keepalive_timeout)
            .on_connection_init(on_connection_init),
    );

    while let Some(message) = gql_ws.next().await {
        let (frame, is_close) = match message {
            http::WsMessage::Text(text) => {
                // A server-side `complete` is the only signal that a query or
                // mutation (which the transport also delivers as `subscribe`)
                // has finished; without releasing the slot here, N short-lived
                // queries would permanently exhaust a cap of N.
                //
                // The substring check is a prefilter: parsing every outgoing
                // `next` payload would tax high-throughput subscriptions.
                if text.contains("complete")
                    && let Some(id) = completed_operation_id(&text)
                {
                    lock_operations(&open_operations).remove(&id);
                }
                (WsFrame::Text(text.into()), false)
            }
            http::WsMessage::Close(code, reason) => (
                WsFrame::Close(Some(CloseFrame {
                    code: code.into(),
                    reason: reason.into(),
                })),
                true,
            ),
        };

        if let Err(err) = sink.send(frame).await {
            warn!("graphql-ws: failed to send frame to client: {err}");
            break;
        }
        if is_close {
            break;
        }
    }

    let _ = sink.close().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::{EmptyMutation, Object, Schema, Subscription};

    struct Query;

    #[Object]
    impl Query {
        async fn hello(&self) -> &str {
            "world"
        }
    }

    struct SubscriptionRoot;

    #[Subscription]
    impl SubscriptionRoot {
        async fn counter(&self) -> impl async_graphql::futures_util::Stream<Item = i32> {
            async_graphql::futures_util::stream::iter(0..3)
        }

        /// Never yields and never completes, so a test can hold a slot open
        /// for the whole connection without racing against its own `complete`.
        async fn forever(&self) -> impl async_graphql::futures_util::Stream<Item = i32> {
            async_graphql::futures_util::stream::pending::<i32>()
        }
    }

    type TestSchema = Schema<Query, EmptyMutation, SubscriptionRoot>;

    fn test_schema() -> TestSchema {
        Schema::new(Query, EmptyMutation, SubscriptionRoot)
    }

    #[test]
    fn select_protocol_picks_first_supported_offer() {
        assert_eq!(
            select_protocol(Some("graphql-transport-ws")).unwrap(),
            Protocols::GraphQLWS
        );
        assert_eq!(
            select_protocol(Some("graphql-ws")).unwrap(),
            Protocols::SubscriptionsTransportWS
        );
        assert_eq!(
            select_protocol(Some("bogus, graphql-transport-ws")).unwrap(),
            Protocols::GraphQLWS
        );
    }

    #[test]
    fn select_protocol_rejects_missing_or_unsupported_header() {
        assert!(select_protocol(None).is_err());
        assert!(select_protocol(Some("bogus-protocol")).is_err());
    }

    async fn websocket_pair() -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let server = WebSocketStream::from_raw_socket(
            server_io,
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        let client = WebSocketStream::from_raw_socket(
            client_io,
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        (server, client)
    }

    #[tokio::test]
    async fn executes_a_query_over_the_websocket_transport() {
        let (server, mut client) = websocket_pair().await;

        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            WebSocketConfig::default(),
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_init"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let ack = client.next().await.unwrap().unwrap();
        assert!(ack.into_text().unwrap().contains("connection_ack"));

        client
            .send(WsFrame::Text(
                serde_json::json!({
                    "id": "1",
                    "type": "subscribe",
                    "payload": {"query": "query { hello }"}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let next = client.next().await.unwrap().unwrap();
        let text = next.into_text().unwrap();
        assert!(text.contains("\"hello\":\"world\""));
        assert!(text.contains("\"id\":\"1\""));

        let complete = client.next().await.unwrap().unwrap();
        assert!(complete.into_text().unwrap().contains("complete"));

        drop(client);
        let _ = serve.await;
    }

    #[tokio::test]
    async fn streams_subscription_results_and_completes() {
        let (server, mut client) = websocket_pair().await;

        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            WebSocketConfig::default(),
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_init"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        client.next().await.unwrap().unwrap(); // connection_ack

        client
            .send(WsFrame::Text(
                serde_json::json!({
                    "id": "sub-1",
                    "type": "subscribe",
                    "payload": {"query": "subscription { counter }"}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        for expected in 0..3 {
            let msg = client.next().await.unwrap().unwrap();
            let text = msg.into_text().unwrap();
            assert!(text.contains(&format!("\"counter\":{expected}")));
        }

        let complete = client.next().await.unwrap().unwrap();
        assert!(complete.into_text().unwrap().contains("complete"));

        client.send(WsFrame::Close(None)).await.unwrap();
        drop(client);
        let _ = serve.await;
    }

    #[tokio::test]
    async fn connection_terminate_ends_the_session() {
        let (server, mut client) = websocket_pair().await;

        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            WebSocketConfig::default(),
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_init"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        client.next().await.unwrap().unwrap(); // connection_ack

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_terminate"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();

        // The server ends the stream on its side in response to
        // `connection_terminate`; the client either observes the resulting
        // close frame or the connection simply ending.
        match client.next().await {
            None => {}
            Some(Ok(frame)) => assert!(frame.is_close()),
            Some(Err(_)) => {}
        }

        let result = tokio::time::timeout(Duration::from_secs(5), serve)
            .await
            .expect("serve_graphql_websocket did not return after connection_terminate")
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn keepalive_timeout_closes_idle_connection() {
        let (server, mut client) = websocket_pair().await;

        let config = WebSocketConfig::default().with_keepalive_timeout(Duration::from_millis(100));
        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            config,
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_init"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        client.next().await.unwrap().unwrap(); // connection_ack

        // Stay silent from here on; the keepalive timer should fire and the
        // server should close the connection on its own, rather than hang
        // forever waiting on an unresponsive client.
        let closed = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("keepalive_timeout did not fire")
            .unwrap()
            .unwrap();
        assert!(closed.is_close());

        let result = tokio::time::timeout(Duration::from_secs(5), serve)
            .await
            .expect("serve_graphql_websocket did not return after keepalive timeout")
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn max_subscriptions_cap_drops_over_limit_subscribe() {
        let (server, mut client) = websocket_pair().await;

        let config = WebSocketConfig::default().with_max_subscriptions(1);
        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            config,
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_init"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        client.next().await.unwrap().unwrap(); // connection_ack

        // `forever` never completes, so this slot stays claimed for the rest
        // of the connection and the cap check below cannot race a `complete`.
        client
            .send(WsFrame::Text(
                serde_json::json!({
                    "id": "sub-1",
                    "type": "subscribe",
                    "payload": {"query": "subscription { forever }"}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        // Sent while sub-1 is already open and the cap is 1; this subscribe
        // should be silently dropped and never produce output. Inbound
        // messages are processed in order, so sub-1 has claimed its slot
        // before sub-2 is examined regardless of timing.
        client
            .send(WsFrame::Text(
                serde_json::json!({
                    "id": "sub-2",
                    "type": "subscribe",
                    "payload": {"query": "query { hello }"}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        // Nothing should ever arrive for sub-2, so wait out a window rather
        // than for a terminating frame that will never come.
        let saw_sub2 = tokio::time::timeout(Duration::from_millis(500), async {
            while let Some(Ok(msg)) = client.next().await {
                if let Ok(text) = msg.into_text()
                    && text.contains("\"id\":\"sub-2\"")
                {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);

        assert!(
            !saw_sub2,
            "over-limit subscription should never produce output"
        );

        drop(client);
        // `forever` never ends, so the session would outlive the test waiting
        // on it; nothing above depends on how the task finishes.
        serve.abort();
    }

    #[tokio::test]
    async fn completed_operation_frees_a_subscription_slot() {
        let (server, mut client) = websocket_pair().await;

        // The transport delivers queries as `subscribe` too, so a cap of 1
        // must not be permanently consumed by a single finished query.
        let config = WebSocketConfig::default().with_max_subscriptions(1);
        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            config,
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({"type": "connection_init"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        client.next().await.unwrap().unwrap(); // connection_ack

        for id in ["q-1", "q-2"] {
            client
                .send(WsFrame::Text(
                    serde_json::json!({
                        "id": id,
                        "type": "subscribe",
                        "payload": {"query": "query { hello }"}
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            // Drain this operation's `next` and `complete` before issuing the
            // next one; the server releases the slot as it emits `complete`.
            let next = tokio::time::timeout(Duration::from_secs(5), client.next())
                .await
                .expect("query result should arrive - the slot was never released")
                .unwrap()
                .unwrap();
            let text = next.into_text().unwrap();
            assert!(text.contains("\"hello\":\"world\""));
            assert!(text.contains(&format!("\"id\":\"{id}\"")));

            let complete = client.next().await.unwrap().unwrap();
            assert!(complete.into_text().unwrap().contains("complete"));
        }

        drop(client);
        let _ = serve.await;
    }

    #[tokio::test]
    async fn rejected_connection_init_closes_the_session() {
        let (server, mut client) = websocket_pair().await;

        let config = WebSocketConfig::default().with_on_connection_init(|payload| async move {
            if payload.get("token").and_then(|t| t.as_str()) == Some("secret") {
                Ok(Data::default())
            } else {
                Err(async_graphql::Error::new("unauthorized"))
            }
        });
        let serve = tokio::spawn(serve_graphql_websocket(
            server,
            test_schema(),
            Protocols::GraphQLWS,
            config,
        ));

        client
            .send(WsFrame::Text(
                serde_json::json!({
                    "type": "connection_init",
                    "payload": {"token": "wrong"}
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        let closed = client.next().await.unwrap().unwrap();
        assert!(closed.is_close());

        let result = tokio::time::timeout(Duration::from_secs(5), serve)
            .await
            .expect("serve_graphql_websocket did not return after rejected connection_init")
            .unwrap();
        assert!(result.is_ok());
    }
}
