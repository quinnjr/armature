//! Collaboration session management
//!
//! Manages collaborative editing sessions, including document state,
//! user presence, and synchronization.

use crate::presence::{CursorPosition, SelectionRange};
use crate::{
    CollabError, CollabResult, Document, DocumentState, PresenceManager, ReplicaId, SyncMessage,
    SyncProtocol, SyncState, SyncStats, UserPresence, VectorClock,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

/// Collaboration session
pub struct CollabSession {
    /// Session ID
    pub id: Uuid,
    /// Document being edited
    document: Arc<RwLock<Document>>,
    /// Presence manager
    presence: PresenceManager,
    /// Session configuration
    config: SessionConfig,
    /// Event broadcaster
    events: broadcast::Sender<SessionEvent>,
    /// Connected clients
    clients: DashMap<ReplicaId, ClientConnection>,
    /// Session state
    state: Arc<RwLock<SessionState>>,
    /// Created timestamp
    created_at: DateTime<Utc>,
    /// Sync protocol handler that backs state transfer for joining peers
    sync: Arc<Mutex<SyncProtocol>>,
    /// Accumulated sync statistics for this session
    sync_stats: Arc<RwLock<SyncStats>>,
}

/// Session configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Maximum number of clients
    pub max_clients: usize,
    /// Idle timeout in seconds
    pub idle_timeout_secs: u64,
    /// Enable presence tracking
    pub enable_presence: bool,
    /// Enable cursor sync
    pub enable_cursors: bool,
    /// Enable selection sync
    pub enable_selections: bool,
    /// Sync interval in milliseconds
    pub sync_interval_ms: u64,
    /// Max operations per sync
    pub max_ops_per_sync: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_clients: 100,
            idle_timeout_secs: 3600, // 1 hour
            enable_presence: true,
            enable_cursors: true,
            enable_selections: true,
            sync_interval_ms: 100,
            max_ops_per_sync: 1000,
        }
    }
}

/// Session state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Session status
    pub status: SessionStatus,
    /// Number of connected clients
    pub client_count: usize,
    /// Total operations processed
    pub operations_count: u64,
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
    /// Vector clock for session
    pub vclock: VectorClock,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            status: SessionStatus::Active,
            client_count: 0,
            operations_count: 0,
            last_activity: Utc::now(),
            vclock: VectorClock::new(),
        }
    }
}

/// Session status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Session is active
    Active,
    /// Session is paused
    Paused,
    /// Session is read-only
    ReadOnly,
    /// Session is closing
    Closing,
    /// Session is closed
    Closed,
}

/// Client connection info
#[derive(Debug, Clone)]
pub struct ClientConnection {
    /// Replica ID
    pub replica_id: ReplicaId,
    /// User presence
    pub presence: UserPresence,
    /// Connected timestamp
    pub connected_at: DateTime<Utc>,
    /// Last message timestamp
    pub last_message: DateTime<Utc>,
    /// Operations sent
    pub ops_sent: u64,
    /// Operations received
    pub ops_received: u64,
}

/// Session events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    /// Client joined
    ClientJoined {
        replica_id: ReplicaId,
        user_id: String,
        name: String,
    },
    /// Client left
    ClientLeft { replica_id: ReplicaId },
    /// Document changed
    DocumentChanged {
        replica_id: ReplicaId,
        field: String,
        version: u64,
    },
    /// Cursor moved
    CursorMoved {
        replica_id: ReplicaId,
        position: crate::presence::CursorPosition,
    },
    /// Selection changed
    SelectionChanged {
        replica_id: ReplicaId,
        selection: crate::presence::SelectionRange,
    },
    /// Presence updated
    PresenceUpdated { replica_id: ReplicaId },
    /// Session state changed
    StateChanged { status: SessionStatus },
    /// Sync required
    SyncRequired { replica_id: ReplicaId },
}

impl CollabSession {
    /// Create a new collaboration session
    pub fn new(document: Document) -> Self {
        Self::with_config(document, SessionConfig::default())
    }

    /// Create a session with custom configuration
    pub fn with_config(document: Document, config: SessionConfig) -> Self {
        let (events, _) = broadcast::channel(1000);
        let replica = document.replica();

        Self {
            id: Uuid::new_v4(),
            document: Arc::new(RwLock::new(document)),
            presence: PresenceManager::new(),
            config,
            events,
            clients: DashMap::new(),
            state: Arc::new(RwLock::new(SessionState::default())),
            created_at: Utc::now(),
            sync: Arc::new(Mutex::new(SyncProtocol::new(replica))),
            sync_stats: Arc::new(RwLock::new(SyncStats::default())),
        }
    }

    /// Access the session configuration.
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// Get the session ID
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Get the document
    pub async fn document(&self) -> tokio::sync::RwLockReadGuard<'_, Document> {
        self.document.read().await
    }

    /// Get the document for writing
    pub async fn document_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, Document> {
        self.document.write().await
    }

    /// Get the presence manager
    pub fn presence(&self) -> &PresenceManager {
        &self.presence
    }

    /// Get session state
    pub async fn state(&self) -> SessionState {
        self.state.read().await.clone()
    }

    /// Join the session
    pub async fn join(
        &self,
        replica_id: ReplicaId,
        user_id: impl Into<String>,
        name: impl Into<String>,
    ) -> CollabResult<broadcast::Receiver<SessionEvent>> {
        let state = self.state.read().await;
        if state.status == SessionStatus::Closed {
            return Err(CollabError::SessionNotFound(self.id));
        }
        drop(state);

        if self.clients.len() >= self.config.max_clients {
            return Err(CollabError::PermissionDenied("Session is full".to_string()));
        }

        let user_id = user_id.into();
        let name = name.into();
        let presence = UserPresence::new(replica_id, user_id.clone(), name.clone());

        let connection = ClientConnection {
            replica_id,
            presence: presence.clone(),
            connected_at: Utc::now(),
            last_message: Utc::now(),
            ops_sent: 0,
            ops_received: 0,
        };

        self.clients.insert(replica_id, connection);
        if self.config.enable_presence {
            self.presence.update(presence).await;
        }

        // Update state
        {
            let mut state = self.state.write().await;
            state.client_count = self.clients.len();
            state.last_activity = Utc::now();
        }

        // Broadcast join event
        let _ = self.events.send(SessionEvent::ClientJoined {
            replica_id,
            user_id,
            name,
        });

        // A freshly-joined peer needs the current document state.
        let _ = self.events.send(SessionEvent::SyncRequired { replica_id });

        Ok(self.events.subscribe())
    }

    /// Leave the session
    pub async fn leave(&self, replica_id: &ReplicaId) {
        self.clients.remove(replica_id);
        self.presence.remove(replica_id).await;

        // Update state
        {
            let mut state = self.state.write().await;
            state.client_count = self.clients.len();
            state.last_activity = Utc::now();
        }

        // Broadcast leave event
        let _ = self.events.send(SessionEvent::ClientLeft {
            replica_id: *replica_id,
        });
    }

    /// Get connected client count
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// Get all connected replica IDs
    pub fn connected_replicas(&self) -> Vec<ReplicaId> {
        self.clients.iter().map(|r| *r.key()).collect()
    }

    /// Check if a replica is connected
    pub fn is_connected(&self, replica_id: &ReplicaId) -> bool {
        self.clients.contains_key(replica_id)
    }

    /// Subscribe to session events
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// Broadcast an event
    pub fn broadcast(&self, event: SessionEvent) {
        let _ = self.events.send(event);
    }

    /// Update session status
    pub async fn set_status(&self, status: SessionStatus) {
        {
            let mut state = self.state.write().await;
            state.status = status;
        }

        let _ = self.events.send(SessionEvent::StateChanged { status });
    }

    /// Close the session
    pub async fn close(&self) {
        self.set_status(SessionStatus::Closing).await;

        // Notify all clients
        for client in self.clients.iter() {
            let _ = self.events.send(SessionEvent::ClientLeft {
                replica_id: *client.key(),
            });
        }

        self.clients.clear();
        self.set_status(SessionStatus::Closed).await;
    }

    /// Touch the session's last-activity timestamp.
    async fn touch_activity(&self) {
        let mut state = self.state.write().await;
        state.last_activity = Utc::now();
    }

    /// Record and broadcast a document change made by a replica.
    ///
    /// Bumps the document version, records the operation and emits a
    /// [`SessionEvent::DocumentChanged`]. Returns the new version.
    pub async fn record_change(&self, replica_id: ReplicaId, field: impl Into<String>) -> u64 {
        let version = {
            let mut doc = self.document.write().await;
            doc.bump_version()
        };

        // Track the sending client's outbound operation count.
        if let Some(mut client) = self.clients.get_mut(&replica_id) {
            client.ops_sent += 1;
            client.last_message = Utc::now();
        }

        {
            let mut state = self.state.write().await;
            state.operations_count += 1;
            state.last_activity = Utc::now();
            state.vclock.increment(replica_id);
        }

        let _ = self.events.send(SessionEvent::DocumentChanged {
            replica_id,
            field: field.into(),
            version,
        });

        version
    }

    /// Update a client's cursor position (honors `enable_cursors`).
    pub async fn update_cursor(&self, replica_id: ReplicaId, position: CursorPosition) {
        if !self.config.enable_cursors {
            return;
        }
        self.presence.update_cursor(&replica_id, position).await;
        let _ = self.events.send(SessionEvent::CursorMoved {
            replica_id,
            position,
        });
        self.touch_activity().await;
    }

    /// Update a client's selection range (honors `enable_selections`).
    pub async fn update_selection(&self, replica_id: ReplicaId, selection: SelectionRange) {
        if !self.config.enable_selections {
            return;
        }
        self.presence.update_selection(&replica_id, selection).await;
        let _ = self.events.send(SessionEvent::SelectionChanged {
            replica_id,
            selection,
        });
        self.touch_activity().await;
    }

    /// Update a client's presence (honors `enable_presence`).
    pub async fn update_presence(&self, presence: UserPresence) {
        if !self.config.enable_presence {
            return;
        }
        let replica_id = presence.replica_id;
        self.presence.update(presence).await;
        let _ = self
            .events
            .send(SessionEvent::PresenceUpdated { replica_id });
        self.touch_activity().await;
    }

    /// Build a snapshot of the current document state for sync transfer.
    pub async fn document_state(&self) -> DocumentState {
        let doc = self.document.read().await;
        let vclock = self.state.read().await.vclock.clone();
        DocumentState {
            doc_id: doc.id().to_string(),
            data: doc.to_json().unwrap_or_default().into_bytes(),
            version: doc.version(),
            vclock,
        }
    }

    /// Handle an incoming sync message, transferring or applying document state.
    ///
    /// A `SyncRequest` is answered with a `SyncResponse` carrying the current
    /// document state; a `SyncResponse` is applied by merging the incoming
    /// document. Other message kinds are delegated to the sync protocol, with
    /// the number of forwarded responses bounded by `max_ops_per_sync`.
    pub async fn handle_sync(&self, msg: SyncMessage) -> CollabResult<Vec<SyncMessage>> {
        {
            let mut s = self.sync_stats.write().await;
            s.messages_received += 1;
            s.last_sync = Some(Utc::now());
        }

        match msg {
            SyncMessage::SyncRequest { replica_id, .. } => {
                let state = self.document_state().await;
                let local = self.sync.lock().replica_id();
                let vclock = self.state.read().await.vclock.clone();

                let _ = self.events.send(SessionEvent::SyncRequired { replica_id });

                {
                    let mut s = self.sync_stats.write().await;
                    s.messages_sent += 1;
                    s.operations_synced += 1;
                }

                Ok(vec![SyncMessage::SyncResponse {
                    replica_id: local,
                    state,
                    vclock,
                }])
            }
            SyncMessage::SyncResponse { state, vclock, .. } => {
                if let Ok(incoming) = Document::from_json(&String::from_utf8_lossy(&state.data)) {
                    {
                        let mut doc = self.document.write().await;
                        doc.merge(&incoming);
                    }
                    {
                        let mut st = self.state.write().await;
                        st.vclock.merge(&vclock);
                        st.last_activity = Utc::now();
                    }
                    let mut s = self.sync_stats.write().await;
                    s.operations_synced += 1;
                }
                Ok(vec![])
            }
            other => {
                let mut responses = {
                    let mut proto = self.sync.lock();
                    proto.handle_message(other)?
                };
                // Never forward more than the configured batch size.
                responses.truncate(self.config.max_ops_per_sync.max(1));
                {
                    let mut s = self.sync_stats.write().await;
                    s.messages_sent += responses.len() as u64;
                }
                Ok(responses)
            }
        }
    }

    /// Current sync-protocol state.
    pub fn sync_state(&self) -> SyncState {
        self.sync.lock().state()
    }

    /// Snapshot of accumulated sync statistics.
    pub async fn sync_stats(&self) -> SyncStats {
        self.sync_stats.read().await.clone()
    }

    /// The configured sync interval as a `Duration`.
    pub fn sync_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.config.sync_interval_ms)
    }

    /// Whether enough time has elapsed since the last sync to sync again,
    /// per the configured `sync_interval_ms`.
    pub async fn should_sync(&self) -> bool {
        let last = self.sync_stats.read().await.last_sync;
        match last {
            Some(t) => {
                (Utc::now() - t).num_milliseconds().max(0) as u64 >= self.config.sync_interval_ms
            }
            None => true,
        }
    }

    /// Whether the session is idle per its configured `idle_timeout_secs`
    /// (no connected clients and no activity within the timeout).
    pub async fn is_idle(&self) -> bool {
        let state = self.state.read().await;
        let idle_for = (Utc::now() - state.last_activity).num_seconds().max(0) as u64;
        state.client_count == 0 && idle_for >= self.config.idle_timeout_secs
    }

    /// Get session info
    pub async fn info(&self) -> SessionInfo {
        let state = self.state.read().await;
        let doc = self.document.read().await;

        SessionInfo {
            id: self.id,
            document_id: doc.id().to_string(),
            client_count: self.clients.len(),
            status: state.status,
            operations_count: state.operations_count,
            created_at: self.created_at,
            last_activity: state.last_activity,
        }
    }
}

/// Session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session ID
    pub id: Uuid,
    /// Document ID
    pub document_id: String,
    /// Connected clients
    pub client_count: usize,
    /// Session status
    pub status: SessionStatus,
    /// Total operations
    pub operations_count: u64,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity
    pub last_activity: DateTime<Utc>,
}

/// Session manager for handling multiple sessions
#[derive(Default)]
pub struct SessionManager {
    sessions: DashMap<Uuid, Arc<CollabSession>>,
    doc_sessions: DashMap<String, Uuid>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            doc_sessions: DashMap::new(),
        }
    }

    /// Create a new session for a document
    pub fn create(&self, document: Document) -> Arc<CollabSession> {
        let doc_id = document.id().to_string();
        let session = Arc::new(CollabSession::new(document));
        let session_id = session.id();

        self.sessions.insert(session_id, Arc::clone(&session));
        self.doc_sessions.insert(doc_id, session_id);

        session
    }

    /// Get or create a session for a document
    pub fn get_or_create(&self, document: Document) -> Arc<CollabSession> {
        let doc_id = document.id().to_string();

        if let Some(session_id) = self.doc_sessions.get(&doc_id)
            && let Some(session) = self.sessions.get(&session_id)
        {
            return Arc::clone(&session);
        }

        self.create(document)
    }

    /// Get a session by ID
    pub fn get(&self, session_id: &Uuid) -> Option<Arc<CollabSession>> {
        self.sessions.get(session_id).map(|r| Arc::clone(&r))
    }

    /// Get a session by document ID
    pub fn get_by_document(&self, doc_id: &str) -> Option<Arc<CollabSession>> {
        self.doc_sessions
            .get(doc_id)
            .and_then(|id| self.sessions.get(&id).map(|r| Arc::clone(&r)))
    }

    /// Remove a session
    pub async fn remove(&self, session_id: &Uuid) {
        if let Some((_, session)) = self.sessions.remove(session_id) {
            let doc_id = session.document().await.id().to_string();
            self.doc_sessions.remove(&doc_id);
            session.close().await;
        }
    }

    /// List all sessions
    pub fn list(&self) -> Vec<Uuid> {
        self.sessions.iter().map(|r| *r.key()).collect()
    }

    /// Get session count
    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// Clean up idle sessions
    pub async fn cleanup_idle(&self, max_idle_secs: u64) {
        let cutoff = Utc::now() - chrono::Duration::seconds(max_idle_secs as i64);
        let mut to_remove = Vec::new();

        for entry in self.sessions.iter() {
            let state = entry.value().state().await;
            if state.last_activity < cutoff && state.client_count == 0 {
                to_remove.push(*entry.key());
            }
        }

        for session_id in to_remove {
            self.remove(&session_id).await;
        }
    }

    /// Clean up idle sessions, honoring each session's own configured
    /// `idle_timeout_secs` rather than a single global value.
    pub async fn cleanup_idle_by_config(&self) {
        let mut to_remove = Vec::new();
        for entry in self.sessions.iter() {
            if entry.value().is_idle().await {
                to_remove.push(*entry.key());
            }
        }
        for session_id in to_remove {
            self.remove(&session_id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_join_leave() {
        let doc = Document::new("test-doc");
        let session = CollabSession::new(doc);

        let replica = ReplicaId::new();
        let _rx = session.join(replica, "user1", "Alice").await.unwrap();

        assert_eq!(session.client_count(), 1);
        assert!(session.is_connected(&replica));

        session.leave(&replica).await;
        assert_eq!(session.client_count(), 0);
    }

    #[tokio::test]
    async fn test_session_manager() {
        let manager = SessionManager::new();

        let doc1 = Document::new("doc1");
        let session1 = manager.create(doc1);

        let doc2 = Document::new("doc2");
        let _session2 = manager.create(doc2);

        assert_eq!(manager.count(), 2);
        assert!(manager.get(&session1.id()).is_some());
        assert!(manager.get_by_document("doc1").is_some());

        manager.remove(&session1.id()).await;
        assert_eq!(manager.count(), 1);
    }

    // --- Regression tests (Workflow 8 · A10) ---

    /// A `SyncRequest` must transfer the current document state to the peer.
    #[tokio::test]
    async fn test_handle_sync_transfers_state() {
        let mut doc = Document::new("doc-sync");
        doc.set_string("title", "Hello");
        let session = CollabSession::new(doc);

        let peer = ReplicaId::new();
        let req = SyncMessage::SyncRequest {
            replica_id: peer,
            vclock: VectorClock::new(),
            doc_id: "doc-sync".to_string(),
        };

        let responses = session.handle_sync(req).await.unwrap();
        assert_eq!(responses.len(), 1);
        match &responses[0] {
            SyncMessage::SyncResponse { state, .. } => {
                let restored = Document::from_json(&String::from_utf8_lossy(&state.data)).unwrap();
                assert_eq!(restored.get_string("title"), Some("Hello"));
            }
            _ => panic!("expected a SyncResponse"),
        }

        let stats = session.sync_stats().await;
        assert!(stats.messages_sent >= 1);
        assert!(stats.messages_received >= 1);
    }

    /// A joining peer applying a `SyncResponse` receives the document state.
    #[tokio::test]
    async fn test_sync_response_applies_state() {
        let mut source_doc = Document::new("doc1");
        source_doc.set_string("body", "content");
        let source = CollabSession::new(source_doc);

        let peer = ReplicaId::new();
        let req = SyncMessage::SyncRequest {
            replica_id: peer,
            vclock: VectorClock::new(),
            doc_id: "doc1".to_string(),
        };
        let response = source.handle_sync(req).await.unwrap().pop().unwrap();

        // A fresh joining session applies the response.
        let joiner = CollabSession::new(Document::new("doc1"));
        joiner.handle_sync(response).await.unwrap();
        let doc = joiner.document().await;
        assert_eq!(doc.get_string("body"), Some("content"));
    }

    /// The dormant `SessionEvent` variants must actually be emitted.
    #[tokio::test]
    async fn test_dormant_events_emitted() {
        let doc = Document::new("evt-doc");
        let session = CollabSession::new(doc);
        let mut rx = session.subscribe();

        let replica = ReplicaId::new();
        // DocumentChanged
        session.record_change(replica, "title").await;
        // CursorMoved
        session.update_cursor(replica, CursorPosition::new(3)).await;
        // SelectionChanged
        session
            .update_selection(
                replica,
                SelectionRange::new(CursorPosition::new(0), CursorPosition::new(2)),
            )
            .await;
        // PresenceUpdated
        session
            .update_presence(UserPresence::new(replica, "u", "User"))
            .await;
        // SyncRequired
        session
            .handle_sync(SyncMessage::SyncRequest {
                replica_id: replica,
                vclock: VectorClock::new(),
                doc_id: "evt-doc".to_string(),
            })
            .await
            .unwrap();

        let mut seen_doc = false;
        let mut seen_cursor = false;
        let mut seen_sel = false;
        let mut seen_presence = false;
        let mut seen_sync = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                SessionEvent::DocumentChanged { .. } => seen_doc = true,
                SessionEvent::CursorMoved { .. } => seen_cursor = true,
                SessionEvent::SelectionChanged { .. } => seen_sel = true,
                SessionEvent::PresenceUpdated { .. } => seen_presence = true,
                SessionEvent::SyncRequired { .. } => seen_sync = true,
                _ => {}
            }
        }
        assert!(seen_doc, "DocumentChanged not emitted");
        assert!(seen_cursor, "CursorMoved not emitted");
        assert!(seen_sel, "SelectionChanged not emitted");
        assert!(seen_presence, "PresenceUpdated not emitted");
        assert!(seen_sync, "SyncRequired not emitted");
    }

    /// Disabled config knobs must suppress the corresponding events.
    #[tokio::test]
    async fn test_config_knobs_honored() {
        let config = SessionConfig {
            enable_cursors: false,
            enable_selections: false,
            ..SessionConfig::default()
        };
        let session = CollabSession::with_config(Document::new("cfg-doc"), config);
        let mut rx = session.subscribe();
        let replica = ReplicaId::new();

        session.update_cursor(replica, CursorPosition::new(1)).await;
        session
            .update_selection(
                replica,
                SelectionRange::new(CursorPosition::new(0), CursorPosition::new(1)),
            )
            .await;

        // No cursor/selection events should have been emitted.
        let mut any = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(
                ev,
                SessionEvent::CursorMoved { .. } | SessionEvent::SelectionChanged { .. }
            ) {
                any = true;
            }
        }
        assert!(
            !any,
            "cursor/selection events emitted despite being disabled"
        );
    }
}
