//! Text CRDT for collaborative text editing
//!
//! Implements RGA (Replicated Growable Array) for collaborative text editing.
//! RGA provides strong consistency guarantees and preserves user intentions
//! during concurrent edits.

use crate::{Crdt, LogicalClock, ReplicaId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a character in the text
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CharId {
    /// Logical timestamp when the character was inserted
    pub timestamp: LogicalClock,
    /// Unique ID for disambiguation
    pub uuid: Uuid,
}

impl CharId {
    /// Create a new character ID
    pub fn new(timestamp: LogicalClock) -> Self {
        Self {
            timestamp,
            uuid: Uuid::new_v4(),
        }
    }

    /// Special ID for the beginning of the document
    pub fn root() -> Self {
        Self {
            timestamp: LogicalClock::new(0, ReplicaId::from_uuid(Uuid::nil())),
            uuid: Uuid::nil(),
        }
    }

    /// Check if this is the root ID
    pub fn is_root(&self) -> bool {
        self.uuid == Uuid::nil()
    }
}

impl PartialOrd for CharId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CharId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.timestamp
            .cmp(&other.timestamp)
            .then_with(|| self.uuid.cmp(&other.uuid))
    }
}

/// A character node in the RGA
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharNode {
    /// Character ID
    pub id: CharId,
    /// The character value.
    ///
    /// This is `None` only for the synthetic root node. Deleting a character
    /// sets the [`deleted`](Self::deleted) tombstone flag but *preserves* the
    /// original value so it can be replayed faithfully during sync — a deleted
    /// character must never be exported as `Insert '\0'`.
    pub value: Option<char>,
    /// ID of the character this was inserted after
    pub after: CharId,
    /// Tombstone flag: `true` once the character has been deleted.
    #[serde(default)]
    pub deleted: bool,
}

impl CharNode {
    /// Create a new character node
    pub fn new(id: CharId, value: char, after: CharId) -> Self {
        Self {
            id,
            value: Some(value),
            after,
            deleted: false,
        }
    }

    /// Check if this node is deleted (tombstoned)
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// Check if this node is visible (has content and is not tombstoned)
    pub fn is_visible(&self) -> bool {
        !self.deleted && self.value.is_some()
    }

    /// Delete this node (tombstone). The original value is preserved so the
    /// deletion can be replayed as a real `Delete` op rather than losing data.
    pub fn delete(&mut self) {
        self.deleted = true;
    }
}

/// Text operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TextOp {
    /// Insert a character after a given position
    Insert {
        id: CharId,
        value: char,
        after: CharId,
    },
    /// Delete a character
    Delete { id: CharId },
}

/// RGA Text CRDT
///
/// A replicated growable array for collaborative text editing.
/// Supports insert and delete operations with strong consistency.
///
/// # Example
///
/// ```rust,ignore
/// use armature_collab::{RgaText, ReplicaId, LogicalClock};
///
/// let replica = ReplicaId::new();
/// let mut clock = LogicalClock::new(0, replica);
///
/// let mut text = RgaText::new(replica);
///
/// // Insert "Hello"
/// text.insert(0, 'H');
/// text.insert(1, 'e');
/// text.insert(2, 'l');
/// text.insert(3, 'l');
/// text.insert(4, 'o');
///
/// assert_eq!(text.to_string(), "Hello");
///
/// // Delete 'e'
/// text.delete(1);
/// assert_eq!(text.to_string(), "Hllo");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RgaText {
    /// Replica ID for this instance
    replica: ReplicaId,
    /// Logical clock
    clock: LogicalClock,
    /// All nodes indexed by their ID
    nodes: HashMap<CharId, CharNode>,
    /// Ordered list of character IDs (for traversal)
    sequence: Vec<CharId>,
}

impl RgaText {
    /// Create a new RGA text
    pub fn new(replica: ReplicaId) -> Self {
        let mut nodes = HashMap::new();
        let root = CharNode {
            id: CharId::root(),
            value: None,
            after: CharId::root(),
            deleted: false,
        };
        nodes.insert(CharId::root(), root);

        Self {
            replica,
            clock: LogicalClock::new(0, replica),
            nodes,
            sequence: vec![CharId::root()],
        }
    }

    /// Get the current text as a string
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.sequence
            .iter()
            .filter_map(|id| {
                self.nodes
                    .get(id)
                    .filter(|n| n.is_visible())
                    .and_then(|n| n.value)
            })
            .collect()
    }

    /// Get the length of the visible text
    pub fn len(&self) -> usize {
        self.sequence
            .iter()
            .filter(|id| self.nodes.get(id).map(|n| n.is_visible()).unwrap_or(false))
            .count()
    }

    /// Check if the text is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Insert a character at a position
    pub fn insert(&mut self, pos: usize, ch: char) -> TextOp {
        let after_id = self.id_at_position(pos);
        let id = CharId::new(self.clock.tick());

        let node = CharNode::new(id, ch, after_id);
        self.nodes.insert(id, node);

        // Find insertion point in sequence
        let insert_pos = self.find_insert_position(after_id, id);
        self.sequence.insert(insert_pos, id);

        TextOp::Insert {
            id,
            value: ch,
            after: after_id,
        }
    }

    /// Insert a string at a position.
    ///
    /// Rather than calling [`insert`](Self::insert) per character — which would
    /// re-scan the sequence by visible position on every codepoint (quadratic in
    /// the string length) — this resolves the anchor once and threads each new
    /// character directly after the previous one, since the run is contiguous.
    pub fn insert_str(&mut self, pos: usize, s: &str) -> Vec<TextOp> {
        let mut ops = Vec::with_capacity(s.len());
        if s.is_empty() {
            return ops;
        }

        // Resolve the anchor character and its index in the sequence exactly
        // once; subsequent inserts chain off the previously inserted node.
        let mut after_id = self.id_at_position(pos);
        let mut seq_idx = self
            .sequence
            .iter()
            .position(|&id| id == after_id)
            .unwrap_or(0);

        for (i, ch) in s.chars().enumerate() {
            let id = CharId::new(self.clock.tick());
            self.nodes.insert(id, CharNode::new(id, ch, after_id));

            // The first character honors concurrent-sibling tie-breaking; every
            // following character in the run goes immediately after its
            // predecessor (no other node shares that `after` yet).
            let insert_pos = if i == 0 {
                self.find_insert_position(after_id, id)
            } else {
                seq_idx + 1
            };
            self.sequence.insert(insert_pos, id);

            ops.push(TextOp::Insert {
                id,
                value: ch,
                after: after_id,
            });

            after_id = id;
            seq_idx = insert_pos;
        }

        ops
    }

    /// Delete a character at a position
    pub fn delete(&mut self, pos: usize) -> Option<TextOp> {
        let id = self.visible_id_at_position(pos)?;

        if let Some(node) = self.nodes.get_mut(&id) {
            node.delete();
            Some(TextOp::Delete { id })
        } else {
            None
        }
    }

    /// Delete a range of characters
    pub fn delete_range(&mut self, start: usize, len: usize) -> Vec<TextOp> {
        let mut ops = Vec::new();

        // Delete from end to start to maintain positions
        for i in (0..len).rev() {
            if let Some(op) = self.delete(start + i) {
                ops.push(op);
            }
        }

        ops
    }

    /// Apply a remote operation.
    ///
    /// Delegates to [`apply_many`](Self::apply_many) with a single op so that
    /// the out-of-order handling (a `Delete` whose target `Insert` has not yet
    /// arrived) and the deterministic rebuild are shared with the batch path.
    pub fn apply(&mut self, op: TextOp) {
        self.apply_many(std::iter::once(op));
    }

    /// Apply a batch of remote operations, rebuilding the sequence **once**.
    ///
    /// The single-op [`apply`](Self::apply) rebuilds the whole sequence per
    /// remote insert, so replaying an N-character log is O(N² log N). Because
    /// the linearization is a pure function of the node set, we can mutate the
    /// node set for every op in the batch and rebuild exactly once at the end —
    /// same converged result, O(N log N) total.
    ///
    /// Out-of-order delivery is handled: a `Delete` for an id we have not seen
    /// yet pre-creates a tombstoned placeholder node (value `None`, `deleted`),
    /// and a later `Insert` for that id fills in its real value and anchor while
    /// *keeping it deleted* — so a resurrected character can never become
    /// visible.
    pub fn apply_many(&mut self, ops: impl IntoIterator<Item = TextOp>) {
        let mut node_set_changed = false;

        for op in ops {
            match op {
                TextOp::Insert { id, value, after } => {
                    // Update clock
                    self.clock.merge(&id.timestamp);

                    if let Some(existing) = self.nodes.get_mut(&id) {
                        // A tombstoned placeholder pre-created by an
                        // out-of-order `Delete`: fill in the real value/anchor
                        // but keep the tombstone so it stays invisible.
                        if existing.value.is_none() && existing.deleted {
                            existing.value = Some(value);
                            existing.after = after;
                            node_set_changed = true;
                        }
                        // Otherwise already present — idempotent, skip.
                        continue;
                    }

                    self.nodes.insert(id, CharNode::new(id, value, after));
                    node_set_changed = true;
                }
                TextOp::Delete { id } => {
                    if let Some(node) = self.nodes.get_mut(&id) {
                        node.delete();
                    } else {
                        // Delete arrived before its Insert: record the deletion
                        // as a tombstoned placeholder so the later Insert merges
                        // into it and the character never resurrects.
                        self.nodes.insert(
                            id,
                            CharNode {
                                id,
                                value: None,
                                after: CharId::root(),
                                deleted: true,
                            },
                        );
                        node_set_changed = true;
                    }
                }
            }
        }

        // Tombstoning an existing node only flips a flag (order is unchanged),
        // so a rebuild is only needed when the node set itself grew.
        if node_set_changed {
            self.rebuild_sequence();
        }
    }

    /// Get the character ID at a position (including deleted)
    fn id_at_position(&self, pos: usize) -> CharId {
        if pos == 0 {
            return CharId::root();
        }

        let mut visible_count = 0;
        for id in &self.sequence {
            if let Some(node) = self.nodes.get(id)
                && node.is_visible()
            {
                visible_count += 1;
                if visible_count == pos {
                    return *id;
                }
            }
        }

        // If position is past the end, return the last visible ID
        for id in self.sequence.iter().rev() {
            if let Some(node) = self.nodes.get(id)
                && node.is_visible()
            {
                return *id;
            }
        }

        CharId::root()
    }

    /// Get the visible character ID at a position
    fn visible_id_at_position(&self, pos: usize) -> Option<CharId> {
        let mut visible_count = 0;
        for id in &self.sequence {
            if let Some(node) = self.nodes.get(id)
                && node.is_visible()
            {
                if visible_count == pos {
                    return Some(*id);
                }
                visible_count += 1;
            }
        }
        None
    }

    /// Rebuild the ordered `sequence` deterministically from the node set.
    ///
    /// The RGA linearization is a pre-order walk of the insertion tree (each
    /// node is a child of its `after` anchor) with siblings ordered by
    /// descending [`CharId`] ("higher id wins"). Because this is a pure function
    /// of the node set, two replicas that hold the same nodes always produce the
    /// identical sequence — the property incremental placement failed to
    /// guarantee across concurrent multi-character runs.
    fn rebuild_sequence(&mut self) {
        let mut children: HashMap<CharId, Vec<CharId>> = HashMap::new();
        for node in self.nodes.values() {
            if node.id.is_root() {
                continue;
            }
            children.entry(node.after).or_default().push(node.id);
        }
        for kids in children.values_mut() {
            // Descending: higher id first (preorder precedence).
            kids.sort_unstable_by(|a, b| b.cmp(a));
        }

        let mut sequence = Vec::with_capacity(self.nodes.len());
        let mut stack = vec![CharId::root()];
        while let Some(id) = stack.pop() {
            sequence.push(id);
            if let Some(kids) = children.get(&id) {
                // Push ascending so the highest child is popped (visited) first.
                for &child in kids.iter().rev() {
                    stack.push(child);
                }
            }
        }

        // Defensive: place any orphaned nodes (anchor unreachable from root)
        // deterministically so nothing is silently dropped.
        if sequence.len() < self.nodes.len() {
            let present: std::collections::HashSet<CharId> = sequence.iter().copied().collect();
            let mut orphans: Vec<CharId> = self
                .nodes
                .keys()
                .copied()
                .filter(|id| !present.contains(id))
                .collect();
            orphans.sort_unstable();
            sequence.extend(orphans);
        }

        self.sequence = sequence;
    }

    /// Find the correct insert position for a new character
    fn find_insert_position(&self, after: CharId, new_id: CharId) -> usize {
        let after_pos = self
            .sequence
            .iter()
            .position(|&id| id == after)
            .unwrap_or(0);

        // Find the first position where we should insert (after all concurrent inserts at same position)
        let mut insert_pos = after_pos + 1;

        while insert_pos < self.sequence.len() {
            let existing_id = self.sequence[insert_pos];
            if let Some(existing_node) = self.nodes.get(&existing_id) {
                // If existing node was also inserted after the same position
                if existing_node.after == after {
                    // Higher ID wins (insert before lower ID)
                    if existing_id < new_id {
                        break;
                    }
                    insert_pos += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        insert_pos
    }

    /// Get all operations (for sync).
    ///
    /// Emits an `Insert` for every non-root character in sequence order carrying
    /// its *real* value (never a `'\0'` placeholder), followed by a `Delete` for
    /// each tombstoned character. A replaying peer therefore reconstructs the
    /// exact document — a deleted character replays as a delete, not as a
    /// resurrected NUL character.
    pub fn operations(&self) -> Vec<TextOp> {
        let mut ops = Vec::with_capacity(self.sequence.len());

        // Inserts first, in sequence order, so every `after` anchor precedes its
        // dependents.
        for id in &self.sequence {
            if let Some(node) = self.nodes.get(id)
                && !node.id.is_root()
                && let Some(value) = node.value
            {
                ops.push(TextOp::Insert {
                    id: node.id,
                    value,
                    after: node.after,
                });
            }
        }

        // Then a real Delete for every tombstone.
        for id in &self.sequence {
            if let Some(node) = self.nodes.get(id)
                && !node.id.is_root()
                && node.is_deleted()
            {
                ops.push(TextOp::Delete { id: node.id });
            }
        }

        ops
    }

    /// Get character at position
    pub fn char_at(&self, pos: usize) -> Option<char> {
        let id = self.visible_id_at_position(pos)?;
        self.nodes
            .get(&id)
            .filter(|n| n.is_visible())
            .and_then(|n| n.value)
    }
}

impl Crdt for RgaText {
    fn merge(&mut self, other: &Self) {
        // Iterating `other.nodes` (a HashMap) in its arbitrary order and placing
        // each node incrementally is non-deterministic and, worse, incorrect:
        // incremental placement fails to skip an anchor's whole subtree, so two
        // replicas with the same nodes can diverge (e.g. concurrent multi-char
        // runs interleave differently).
        //
        // Instead we merge the node sets — taking every missing node and
        // propagating tombstones — then recompute the sequence with a single
        // deterministic tree walk. Identical node sets therefore always yield an
        // identical document, independent of merge direction or HashMap order.
        self.clock.merge(&other.clock);

        for (id, node) in &other.nodes {
            if node.id.is_root() {
                continue;
            }
            self.nodes.entry(*id).or_insert_with(|| CharNode {
                id: node.id,
                value: node.value,
                after: node.after,
                deleted: node.deleted,
            });
            // Propagate the tombstone for nodes we already held.
            if node.is_deleted()
                && let Some(our_node) = self.nodes.get_mut(id)
            {
                our_node.deleted = true;
            }
        }

        self.rebuild_sequence();
    }
}

/// Cursor position in collaborative text
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TextCursor {
    /// Character ID the cursor is after
    pub after: CharId,
    /// Visual offset from start
    pub offset: usize,
}

impl TextCursor {
    /// Create a cursor at a position
    pub fn at(offset: usize, text: &RgaText) -> Self {
        let after = if offset == 0 {
            CharId::root()
        } else {
            text.visible_id_at_position(offset - 1)
                .unwrap_or(CharId::root())
        };

        Self { after, offset }
    }

    /// Move cursor left
    pub fn move_left(&mut self, text: &RgaText) {
        if self.offset > 0 {
            self.offset -= 1;
            self.after = if self.offset == 0 {
                CharId::root()
            } else {
                text.visible_id_at_position(self.offset - 1)
                    .unwrap_or(CharId::root())
            };
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self, text: &RgaText) {
        if self.offset < text.len() {
            self.offset += 1;
            self.after = text
                .visible_id_at_position(self.offset - 1)
                .unwrap_or(CharId::root());
        }
    }
}

/// Text selection in collaborative text
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TextSelection {
    /// Anchor position
    pub anchor: usize,
    /// Focus (cursor) position
    pub focus: usize,
}

impl TextSelection {
    /// Create a collapsed selection (cursor)
    pub fn cursor(pos: usize) -> Self {
        Self {
            anchor: pos,
            focus: pos,
        }
    }

    /// Create a selection range
    pub fn range(start: usize, end: usize) -> Self {
        Self {
            anchor: start,
            focus: end,
        }
    }

    /// Check if selection is collapsed (cursor)
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// Get the start of the selection
    pub fn start(&self) -> usize {
        self.anchor.min(self.focus)
    }

    /// Get the end of the selection
    pub fn end(&self) -> usize {
        self.anchor.max(self.focus)
    }

    /// Get selection length
    pub fn len(&self) -> usize {
        self.end() - self.start()
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.is_collapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rga_insert() {
        let replica = ReplicaId::new();
        let mut text = RgaText::new(replica);

        text.insert(0, 'H');
        text.insert(1, 'i');

        assert_eq!(text.to_string(), "Hi");
    }

    #[test]
    fn test_rga_delete() {
        let replica = ReplicaId::new();
        let mut text = RgaText::new(replica);

        text.insert(0, 'H');
        text.insert(1, 'e');
        text.insert(2, 'y');

        assert_eq!(text.to_string(), "Hey");

        text.delete(1); // Delete 'e'
        assert_eq!(text.to_string(), "Hy");
    }

    #[test]
    fn test_rga_merge() {
        let replica1 = ReplicaId::new();
        let replica2 = ReplicaId::new();

        let mut text1 = RgaText::new(replica1);
        let mut text2 = RgaText::new(replica2);

        text1.insert(0, 'A');
        text2.insert(0, 'B');

        text1.merge(&text2);
        text2.merge(&text1);

        // Both should converge to the same state
        assert_eq!(text1.to_string(), text2.to_string());
        assert_eq!(text1.len(), 2);
    }

    #[test]
    fn test_rga_concurrent_insert() {
        let replica1 = ReplicaId::new();
        let replica2 = ReplicaId::new();

        let mut text1 = RgaText::new(replica1);
        let mut text2 = RgaText::new(replica2);

        // Both insert at position 0
        let op1 = text1.insert(0, 'X');
        let op2 = text2.insert(0, 'Y');

        // Apply each other's operations
        text1.apply(op2);
        text2.apply(op1);

        // Should converge
        assert_eq!(text1.to_string(), text2.to_string());
    }

    #[test]
    fn test_text_cursor() {
        let replica = ReplicaId::new();
        let mut text = RgaText::new(replica);

        text.insert_str(0, "Hello");

        let mut cursor = TextCursor::at(2, &text);
        assert_eq!(cursor.offset, 2);

        cursor.move_right(&text);
        assert_eq!(cursor.offset, 3);

        cursor.move_left(&text);
        assert_eq!(cursor.offset, 2);
    }

    #[test]
    fn test_text_selection() {
        let sel = TextSelection::range(2, 5);
        assert_eq!(sel.start(), 2);
        assert_eq!(sel.end(), 5);
        assert_eq!(sel.len(), 3);
        assert!(!sel.is_collapsed());
    }

    // --- Regression tests (Workflow 8 · A10) ---

    /// A tombstoned character must replay as a real `Delete`, never as an
    /// `Insert '\0'` that resurrects deleted text on a peer.
    #[test]
    fn test_operations_delete_replays_as_delete() {
        let replica = ReplicaId::new();
        let mut text = RgaText::new(replica);
        text.insert_str(0, "Hello");
        text.delete(1); // delete 'e' -> "Hllo"
        assert_eq!(text.to_string(), "Hllo");

        // Replay the exported operation log onto a fresh peer.
        let ops = text.operations();
        // No insert in the log may carry a NUL placeholder.
        for op in &ops {
            if let TextOp::Insert { value, .. } = op {
                assert_ne!(*value, '\0', "tombstone exported as Insert '\\0'");
            }
        }
        // A real Delete must be present.
        assert!(
            ops.iter().any(|op| matches!(op, TextOp::Delete { .. })),
            "no Delete op emitted for the tombstone"
        );

        let mut peer = RgaText::new(ReplicaId::new());
        for op in ops {
            peer.apply(op);
        }
        assert_eq!(peer.to_string(), "Hllo");
        assert!(!peer.to_string().contains('\0'));
    }

    /// Two replicas applying a multi-character interleave concurrently must
    /// converge to an identical document regardless of merge direction.
    #[test]
    fn test_merge_converges_multichar_interleave() {
        let r1 = ReplicaId::new();
        let r2 = ReplicaId::new();
        let mut a = RgaText::new(r1);
        let mut b = RgaText::new(r2);

        // Concurrent multi-character edits at the same position.
        a.insert_str(0, "abc");
        b.insert_str(0, "xyz");

        // Merge both directions; both must reach the same state.
        let a_before = a.clone();
        a.merge(&b);
        b.merge(&a_before);

        assert_eq!(
            a.to_string(),
            b.to_string(),
            "replicas diverged after interleaved merge"
        );
        assert_eq!(a.len(), 6);
    }

    /// Merge must be deterministic regardless of HashMap iteration order.
    ///
    /// "Hello" and "World" are authored concurrently at the same anchor, so an
    /// order-dependent (incremental-placement) merge interleaves them
    /// differently depending on which order it walks the peer's nodes. We build
    /// the peer several times by applying the SAME insert ops in DISTINCT orders
    /// — identical node set, but distinct `HashMap`s whose iteration orders
    /// genuinely vary (std `RandomState` seeds each map independently) — and
    /// also merge in both directions. A merge that leaks iteration order would
    /// diverge across these; the deterministic tree-walk merge must not.
    #[test]
    fn test_merge_is_deterministic() {
        let r1 = ReplicaId::new();
        let r2 = ReplicaId::new();

        let mut base = RgaText::new(r1);
        base.insert_str(0, "Hello");

        // Capture the peer's insert ops once, then replay them in many orders.
        let mut origin = RgaText::new(r2);
        let peer_ops = origin.insert_str(0, "World");
        assert_eq!(origin.to_string(), "World");

        // Distinct application orders of the same ops: every rotation plus the
        // reverse. Each rebuild yields the identical node set in a fresh map.
        let mut orderings: Vec<Vec<TextOp>> = Vec::new();
        for k in 0..peer_ops.len() {
            let mut o = peer_ops.clone();
            o.rotate_left(k);
            orderings.push(o);
        }
        let mut rev = peer_ops.clone();
        rev.reverse();
        orderings.push(rev);

        let mut results = Vec::new();
        for ord in &orderings {
            // Rebuild the peer from this application order.
            let mut peer = RgaText::new(r2);
            for op in ord.iter().cloned() {
                peer.apply(op);
            }
            // Same logical content reached via every order.
            assert_eq!(peer.to_string(), "World");

            // Merge both directions; both must land on the same string.
            let mut lhs = base.clone();
            lhs.merge(&peer);
            results.push(lhs.to_string());

            let mut rhs = peer.clone();
            rhs.merge(&base);
            results.push(rhs.to_string());
        }

        let first = &results[0];
        for r in &results {
            assert_eq!(
                r, first,
                "merge produced HashMap-iteration-order-dependent output"
            );
        }
        // All ten characters must survive in every result.
        assert_eq!(first.chars().count(), 10);
    }

    /// Op-based delivery may deliver a `Delete` before the `Insert` it targets.
    /// The tombstone must survive so the later `Insert` does NOT resurrect the
    /// character. (Fails against the old code, which dropped the Delete.)
    #[test]
    fn test_apply_delete_before_insert_does_not_resurrect() {
        // Author a character on one replica to get a concrete Insert op.
        let author = ReplicaId::new();
        let mut src = RgaText::new(author);
        let insert_op = src.insert(0, 'Z');
        let id = match insert_op {
            TextOp::Insert { id, .. } => id,
            _ => unreachable!(),
        };
        let delete_op = TextOp::Delete { id };

        // Peer receives them OUT OF ORDER: Delete first, then Insert.
        let mut peer = RgaText::new(ReplicaId::new());
        peer.apply(delete_op);
        peer.apply(insert_op);

        assert!(
            !peer.to_string().contains('Z'),
            "deleted character resurrected after out-of-order Delete/Insert"
        );
        assert_eq!(peer.len(), 0);

        // And the tombstone must replay faithfully to a third peer.
        let mut third = RgaText::new(ReplicaId::new());
        third.apply_many(peer.operations());
        assert!(!third.to_string().contains('Z'));
        assert_eq!(third.len(), 0);
    }

    /// `apply_many` replaying an `operations()` log yields the identical
    /// document that per-op `apply` would, but rebuilds the sequence once.
    #[test]
    fn test_apply_many_matches_per_op_apply() {
        let author = ReplicaId::new();
        let mut src = RgaText::new(author);
        src.insert_str(0, "Hello, world");
        src.delete(5); // drop the comma
        let expected = src.to_string();
        let ops = src.operations();

        // Per-op replay.
        let mut a = RgaText::new(ReplicaId::new());
        for op in ops.clone() {
            a.apply(op);
        }

        // Batch replay.
        let mut b = RgaText::new(ReplicaId::new());
        b.apply_many(ops);

        assert_eq!(a.to_string(), expected);
        assert_eq!(b.to_string(), expected);
        assert_eq!(a.to_string(), b.to_string());
    }

    /// Deleting a character must preserve it as a real tombstone, never
    /// re-export it as an `Insert '\0'` placeholder that resurrects on a peer.
    ///
    /// Asserting on a merged `to_string()` does NOT catch the resurrection bug:
    /// even the buggy path re-inserts the deleted node as `'\0'` and then
    /// tombstones it, so `to_string()` hides it. The bug is deterministically
    /// visible only in the exported op-log, so we assert there.
    #[test]
    fn test_merge_preserves_deletes() {
        let r1 = ReplicaId::new();
        let mut a = RgaText::new(r1);
        a.insert_str(0, "Hello");
        let del_op = a.delete(0).expect("delete should return an op"); // -> "ello"
        let deleted_id = match del_op {
            TextOp::Delete { id } => id,
            _ => unreachable!("delete must return a Delete op"),
        };
        assert_eq!(a.to_string(), "ello");

        // The exported log must carry a REAL Delete for the tombstoned id and
        // must never re-insert that character as a NUL placeholder.
        let ops = a.operations();
        assert!(
            ops.iter()
                .any(|op| matches!(op, TextOp::Delete { id } if *id == deleted_id)),
            "no real Delete emitted for the tombstoned character"
        );
        for op in &ops {
            if let TextOp::Insert { value, .. } = op {
                assert_ne!(*value, '\0', "tombstone re-exported as Insert '\\0'");
            }
        }

        // Replaying that log onto a peer preserves the deletion faithfully.
        let mut b = RgaText::new(ReplicaId::new());
        b.apply_many(ops);
        assert_eq!(b.to_string(), "ello");
        assert!(!b.to_string().contains('\0'));
    }
}
