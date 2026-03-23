//! Presence and awareness for collaborative sessions
//!
//! Tracks which users are online, their cursors, selections, and other
//! real-time awareness information.

use crate::ReplicaId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// User presence information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPresence {
    /// User's replica ID
    pub replica_id: ReplicaId,
    /// User ID (application-specific)
    pub user_id: String,
    /// Display name
    pub name: String,
    /// User color (for cursors, highlights)
    pub color: String,
    /// Current status
    pub status: PresenceStatus,
    /// Cursor position (if applicable)
    pub cursor: Option<CursorPosition>,
    /// Selection (if applicable)
    pub selection: Option<SelectionRange>,
    /// Custom metadata
    pub metadata: HashMap<String, serde_json::Value>,
    /// Last activity timestamp
    pub last_seen: DateTime<Utc>,
}

impl UserPresence {
    /// Create a new user presence
    pub fn new(replica_id: ReplicaId, user_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            replica_id,
            user_id: user_id.into(),
            name: name.into(),
            color: generate_color(&replica_id),
            status: PresenceStatus::Active,
            cursor: None,
            selection: None,
            metadata: HashMap::new(),
            last_seen: Utc::now(),
        }
    }

    /// Update last seen timestamp
    pub fn touch(&mut self) {
        self.last_seen = Utc::now();
    }

    /// Set cursor position
    pub fn set_cursor(&mut self, position: CursorPosition) {
        self.cursor = Some(position);
        self.touch();
    }

    /// Clear cursor
    pub fn clear_cursor(&mut self) {
        self.cursor = None;
        self.touch();
    }

    /// Set selection
    pub fn set_selection(&mut self, selection: SelectionRange) {
        self.selection = Some(selection);
        self.touch();
    }

    /// Clear selection
    pub fn clear_selection(&mut self) {
        self.selection = None;
        self.touch();
    }

    /// Set status
    pub fn set_status(&mut self, status: PresenceStatus) {
        self.status = status;
        self.touch();
    }

    /// Set metadata
    pub fn set_metadata(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.metadata.insert(key.into(), value);
        self.touch();
    }

    /// Check if user is considered online
    pub fn is_online(&self, timeout: Duration) -> bool {
        let elapsed = Utc::now().signed_duration_since(self.last_seen);
        elapsed < chrono::Duration::from_std(timeout).unwrap_or(chrono::Duration::seconds(30))
    }
}

/// User presence status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    /// User is actively editing
    Active,
    /// User is viewing but not editing
    Viewing,
    /// User is idle
    Idle,
    /// User is away
    Away,
    /// User is offline
    Offline,
}

impl Default for PresenceStatus {
    fn default() -> Self {
        Self::Active
    }
}

/// Cursor position in a document
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CursorPosition {
    /// Field name (for documents with multiple text fields)
    pub field: Option<u32>,
    /// Line number (0-indexed)
    pub line: u32,
    /// Column number (0-indexed)
    pub column: u32,
    /// Character offset from document start
    pub offset: u32,
}

impl CursorPosition {
    /// Create a new cursor position
    pub fn new(offset: u32) -> Self {
        Self {
            field: None,
            line: 0,
            column: offset,
            offset,
        }
    }

    /// Create with line and column
    pub fn at(line: u32, column: u32) -> Self {
        Self {
            field: None,
            line,
            column,
            offset: 0,
        }
    }

    /// Set the field
    pub fn with_field(mut self, field: u32) -> Self {
        self.field = Some(field);
        self
    }
}

/// Selection range in a document
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SelectionRange {
    /// Start position
    pub start: CursorPosition,
    /// End position
    pub end: CursorPosition,
    /// Selection direction
    pub direction: SelectionDirection,
}

impl SelectionRange {
    /// Create a new selection range
    pub fn new(start: CursorPosition, end: CursorPosition) -> Self {
        Self {
            start,
            end,
            direction: SelectionDirection::Forward,
        }
    }

    /// Create a collapsed selection (cursor)
    pub fn collapsed(pos: CursorPosition) -> Self {
        Self {
            start: pos,
            end: pos,
            direction: SelectionDirection::None,
        }
    }

    /// Check if selection is collapsed
    pub fn is_collapsed(&self) -> bool {
        self.start.offset == self.end.offset
    }

    /// Get selection length
    pub fn len(&self) -> u32 {
        self.end.offset.saturating_sub(self.start.offset)
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Selection direction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionDirection {
    /// No direction (collapsed)
    None,
    /// Selection goes forward (start < end)
    Forward,
    /// Selection goes backward (start > end)
    Backward,
}

/// Presence manager for a collaboration session
#[derive(Debug)]
pub struct PresenceManager {
    /// All user presences
    users: Arc<RwLock<HashMap<ReplicaId, UserPresence>>>,
    /// Timeout for considering users offline
    timeout: Duration,
}

impl PresenceManager {
    /// Create a new presence manager
    pub fn new() -> Self {
        Self {
            users: Arc::new(RwLock::new(HashMap::new())),
            timeout: Duration::from_secs(30),
        }
    }

    /// Create with custom timeout
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            users: Arc::new(RwLock::new(HashMap::new())),
            timeout,
        }
    }

    /// Add or update a user
    pub async fn update(&self, presence: UserPresence) {
        let mut users = self.users.write().await;
        users.insert(presence.replica_id, presence);
    }

    /// Remove a user
    pub async fn remove(&self, replica_id: &ReplicaId) {
        let mut users = self.users.write().await;
        users.remove(replica_id);
    }

    /// Get a user's presence
    pub async fn get(&self, replica_id: &ReplicaId) -> Option<UserPresence> {
        let users = self.users.read().await;
        users.get(replica_id).cloned()
    }

    /// Get all online users
    pub async fn online_users(&self) -> Vec<UserPresence> {
        let users = self.users.read().await;
        users
            .values()
            .filter(|u| u.is_online(self.timeout))
            .cloned()
            .collect()
    }

    /// Get all users (including offline)
    pub async fn all_users(&self) -> Vec<UserPresence> {
        let users = self.users.read().await;
        users.values().cloned().collect()
    }

    /// Get user count
    pub async fn count(&self) -> usize {
        let users = self.users.read().await;
        users.len()
    }

    /// Get online user count
    pub async fn online_count(&self) -> usize {
        let users = self.users.read().await;
        users.values().filter(|u| u.is_online(self.timeout)).count()
    }

    /// Update cursor for a user
    pub async fn update_cursor(&self, replica_id: &ReplicaId, cursor: CursorPosition) {
        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(replica_id) {
            user.set_cursor(cursor);
        }
    }

    /// Update selection for a user
    pub async fn update_selection(&self, replica_id: &ReplicaId, selection: SelectionRange) {
        let mut users = self.users.write().await;
        if let Some(user) = users.get_mut(replica_id) {
            user.set_selection(selection);
        }
    }

    /// Clean up stale users
    pub async fn cleanup_stale(&self, max_age: Duration) {
        let mut users = self.users.write().await;
        let cutoff =
            Utc::now() - chrono::Duration::from_std(max_age).unwrap_or(chrono::Duration::hours(1));

        users.retain(|_, u| u.last_seen > cutoff);
    }

    /// Broadcast presence update to all subscribers
    pub async fn broadcast(&self) -> PresenceSnapshot {
        let users = self.users.read().await;
        PresenceSnapshot {
            users: users.values().cloned().collect(),
            timestamp: Utc::now(),
        }
    }
}

impl Default for PresenceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of all presence information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSnapshot {
    /// All user presences
    pub users: Vec<UserPresence>,
    /// Snapshot timestamp
    pub timestamp: DateTime<Utc>,
}

impl PresenceSnapshot {
    /// Get online users
    pub fn online(&self, timeout: Duration) -> Vec<&UserPresence> {
        self.users.iter().filter(|u| u.is_online(timeout)).collect()
    }

    /// Find a user by ID
    pub fn find_user(&self, user_id: &str) -> Option<&UserPresence> {
        self.users.iter().find(|u| u.user_id == user_id)
    }

    /// Find a user by replica ID
    pub fn find_replica(&self, replica_id: &ReplicaId) -> Option<&UserPresence> {
        self.users.iter().find(|u| &u.replica_id == replica_id)
    }
}

/// Generate a consistent color for a replica ID
fn generate_color(replica_id: &ReplicaId) -> String {
    // Generate a color based on the UUID
    let bytes = replica_id.0.as_bytes();
    let hue = (bytes[0] as u32 * 256 + bytes[1] as u32) % 360;
    format!("hsl({}, 70%, 50%)", hue)
}

/// Presence update event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PresenceEvent {
    /// User joined
    Join(UserPresence),
    /// User left
    Leave(ReplicaId),
    /// User updated their presence
    Update(UserPresence),
    /// User cursor moved
    CursorMove {
        replica_id: ReplicaId,
        cursor: CursorPosition,
    },
    /// User selection changed
    SelectionChange {
        replica_id: ReplicaId,
        selection: SelectionRange,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_presence() {
        let replica = ReplicaId::new();
        let mut presence = UserPresence::new(replica, "user1", "Alice");

        assert_eq!(presence.name, "Alice");
        assert_eq!(presence.status, PresenceStatus::Active);

        presence.set_cursor(CursorPosition::new(10));
        assert!(presence.cursor.is_some());
    }

    #[tokio::test]
    async fn test_presence_manager() {
        let manager = PresenceManager::new();

        let replica1 = ReplicaId::new();
        let replica2 = ReplicaId::new();

        manager
            .update(UserPresence::new(replica1, "user1", "Alice"))
            .await;
        manager
            .update(UserPresence::new(replica2, "user2", "Bob"))
            .await;

        assert_eq!(manager.count().await, 2);
        assert_eq!(manager.online_count().await, 2);

        manager.remove(&replica1).await;
        assert_eq!(manager.count().await, 1);
    }

    #[test]
    fn test_selection_range() {
        let start = CursorPosition::new(5);
        let end = CursorPosition::new(10);
        let selection = SelectionRange::new(start, end);

        assert_eq!(selection.len(), 5);
        assert!(!selection.is_collapsed());
    }
}
