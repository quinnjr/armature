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
    /// The character value (None if deleted)
    pub value: Option<char>,
    /// ID of the character this was inserted after
    pub after: CharId,
}

impl CharNode {
    /// Create a new character node
    pub fn new(id: CharId, value: char, after: CharId) -> Self {
        Self {
            id,
            value: Some(value),
            after,
        }
    }

    /// Check if this node is deleted
    pub fn is_deleted(&self) -> bool {
        self.value.is_none()
    }

    /// Delete this node (tombstone)
    pub fn delete(&mut self) {
        self.value = None;
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
    pub fn to_string(&self) -> String {
        self.sequence
            .iter()
            .filter_map(|id| self.nodes.get(id).and_then(|n| n.value))
            .collect()
    }

    /// Get the length of the visible text
    pub fn len(&self) -> usize {
        self.sequence
            .iter()
            .filter(|id| {
                self.nodes
                    .get(id)
                    .map(|n| n.value.is_some())
                    .unwrap_or(false)
            })
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

    /// Insert a string at a position
    pub fn insert_str(&mut self, pos: usize, s: &str) -> Vec<TextOp> {
        let mut ops = Vec::new();
        let mut current_pos = pos;

        for ch in s.chars() {
            ops.push(self.insert(current_pos, ch));
            current_pos += 1;
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

    /// Apply a remote operation
    pub fn apply(&mut self, op: TextOp) {
        match op {
            TextOp::Insert { id, value, after } => {
                // Update clock
                self.clock.merge(&id.timestamp);

                // Skip if already present
                if self.nodes.contains_key(&id) {
                    return;
                }

                let node = CharNode::new(id, value, after);
                self.nodes.insert(id, node);

                // Find correct position
                let insert_pos = self.find_insert_position(after, id);
                self.sequence.insert(insert_pos, id);
            }
            TextOp::Delete { id } => {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.delete();
                }
            }
        }
    }

    /// Get the character ID at a position (including deleted)
    fn id_at_position(&self, pos: usize) -> CharId {
        if pos == 0 {
            return CharId::root();
        }

        let mut visible_count = 0;
        for id in &self.sequence {
            if let Some(node) = self.nodes.get(id) {
                if node.value.is_some() {
                    visible_count += 1;
                    if visible_count == pos {
                        return *id;
                    }
                }
            }
        }

        // If position is past the end, return the last visible ID
        for id in self.sequence.iter().rev() {
            if let Some(node) = self.nodes.get(id) {
                if node.value.is_some() {
                    return *id;
                }
            }
        }

        CharId::root()
    }

    /// Get the visible character ID at a position
    fn visible_id_at_position(&self, pos: usize) -> Option<CharId> {
        let mut visible_count = 0;
        for id in &self.sequence {
            if let Some(node) = self.nodes.get(id) {
                if node.value.is_some() {
                    if visible_count == pos {
                        return Some(*id);
                    }
                    visible_count += 1;
                }
            }
        }
        None
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

    /// Get all operations (for sync)
    pub fn operations(&self) -> Vec<TextOp> {
        self.sequence
            .iter()
            .filter_map(|id| {
                self.nodes.get(id).and_then(|node| {
                    if node.id.is_root() {
                        None
                    } else {
                        Some(TextOp::Insert {
                            id: node.id,
                            value: node.value.unwrap_or('\0'),
                            after: node.after,
                        })
                    }
                })
            })
            .collect()
    }

    /// Get character at position
    pub fn char_at(&self, pos: usize) -> Option<char> {
        let id = self.visible_id_at_position(pos)?;
        self.nodes.get(&id).and_then(|n| n.value)
    }
}

impl Crdt for RgaText {
    fn merge(&mut self, other: &Self) {
        // Apply all operations from other that we don't have
        for (id, node) in &other.nodes {
            if !self.nodes.contains_key(id) {
                self.apply(TextOp::Insert {
                    id: *id,
                    value: node.value.unwrap_or('\0'),
                    after: node.after,
                });
            }

            // Apply tombstones
            if node.is_deleted() {
                if let Some(our_node) = self.nodes.get_mut(id) {
                    our_node.delete();
                }
            }
        }
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
}
