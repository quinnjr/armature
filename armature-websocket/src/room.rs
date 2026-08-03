//! Room-based message broadcasting.

use crate::connection::{Connection, ConnectionId};
use crate::error::{WebSocketError, WebSocketResult};
use crate::message::Message;
use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;

/// Unique identifier for a room.
pub type RoomId = String;

/// A room for grouping WebSocket connections.
#[derive(Debug)]
pub struct Room {
    /// Room identifier
    pub id: RoomId,
    /// Connection IDs in this room
    members: DashMap<ConnectionId, ()>,
}

impl Room {
    /// Create a new room.
    pub fn new(id: RoomId) -> Self {
        Self {
            id,
            members: DashMap::new(),
        }
    }

    /// Add a connection to the room.
    pub fn join(&self, connection_id: ConnectionId) {
        self.members.insert(connection_id, ());
    }

    /// Remove a connection from the room.
    pub fn leave(&self, connection_id: &str) -> bool {
        self.members.remove(connection_id).is_some()
    }

    /// Check if a connection is in the room.
    pub fn contains(&self, connection_id: &str) -> bool {
        self.members.contains_key(connection_id)
    }

    /// Get the number of connections in the room.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Check if the room is empty.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Get all connection IDs in the room.
    ///
    /// Allocates a `Vec` and clones every id. Use
    /// [`Room::for_each_member`] on hot paths such as broadcast.
    pub fn members(&self) -> Vec<ConnectionId> {
        self.members.iter().map(|r| r.key().clone()).collect()
    }

    /// Visit each member id in place, without allocating or cloning.
    ///
    /// Holds a shard lock on the member map for the duration, so `f` must not
    /// touch this same room's membership (`join`/`leave`) - it may freely touch
    /// other maps, which is what broadcast does.
    pub fn for_each_member<F: FnMut(&str)>(&self, mut f: F) {
        for entry in self.members.iter() {
            f(entry.key());
        }
    }
}

/// Manages rooms and their members.
pub struct RoomManager {
    /// All rooms
    rooms: DashMap<RoomId, Arc<Room>>,
    /// Mapping of connection ID to room IDs
    connection_rooms: DashMap<ConnectionId, HashSet<RoomId>>,
    /// All connections
    connections: DashMap<ConnectionId, Connection>,
}

impl RoomManager {
    /// Create a new room manager.
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            connection_rooms: DashMap::new(),
            connections: DashMap::new(),
        }
    }

    /// Register a connection.
    pub fn register_connection(&self, connection: Connection) {
        let id = connection.id.clone();
        self.connections.insert(id.clone(), connection);
        self.connection_rooms.insert(id, HashSet::new());
    }

    /// Unregister a connection and remove it from all rooms.
    pub fn unregister_connection(&self, connection_id: &str) {
        if let Some((_, room_ids)) = self.connection_rooms.remove(connection_id) {
            for room_id in room_ids {
                if let Some(room) = self.rooms.get(&room_id) {
                    room.leave(connection_id);
                }
                // Atomically remove room if empty (avoids TOCTOU race)
                self.rooms.remove_if(&room_id, |_, room| room.is_empty());
            }
        }
        self.connections.remove(connection_id);
    }

    /// Get a connection by ID.
    pub fn get_connection(&self, connection_id: &str) -> Option<Connection> {
        self.connections.get(connection_id).map(|c| c.clone())
    }

    /// Create a room if it doesn't exist.
    pub fn create_room(&self, room_id: RoomId) -> Arc<Room> {
        self.rooms
            .entry(room_id.clone())
            .or_insert_with(|| Arc::new(Room::new(room_id)))
            .clone()
    }

    /// Get a room by ID.
    pub fn get_room(&self, room_id: &str) -> Option<Arc<Room>> {
        self.rooms.get(room_id).map(|r| r.clone())
    }

    /// Delete a room.
    pub fn delete_room(&self, room_id: &str) -> bool {
        if let Some((_, room)) = self.rooms.remove(room_id) {
            // Remove room from all connection's room sets
            for member_id in room.members() {
                if let Some(mut rooms) = self.connection_rooms.get_mut(&member_id) {
                    rooms.remove(room_id);
                }
            }
            true
        } else {
            false
        }
    }

    /// Join a connection to a room.
    pub fn join_room(&self, connection_id: &str, room_id: &str) -> WebSocketResult<()> {
        if !self.connections.contains_key(connection_id) {
            return Err(WebSocketError::ConnectionNotFound(
                connection_id.to_string(),
            ));
        }

        let room = self.create_room(room_id.to_string());
        room.join(connection_id.to_string());

        if let Some(mut rooms) = self.connection_rooms.get_mut(connection_id) {
            rooms.insert(room_id.to_string());
        }

        Ok(())
    }

    /// Remove a connection from a room.
    pub fn leave_room(&self, connection_id: &str, room_id: &str) -> WebSocketResult<()> {
        if let Some(room) = self.rooms.get(room_id) {
            room.leave(connection_id);
        }

        if let Some(mut rooms) = self.connection_rooms.get_mut(connection_id) {
            rooms.remove(room_id);
        }

        // Atomically remove room if empty (avoids TOCTOU race)
        self.rooms.remove_if(room_id, |_, room| room.is_empty());

        Ok(())
    }

    /// Broadcast a message to all connections in a room.
    pub fn broadcast_to_room(&self, room_id: &str, message: Message) -> WebSocketResult<usize> {
        let room = self
            .rooms
            .get(room_id)
            .ok_or_else(|| WebSocketError::RoomNotFound(room_id.to_string()))?;

        // Iterate the membership map directly rather than materializing a
        // `Vec<ConnectionId>` of cloned ids first: on a broadcast that is one
        // allocation plus one string clone per recipient, before a single byte
        // is sent.
        let mut sent_count = 0;
        room.for_each_member(|member_id| {
            if let Some(conn) = self.connections.get(member_id)
                && conn.send(message.clone()).is_ok()
            {
                sent_count += 1;
            }
        });

        Ok(sent_count)
    }

    /// Broadcast a message to all connections in a room except one.
    pub fn broadcast_to_room_except(
        &self,
        room_id: &str,
        message: Message,
        except_id: &str,
    ) -> WebSocketResult<usize> {
        let room = self
            .rooms
            .get(room_id)
            .ok_or_else(|| WebSocketError::RoomNotFound(room_id.to_string()))?;

        let mut sent_count = 0;
        room.for_each_member(|member_id| {
            if member_id != except_id
                && let Some(conn) = self.connections.get(member_id)
                && conn.send(message.clone()).is_ok()
            {
                sent_count += 1;
            }
        });

        Ok(sent_count)
    }

    /// Broadcast a message to all connections.
    pub fn broadcast_all(&self, message: Message) -> usize {
        let mut sent_count = 0;
        for conn in self.connections.iter() {
            if conn.send(message.clone()).is_ok() {
                sent_count += 1;
            }
        }
        sent_count
    }

    /// Get all room IDs.
    pub fn room_ids(&self) -> Vec<RoomId> {
        self.rooms.iter().map(|r| r.key().clone()).collect()
    }

    /// Get all connection IDs.
    pub fn connection_ids(&self) -> Vec<ConnectionId> {
        self.connections.iter().map(|c| c.key().clone()).collect()
    }

    /// Get the total number of connections.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get the total number of rooms.
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }
}

impl Default for RoomManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn make_connection(id: &str) -> (Connection, mpsc::UnboundedReceiver<Message>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Connection::new(id.to_string(), None, tx), rx)
    }

    #[test]
    fn broadcast_sent_count_reflects_actual_deliveries() {
        let manager = RoomManager::new();
        let (conn_a, mut rx_a) = make_connection("a");
        let (conn_b, rx_b) = make_connection("b");
        let (conn_c, mut rx_c) = make_connection("c");

        manager.register_connection(conn_a);
        manager.register_connection(conn_b);
        manager.register_connection(conn_c);

        manager.join_room("a", "room1").unwrap();
        manager.join_room("b", "room1").unwrap();
        manager.join_room("c", "room1").unwrap();

        // Simulate a dead connection: drop its receiver so the underlying
        // channel is closed and Connection::send() fails for "b".
        drop(rx_b);

        let sent = manager
            .broadcast_to_room("room1", Message::text("hi"))
            .unwrap();

        assert_eq!(
            sent, 2,
            "sent_count must reflect only the connections that actually received the message"
        );
        assert!(rx_a.try_recv().is_ok());
        assert!(rx_c.try_recv().is_ok());
    }

    #[test]
    fn broadcast_except_excludes_sender_and_dead_connections() {
        let manager = RoomManager::new();
        let (conn_a, mut rx_a) = make_connection("a");
        let (conn_b, rx_b) = make_connection("b");
        let (conn_c, mut rx_c) = make_connection("c");

        manager.register_connection(conn_a);
        manager.register_connection(conn_b);
        manager.register_connection(conn_c);

        manager.join_room("a", "room1").unwrap();
        manager.join_room("b", "room1").unwrap();
        manager.join_room("c", "room1").unwrap();

        drop(rx_b);

        let sent = manager
            .broadcast_to_room_except("room1", Message::text("hi"), "a")
            .unwrap();

        assert_eq!(sent, 1, "only 'c' should receive: 'a' excluded, 'b' dead");
        assert!(rx_a.try_recv().is_err());
        assert!(rx_c.try_recv().is_ok());
    }

    #[test]
    fn room_persists_while_a_member_remains_and_is_removed_once_empty() {
        let manager = RoomManager::new();
        let (conn_a, _rx_a) = make_connection("a");
        let (conn_b, _rx_b) = make_connection("b");
        manager.register_connection(conn_a);
        manager.register_connection(conn_b);

        manager.join_room("a", "room1").unwrap();
        manager.join_room("b", "room1").unwrap();

        assert!(manager.get_room("room1").is_some());

        manager.leave_room("a", "room1").unwrap();
        assert!(
            manager.get_room("room1").is_some(),
            "room must not be removed while a member remains"
        );

        manager.leave_room("b", "room1").unwrap();
        assert!(
            manager.get_room("room1").is_none(),
            "room should be removed once its last member leaves"
        );
    }

    #[test]
    fn room_removed_when_last_connection_is_unregistered() {
        let manager = RoomManager::new();
        let (conn_a, _rx_a) = make_connection("a");
        manager.register_connection(conn_a);
        manager.join_room("a", "room1").unwrap();

        assert!(manager.get_room("room1").is_some());

        manager.unregister_connection("a");

        assert!(
            manager.get_room("room1").is_none(),
            "room should be cleaned up once its last connection is unregistered"
        );
    }
}
