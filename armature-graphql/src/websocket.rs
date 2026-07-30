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
//! 3. Calling [`serve_graphql_websocket`] with that stream and the built
//!    [`async_graphql::Schema`].
//!
//! Only text frames are treated as GraphQL-over-WebSocket protocol messages;
//! binary frames are ignored, matching the text-only `graphql-ws` /
//! `graphql-transport-ws` specs. Ping/Pong at the WebSocket-frame level are
//! left to `tokio-tungstenite`'s own automatic handling, consistent with
//! [`armature_core::websocket::handle_websocket`].

use armature_core::Error as ArmatureError;
use async_graphql::Executor;
use async_graphql::futures_util::{SinkExt, StreamExt};
use async_graphql::http::{self, ClientMessage};
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

/// Drive a GraphQL-over-WebSocket session to completion on an already
/// upgraded connection.
///
/// Runs until the client disconnects, sends `ConnectionTerminate`, or the
/// connection is closed due to a protocol error (e.g. a keep-alive
/// timeout). All queries, mutations, and subscriptions sent over the
/// connection are executed against `executor` (typically an
/// [`async_graphql::Schema`]); subscription result streams are pushed back
/// to the client as they yield items.
pub async fn serve_graphql_websocket<S, E>(
    ws_stream: WebSocketStream<S>,
    executor: E,
    protocol: Protocols,
) -> Result<(), ArmatureError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    E: Executor,
{
    let (mut sink, stream) = ws_stream.split();

    let input = stream.filter_map(|frame| async move {
        match frame {
            Ok(WsFrame::Text(text)) => Some(ClientMessage::from_bytes(text.as_bytes())),
            // Binary/Ping/Pong/Close frames carry no graphql-ws protocol
            // message; skip them and keep reading. tokio-tungstenite
            // answers protocol-level Pings automatically, and a Close
            // frame ends the underlying stream on its own on the next poll.
            Ok(_) => None,
            // A transport-level error also ends the underlying stream on
            // the next poll; there is no graphql-ws message to report it as.
            Err(_) => None,
        }
    });

    let mut gql_ws = Box::pin(http::WebSocket::from_message_stream(
        executor, input, protocol,
    ));

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

        if sink.send(frame).await.is_err() {
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
}
