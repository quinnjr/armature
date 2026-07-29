#![allow(clippy::all)]
//! Real-time Communication API Example
//!
//! This example demonstrates real-time communication patterns with Armature:
//! - A genuine, standalone WebSocket chat server (`armature_websocket`) for
//!   bidirectional, room-based communication
//! - Server-Sent Event *formatting* (`armature_core::sse::ServerSentEvent`)
//!   for structured server events
//! - Broadcasting chat messages to multiple clients via `tokio::broadcast`
//! - Room-based messaging
//!
//! ## Architecture note: why SSE here is a snapshot, not a live stream
//!
//! Armature's HTTP router (`#[routes]` / `HttpResponse`) buffers an entire
//! response body in memory and has no chunked-transfer or connection-hijack
//! hook, so a `#[routes]`-registered handler cannot keep a response open and
//! push events to it over time. `armature_core::streaming::StreamingResponse`
//! exists but isn't wired into the request-dispatch pipeline, and
//! `armature_core::sse::SseBroadcaster`/`SseStream` need exactly that kind of
//! open connection to be useful. So `/api/events/*` below returns real,
//! correctly-formatted `ServerSentEvent` payloads (the same wire format a
//! live `EventSource` stream would use) as a single buffered snapshot,
//! rather than pretending to be a live stream it cannot actually deliver.
//!
//! ## Architecture note: why WebSocket here is a separate port
//!
//! For the same reason, the router has no HTTP-Upgrade support, so a
//! WebSocket route cannot live on the REST API's port either.
//! `armature_websocket::WebSocketServer` is a real, independent
//! tokio-tungstenite-based server; it's spawned here on its own TCP port
//! (3001) with real room-based broadcasting, heartbeats, and connection
//! lifecycle handling -- not a hypothetical endpoint description.
//!
//! Run with: `cargo run --example realtime_api`

use armature::prelude::*;
use armature_websocket::{
    Message as WsMessage, RoomManager, WebSocketHandler, WebSocketServerBuilder,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tokio::time::{Duration, interval};

// =============================================================================
// Message Types
// =============================================================================

/// Chat message for REST-triggered broadcast communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: u64,
    pub username: String,
    pub content: String,
    pub timestamp: String,
    pub room: Option<String>,
}

// =============================================================================
// Broadcasting Service (REST chat)
// =============================================================================

/// Service for broadcasting messages to connected clients.
#[derive(Debug, Clone)]
pub struct BroadcastService {
    sender: broadcast::Sender<ChatMessage>,
    message_id: Arc<AtomicU64>,
}

impl BroadcastService {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            message_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Broadcast a message to all subscribers.
    pub fn broadcast(
        &self,
        username: String,
        content: String,
        room: Option<String>,
    ) -> ChatMessage {
        let id = self.message_id.fetch_add(1, Ordering::SeqCst);
        let message = ChatMessage {
            id,
            username,
            content,
            timestamp: chrono::Utc::now().to_rfc3339(),
            room,
        };

        // Send to all subscribers (ignore errors if no subscribers)
        let _ = self.sender.send(message.clone());
        message
    }

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

// =============================================================================
// SSE Event Log (snapshot-based; see module docs)
// =============================================================================

/// Maximum number of buffered events kept for the `/api/events/stream`
/// snapshot before the oldest is dropped.
const EVENT_LOG_CAPACITY: usize = 20;

static EVENT_LOG: std::sync::OnceLock<RwLock<VecDeque<ServerSentEvent>>> =
    std::sync::OnceLock::new();

fn get_event_log() -> &'static RwLock<VecDeque<ServerSentEvent>> {
    EVENT_LOG.get_or_init(|| RwLock::new(VecDeque::with_capacity(EVENT_LOG_CAPACITY)))
}

/// Record a real, correctly-formatted `ServerSentEvent` into the ring
/// buffer that `/api/events/stream` later returns as a snapshot.
async fn record_event(event_type: &str, data: serde_json::Value) {
    let event = ServerSentEvent::with_event(event_type.to_string(), data.to_string());
    let mut log = get_event_log().write().await;
    if log.len() == EVENT_LOG_CAPACITY {
        log.pop_front();
    }
    log.push_back(event);
}

// =============================================================================
// WebSocket Chat (real, standalone armature_websocket server)
// =============================================================================

/// Default room new WebSocket connections are placed in until they send a
/// `join` action for a different room.
const LOBBY_ROOM: &str = "lobby";

/// Inbound WebSocket client action, matching the schema advertised by
/// `GET /api/ws/info`.
#[derive(Debug, Deserialize)]
struct WsAction {
    action: String,
    room: Option<String>,
    content: Option<String>,
    username: Option<String>,
}

/// Real WebSocket chat handler. Bridges `join` / `leave` / `message`
/// actions from clients into `armature_websocket::RoomManager` broadcasts.
#[derive(Default)]
struct ChatWsHandler {
    /// Tracks which room each connection is currently in, since
    /// `RoomManager` only tracks membership sets, not a "current room"
    /// per client.
    current_room: RwLock<HashMap<String, String>>,
}

#[async_trait]
impl WebSocketHandler for ChatWsHandler {
    async fn on_connect(&self, connection_id: &str) {
        let rooms = get_ws_room_manager();
        let _ = rooms.join_room(connection_id, LOBBY_ROOM);
        self.current_room
            .write()
            .await
            .insert(connection_id.to_string(), LOBBY_ROOM.to_string());

        if let Some(conn) = rooms.get_connection(connection_id) {
            let _ = conn.send_json(&serde_json::json!({
                "type": "welcome",
                "room": LOBBY_ROOM,
                "connection_id": connection_id,
            }));
        }
    }

    async fn on_message(&self, connection_id: &str, message: WsMessage) {
        let Some(text) = message.as_text() else {
            return;
        };

        let rooms = get_ws_room_manager();

        let action: WsAction = match serde_json::from_str(text) {
            Ok(action) => action,
            Err(e) => {
                if let Some(conn) = rooms.get_connection(connection_id) {
                    let _ = conn.send_json(&serde_json::json!({
                        "type": "error",
                        "message": format!("invalid message: {}", e),
                    }));
                }
                return;
            }
        };

        match action.action.as_str() {
            "join" => {
                let Some(room) = action.room else { return };
                let previous = self
                    .current_room
                    .write()
                    .await
                    .insert(connection_id.to_string(), room.clone());
                if let Some(prev) = previous
                    && prev != room
                {
                    let _ = rooms.leave_room(connection_id, &prev);
                }
                let _ = rooms.join_room(connection_id, &room);
                if let Ok(msg) = WsMessage::json(&serde_json::json!({
                    "type": "joined",
                    "room": room,
                    "username": action.username,
                })) {
                    let _ = rooms.broadcast_to_room(&room, msg);
                }
            }
            "leave" => {
                let Some(room) = action.room else { return };
                let _ = rooms.leave_room(connection_id, &room);
            }
            "message" => {
                let room = self
                    .current_room
                    .read()
                    .await
                    .get(connection_id)
                    .cloned()
                    .unwrap_or_else(|| LOBBY_ROOM.to_string());
                let payload = serde_json::json!({
                    "type": "message",
                    "room": room,
                    "username": action.username.unwrap_or_else(|| "anonymous".to_string()),
                    "content": action.content.unwrap_or_default(),
                });
                if let Ok(msg) = WsMessage::json(&payload) {
                    let _ = rooms.broadcast_to_room(&room, msg);
                }
            }
            _ => {
                if let Some(conn) = rooms.get_connection(connection_id) {
                    let _ = conn.send_json(&serde_json::json!({
                        "type": "error",
                        "message": format!("unknown action: {}", action.action),
                    }));
                }
            }
        }
    }

    async fn on_disconnect(&self, connection_id: &str) {
        self.current_room.write().await.remove(connection_id);
    }
}

// =============================================================================
// Global State
// =============================================================================

static BROADCAST_SERVICE: std::sync::OnceLock<BroadcastService> = std::sync::OnceLock::new();

fn get_broadcast_service() -> &'static BroadcastService {
    BROADCAST_SERVICE
        .get()
        .expect("BroadcastService not initialized")
}

/// The `RoomManager` actually used internally by the running
/// `WebSocketServer` (obtained from the server after it's built, and set
/// before the HTTP listener starts accepting requests).
static WS_ROOM_MANAGER: std::sync::OnceLock<Arc<RoomManager>> = std::sync::OnceLock::new();

fn get_ws_room_manager() -> &'static Arc<RoomManager> {
    WS_ROOM_MANAGER
        .get()
        .expect("WebSocket room manager not initialized")
}

// =============================================================================
// Controllers
// =============================================================================

/// REST API endpoints for chat.
#[controller("/api/chat")]
#[derive(Default, Clone)]
struct ChatController;

#[routes]
impl ChatController {
    /// POST /api/chat/messages - Send a message (broadcasts to all clients)
    #[post("/messages")]
    async fn send_message(req: HttpRequest) -> Result<HttpResponse, Error> {
        #[derive(Deserialize)]
        struct SendMessage {
            username: String,
            content: String,
            room: Option<String>,
        }

        let body: SendMessage = req
            .json()
            .map_err(|e| Error::bad_request(format!("Invalid JSON: {}", e)))?;

        if body.content.trim().is_empty() {
            return Err(Error::validation("Message content cannot be empty"));
        }

        let message = get_broadcast_service().broadcast(body.username, body.content, body.room);
        let event_data = serde_json::to_value(&message)
            .map_err(|e| Error::Internal(format!("Failed to serialize event: {}", e)))?;
        record_event("chat_message", event_data).await;
        HttpResponse::json(&message)
    }

    /// GET /api/chat/stats - Get chat statistics
    #[get("/stats")]
    async fn get_stats() -> Result<HttpResponse, Error> {
        HttpResponse::json(&serde_json::json!({
            "active_connections": get_broadcast_service().subscriber_count(),
            "status": "online",
        }))
    }
}

/// Server-Sent Event snapshot endpoints (see module docs for why these are
/// one-shot buffered snapshots rather than a live `EventSource` stream).
#[controller("/api/events")]
#[derive(Default, Clone)]
struct EventsController;

#[routes]
impl EventsController {
    /// GET /api/events/stream - snapshot of recently buffered server events,
    /// formatted using `armature_core::sse::ServerSentEvent`.
    #[get("/stream")]
    async fn event_stream() -> Result<HttpResponse, Error> {
        let log = get_event_log().read().await;
        let mut body = String::new();
        for event in log.iter() {
            body.push_str(&event.to_string());
        }
        drop(log);

        if body.is_empty() {
            body.push_str(
                &ServerSentEvent::with_event(
                    "info".to_string(),
                    serde_json::json!({ "message": "no events recorded yet" }).to_string(),
                )
                .to_string(),
            );
        }

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/event-stream".to_string())
            .with_header("Cache-Control".to_string(), "no-cache".to_string())
            .with_body(body.into_bytes()))
    }

    /// GET /api/events/heartbeat - a single SSE-formatted heartbeat event
    /// (not a persistent keep-alive stream; see module docs).
    #[get("/heartbeat")]
    async fn heartbeat() -> Result<HttpResponse, Error> {
        let event = ServerSentEvent::with_event(
            "heartbeat".to_string(),
            serde_json::json!({ "server_time": chrono::Utc::now().to_rfc3339() }).to_string(),
        );

        Ok(HttpResponse::ok()
            .with_header("Content-Type".to_string(), "text/event-stream".to_string())
            .with_header("Cache-Control".to_string(), "no-cache".to_string())
            .with_body(event.to_string().into_bytes()))
    }
}

/// WebSocket endpoint info -- the actual WebSocket chat server is a
/// standalone `armature_websocket::WebSocketServer` on its own port (see
/// module docs); these routes just report on it over REST.
#[controller("/api/ws")]
#[derive(Default, Clone)]
struct WebSocketController;

#[routes]
impl WebSocketController {
    /// GET /api/ws/info - WebSocket connection info for the real, running
    /// standalone WebSocket server.
    #[get("/info")]
    async fn ws_info() -> Result<HttpResponse, Error> {
        HttpResponse::json(&serde_json::json!({
            "websocket_url": "ws://127.0.0.1:3001",
            "note": "Standalone armature_websocket::WebSocketServer on its own TCP port -- \
                     Armature's HTTP router has no Upgrade/hijack support, so this cannot be \
                     a route on the REST API's port (3000).",
            "supported_protocols": ["chat.v1"],
            "message_format": {
                "type": "string",
                "schema": {
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["join", "leave", "message"]},
                        "room": {"type": "string"},
                        "username": {"type": "string"},
                        "content": {"type": "string"},
                    }
                }
            }
        }))
    }

    /// GET /api/ws/stats - live connection count for the WebSocket lobby
    /// room, read from the real `RoomManager` backing the running server.
    #[get("/stats")]
    async fn ws_stats() -> Result<HttpResponse, Error> {
        let rooms = get_ws_room_manager();
        let lobby_connections = rooms.get_room(LOBBY_ROOM).map(|r| r.len()).unwrap_or(0);
        HttpResponse::json(&serde_json::json!({
            "lobby_connections": lobby_connections,
        }))
    }
}

// =============================================================================
// Demo: Periodic Event Generator
// =============================================================================

/// Spawns a background task that broadcasts periodic events.
async fn spawn_event_generator() {
    let mut interval = interval(Duration::from_secs(30));
    let events = vec![
        ("System", "Server health check completed"),
        (
            "Bot",
            "Did you know? Armature supports a real WebSocket server, SSE-formatted events, and REST!",
        ),
        ("System", "Connected clients: checking..."),
    ];
    let mut event_idx = 0;

    loop {
        interval.tick().await;

        let (username, content) = events[event_idx % events.len()];
        let content = if content.contains("checking") {
            format!(
                "Connected clients: {}",
                get_broadcast_service().subscriber_count()
            )
        } else {
            content.to_string()
        };

        let message = get_broadcast_service().broadcast(
            username.to_string(),
            content,
            Some("announcements".to_string()),
        );
        record_event(
            "system_event",
            serde_json::json!({
                "username": message.username,
                "content": message.content,
                "room": message.room,
            }),
        )
        .await;

        event_idx += 1;
    }
}

// =============================================================================
// Module
// =============================================================================

#[module(
    controllers: [ChatController, EventsController, WebSocketController]
)]
#[derive(Default, Clone)]
struct AppModule;

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() {
    println!("Starting Real-time API example");

    // Create broadcast service
    let broadcast = BroadcastService::new(100);
    BROADCAST_SERVICE
        .set(broadcast)
        .expect("Failed to set broadcast service");

    // Spawn background event generator
    tokio::spawn(async move {
        spawn_event_generator().await;
    });

    // Build and spawn the real, standalone WebSocket chat server. It owns
    // its own `RoomManager`; we grab a clone of that exact instance so the
    // HTTP-side `/api/ws/*` routes and the `ChatWsHandler` callbacks are
    // operating on the same rooms as real connections.
    let ws_server = WebSocketServerBuilder::new()
        .bind("127.0.0.1:3001")
        .expect("invalid WebSocket bind address")
        .build(ChatWsHandler::default());
    if WS_ROOM_MANAGER
        .set(Arc::clone(ws_server.room_manager()))
        .is_err()
    {
        panic!("Failed to set WebSocket room manager");
    }

    tokio::spawn(async move {
        if let Err(e) = ws_server.run().await {
            eprintln!("WebSocket server error: {}", e);
        }
    });

    println!("HTTP server running at http://127.0.0.1:3000");
    println!("WebSocket server running at ws://127.0.0.1:3001");
    println!();
    println!("Available endpoints:");
    println!();
    println!("  Chat API:");
    println!("    POST /api/chat/messages - Send a chat message");
    println!("    GET  /api/chat/stats    - Get chat statistics");
    println!();
    println!("  Events API (one-shot SSE-formatted snapshots, not a live stream):");
    println!("    GET  /api/events/stream    - Snapshot of recent server events");
    println!("    GET  /api/events/heartbeat - Single heartbeat event");
    println!();
    println!("  WebSocket (real, standalone server on its own port):");
    println!("    GET  /api/ws/info  - WebSocket connection details");
    println!("    GET  /api/ws/stats - Live lobby connection count");
    println!("    WS   ws://127.0.0.1:3001 - connect and send:");
    println!(r#"      {{"action":"join","room":"general","username":"Alice"}}"#);
    println!(r#"      {{"action":"message","content":"Hello!","username":"Alice"}}"#);
    println!();
    println!("Test sending a message:");
    println!(r#"  curl -X POST http://localhost:3000/api/chat/messages \"#);
    println!(r#"    -H "Content-Type: application/json" \"#);
    println!(r#"    -d '{{"username":"Alice","content":"Hello!","room":"general"}}'"#);

    let app = Application::create::<AppModule>().await;
    app.listen(3000).await.expect("Server failed");
}
