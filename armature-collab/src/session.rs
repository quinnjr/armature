//! Collaboration session management
//!
//! Manages collaborative editing sessions, including document state,
//! user presence, and synchronization.

use crate::{
    CollabError, CollabResult, Document, PresenceManager, ReplicaId, UserPresence, VectorClock,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
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

        Self {
            id: Uuid::new_v4(),
            document: Arc::new(RwLock::new(document)),
            presence: PresenceManager::new(),
            config,
            events,
            clients: DashMap::new(),
            state: Arc::new(RwLock::new(SessionState::default())),
            created_at: Utc::now(),
        }
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
        self.presence.update(presence).await;

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

    /// Record an operation
    pub async fn record_operation(&self, replica_id: &ReplicaId) {
        if let Some(mut client) = self.clients.get_mut(replica_id) {
            client.ops_received += 1;
            client.last_message = Utc::now();
        }

        let mut state = self.state.write().await;
        state.operations_count += 1;
        state.last_activity = Utc::now();
        state.vclock.increment(*replica_id);
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

        if let Some(session_id) = self.doc_sessions.get(&doc_id) {
            if let Some(session) = self.sessions.get(&session_id) {
                return Arc::clone(&session);
            }
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
        let session2 = manager.create(doc2);

        assert_eq!(manager.count(), 2);
        assert!(manager.get(&session1.id()).is_some());
        assert!(manager.get_by_document("doc1").is_some());

        manager.remove(&session1.id()).await;
        assert_eq!(manager.count(), 1);
    }
}
