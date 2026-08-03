//! Synchronization protocol for collaborative editing
//!
//! Provides message types and sync logic for keeping replicas in sync.

use crate::{CollabError, CollabResult, ReplicaId, VectorClock};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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
        /// Causal dependencies: operation ids that must be applied first
        #[serde(default)]
        deps: Vec<Uuid>,
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
///
/// # Memory
///
/// `max_size` bounds the *pending* queue only. It does **not** bound the total
/// memory this buffer holds: [`applied`](Self::mark_applied) records the id of
/// every operation ever applied, because that set is what makes duplicate
/// delivery a no-op and what decides whether a causal dependency is satisfied.
/// Operation ids are opaque `Uuid`s with no ordering, and [`PendingOp`] carries
/// no causal coordinates, so there is no watermark below which an id is
/// provably safe to forget: a long-lived session's applied set grows without
/// limit at 16 bytes plus hashtable overhead per operation.
///
/// [`with_applied_limit`](Self::with_applied_limit) trades that unbounded
/// growth for a finite deduplication window, when a caller can accept the
/// consequences described there. It is off by default because forgetting an id
/// too early re-applies a redelivered operation, and only the caller knows
/// whether its operations are idempotent.
#[derive(Debug, Default)]
pub struct OperationBuffer {
    /// Pending operations (waiting for dependencies)
    pending: Vec<PendingOp>,
    /// Operation IDs acknowledged by a *peer* (delivery receipt only).
    ///
    /// A peer ack does NOT mean the op has been applied locally, so this set
    /// must never be used to decide whether a causal dependency is satisfied.
    acked: HashSet<Uuid>,
    /// Operation IDs that have been *applied locally* (returned from
    /// [`ready`](Self::ready) or marked via [`mark_applied`](Self::mark_applied)).
    /// Causal dependencies are satisfied only from this set.
    ///
    /// Unbounded unless `applied_limit` is set — see the type-level docs.
    applied: HashSet<Uuid>,
    /// Insertion order of `applied`, maintained only while `applied_limit` is
    /// set so the default (unbounded) configuration pays nothing for it.
    applied_order: VecDeque<Uuid>,
    /// Optional bound on `applied`; `None` keeps every id forever.
    applied_limit: Option<usize>,
    /// Maximum number of *pending* operations.
    ///
    /// This is not a bound on the buffer's total memory — see the type-level
    /// docs for why `applied` is unbounded by default.
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
    /// Create a new operation buffer.
    ///
    /// `max_size` bounds the pending queue only; the applied-id set is
    /// unbounded. See the type-level docs and
    /// [`with_applied_limit`](Self::with_applied_limit).
    pub fn new(max_size: usize) -> Self {
        Self {
            pending: Vec::new(),
            acked: HashSet::new(),
            applied: HashSet::new(),
            applied_order: VecDeque::new(),
            applied_limit: None,
            max_size,
        }
    }

    /// Bound the applied-id set to `limit` entries, oldest evicted first.
    ///
    /// # What this trades away
    ///
    /// The applied set is what makes a redelivered operation a no-op. Once an
    /// id is evicted, a peer that resends that operation — after a reconnect,
    /// say — will have it released from [`ready`](Self::ready) a second time,
    /// and an operation whose application is not idempotent will be applied
    /// twice. Set this only when either the operations are idempotent (most
    /// state-based CRDT merges are) or the transport cannot redeliver beyond
    /// `limit` operations of history.
    ///
    /// Ids that are still named as dependencies by a pending operation are
    /// never evicted, regardless of age, so bounding the set cannot deadlock
    /// the pending queue.
    ///
    /// Intended to be chained onto [`new`](Self::new). Calling it later is
    /// supported but the eviction order of ids applied before the call is
    /// unspecified.
    pub fn with_applied_limit(mut self, limit: usize) -> Self {
        self.applied_limit = Some(limit.max(1));
        if self.applied_order.len() != self.applied.len() {
            self.applied_order = self.applied.iter().copied().collect();
        }
        self.trim_applied();
        self
    }

    /// Number of operation ids currently retained as applied.
    ///
    /// Exposed so a long-lived session can observe the growth described in the
    /// type-level docs rather than discovering it as memory pressure.
    pub fn applied_count(&self) -> usize {
        self.applied.len()
    }

    /// Record an id as applied locally, maintaining the eviction order.
    fn record_applied(&mut self, id: Uuid) {
        if self.applied.insert(id) && self.applied_limit.is_some() {
            self.applied_order.push_back(id);
            self.trim_applied();
        }
    }

    /// Evict the oldest applied ids down to `applied_limit`, skipping any id a
    /// pending operation still depends on.
    fn trim_applied(&mut self) {
        let Some(limit) = self.applied_limit else {
            return;
        };
        if self.applied.len() <= limit {
            return;
        }

        let needed: HashSet<Uuid> = self
            .pending
            .iter()
            .flat_map(|op| op.deps.iter().copied())
            .collect();

        let mut excess = self.applied.len() - limit;
        let mut retained = Vec::new();
        while excess > 0 {
            let Some(id) = self.applied_order.pop_front() else {
                break;
            };
            if needed.contains(&id) {
                // Still load-bearing for a buffered op: keep it, but let it
                // age out on a later pass once that op has been released.
                retained.push(id);
            } else {
                self.applied.remove(&id);
                excess -= 1;
            }
        }
        // Put the retained ids back at the front, oldest first, so they remain
        // the next candidates for eviction.
        for id in retained.into_iter().rev() {
            self.applied_order.push_front(id);
        }
    }

    /// Add an operation to the buffer
    pub fn add(&mut self, op: PendingOp) -> bool {
        if self.pending.len() >= self.max_size {
            // Evict the OLDEST operations to make room, keeping the most recent
            // half. (The previous implementation sorted ascending and then
            // `truncate`d, which kept the oldest half and discarded the newest
            // arrivals — the exact opposite of "remove oldest".)
            self.pending.sort_by_key(|a| a.received_at);
            let keep = (self.max_size / 2).max(1).min(self.pending.len());
            let drop = self.pending.len() - keep;
            self.pending.drain(0..drop);
        }

        // Check if already applied locally (dedup); a peer ack alone is not
        // enough to consider an op processed.
        if self.applied.contains(&op.id) {
            return false;
        }

        self.pending.push(op);
        true
    }

    /// Check whether an operation with the given id is currently buffered.
    pub fn contains(&self, id: &Uuid) -> bool {
        self.pending.iter().any(|op| op.id == *id)
    }

    /// Get operations ready to be applied.
    ///
    /// A dependency is satisfied only when the dep op has been **applied
    /// locally** (`applied` set) — a peer ack is not sufficient, as that would
    /// let an op be released before the op it causally depends on has actually
    /// been applied here. The scan runs to a fixpoint so a chain buffered in
    /// reverse order (`B` added before its dep `A`) is fully released in a
    /// single call rather than one op per call.
    ///
    /// Released operations come back in dependency order: an operation always
    /// appears after every buffered operation it waited on.
    ///
    /// # Cost
    ///
    /// Linear in the number of buffered operations plus their dependency
    /// edges. The previous implementation re-scanned the whole queue once per
    /// fixpoint pass and used `Vec::remove` (an O(n) shift) per released
    /// operation, so a reverse-ordered chain of length n — the case the
    /// fixpoint exists for — cost O(n^3), with n up to `max_size` (10,000 in
    /// [`SyncProtocol`]).
    pub fn ready(&mut self) -> Vec<PendingOp> {
        let n = self.pending.len();
        if n == 0 {
            return Vec::new();
        }

        // Build, once: how many dependencies each buffered op is still waiting
        // on, and which ops are waiting on each id. Deps already in `applied`
        // are not counted, and a dep that names an op nobody buffered simply
        // never gets decremented — that op stays pending, as before.
        let mut unmet = vec![0usize; n];
        let mut dependants: HashMap<Uuid, Vec<usize>> = HashMap::new();
        for (i, op) in self.pending.iter().enumerate() {
            for dep in &op.deps {
                if !self.applied.contains(dep) {
                    unmet[i] += 1;
                    dependants.entry(*dep).or_default().push(i);
                }
            }
        }

        let mut queue: VecDeque<usize> = (0..n).filter(|i| unmet[*i] == 0).collect();
        let mut order = Vec::new();
        while let Some(i) = queue.pop_front() {
            let id = self.pending[i].id;
            order.push(i);
            self.record_applied(id);

            if let Some(waiting) = dependants.get(&id) {
                for &j in waiting {
                    // Guarded rather than unconditional: two buffered ops may
                    // share an id (a redelivery buffered before the first copy
                    // was released), and decrementing past zero would release
                    // the dependant twice.
                    if unmet[j] > 0 {
                        unmet[j] -= 1;
                        if unmet[j] == 0 {
                            queue.push_back(j);
                        }
                    }
                }
            }
        }

        // Take the released ops out by index instead of removing them one at a
        // time: `Vec::remove` shifts the tail on every call, and `swap_remove`
        // would scramble the order of the ops left behind.
        let mut slots: Vec<Option<PendingOp>> = self.pending.drain(..).map(Some).collect();
        let mut ready = Vec::with_capacity(order.len());
        for i in order {
            ready.push(
                slots[i]
                    .take()
                    .expect("each index is enqueued at most once"),
            );
        }
        self.pending = slots.into_iter().flatten().collect();

        ready
    }

    /// Mark an operation as acknowledged by a peer (delivery receipt).
    ///
    /// This records a peer's receipt only; it does NOT satisfy a local causal
    /// dependency — use [`mark_applied`](Self::mark_applied) for that.
    pub fn ack(&mut self, op_id: Uuid) {
        self.acked.insert(op_id);
    }

    /// Mark an operation as applied locally, so ops depending on it become
    /// eligible. Use this for dependencies applied outside this buffer (e.g.
    /// prior to it being constructed or via a state-based sync).
    pub fn mark_applied(&mut self, op_id: Uuid) {
        self.record_applied(op_id);
    }

    /// Get pending count
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Clear old acknowledgments
    ///
    /// This touches the peer-ack set only. It does **not** shrink the
    /// applied-id set — see the type-level docs and
    /// [`with_applied_limit`](Self::with_applied_limit), which is the knob for
    /// that, because dropping an applied id has correctness consequences a
    /// blind GC pass cannot reason about.
    pub fn gc(&mut self, keep_count: usize) {
        if self.acked.len() > keep_count * 2 {
            // Keep only recent acks - this is a simplification
            // In production, you'd want a more sophisticated GC
            let to_keep: HashSet<_> = self
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
    /// Accumulated sync statistics
    stats: SyncStats,
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
            stats: SyncStats::default(),
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
        self.stats.messages_received += 1;
        self.stats.last_sync = Some(chrono::Utc::now());

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
                deps,
                vclock,
            } => {
                self.vclock.merge(&vclock);
                self.stats.operations_synced += 1;

                let pending = PendingOp {
                    id: op_id,
                    data,
                    deps, // real causal dependencies carried by the message
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

        self.stats.messages_sent += responses.len() as u64;
        Ok(responses)
    }

    /// Create an operation message with no causal dependencies.
    pub fn create_operation(&mut self, op_id: Uuid, data: Vec<u8>) -> SyncMessage {
        self.create_operation_with_deps(op_id, data, Vec::new())
    }

    /// Create an operation message carrying explicit causal dependencies.
    pub fn create_operation_with_deps(
        &mut self,
        op_id: Uuid,
        data: Vec<u8>,
        deps: Vec<Uuid>,
    ) -> SyncMessage {
        self.vclock.increment(self.replica_id);
        self.stats.messages_sent += 1;
        self.stats.operations_synced += 1;
        self.stats.last_sync = Some(chrono::Utc::now());

        SyncMessage::Operation {
            replica_id: self.replica_id,
            op_id,
            data,
            deps,
            vclock: self.vclock.clone(),
        }
    }

    /// Access sync statistics accumulated by this protocol handler.
    pub fn stats(&self) -> &SyncStats {
        &self.stats
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

    /// Overflow must evict the OLDEST operations, not the newest arrivals.
    #[test]
    fn test_operation_buffer_evicts_oldest() {
        let mut buffer = OperationBuffer::new(4);
        let base = chrono::Utc::now();

        let mut ids = Vec::new();
        for i in 0..4 {
            let id = Uuid::new_v4();
            ids.push(id);
            buffer.add(PendingOp {
                id,
                data: vec![],
                deps: vec![],
                received_at: base + chrono::Duration::seconds(i),
            });
        }
        assert_eq!(buffer.pending_count(), 4);

        // A newer arrival triggers eviction.
        let newest = Uuid::new_v4();
        buffer.add(PendingOp {
            id: newest,
            data: vec![],
            deps: vec![],
            received_at: base + chrono::Duration::seconds(100),
        });

        // The two oldest must be gone; the newest arrival must be retained.
        assert!(!buffer.contains(&ids[0]), "oldest op was not evicted");
        assert!(
            !buffer.contains(&ids[1]),
            "second-oldest op was not evicted"
        );
        assert!(buffer.contains(&newest), "newest arrival was discarded");
    }

    /// A buffered op with unmet deps must not be ready until its deps are acked.
    #[test]
    fn test_operation_buffer_honors_deps() {
        let mut buffer = OperationBuffer::new(100);
        let dep = Uuid::new_v4();
        let op = PendingOp {
            id: Uuid::new_v4(),
            data: vec![],
            deps: vec![dep],
            received_at: chrono::Utc::now(),
        };
        buffer.add(op.clone());

        // Dependency not yet satisfied -> not ready.
        assert!(buffer.ready().is_empty());
        assert_eq!(buffer.pending_count(), 1);

        // Satisfy the dependency by marking it locally applied; now ready.
        buffer.mark_applied(dep);
        let ready = buffer.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, op.id);
    }

    /// A peer `Ack` must NOT satisfy a local causal dependency — only local
    /// application does. (Fails against the old code, where `ack` released the
    /// dependent, a causal-order violation.)
    #[test]
    fn test_peer_ack_does_not_satisfy_local_dep() {
        let mut buffer = OperationBuffer::new(100);
        let dep = Uuid::new_v4();
        let op = PendingOp {
            id: Uuid::new_v4(),
            data: vec![],
            deps: vec![dep],
            received_at: chrono::Utc::now(),
        };
        buffer.add(op.clone());

        // A peer acked the dep, but we have not applied it locally.
        buffer.ack(dep);
        assert!(
            buffer.ready().is_empty(),
            "peer ack must not release a causally-dependent op"
        );
        assert_eq!(buffer.pending_count(), 1);

        // Now it is actually applied locally -> the dependent is released.
        buffer.mark_applied(dep);
        let ready = buffer.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, op.id);
    }

    /// A chain buffered in reverse order (dependent added before its dep) must
    /// be fully released in a single `ready()` call, dep before dependent.
    #[test]
    fn test_ready_releases_reordered_chain_in_one_pass() {
        let mut buffer = OperationBuffer::new(100);
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let now = chrono::Utc::now();

        // B depends on A, but B is added FIRST.
        buffer.add(PendingOp {
            id: b,
            data: vec![],
            deps: vec![a],
            received_at: now,
        });
        buffer.add(PendingOp {
            id: a,
            data: vec![],
            deps: vec![],
            received_at: now,
        });

        let ready = buffer.ready();
        assert_eq!(ready.len(), 2, "reordered chain not fully released");
        assert_eq!(ready[0].id, a, "dep must be released before dependent");
        assert_eq!(ready[1].id, b);
        assert_eq!(buffer.pending_count(), 0);
    }

    /// A long chain buffered in strictly reverse dependency order is the case
    /// the old `ready()` cost O(n^3) on. The rewritten worklist must release
    /// exactly the same set, in a dep-before-dependent order, in one call.
    #[test]
    fn test_ready_releases_long_reverse_ordered_chain() {
        const CHAIN: usize = 500;

        let mut buffer = OperationBuffer::new(10_000);
        let now = chrono::Utc::now();
        let ids: Vec<Uuid> = (0..CHAIN).map(|_| Uuid::new_v4()).collect();

        // op[k] depends on op[k - 1]; added last-to-first, so every add sits in
        // front of the dependency it is waiting on.
        for k in (0..CHAIN).rev() {
            buffer.add(PendingOp {
                id: ids[k],
                data: vec![],
                deps: if k == 0 { vec![] } else { vec![ids[k - 1]] },
                received_at: now,
            });
        }
        assert_eq!(buffer.pending_count(), CHAIN);

        let ready = buffer.ready();
        assert_eq!(ready.len(), CHAIN, "chain not fully released");
        assert_eq!(buffer.pending_count(), 0);

        // Same result set, and the causal order is preserved end to end.
        let released: Vec<Uuid> = ready.iter().map(|op| op.id).collect();
        assert_eq!(released, ids, "ops released out of dependency order");
    }

    /// An op whose dependency was never buffered stays pending, and the ops
    /// that *are* satisfiable are still released around it.
    #[test]
    fn test_ready_leaves_ops_with_unbuffered_deps_pending() {
        let mut buffer = OperationBuffer::new(100);
        let now = chrono::Utc::now();
        let missing = Uuid::new_v4();
        let stuck = Uuid::new_v4();
        let free = Uuid::new_v4();

        buffer.add(PendingOp {
            id: stuck,
            data: vec![],
            deps: vec![missing],
            received_at: now,
        });
        buffer.add(PendingOp {
            id: free,
            data: vec![],
            deps: vec![],
            received_at: now,
        });

        let ready = buffer.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, free);
        assert!(buffer.contains(&stuck));
        assert_eq!(buffer.pending_count(), 1);

        // Once the missing dep is applied elsewhere, the stuck op is released.
        buffer.mark_applied(missing);
        let ready = buffer.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, stuck);
    }

    /// `max_size` bounds the pending queue only; the applied set is what grows.
    #[test]
    fn test_applied_set_is_unbounded_by_default() {
        let mut buffer = OperationBuffer::new(4);
        for _ in 0..50 {
            buffer.add(PendingOp {
                id: Uuid::new_v4(),
                data: vec![],
                deps: vec![],
                received_at: chrono::Utc::now(),
            });
            buffer.ready();
        }
        assert!(buffer.pending_count() <= 4);
        assert_eq!(buffer.applied_count(), 50);
    }

    /// The opt-in bound evicts oldest-first but never an id a pending op still
    /// depends on, so bounding the set cannot deadlock the queue.
    #[test]
    fn test_applied_limit_evicts_oldest_but_keeps_live_deps() {
        let mut buffer = OperationBuffer::new(100).with_applied_limit(3);
        let now = chrono::Utc::now();

        let live_dep = Uuid::new_v4();
        buffer.mark_applied(live_dep);

        // A pending op still names `live_dep`, so it must survive eviction.
        let waiting = Uuid::new_v4();
        let blocker = Uuid::new_v4();
        buffer.add(PendingOp {
            id: waiting,
            data: vec![],
            deps: vec![live_dep, blocker],
            received_at: now,
        });

        for _ in 0..10 {
            buffer.mark_applied(Uuid::new_v4());
        }

        assert!(buffer.applied_count() <= 4, "applied set was not bounded");

        // The still-referenced dep survived, so releasing `waiting` only needs
        // its other dependency.
        buffer.mark_applied(blocker);
        let ready = buffer.ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, waiting);
    }

    /// Operation messages carry real deps through to the buffer.
    #[test]
    fn test_create_operation_with_deps_threads_deps() {
        let replica = ReplicaId::new();
        let mut sender = SyncProtocol::new(replica);
        let dep = Uuid::new_v4();
        let op_id = Uuid::new_v4();

        let msg = sender.create_operation_with_deps(op_id, vec![1, 2, 3], vec![dep]);
        match &msg {
            SyncMessage::Operation { deps, .. } => assert_eq!(deps, &vec![dep]),
            _ => panic!("expected Operation"),
        }

        let mut receiver = SyncProtocol::new(ReplicaId::new());
        receiver.handle_message(msg).unwrap();
        // The op is buffered with an unmet dep, so nothing is ready yet.
        assert_eq!(receiver.pending_count(), 1);
        assert!(receiver.ready_operations().is_empty());
        // ops_sent / messages accounting is tracked.
        assert!(sender.stats().messages_sent >= 1);
        assert!(receiver.stats().messages_received >= 1);
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
