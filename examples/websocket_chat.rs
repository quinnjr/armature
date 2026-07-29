#![allow(
    dead_code,
    clippy::default_constructed_unit_structs,
    clippy::needless_borrow,
    clippy::unnecessary_lazy_evaluations
)]
// WebSocket chat room example
//
// This example runs two servers side by side:
//   - The Armature HTTP application (REST endpoints) on `HTTP_PORT`.
//   - A *real* WebSocket server, built on `armature-websocket`
//     (tokio-tungstenite under the hood), on `WS_PORT`.
//
// `armature_core::websocket::{WebSocketManager, WebSocketRoom}` only provide
// in-process broadcast primitives with no client transport, so they cannot
// accept a real WebSocket connection on their own. `armature-websocket`'s
// `WebSocketServer` is a genuine tokio-tungstenite based server that accepts
// TCP connections and performs the WebSocket upgrade handshake itself.
//
// Both servers share the same `armature_websocket::RoomManager`, so a chat
// message sent by a real WebSocket client is broadcast to every other
// WebSocket client in the room, and a message posted over the HTTP API lands
// in the same room too.

use armature::prelude::*;
use armature_websocket::{
    Message as WsMessage, RoomManager, WebSocketError, WebSocketHandler, WebSocketServerBuilder,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const HTTP_PORT: u16 = 3005;
const WS_PORT: u16 = 3006;

/// Shared with the standalone WebSocket server so the HTTP handlers can
/// broadcast into (and inspect) the exact same rooms real WebSocket clients
/// are connected to. Set once, at startup, by `start_ws_server`.
static CHAT_ROOMS: OnceLock<Arc<RoomManager>> = OnceLock::new();

fn chat_rooms() -> &'static Arc<RoomManager> {
    CHAT_ROOMS
        .get()
        .expect("WebSocket server must be started before serving HTTP requests")
}

// Duplicated intentionally in examples/server_sent_events.rs to keep this
// example self-contained/copy-pasteable.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    user: String,
    message: String,
    timestamp: u64,
}

impl ChatMessage {
    fn system(text: impl Into<String>) -> Self {
        Self {
            user: "system".to_string(),
            message: text.into(),
            timestamp: now_millis(),
        }
    }
}

/// Messages a connected WebSocket client can send as JSON text frames. The
/// first message from a fresh connection must be `join`; `chat` only works
/// once a room has been joined.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientEvent {
    Join { room: String, user: String },
    Chat { message: String },
    Leave,
}

/// Per-connection state the WebSocket handler needs to remember between
/// messages: which room the connection joined, and under which display name.
#[derive(Debug, Clone)]
struct Session {
    room: String,
    user: String,
}

/// Real WebSocket connection handler: implements the join/chat/leave
/// protocol on top of `armature_websocket`'s `Connection` and `RoomManager`
/// primitives.
#[derive(Clone, Default)]
struct ChatWsHandler {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl ChatWsHandler {
    fn reply_error(&self, rooms: &RoomManager, connection_id: &str, text: &str) {
        if let Some(conn) = rooms.get_connection(connection_id) {
            let _ = conn.send_json(&serde_json::json!({ "type": "error", "message": text }));
        }
    }
}

#[async_trait]
impl WebSocketHandler for ChatWsHandler {
    async fn on_connect(&self, connection_id: &str) {
        println!("[ws] client {connection_id} connected");
    }

    async fn on_message(&self, connection_id: &str, message: WsMessage) {
        let Some(text) = message.as_text() else {
            // Binary/other frames aren't part of this example's protocol.
            return;
        };

        let rooms = chat_rooms();

        let event: ClientEvent = match serde_json::from_str(text) {
            Ok(event) => event,
            Err(e) => {
                self.reply_error(rooms, connection_id, &format!("invalid message: {e}"));
                return;
            }
        };

        match event {
            ClientEvent::Join { room, user } => {
                let new_session = Session {
                    room: room.clone(),
                    user: user.clone(),
                };
                let previous = self
                    .sessions
                    .lock()
                    .unwrap()
                    .insert(connection_id.to_string(), new_session);

                // Only one room per connection in this example: leave the
                // previous one (if any) before joining the new one.
                if let Some(prev) = &previous {
                    let _ = rooms.leave_room(connection_id, &prev.room);
                }

                if let Err(e) = rooms.join_room(connection_id, &room) {
                    self.reply_error(rooms, connection_id, &format!("failed to join: {e}"));
                    self.sessions.lock().unwrap().remove(connection_id);
                    return;
                }

                println!("[ws] {user} ({connection_id}) joined room '{room}'");

                let announcement = ChatMessage::system(format!("{user} joined the room"));
                if let Ok(payload) = WsMessage::json(&announcement) {
                    let _ = rooms.broadcast_to_room(&room, payload);
                }
            }
            ClientEvent::Chat { message } => {
                let session = self.sessions.lock().unwrap().get(connection_id).cloned();
                let Some(session) = session else {
                    self.reply_error(rooms, connection_id, "join a room before sending messages");
                    return;
                };

                let chat_message = ChatMessage {
                    user: session.user,
                    message,
                    timestamp: now_millis(),
                };

                match WsMessage::json(&chat_message) {
                    Ok(payload) => {
                        let _ = rooms.broadcast_to_room(&session.room, payload);
                    }
                    Err(e) => {
                        self.reply_error(
                            rooms,
                            connection_id,
                            &format!("failed to encode message: {e}"),
                        );
                    }
                }
            }
            ClientEvent::Leave => {
                if let Some(session) = self.sessions.lock().unwrap().remove(connection_id) {
                    let _ = rooms.leave_room(connection_id, &session.room);
                    println!(
                        "[ws] {} ({connection_id}) left room '{}'",
                        session.user, session.room
                    );
                    let announcement =
                        ChatMessage::system(format!("{} left the room", session.user));
                    if let Ok(payload) = WsMessage::json(&announcement) {
                        let _ = rooms.broadcast_to_room(&session.room, payload);
                    }
                }
            }
        }
    }

    async fn on_disconnect(&self, connection_id: &str) {
        // `RoomManager::unregister_connection` (called by the server right
        // after this hook returns) already removes the connection from
        // every room it was a member of; we only need to forget our own
        // bookkeeping and let the other members know.
        if let Some(session) = self.sessions.lock().unwrap().remove(connection_id) {
            println!(
                "[ws] {} ({connection_id}) disconnected from room '{}'",
                session.user, session.room
            );
            let rooms = chat_rooms();
            let announcement = ChatMessage::system(format!("{} disconnected", session.user));
            if let Ok(payload) = WsMessage::json(&announcement) {
                let _ = rooms.broadcast_to_room_except(&session.room, payload, connection_id);
            }
        }
    }

    async fn on_error(&self, connection_id: &str, error: &WebSocketError) {
        eprintln!("[ws] connection {connection_id} error: {error}");
    }
}

/// Starts the real WebSocket server in the background and publishes the
/// `RoomManager` it owns to `CHAT_ROOMS`, so the HTTP layer can share it.
fn start_ws_server() {
    let bind_addr = SocketAddr::from(([0, 0, 0, 0], WS_PORT));
    let ws_server = WebSocketServerBuilder::new()
        .bind_addr(bind_addr)
        .build(ChatWsHandler::default());

    if CHAT_ROOMS
        .set(Arc::clone(ws_server.room_manager()))
        .is_err()
    {
        panic!("start_ws_server must only be called once");
    }

    tokio::spawn(async move {
        if let Err(e) = ws_server.run().await {
            eprintln!("WebSocket server error: {e}");
        }
    });
}

/// Chat service used by the HTTP controller. It talks to the exact same
/// `RoomManager` the real WebSocket server uses, so an HTTP-posted message
/// reaches connected WebSocket clients and a room's stats reflect real,
/// currently-connected clients.
#[injectable]
#[derive(Default, Clone)]
struct ChatService;

impl ChatService {
    async fn broadcast_to_room(
        &self,
        room_name: &str,
        message: ChatMessage,
    ) -> Result<usize, Error> {
        let payload = WsMessage::json(&message).map_err(|e| Error::Serialization(e.to_string()))?;
        match chat_rooms().broadcast_to_room(room_name, payload) {
            Ok(sent) => Ok(sent),
            // No WebSocket client has joined this room yet -- not an error,
            // just zero recipients.
            Err(WebSocketError::RoomNotFound(_)) => Ok(0),
            Err(e) => Err(Error::Internal(e.to_string())),
        }
    }

    fn room_stats(&self, room_name: &str) -> usize {
        chat_rooms()
            .get_room(room_name)
            .map(|room| room.len())
            .unwrap_or(0)
    }
}

/// Chat controller
#[controller("/chat")]
#[derive(Default, Clone)]
struct ChatController;

#[routes]
impl ChatController {
    #[post("/:room/message")]
    async fn send_message(req: HttpRequest) -> Result<HttpResponse, Error> {
        let room_name = req
            .param("room")
            .ok_or_else(|| Error::Validation("Missing room parameter".to_string()))?;

        let mut message: ChatMessage = req.json()?;
        if message.timestamp == 0 {
            message.timestamp = now_millis();
        }

        let service = ChatService::default();
        let delivered = service.broadcast_to_room(room_name, message).await?;

        HttpResponse::json(&serde_json::json!({
            "status": "sent",
            "delivered_to": delivered
        }))
    }

    #[get("/:room/stats")]
    async fn get_stats(req: HttpRequest) -> Result<HttpResponse, Error> {
        let room_name = req
            .param("room")
            .ok_or_else(|| Error::Validation("Missing room parameter".to_string()))?;

        let service = ChatService::default();
        let count = service.room_stats(room_name);

        HttpResponse::json(&serde_json::json!({
            "room": room_name,
            "connections": count
        }))
    }
}

#[module(
    providers: [ChatService],
    controllers: [ChatController]
)]
#[derive(Default, Clone)]
struct AppModule;

#[tokio::main]
async fn main() {
    println!("🔌 Armature WebSocket Chat Example");
    println!("===================================\n");

    start_ws_server();

    println!("Available endpoints:");
    println!("  POST /chat/:room/message - Send a message to a room (also reaches WS clients)");
    println!("  GET  /chat/:room/stats   - Get live room connection count");
    println!(
        "  WS   ws://localhost:{WS_PORT}       - Real-time chat (armature-websocket, tokio-tungstenite)"
    );
    println!("\nWebSocket protocol (JSON text frames):");
    println!("  Join a room:  {{\"type\":\"join\",\"room\":\"general\",\"user\":\"Alice\"}}");
    println!("  Send chat:    {{\"type\":\"chat\",\"message\":\"Hello!\"}}");
    println!("  Leave a room: {{\"type\":\"leave\"}}");
    println!("\nExample usage:");
    println!("  curl -X POST http://localhost:{HTTP_PORT}/chat/general/message \\");
    println!("    -H 'Content-Type: application/json' \\");
    println!("    -d '{{\"user\":\"Alice\",\"message\":\"Hello!\",\"timestamp\":1234567890}}'");
    println!();

    let app = Application::create::<AppModule>().await;

    if let Err(e) = app.listen(HTTP_PORT).await {
        eprintln!("Server error: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature_websocket::WebSocketClient;
    use std::time::Duration;

    /// Reads messages from `client` until one satisfies `predicate`,
    /// returning it as parsed JSON. System announcements (room joins) and
    /// out-of-order broadcasts are skipped rather than causing a mismatch.
    ///
    /// Panics (via `expect`) if `timeout` elapses or the connection closes
    /// before a matching message arrives -- both are genuine test failures.
    async fn recv_matching(
        client: &mut WebSocketClient,
        timeout: Duration,
        mut predicate: impl FnMut(&serde_json::Value) -> bool,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let message = tokio::time::timeout(remaining, client.recv())
                .await
                .expect("timed out waiting for a matching WebSocket message")
                .expect("connection closed before a matching message arrived");

            let Some(text) = message.as_text() else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            if predicate(&value) {
                return value;
            }
        }
    }

    /// Connects to `url`, retrying for a bit: `start_ws_server` spawns the
    /// listener's bind + accept loop onto a background task, so it may not
    /// be listening yet the instant `start_ws_server()` returns.
    async fn connect_with_retry(url: &str, timeout: Duration) -> WebSocketClient {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match WebSocketClient::connect(url).await {
                Ok(client) => return client,
                Err(e) => {
                    if tokio::time::Instant::now() >= deadline {
                        panic!("failed to connect to {url} within {timeout:?}: {e}");
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }

    /// End-to-end coverage for the doc comment's central claim: a chat
    /// message sent by one real WebSocket client is broadcast to every other
    /// WebSocket client in the same room (via `start_ws_server`'s listener
    /// and `ChatWsHandler`'s join/chat protocol), and a message posted
    /// through the HTTP-facing `ChatService` (the same service
    /// `ChatController::send_message` delegates to) reaches those same
    /// WebSocket clients through the shared `CHAT_ROOMS` `RoomManager`.
    ///
    /// Both scenarios live in one test (rather than two) because
    /// `start_ws_server` publishes to the process-global `CHAT_ROOMS`
    /// `OnceLock` and panics if called more than once; splitting this across
    /// multiple `#[tokio::test]` functions (which run concurrently in the
    /// same process) would race on that global.
    #[tokio::test]
    async fn ws_broadcast_and_http_bridge_share_the_same_room() {
        start_ws_server();

        let url = format!("ws://127.0.0.1:{WS_PORT}");
        let timeout = Duration::from_secs(5);
        let room = "integration-test-room";

        let mut alice = connect_with_retry(&url, timeout).await;
        let mut bob = connect_with_retry(&url, timeout).await;

        alice
            .send_json(&serde_json::json!({"type": "join", "room": room, "user": "alice"}))
            .expect("alice failed to send join");
        // Alice is the only member so far: this is her own join announcement.
        recv_matching(&mut alice, timeout, |v| v["user"] == "system").await;

        bob.send_json(&serde_json::json!({"type": "join", "room": room, "user": "bob"}))
            .expect("bob failed to send join");
        // `ChatWsHandler` calls `join_room` before broadcasting the "bob
        // joined" announcement, so bob observing this message proves the
        // server has already registered him as a room member -- which in
        // turn guarantees the chat message sent right after this will reach
        // him too.
        recv_matching(&mut bob, timeout, |v| v["user"] == "system").await;

        // --- WS-to-WS broadcast ---
        alice
            .send_json(&serde_json::json!({"type": "chat", "message": "hello from alice"}))
            .expect("alice failed to send chat message");

        let received = recv_matching(&mut bob, timeout, |v| {
            v["user"] == "alice" && v["message"] == "hello from alice"
        })
        .await;
        assert_eq!(received["user"], "alice");
        assert_eq!(received["message"], "hello from alice");

        // --- HTTP-to-WS bridge ---
        // Exercises the exact same code path `ChatController::send_message`
        // delegates to, without needing a full HTTP client.
        let service = ChatService::default();
        let delivered = service
            .broadcast_to_room(
                room,
                ChatMessage {
                    user: "http-poster".to_string(),
                    message: "hello via http".to_string(),
                    timestamp: now_millis(),
                },
            )
            .await
            .expect("broadcast_to_room failed");
        assert_eq!(
            delivered, 2,
            "message posted via the HTTP-facing service should reach both connected WS clients"
        );

        let received = recv_matching(&mut bob, timeout, |v| {
            v["user"] == "http-poster" && v["message"] == "hello via http"
        })
        .await;
        assert_eq!(received["user"], "http-poster");
        assert_eq!(received["message"], "hello via http");
    }
}
