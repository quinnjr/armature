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

    /// Reject `subscribe`/`start` messages once this many subscriptions are
    /// open on this connection. `None` means unbounded, which is not
    /// recommended for internet-facing servers: a single client could
    /// otherwise open unbounded concurrent subscriptions and exhaust server
    /// memory, CPU, or task capacity.
    ///
    /// Subscription IDs are tracked from the client's perspective (a `start`
    /// increments the count, an explicit `stop` decrements it); a
    /// subscription that completes on its own without the client sending
    /// `stop` is not decremented until the connection ends. When the cap is
    /// hit, an over-limit `start` message is silently dropped: the client
    /// simply never receives `next`/`error`/`complete` for that id. This is
    /// a deliberate, non-chatty mitigation, not a protocol error.
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

    let max_subscriptions = config.max_subscriptions;
    let mut open_subscriptions: HashSet<String> = HashSet::new();

    let input = stream.filter_map(move |frame| {
        let result = match frame {
            Ok(WsFrame::Text(text)) => match ClientMessage::from_bytes(text.as_bytes()) {
                Ok(ClientMessage::Start { id, payload }) => {
                    let at_cap = max_subscriptions.is_some_and(|max| {
                        !open_subscriptions.contains(&id) && open_subscriptions.len() >= max
                    });
                    if at_cap {
                        warn!(
                            "graphql-ws: dropping subscribe id={id} - max_subscriptions cap reached"
                        );
                        None
                    } else {
                        open_subscriptions.insert(id.clone());
                        Some(Ok(ClientMessage::Start { id, payload }))
                    }
                }
                Ok(ClientMessage::Stop { id }) => {
                    open_subscriptions.remove(&id);
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
            http::WsMessage::Text(text) => (WsFrame::Text(text.into()), false),
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

        // Sent while sub-1 is already open and the cap is 1; this subscribe
        // should be silently dropped and never produce output.
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

        let mut saw_sub2 = false;
        loop {
            let msg = client.next().await.unwrap().unwrap();
            let text = msg.into_text().unwrap();
            if text.contains("\"id\":\"sub-2\"") {
                saw_sub2 = true;
            }
            if text.contains("complete") {
                break;
            }
        }
        assert!(
            !saw_sub2,
            "over-limit subscription should never produce output"
        );

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
