//! Synchronization protocol for collaborative editing
//!
//! Provides message types and sync logic for keeping replicas in sync.

use crate::{CollabError, CollabResult, ReplicaId, VectorClock};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sync message types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncMessage {
    /// Request to sync state
    SyncRequest {
        /// Requesting replica
        replica_id: ReplicaId,
        /// Current vector clock
        vclock: VectorClock,
        /// Document ID
        doc_id: String,
    },
    /// Response with state
    SyncResponse {
        /// Responding replica
        replica_id: ReplicaId,
        /// Document state
        state: DocumentState,
        /// Vector clock
        vclock: VectorClock,
    },
    /// Operation message
    Operation {
        /// Origin replica
        replica_id: ReplicaId,
        /// Operation ID
        op_id: Uuid,
        /// Operation data (serialized)
        data: Vec<u8>,
        /// Vector clock after operation
        vclock: VectorClock,
    },
    /// Acknowledgment
    Ack {
        /// Acknowledging replica
        replica_id: ReplicaId,
        /// Operation IDs acknowledged
        op_ids: Vec<Uuid>,
    },
    /// Presence update
    Presence {
        /// Replica ID
        replica_id: ReplicaId,
        /// Presence data (serialized)
        data: Vec<u8>,
    },
    /// Heartbeat
    Heartbeat {
        /// Replica ID
        replica_id: ReplicaId,
        /// Vector clock
        vclock: VectorClock,
    },
    /// Error message
    Error {
        /// Error code
        code: SyncErrorCode,
        /// Error message
        message: String,
    },
}

/// Sync error codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncErrorCode {
    /// Unknown error
    Unknown,
    /// Document not found
    DocumentNotFound,
    /// Invalid operation
    InvalidOperation,
    /// Causality violation
    CausalityViolation,
    /// Permission denied
    PermissionDenied,
    /// Rate limited
    RateLimited,
    /// Version mismatch
    VersionMismatch,
}

/// Document state for sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentState {
    /// Document ID
    pub doc_id: String,
    /// Serialized document
    pub data: Vec<u8>,
    /// Document version
    pub version: u64,
    /// Vector clock
    pub vclock: VectorClock,
}

/// Sync state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// Initial state
    Disconnected,
    /// Connecting to peers
    Connecting,
    /// Requesting initial sync
    Syncing,
    /// Fully synchronized
    Synchronized,
    /// Error state
    Error,
}

/// Operation buffer for handling out-of-order operations
#[derive(Debug, Default)]
pub struct OperationBuffer {
    /// Pending operations (waiting for dependencies)
    pending: Vec<PendingOp>,
    /// Acknowledged operation IDs
    acked: std::collections::HashSet<Uuid>,
    /// Maximum buffer size
    max_size: usize,
}

/// Pending operation
#[derive(Debug, Clone)]
pub struct PendingOp {
    /// Operation ID
    pub id: Uuid,
    /// Operation data
    pub data: Vec<u8>,
    /// Required dependencies
    pub deps: Vec<Uuid>,
    /// Received timestamp
    pub received_at: chrono::DateTime<chrono::Utc>,
}

impl OperationBuffer {
    /// Create a new operation buffer
    pub fn new(max_size: usize) -> Self {
        Self {
            pending: Vec::new(),
            acked: std::collections::HashSet::new(),
            max_size,
        }
    }

    /// Add an operation to the buffer
    pub fn add(&mut self, op: PendingOp) -> bool {
        if self.pending.len() >= self.max_size {
            // Remove oldest operations
            self.pending
                .sort_by(|a, b| a.received_at.cmp(&b.received_at));
            self.pending.truncate(self.max_size / 2);
        }

        // Check if already processed
        if self.acked.contains(&op.id) {
            return false;
        }

        self.pending.push(op);
        true
    }

    /// Get operations ready to be applied
    pub fn ready(&mut self) -> Vec<PendingOp> {
        let mut ready = Vec::new();
        let mut i = 0;

        while i < self.pending.len() {
            let deps_satisfied = self.pending[i]
                .deps
                .iter()
                .all(|dep| self.acked.contains(dep));

            if deps_satisfied {
                let op = self.pending.remove(i);
                self.acked.insert(op.id);
                ready.push(op);
            } else {
                i += 1;
            }
        }

        ready
    }

    /// Mark an operation as acknowledged
    pub fn ack(&mut self, op_id: Uuid) {
        self.acked.insert(op_id);
    }

    /// Get pending count
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Clear old acknowledgments
    pub fn gc(&mut self, keep_count: usize) {
        if self.acked.len() > keep_count * 2 {
            // Keep only recent acks - this is a simplification
            // In production, you'd want a more sophisticated GC
            let to_keep: std::collections::HashSet<_> = self
                .pending
                .iter()
                .flat_map(|op| op.deps.iter())
                .cloned()
                .collect();

            self.acked.retain(|id| to_keep.contains(id));
        }
    }
}

/// Sync protocol handler
pub struct SyncProtocol {
    /// Local replica ID
    replica_id: ReplicaId,
    /// Current state
    state: SyncState,
    /// Local vector clock
    vclock: VectorClock,
    /// Operation buffer
    buffer: OperationBuffer,
    /// Pending sync requests
    pending_syncs: std::collections::HashMap<String, VectorClock>,
}

impl SyncProtocol {
    /// Create a new sync protocol handler
    pub fn new(replica_id: ReplicaId) -> Self {
        Self {
            replica_id,
            state: SyncState::Disconnected,
            vclock: VectorClock::new(),
            buffer: OperationBuffer::new(10000),
            pending_syncs: std::collections::HashMap::new(),
        }
    }

    /// Get current state
    pub fn state(&self) -> SyncState {
        self.state
    }

    /// Get replica ID
    pub fn replica_id(&self) -> ReplicaId {
        self.replica_id
    }

    /// Get current vector clock
    pub fn vclock(&self) -> &VectorClock {
        &self.vclock
    }

    /// Start connecting
    pub fn connect(&mut self) {
        self.state = SyncState::Connecting;
    }

    /// Request sync for a document
    pub fn request_sync(&mut self, doc_id: String) -> SyncMessage {
        self.pending_syncs
            .insert(doc_id.clone(), self.vclock.clone());

        SyncMessage::SyncRequest {
            replica_id: self.replica_id,
            vclock: self.vclock.clone(),
            doc_id,
        }
    }

    /// Handle incoming sync message
    pub fn handle_message(&mut self, msg: SyncMessage) -> CollabResult<Vec<SyncMessage>> {
        let mut responses = Vec::new();

        match msg {
            SyncMessage::SyncRequest {
                replica_id: _,
                vclock,
                doc_id: _,
            } => {
                // Would fetch document and return state
                // This is handled by the session layer
                self.vclock.merge(&vclock);
            }
            SyncMessage::SyncResponse { vclock, .. } => {
                self.vclock.merge(&vclock);
                self.state = SyncState::Synchronized;
            }
            SyncMessage::Operation {
                replica_id: _,
                op_id,
                data,
                vclock,
            } => {
                self.vclock.merge(&vclock);

                let pending = PendingOp {
                    id: op_id,
                    data,
                    deps: Vec::new(), // Deps would come from operation
                    received_at: chrono::Utc::now(),
                };

                self.buffer.add(pending);

                // Acknowledge
                responses.push(SyncMessage::Ack {
                    replica_id: self.replica_id,
                    op_ids: vec![op_id],
                });
            }
            SyncMessage::Ack { op_ids, .. } => {
                for op_id in op_ids {
                    self.buffer.ack(op_id);
                }
            }
            SyncMessage::Heartbeat { vclock, .. } => {
                self.vclock.merge(&vclock);
            }
            SyncMessage::Error { code, message } => {
                self.state = SyncState::Error;
                return Err(CollabError::Sync(format!("{:?}: {}", code, message)));
            }
            _ => {}
        }

        Ok(responses)
    }

    /// Create an operation message
    pub fn create_operation(&mut self, op_id: Uuid, data: Vec<u8>) -> SyncMessage {
        self.vclock.increment(self.replica_id);

        SyncMessage::Operation {
            replica_id: self.replica_id,
            op_id,
            data,
            vclock: self.vclock.clone(),
        }
    }

    /// Create a heartbeat message
    pub fn heartbeat(&self) -> SyncMessage {
        SyncMessage::Heartbeat {
            replica_id: self.replica_id,
            vclock: self.vclock.clone(),
        }
    }

    /// Get ready operations from buffer
    pub fn ready_operations(&mut self) -> Vec<PendingOp> {
        self.buffer.ready()
    }

    /// Get pending operation count
    pub fn pending_count(&self) -> usize {
        self.buffer.pending_count()
    }
}

/// Sync statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncStats {
    /// Messages sent
    pub messages_sent: u64,
    /// Messages received
    pub messages_received: u64,
    /// Operations synced
    pub operations_synced: u64,
    /// Bytes sent
    pub bytes_sent: u64,
    /// Bytes received
    pub bytes_received: u64,
    /// Sync latency (ms)
    pub latency_ms: f64,
    /// Last sync timestamp
    pub last_sync: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_buffer() {
        let mut buffer = OperationBuffer::new(100);

        let op1 = PendingOp {
            id: Uuid::new_v4(),
            data: vec![],
            deps: vec![],
            received_at: chrono::Utc::now(),
        };

        buffer.add(op1.clone());
        assert_eq!(buffer.pending_count(), 1);

        let ready = buffer.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(buffer.pending_count(), 0);
    }

    #[test]
    fn test_sync_protocol() {
        let replica = ReplicaId::new();
        let mut protocol = SyncProtocol::new(replica);

        assert_eq!(protocol.state(), SyncState::Disconnected);

        protocol.connect();
        assert_eq!(protocol.state(), SyncState::Connecting);

        let sync_req = protocol.request_sync("doc1".to_string());
        assert!(matches!(sync_req, SyncMessage::SyncRequest { .. }));
    }

    #[test]
    fn test_sync_message_serialization() {
        let msg = SyncMessage::Heartbeat {
            replica_id: ReplicaId::new(),
            vclock: VectorClock::new(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SyncMessage = serde_json::from_str(&json).unwrap();

        assert!(matches!(parsed, SyncMessage::Heartbeat { .. }));
    }
}
