//! Collaborative document abstraction
//!
//! Provides a unified interface for managing collaborative documents
//! with multiple CRDT fields.

use crate::crdt::{Crdt, LwwMap, LwwRegister, OrSet, PnCounter};
use crate::{CollabError, CollabResult, LogicalClock, ReplicaId, VectorClock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[cfg(feature = "text")]
use crate::text::RgaText;

/// Change event for a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChange {
    /// Document ID
    pub doc_id: String,
    /// Field that changed
    pub field: String,
    /// Type of change
    pub change_type: ChangeType,
    /// Replica that made the change
    pub replica: ReplicaId,
    /// Timestamp of the change
    pub timestamp: LogicalClock,
}

/// Type of document change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    /// Field was updated
    Update,
    /// Field was created
    Create,
    /// Field was deleted
    Delete,
}

/// Field value types supported in documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldValue {
    /// String value (LWW)
    String(LwwRegister<String>),
    /// Integer value (LWW)
    Integer(LwwRegister<i64>),
    /// Float value (LWW)
    Float(LwwRegister<f64>),
    /// Boolean value (LWW)
    Boolean(LwwRegister<bool>),
    /// Counter value
    Counter(PnCounter),
    /// Set of strings
    StringSet(OrSet<String>),
    /// Map of string to string
    StringMap(LwwMap<String, String>),
    /// Collaborative text
    #[cfg(feature = "text")]
    Text(RgaText),
}

/// A collaborative document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Document ID
    pub id: String,
    /// Replica ID
    replica: ReplicaId,
    /// Logical clock
    clock: LogicalClock,
    /// Vector clock for causality tracking
    vclock: VectorClock,
    /// Document fields
    fields: HashMap<String, FieldValue>,
    /// Document version
    version: u64,
    /// Created timestamp
    created_at: chrono::DateTime<chrono::Utc>,
    /// Updated timestamp
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl Document {
    /// Create a new document
    pub fn new(id: impl Into<String>) -> Self {
        let replica = ReplicaId::new();
        let now = chrono::Utc::now();

        Self {
            id: id.into(),
            replica,
            clock: LogicalClock::new(0, replica),
            vclock: VectorClock::new(),
            fields: HashMap::new(),
            version: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a document with a specific replica ID
    pub fn with_replica(id: impl Into<String>, replica: ReplicaId) -> Self {
        let now = chrono::Utc::now();

        Self {
            id: id.into(),
            replica,
            clock: LogicalClock::new(0, replica),
            vclock: VectorClock::new(),
            fields: HashMap::new(),
            version: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Get the document ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the replica ID
    pub fn replica(&self) -> ReplicaId {
        self.replica
    }

    /// Get the current version
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Tick the clock and return the new timestamp
    fn tick(&mut self) -> LogicalClock {
        self.vclock.increment(self.replica);
        self.version += 1;
        self.updated_at = chrono::Utc::now();
        self.clock.tick()
    }

    /// Set a string field
    pub fn set_string(&mut self, field: impl Into<String>, value: impl Into<String>) {
        let field = field.into();
        let value = value.into();
        let ts = self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::String(reg)) => {
                reg.set(value, ts);
            }
            _ => {
                self.fields
                    .insert(field, FieldValue::String(LwwRegister::new(value, ts)));
            }
        }
    }

    /// Get a string field
    pub fn get_string(&self, field: &str) -> Option<&str> {
        match self.fields.get(field) {
            Some(FieldValue::String(reg)) => Some(reg.get()),
            _ => None,
        }
    }

    /// Set an integer field
    pub fn set_integer(&mut self, field: impl Into<String>, value: i64) {
        let field = field.into();
        let ts = self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::Integer(reg)) => {
                reg.set(value, ts);
            }
            _ => {
                self.fields
                    .insert(field, FieldValue::Integer(LwwRegister::new(value, ts)));
            }
        }
    }

    /// Get an integer field
    pub fn get_integer(&self, field: &str) -> Option<i64> {
        match self.fields.get(field) {
            Some(FieldValue::Integer(reg)) => Some(*reg.get()),
            _ => None,
        }
    }

    /// Set a boolean field
    pub fn set_boolean(&mut self, field: impl Into<String>, value: bool) {
        let field = field.into();
        let ts = self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::Boolean(reg)) => {
                reg.set(value, ts);
            }
            _ => {
                self.fields
                    .insert(field, FieldValue::Boolean(LwwRegister::new(value, ts)));
            }
        }
    }

    /// Get a boolean field
    pub fn get_boolean(&self, field: &str) -> Option<bool> {
        match self.fields.get(field) {
            Some(FieldValue::Boolean(reg)) => Some(*reg.get()),
            _ => None,
        }
    }

    /// Increment a counter field
    pub fn increment(&mut self, field: impl Into<String>) {
        let field = field.into();
        self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::Counter(counter)) => {
                counter.increment(self.replica);
            }
            _ => {
                let mut counter = PnCounter::new();
                counter.increment(self.replica);
                self.fields.insert(field, FieldValue::Counter(counter));
            }
        }
    }

    /// Decrement a counter field
    pub fn decrement(&mut self, field: impl Into<String>) {
        let field = field.into();
        self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::Counter(counter)) => {
                counter.decrement(self.replica);
            }
            _ => {
                let mut counter = PnCounter::new();
                counter.decrement(self.replica);
                self.fields.insert(field, FieldValue::Counter(counter));
            }
        }
    }

    /// Get a counter value
    pub fn get_counter(&self, field: &str) -> i64 {
        match self.fields.get(field) {
            Some(FieldValue::Counter(counter)) => counter.value(),
            _ => 0,
        }
    }

    /// Add to a set field
    pub fn add_to_set(&mut self, field: impl Into<String>, value: impl Into<String>) {
        let field = field.into();
        let value = value.into();
        let ts = self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::StringSet(set)) => {
                set.add(value, ts);
            }
            _ => {
                let mut set = OrSet::new();
                set.add(value, ts);
                self.fields.insert(field, FieldValue::StringSet(set));
            }
        }
    }

    /// Remove from a set field
    pub fn remove_from_set(&mut self, field: &str, value: &str) {
        self.tick();
        if let Some(FieldValue::StringSet(set)) = self.fields.get_mut(field) {
            set.remove(&value.to_string());
        }
    }

    /// Check if a set contains a value
    pub fn set_contains(&self, field: &str, value: &str) -> bool {
        match self.fields.get(field) {
            Some(FieldValue::StringSet(set)) => set.contains(&value.to_string()),
            _ => false,
        }
    }

    /// Get all values from a set
    pub fn get_set(&self, field: &str) -> Vec<String> {
        match self.fields.get(field) {
            Some(FieldValue::StringSet(set)) => set.elements().cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// Set a map entry
    pub fn set_map_entry(
        &mut self,
        field: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        let field = field.into();
        let key = key.into();
        let value = value.into();
        let ts = self.tick();

        match self.fields.get_mut(&field) {
            Some(FieldValue::StringMap(map)) => {
                map.set(key, value, ts);
            }
            _ => {
                let mut map = LwwMap::new();
                map.set(key, value, ts);
                self.fields.insert(field, FieldValue::StringMap(map));
            }
        }
    }

    /// Get a map entry
    pub fn get_map_entry(&self, field: &str, key: &str) -> Option<&str> {
        match self.fields.get(field) {
            Some(FieldValue::StringMap(map)) => map.get(&key.to_string()).map(|s| s.as_str()),
            _ => None,
        }
    }

    /// Create or get a text field
    #[cfg(feature = "text")]
    pub fn text(&mut self, field: impl Into<String>) -> &mut RgaText {
        let field = field.into();

        if !matches!(self.fields.get(&field), Some(FieldValue::Text(_))) {
            let text = RgaText::new(self.replica);
            self.fields.insert(field.clone(), FieldValue::Text(text));
        }

        match self.fields.get_mut(&field) {
            Some(FieldValue::Text(text)) => text,
            _ => unreachable!(),
        }
    }

    /// Get text content
    #[cfg(feature = "text")]
    pub fn get_text(&self, field: &str) -> Option<String> {
        match self.fields.get(field) {
            Some(FieldValue::Text(text)) => Some(text.to_string()),
            _ => None,
        }
    }

    /// Merge with another document
    pub fn merge(&mut self, other: &Self) {
        self.vclock.merge(&other.vclock);
        self.clock.merge(&other.clock);

        for (field, value) in &other.fields {
            match (self.fields.get_mut(field), value) {
                (Some(FieldValue::String(a)), FieldValue::String(b)) => a.merge(b),
                (Some(FieldValue::Integer(a)), FieldValue::Integer(b)) => a.merge(b),
                (Some(FieldValue::Float(a)), FieldValue::Float(b)) => a.merge(b),
                (Some(FieldValue::Boolean(a)), FieldValue::Boolean(b)) => a.merge(b),
                (Some(FieldValue::Counter(a)), FieldValue::Counter(b)) => a.merge(b),
                (Some(FieldValue::StringSet(a)), FieldValue::StringSet(b)) => a.merge(b),
                (Some(FieldValue::StringMap(a)), FieldValue::StringMap(b)) => a.merge(b),
                #[cfg(feature = "text")]
                (Some(FieldValue::Text(a)), FieldValue::Text(b)) => a.merge(b),
                (None, _) => {
                    self.fields.insert(field.clone(), value.clone());
                }
                _ => {} // Type mismatch, skip
            }
        }

        self.version = self.version.max(other.version);
        if other.updated_at > self.updated_at {
            self.updated_at = other.updated_at;
        }
    }

    /// Get all field names
    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.fields.keys().map(|s| s.as_str())
    }

    /// Check if a field exists
    pub fn has_field(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// Serialize the document to JSON
    pub fn to_json(&self) -> CollabResult<String> {
        serde_json::to_string(self).map_err(CollabError::from)
    }

    /// Deserialize a document from JSON
    pub fn from_json(json: &str) -> CollabResult<Self> {
        serde_json::from_str(json).map_err(CollabError::from)
    }
}

/// Shared document handle for concurrent access
pub type SharedDocument = Arc<RwLock<Document>>;

/// Create a shared document
pub fn shared_document(doc: Document) -> SharedDocument {
    Arc::new(RwLock::new(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_string_fields() {
        let mut doc = Document::new("test-doc");

        doc.set_string("name", "Alice");
        assert_eq!(doc.get_string("name"), Some("Alice"));

        doc.set_string("name", "Bob");
        assert_eq!(doc.get_string("name"), Some("Bob"));
    }

    #[test]
    fn test_document_counter() {
        let mut doc = Document::new("test-doc");

        doc.increment("count");
        doc.increment("count");
        doc.decrement("count");

        assert_eq!(doc.get_counter("count"), 1);
    }

    #[test]
    fn test_document_set() {
        let mut doc = Document::new("test-doc");

        doc.add_to_set("tags", "rust");
        doc.add_to_set("tags", "crdt");

        assert!(doc.set_contains("tags", "rust"));
        assert!(doc.set_contains("tags", "crdt"));
        assert!(!doc.set_contains("tags", "python"));

        doc.remove_from_set("tags", "rust");
        assert!(!doc.set_contains("tags", "rust"));
    }

    #[test]
    fn test_document_merge() {
        let mut doc1 = Document::new("test-doc");
        let mut doc2 = Document::with_replica("test-doc", ReplicaId::new());

        doc1.set_string("field1", "value1");
        doc2.set_string("field2", "value2");

        doc1.merge(&doc2);

        assert_eq!(doc1.get_string("field1"), Some("value1"));
        assert_eq!(doc1.get_string("field2"), Some("value2"));
    }

    #[cfg(feature = "text")]
    #[test]
    fn test_document_text() {
        let mut doc = Document::new("test-doc");

        doc.text("content").insert_str(0, "Hello, World!");

        assert_eq!(doc.get_text("content"), Some("Hello, World!".to_string()));
    }
}
