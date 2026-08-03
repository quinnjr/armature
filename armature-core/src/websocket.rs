// WebSocket support for Armature

use crate::Error;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::Instrument;

/// WebSocket message type
#[derive(Debug, Clone)]
pub enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

impl From<WsMessage> for WebSocketMessage {
    fn from(msg: WsMessage) -> Self {
        match msg {
            WsMessage::Text(text) => WebSocketMessage::Text(text.to_string()),
            WsMessage::Binary(data) => WebSocketMessage::Binary(data.to_vec()),
            WsMessage::Ping(data) => WebSocketMessage::Ping(data.to_vec()),
            WsMessage::Pong(data) => WebSocketMessage::Pong(data.to_vec()),
            WsMessage::Close(_) => WebSocketMessage::Close,
            // A raw frame is opaque application data, not a close notification.
            // Mapping it to `Close` would tell the handler the peer is going
            // away when it is not, so surface the payload as binary instead.
            WsMessage::Frame(frame) => WebSocketMessage::Binary(frame.into_payload().to_vec()),
        }
    }
}

impl From<WebSocketMessage> for WsMessage {
    fn from(msg: WebSocketMessage) -> Self {
        match msg {
            WebSocketMessage::Text(text) => WsMessage::Text(text.into()),
            WebSocketMessage::Binary(data) => WsMessage::Binary(data.into()),
            WebSocketMessage::Ping(data) => WsMessage::Ping(data.into()),
            WebSocketMessage::Pong(data) => WsMessage::Pong(data.into()),
            WebSocketMessage::Close => WsMessage::Close(None),
        }
    }
}

/// WebSocket connection handle
pub struct WebSocketConnection {
    id: String,
    tx: broadcast::Sender<WebSocketMessage>,
}

impl WebSocketConnection {
    pub fn new(id: String) -> (Self, broadcast::Receiver<WebSocketMessage>) {
        let (tx, rx) = broadcast::channel(100);
        (Self { id, tx }, rx)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub async fn send(&self, message: WebSocketMessage) -> Result<(), Error> {
        self.tx
            .send(message)
            .map_err(|e| Error::Internal(format!("Failed to send message: {}", e)))?;
        Ok(())
    }

    pub async fn send_text(&self, text: String) -> Result<(), Error> {
        self.send(WebSocketMessage::Text(text)).await
    }

    pub async fn send_json<T: serde::Serialize>(&self, data: &T) -> Result<(), Error> {
        let json = serde_json::to_string(data).map_err(|e| Error::Serialization(e.to_string()))?;
        self.send_text(json).await
    }
}

/// WebSocket room for broadcasting to multiple connections
pub struct WebSocketRoom {
    _name: String,
    connections: Arc<RwLock<HashMap<String, broadcast::Sender<WebSocketMessage>>>>,
}

impl WebSocketRoom {
    pub fn new(name: String) -> Self {
        Self {
            _name: name,
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_connection(&self, id: String, tx: broadcast::Sender<WebSocketMessage>) {
        let mut connections = self.connections.write().await;
        connections.insert(id, tx);
    }

    pub async fn remove_connection(&self, id: &str) {
        let mut connections = self.connections.write().await;
        connections.remove(id);
    }

    /// Broadcast a message to every connection in the room.
    ///
    /// `broadcast::Sender::send` only fails when a connection has no live
    /// receivers left, which means the peer is gone. Those entries are reaped
    /// here so the room does not accumulate dead senders (and so
    /// [`Self::connection_count`] reflects reality) — the send itself is
    /// best-effort, so a departed peer is not an error for the caller.
    pub async fn broadcast(&self, message: WebSocketMessage) -> Result<(), Error> {
        let dead: Vec<String> = {
            let connections = self.connections.read().await;
            connections
                .iter()
                .filter(|(_, tx)| tx.send(message.clone()).is_err())
                .map(|(id, _)| id.clone())
                .collect()
        };

        if !dead.is_empty() {
            let mut connections = self.connections.write().await;
            connections.retain(|id, _| !dead.contains(id));
        }

        Ok(())
    }

    pub async fn broadcast_text(&self, text: String) -> Result<(), Error> {
        self.broadcast(WebSocketMessage::Text(text)).await
    }

    pub async fn broadcast_json<T: serde::Serialize>(&self, data: &T) -> Result<(), Error> {
        let json = serde_json::to_string(data).map_err(|e| Error::Serialization(e.to_string()))?;
        self.broadcast_text(json).await
    }

    pub async fn connection_count(&self) -> usize {
        let connections = self.connections.read().await;
        connections.len()
    }
}

/// WebSocket manager for handling multiple rooms
pub struct WebSocketManager {
    rooms: Arc<RwLock<HashMap<String, Arc<WebSocketRoom>>>>,
}

impl WebSocketManager {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_or_create_room(&self, name: &str) -> Arc<WebSocketRoom> {
        let mut rooms = self.rooms.write().await;
        rooms
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(WebSocketRoom::new(name.to_string())))
            .clone()
    }

    pub async fn get_room(&self, name: &str) -> Option<Arc<WebSocketRoom>> {
        let rooms = self.rooms.read().await;
        rooms.get(name).cloned()
    }

    pub async fn remove_room(&self, name: &str) {
        let mut rooms = self.rooms.write().await;
        rooms.remove(name);
    }
}

impl Default for WebSocketManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle a WebSocket connection
///
/// `connection_id` is not fed into `armature_core::connection_manager` —
/// that module tracks connections under its own internally generated
/// [`connection_manager::ConnectionId`] (`u64`) for buffer/keep-alive
/// tuning, and is itself not wired into any built-in server (see its
/// module docs). There is no registry keyed by an externally supplied
/// `String` id for this module to join. Instead, `connection_id` is
/// attached to a tracing span so every log line emitted while this
/// connection is being handled (including handler/protocol errors below)
/// is attributed to it.
pub async fn handle_websocket<S, F, Fut>(
    stream: WebSocketStream<S>,
    connection_id: String,
    handler: F,
) -> Result<(), Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnMut(WebSocketMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    let span = tracing::info_span!("websocket_connection", connection_id = %connection_id);
    // The span is attached to the future rather than entered with a guard:
    // a guard held across an `.await` stays on the thread while this task is
    // suspended, so every other task scheduled onto that thread would be
    // logged under this connection_id.
    handle_connection(stream, handler).instrument(span).await
}

async fn handle_connection<S, F, Fut>(
    mut stream: WebSocketStream<S>,
    mut handler: F,
) -> Result<(), Error>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnMut(WebSocketMessage) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    // Handle incoming messages
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(msg) => {
                if msg.is_close() {
                    // Let the application run its teardown, then complete the
                    // RFC 6455 closing handshake. Dropping the stream here
                    // would abort the TCP connection before tungstenite's
                    // queued Close (and any queued Pong) reply is flushed, and
                    // the peer would see a truncated connection instead of a
                    // clean close. A failure means the peer is already gone,
                    // which is exactly the state we are heading for anyway.
                    let teardown = handler(WebSocketMessage::Close).await;
                    let _ = stream.close(None).await;
                    if let Err(e) = teardown {
                        tracing::error!(error = %e, "WebSocket handler error");
                        return Err(e);
                    }
                    return Ok(());
                }
                let ws_msg: WebSocketMessage = msg.into();
                if let Err(e) = handler(ws_msg).await {
                    tracing::error!(error = %e, "WebSocket handler error");
                    return Err(e);
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "WebSocket error");
                return Err(Error::Internal(format!("WebSocket protocol error: {e}")));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;
    use std::sync::Mutex;
    use tokio_tungstenite::tungstenite::protocol::Role;
    use tokio_tungstenite::tungstenite::protocol::frame::Frame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::{Data, OpCode};

    /// An in-memory client/server WebSocket pair, already past the handshake.
    async fn socket_pair() -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        (client, server)
    }

    #[test]
    fn websocket_message_from_ws_message_text() {
        let ws = WsMessage::Text("hello".into());
        match WebSocketMessage::from(ws) {
            WebSocketMessage::Text(text) => assert_eq!(text, "hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn websocket_message_from_ws_message_binary() {
        let ws = WsMessage::Binary(vec![1, 2, 3].into());
        match WebSocketMessage::from(ws) {
            WebSocketMessage::Binary(data) => assert_eq!(data, vec![1, 2, 3]),
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    #[test]
    fn websocket_message_from_ws_message_ping_pong() {
        match WebSocketMessage::from(WsMessage::Ping(vec![9].into())) {
            WebSocketMessage::Ping(data) => assert_eq!(data, vec![9]),
            other => panic!("expected Ping, got {other:?}"),
        }
        match WebSocketMessage::from(WsMessage::Pong(vec![7].into())) {
            WebSocketMessage::Pong(data) => assert_eq!(data, vec![7]),
            other => panic!("expected Pong, got {other:?}"),
        }
    }

    #[test]
    fn websocket_message_from_ws_message_close() {
        assert!(matches!(
            WebSocketMessage::from(WsMessage::Close(None)),
            WebSocketMessage::Close
        ));
    }

    #[test]
    fn websocket_message_from_ws_message_frame_is_binary_not_close() {
        let frame = Frame::message(vec![4u8, 5, 6], OpCode::Data(Data::Binary), true);
        match WebSocketMessage::from(WsMessage::Frame(frame)) {
            WebSocketMessage::Binary(data) => assert_eq!(data, vec![4, 5, 6]),
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    #[test]
    fn ws_message_from_websocket_message_round_trip() {
        let ws: WsMessage = WebSocketMessage::Text("hi".to_string()).into();
        assert!(matches!(ws, WsMessage::Text(_)));

        let ws: WsMessage = WebSocketMessage::Binary(vec![1, 2]).into();
        assert!(matches!(ws, WsMessage::Binary(_)));

        let ws: WsMessage = WebSocketMessage::Ping(vec![1]).into();
        assert!(matches!(ws, WsMessage::Ping(_)));

        let ws: WsMessage = WebSocketMessage::Pong(vec![1]).into();
        assert!(matches!(ws, WsMessage::Pong(_)));

        let ws: WsMessage = WebSocketMessage::Close.into();
        assert!(matches!(ws, WsMessage::Close(None)));
    }

    #[test]
    fn connection_new_exposes_id() {
        let (conn, _rx) = WebSocketConnection::new("conn-1".to_string());
        assert_eq!(conn.id(), "conn-1");
    }

    #[tokio::test]
    async fn connection_send_delivers_to_receiver() {
        let (conn, mut rx) = WebSocketConnection::new("conn-2".to_string());
        conn.send_text("hello".to_string()).await.unwrap();
        let received = rx.recv().await.unwrap();
        assert!(matches!(received, WebSocketMessage::Text(t) if t == "hello"));
    }

    #[tokio::test]
    async fn connection_send_json_serializes_payload() {
        let (conn, mut rx) = WebSocketConnection::new("conn-3".to_string());
        conn.send_json(&serde_json::json!({"a": 1})).await.unwrap();
        let received = rx.recv().await.unwrap();
        match received {
            WebSocketMessage::Text(text) => assert_eq!(text, r#"{"a":1}"#),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn room_add_and_remove_connection_tracks_count() {
        let room = WebSocketRoom::new("lobby".to_string());
        assert_eq!(room.connection_count().await, 0);

        let (tx, _rx) = broadcast::channel(10);
        room.add_connection("c1".to_string(), tx).await;
        assert_eq!(room.connection_count().await, 1);

        room.remove_connection("c1").await;
        assert_eq!(room.connection_count().await, 0);
    }

    #[tokio::test]
    async fn room_broadcast_reaches_all_connections() {
        let room = WebSocketRoom::new("lobby".to_string());

        let (tx1, mut rx1) = broadcast::channel(10);
        let (tx2, mut rx2) = broadcast::channel(10);
        room.add_connection("c1".to_string(), tx1).await;
        room.add_connection("c2".to_string(), tx2).await;

        room.broadcast_text("hi all".to_string()).await.unwrap();

        let msg1 = rx1.recv().await.unwrap();
        let msg2 = rx2.recv().await.unwrap();
        assert!(matches!(msg1, WebSocketMessage::Text(t) if t == "hi all"));
        assert!(matches!(msg2, WebSocketMessage::Text(t) if t == "hi all"));
    }

    #[tokio::test]
    async fn room_broadcast_reaps_connections_without_receivers() {
        let room = WebSocketRoom::new("lobby".to_string());

        let (tx1, mut rx1) = broadcast::channel(10);
        let (tx2, rx2) = broadcast::channel(10);
        room.add_connection("c1".to_string(), tx1).await;
        room.add_connection("c2".to_string(), tx2).await;
        assert_eq!(room.connection_count().await, 2);

        // c2's peer is gone: its sender can no longer reach anyone.
        drop(rx2);

        room.broadcast_text("hi".to_string()).await.unwrap();

        assert_eq!(room.connection_count().await, 1);
        assert!(matches!(rx1.recv().await.unwrap(), WebSocketMessage::Text(t) if t == "hi"));
    }

    #[tokio::test]
    async fn handle_websocket_delivers_messages_in_order_then_closes() {
        let (mut client, server) = socket_pair().await;

        let client_task = tokio::spawn(async move {
            client.send(WsMessage::Text("one".into())).await.unwrap();
            client.send(WsMessage::Text("two".into())).await.unwrap();
            client.send(WsMessage::Close(None)).await.unwrap();
            // Drain so the server's close reply has somewhere to land.
            while let Some(Ok(_)) = client.next().await {}
        });

        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let result = handle_websocket(server, "conn-a".to_string(), move |msg| {
            let recorder = Arc::clone(&recorder);
            async move {
                recorder.lock().unwrap().push(msg);
                Ok::<(), Error>(())
            }
        })
        .await;

        client_task.await.unwrap();
        assert!(result.is_ok());

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(matches!(&seen[0], WebSocketMessage::Text(t) if t == "one"));
        assert!(matches!(&seen[1], WebSocketMessage::Text(t) if t == "two"));
        assert!(matches!(seen[2], WebSocketMessage::Close));
    }

    #[tokio::test]
    async fn handle_websocket_stops_and_reports_handler_error() {
        let (mut client, server) = socket_pair().await;

        let client_task = tokio::spawn(async move {
            let _ = client.send(WsMessage::Text("one".into())).await;
            let _ = client.send(WsMessage::Text("two".into())).await;
            while let Some(Ok(_)) = client.next().await {}
        });

        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let result = handle_websocket(server, "conn-b".to_string(), move |msg| {
            let recorder = Arc::clone(&recorder);
            async move {
                recorder.lock().unwrap().push(msg);
                Err(Error::Internal("handler exploded".to_string()))
            }
        })
        .await;

        client_task.await.unwrap();
        match result {
            Err(Error::Internal(msg)) => assert!(msg.contains("handler exploded")),
            other => panic!("expected handler error, got {other:?}"),
        }
        // Consumption stops at the first failure: "two" is never handled.
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn manager_get_or_create_room_is_idempotent() {
        let manager = WebSocketManager::new();
        let room_a = manager.get_or_create_room("room-1").await;
        let room_b = manager.get_or_create_room("room-1").await;
        assert!(Arc::ptr_eq(&room_a, &room_b));
    }

    #[tokio::test]
    async fn manager_get_room_returns_none_when_missing() {
        let manager = WebSocketManager::default();
        assert!(manager.get_room("missing").await.is_none());
    }

    #[tokio::test]
    async fn manager_remove_room_drops_it() {
        let manager = WebSocketManager::new();
        manager.get_or_create_room("room-1").await;
        assert!(manager.get_room("room-1").await.is_some());

        manager.remove_room("room-1").await;
        assert!(manager.get_room("room-1").await.is_none());
    }
}
