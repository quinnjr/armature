//! Audit log storage backends

use crate::AuditEvent;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

/// Audit log storage backend trait
#[async_trait]
pub trait AuditBackend: Send + Sync {
    /// Write an audit event
    async fn write(&self, event: &AuditEvent) -> Result<(), AuditBackendError>;

    /// Flush any pending writes
    async fn flush(&self) -> Result<(), AuditBackendError>;

    /// Read events (if supported)
    async fn read(&self, _limit: usize) -> Result<Vec<AuditEvent>, AuditBackendError> {
        Err(AuditBackendError::NotSupported)
    }

    /// Delete old events (for retention)
    async fn delete_before(
        &self,
        _timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, AuditBackendError> {
        Err(AuditBackendError::NotSupported)
    }
}

/// Audit backend errors
#[derive(Debug, thiserror::Error)]
pub enum AuditBackendError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Operation not supported")]
    NotSupported,

    #[error("Backend error: {0}")]
    Other(String),
}

/// File-based audit backend
///
/// Writes audit events to a file, one JSON object per line.
///
/// The append file handle is opened lazily on the first write and **reused**
/// across subsequent events instead of being reopened for every event, so a
/// high-volume audit stream no longer pays an `open(2)` syscall per event.
pub struct FileBackend {
    path: PathBuf,
    /// Lazily-opened, reused buffered append handle. `None` until the first
    /// write (or after a retention rewrite, which must reopen at the new EOF).
    writer: Mutex<Option<BufWriter<tokio::fs::File>>>,
    /// Number of times the underlying file has been opened; used by tests to
    /// prove the handle is reused across events rather than reopened.
    opens: AtomicUsize,
}

impl FileBackend {
    /// Create a new file backend
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_audit::*;
    ///
    /// let backend = FileBackend::new("audit.log");
    /// ```
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            writer: Mutex::new(None),
            opens: AtomicUsize::new(0),
        }
    }

    /// Number of times the append file handle has been opened.
    ///
    /// A reused handle keeps this at 1 across many writes; per-event reopening
    /// would grow it once per event.
    #[cfg(test)]
    pub fn open_count(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AuditBackend for FileBackend {
    async fn write(&self, event: &AuditEvent) -> Result<(), AuditBackendError> {
        let json = event.to_json()?;

        let mut guard = self.writer.lock().await;
        if guard.is_none() {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .await?;
            self.opens.fetch_add(1, Ordering::SeqCst);
            *guard = Some(BufWriter::new(file));
        }

        // Reuse the open handle for the lifetime of this backend.
        let writer = guard.as_mut().expect("writer initialized above");
        writer.write_all(json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        // Flush the buffer through to the file so events are durable without
        // reopening the file per event.
        writer.flush().await?;

        Ok(())
    }

    async fn flush(&self) -> Result<(), AuditBackendError> {
        let mut guard = self.writer.lock().await;
        if let Some(writer) = guard.as_mut() {
            writer.flush().await?;
        }
        Ok(())
    }

    async fn read(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditBackendError> {
        // Ensure buffered data is on disk before reading.
        self.flush().await?;

        let content = match tokio::fs::read_to_string(&self.path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let events: Vec<AuditEvent> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<AuditEvent>(line).ok())
            .rev()
            .take(limit)
            .collect();

        Ok(events)
    }

    async fn delete_before(
        &self,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, AuditBackendError> {
        // Take the append handle so the file can be rewritten safely; the next
        // write reopens it at the new end-of-file.
        let mut guard = self.writer.lock().await;
        if let Some(mut writer) = guard.take() {
            writer.flush().await?;
        }

        let content = match tokio::fs::read_to_string(&self.path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };

        let mut kept: Vec<&str> = Vec::new();
        let mut deleted = 0usize;

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<AuditEvent>(line) {
                // Keep events at or newer than the cutoff.
                Ok(event) if event.timestamp >= timestamp => kept.push(line),
                // Drop events strictly older than the cutoff.
                Ok(_) => deleted += 1,
                // Preserve lines we cannot parse rather than silently dropping.
                Err(_) => kept.push(line),
            }
        }

        let mut new_content = kept.join("\n");
        if !kept.is_empty() {
            new_content.push('\n');
        }
        tokio::fs::write(&self.path, new_content).await?;

        Ok(deleted)
    }
}

/// Memory backend for testing
///
/// Stores audit events in memory.
#[derive(Clone)]
pub struct MemoryBackend {
    events: std::sync::Arc<tokio::sync::Mutex<Vec<AuditEvent>>>,
}

impl MemoryBackend {
    /// Create a new memory backend
    pub fn new() -> Self {
        Self {
            events: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Get all events
    pub async fn get_events(&self) -> Vec<AuditEvent> {
        self.events.lock().await.clone()
    }

    /// Clear all events
    pub async fn clear(&self) {
        self.events.lock().await.clear();
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditBackend for MemoryBackend {
    async fn write(&self, event: &AuditEvent) -> Result<(), AuditBackendError> {
        self.events.lock().await.push(event.clone());
        Ok(())
    }

    async fn flush(&self) -> Result<(), AuditBackendError> {
        Ok(())
    }

    async fn read(&self, limit: usize) -> Result<Vec<AuditEvent>, AuditBackendError> {
        let events = self.events.lock().await;
        let count = events.len().min(limit);
        Ok(events.iter().rev().take(count).cloned().collect())
    }

    async fn delete_before(
        &self,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, AuditBackendError> {
        let mut events = self.events.lock().await;
        let original_len = events.len();
        events.retain(|e| e.timestamp >= timestamp);
        Ok(original_len - events.len())
    }
}

/// Stdout backend for development
///
/// Prints audit events to stdout.
pub struct StdoutBackend;

impl StdoutBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StdoutBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditBackend for StdoutBackend {
    async fn write(&self, event: &AuditEvent) -> Result<(), AuditBackendError> {
        let json = event.to_json_pretty()?;
        println!("{}", json);
        Ok(())
    }

    async fn flush(&self) -> Result<(), AuditBackendError> {
        Ok(())
    }
}

/// Multiple backend wrapper
///
/// Writes to multiple backends simultaneously.
pub struct MultiBackend {
    backends: Vec<Box<dyn AuditBackend>>,
}

impl MultiBackend {
    /// Create a new multi-backend
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    /// Add a backend
    pub fn with_backend(mut self, backend: Box<dyn AuditBackend>) -> Self {
        self.backends.push(backend);
        self
    }
}

impl Default for MultiBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditBackend for MultiBackend {
    async fn write(&self, event: &AuditEvent) -> Result<(), AuditBackendError> {
        for backend in &self.backends {
            backend.write(event).await?;
        }
        Ok(())
    }

    async fn flush(&self) -> Result<(), AuditBackendError> {
        for backend in &self.backends {
            backend.flush().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_memory_backend() {
        let backend = MemoryBackend::new();
        let event = AuditEvent::new("test.event").user("alice").action("test");

        backend.write(&event).await.unwrap();

        let events = backend.get_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "test.event");
    }

    #[tokio::test]
    async fn test_memory_backend_read() {
        let backend = MemoryBackend::new();

        for i in 0..5 {
            let event = AuditEvent::new(format!("test.{}", i));
            backend.write(&event).await.unwrap();
        }

        let events = backend.read(3).await.unwrap();
        assert_eq!(events.len(), 3);
    }

    #[tokio::test]
    async fn test_file_backend_delete_before() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let backend = FileBackend::new(&path);

        // An event older than the cutoff.
        let mut old_event = AuditEvent::new("old.event");
        old_event.timestamp = chrono::Utc::now() - chrono::Duration::hours(2);
        backend.write(&old_event).await.unwrap();

        // A fresh event newer than the cutoff.
        let new_event = AuditEvent::new("new.event");
        backend.write(&new_event).await.unwrap();
        backend.flush().await.unwrap();

        let cutoff = chrono::Utc::now() - chrono::Duration::hours(1);
        let deleted = backend.delete_before(cutoff).await.unwrap();
        assert_eq!(deleted, 1, "exactly the old event should be deleted");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("new.event"));
        assert!(!content.contains("old.event"));

        // Retention keeps working after a rewrite: a further write must append
        // to the rewritten file, not resurrect the deleted line.
        let another = AuditEvent::new("another.event");
        backend.write(&another).await.unwrap();
        backend.flush().await.unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("another.event"));
        assert!(!content.contains("old.event"));
    }

    #[tokio::test]
    async fn test_file_backend_reuses_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let backend = FileBackend::new(&path);

        for i in 0..5 {
            backend
                .write(&AuditEvent::new(format!("event.{i}")))
                .await
                .unwrap();
        }

        // The append handle must be opened once and reused across all events,
        // not reopened per event.
        assert_eq!(backend.open_count(), 1);

        // All five events are still persisted.
        let events = backend.read(10).await.unwrap();
        assert_eq!(events.len(), 5);
    }

    #[tokio::test]
    async fn test_memory_backend_delete_before() {
        let backend = MemoryBackend::new();

        let old_event = AuditEvent::new("old");
        backend.write(&old_event).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let cutoff = chrono::Utc::now();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let new_event = AuditEvent::new("new");
        backend.write(&new_event).await.unwrap();

        let deleted = backend.delete_before(cutoff).await.unwrap();
        assert_eq!(deleted, 1);

        let events = backend.get_events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "new");
    }
}
