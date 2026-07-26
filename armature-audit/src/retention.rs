//! Retention policies for audit logs

use crate::backend::AuditBackend;
use chrono::{Duration, Utc};
use std::sync::Arc;
use tokio::time::interval;

/// Retention policy configuration
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Maximum age of audit logs
    pub max_age: Duration,

    /// How often to run cleanup
    pub cleanup_interval: std::time::Duration,
}

impl RetentionPolicy {
    /// Create a new retention policy
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_audit::*;
    /// use chrono::Duration;
    ///
    /// // Keep logs for 90 days, cleanup daily
    /// let policy = RetentionPolicy::new(Duration::days(90));
    /// ```
    pub fn new(max_age: Duration) -> Self {
        Self {
            max_age,
            cleanup_interval: std::time::Duration::from_secs(24 * 3600), // Daily by default
        }
    }

    /// Set cleanup interval
    pub fn cleanup_interval(mut self, interval: std::time::Duration) -> Self {
        self.cleanup_interval = interval;
        self
    }

    /// Keep logs for N days
    pub fn days(days: i64) -> Self {
        Self::new(Duration::days(days))
    }

    /// Keep logs for N hours
    pub fn hours(hours: i64) -> Self {
        Self::new(Duration::hours(hours))
    }

    /// Keep logs for N minutes
    pub fn minutes(minutes: i64) -> Self {
        Self::new(Duration::minutes(minutes))
    }
}

/// Retention manager
///
/// Manages automatic cleanup of old audit logs.
pub struct RetentionManager {
    backend: Arc<dyn AuditBackend>,
    policy: RetentionPolicy,
    running: Arc<tokio::sync::Mutex<bool>>,
}

impl RetentionManager {
    /// Create a new retention manager
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_audit::*;
    /// use std::sync::Arc;
    ///
    /// let backend = Arc::new(MemoryBackend::new());
    /// let policy = RetentionPolicy::days(90);
    /// let manager = RetentionManager::new(backend, policy);
    /// ```
    pub fn new(backend: Arc<dyn AuditBackend>, policy: RetentionPolicy) -> Self {
        Self {
            backend,
            policy,
            running: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    /// Run cleanup once
    ///
    /// Deletes audit logs older than the retention period.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_audit::*;
    /// # use std::sync::Arc;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let backend = Arc::new(MemoryBackend::new());
    /// # let policy = RetentionPolicy::days(90);
    /// let manager = RetentionManager::new(backend, policy);
    /// let deleted = manager.cleanup().await?;
    /// println!("Deleted {} old audit logs", deleted);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn cleanup(&self) -> Result<usize, crate::backend::AuditBackendError> {
        let cutoff = Utc::now() - self.policy.max_age;
        tracing::info!("Running audit log cleanup (cutoff: {})", cutoff);

        let deleted = self.backend.delete_before(cutoff).await?;

        if deleted > 0 {
            tracing::info!("Deleted {} old audit logs", deleted);
        }

        Ok(deleted)
    }

    /// Start automatic cleanup
    ///
    /// Runs cleanup periodically in the background.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_audit::*;
    /// # use std::sync::Arc;
    /// # async fn example() {
    /// # let backend = Arc::new(MemoryBackend::new());
    /// # let policy = RetentionPolicy::days(90);
    /// let manager = Arc::new(RetentionManager::new(backend, policy));
    /// manager.start().await;
    /// # }
    /// ```
    pub async fn start(self: Arc<Self>) {
        let mut running = self.running.lock().await;
        if *running {
            tracing::warn!("Retention manager already running");
            return;
        }
        *running = true;
        drop(running);

        let manager = self.clone();
        tokio::spawn(async move {
            let mut ticker = interval(manager.policy.cleanup_interval);

            loop {
                ticker.tick().await;

                let running = manager.running.lock().await;
                if !*running {
                    break;
                }
                drop(running);

                match manager.cleanup().await {
                    Ok(deleted) => {
                        if deleted > 0 {
                            tracing::info!("Retention cleanup: deleted {} logs", deleted);
                        }
                    }
                    // A backend that cannot delete (e.g. stdout) will never be
                    // able to; stop the background task instead of re-erroring
                    // on every tick forever.
                    Err(crate::backend::AuditBackendError::NotSupported) => {
                        tracing::warn!(
                            "Retention backend does not support deletion; \
                             stopping retention manager"
                        );
                        *manager.running.lock().await = false;
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Retention cleanup failed: {}", e);
                    }
                }
            }

            tracing::info!("Retention manager stopped");
        });
    }

    /// Stop automatic cleanup
    pub async fn stop(&self) {
        let mut running = self.running.lock().await;
        *running = false;
    }

    /// Check if manager is running
    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditEvent, MemoryBackend};

    #[test]
    fn test_retention_policy_days() {
        let policy = RetentionPolicy::days(90);
        assert_eq!(policy.max_age.num_days(), 90);
    }

    #[test]
    fn test_retention_policy_hours() {
        let policy = RetentionPolicy::hours(24);
        assert_eq!(policy.max_age.num_hours(), 24);
    }

    #[tokio::test]
    async fn test_retention_manager_cleanup() {
        let backend = Arc::new(MemoryBackend::new());
        let backend_clone = backend.clone();

        // Add old event
        let old_event = AuditEvent::new("old");
        backend.write(&old_event).await.unwrap();

        // Wait a bit
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Add new event
        let new_event = AuditEvent::new("new");
        backend.write(&new_event).await.unwrap();

        // Create retention manager with very short retention
        let policy = RetentionPolicy::new(Duration::milliseconds(5));
        let manager = RetentionManager::new(backend_clone, policy);

        // Wait for retention period to pass
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Run cleanup
        let deleted = manager.cleanup().await.unwrap();

        // Both events should be deleted (they're both old now)
        assert!(deleted > 0);
    }

    #[tokio::test]
    async fn test_retention_manager_cleanup_file_backend() {
        use crate::FileBackend;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let backend = Arc::new(FileBackend::new(&path));

        // One old event, one fresh event.
        let mut old_event = AuditEvent::new("old");
        old_event.timestamp = Utc::now() - Duration::hours(2);
        backend.write(&old_event).await.unwrap();
        backend.write(&AuditEvent::new("new")).await.unwrap();
        backend.flush().await.unwrap();

        // Keep only the last hour.
        let policy = RetentionPolicy::new(Duration::hours(1));
        let manager = RetentionManager::new(backend.clone(), policy);

        let deleted = manager.cleanup().await.unwrap();
        assert_eq!(deleted, 1, "the old file-backed entry should be deleted");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("new"));
        assert!(!content.contains("\"event_type\":\"old\""));
    }

    #[tokio::test]
    async fn test_retention_manager_start_stop() {
        let backend = Arc::new(MemoryBackend::new());
        let policy = RetentionPolicy::minutes(1);
        let manager = Arc::new(RetentionManager::new(backend, policy));

        assert!(!manager.is_running().await);

        manager.clone().start().await;

        // Give it a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        assert!(manager.is_running().await);

        manager.stop().await;

        // Give it a moment to stop
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        assert!(!manager.is_running().await);
    }
}
