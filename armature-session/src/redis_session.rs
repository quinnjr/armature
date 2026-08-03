//! Redis session storage implementation.

use crate::config::SessionConfig;
use crate::error::{SessionError, SessionResult};
use crate::traits::{Session, SessionStore, generate_session_id};
use armature_log::{debug, info};
use async_trait::async_trait;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use std::time::Duration;

/// Number of keys hinted per `SCAN` iteration when sweeping the namespace.
const SCAN_COUNT: usize = 500;

/// Redis-backed session store.
///
/// # ⚠️ Important: Prefer Stateless Architecture
///
/// **Armature strongly recommends stateless architecture using JWT tokens.**
/// Use sessions only when absolutely necessary.
///
/// # Examples
///
/// ```no_run
/// use armature_session::{RedisSessionStore, SessionConfig, SessionStore};
/// use std::time::Duration;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = SessionConfig::redis("redis://localhost:6379")?
///         .with_namespace("myapp:session")
///         .with_default_ttl(Duration::from_secs(3600));
///
///     let store = RedisSessionStore::new(config).await?;
///
///     // Create a session
///     let mut session = store.create(None).await?;
///     session.set("user_id", 123)?;
///     store.save(&session).await?;
///
///     Ok(())
/// }
/// ```
pub struct RedisSessionStore {
    conn: ConnectionManager,
    config: SessionConfig,
}

impl RedisSessionStore {
    /// Create a new Redis session store.
    ///
    /// # Arguments
    ///
    /// * `config` - Session configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_session::{RedisSessionStore, SessionConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = SessionConfig::redis("redis://localhost:6379")?;
    /// let store = RedisSessionStore::new(config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(config: SessionConfig) -> SessionResult<Self> {
        info!("Initializing Redis session store");
        debug!("Session namespace: {}", config.namespace);

        let client = redis::Client::open(config.url.as_str())
            .map_err(|e| SessionError::Connection(e.to_string()))?;

        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| SessionError::Connection(e.to_string()))?;

        info!("Redis session store ready");
        Ok(Self { conn, config })
    }

    /// Get the session key for a given session ID.
    fn session_key(&self, session_id: &str) -> String {
        self.config.session_key(session_id)
    }
}

#[async_trait]
impl SessionStore for RedisSessionStore {
    async fn create(&self, ttl: Option<Duration>) -> SessionResult<Session> {
        let session_id = generate_session_id();
        let ttl = ttl.unwrap_or(self.config.default_ttl);

        // Enforce max TTL
        let ttl = if ttl > self.config.max_ttl {
            self.config.max_ttl
        } else {
            ttl
        };

        let session = Session::new(&session_id, ttl);

        // Save the session
        self.save(&session).await?;

        Ok(session)
    }

    async fn get(&self, session_id: &str) -> SessionResult<Option<Session>> {
        let key = self.session_key(session_id);
        let mut conn = self.conn.clone();

        let data: Option<String> = conn.get(&key).await?;

        match data {
            Some(json) => {
                let session: Session = serde_json::from_str(&json)
                    .map_err(|e| SessionError::Deserialization(e.to_string()))?;

                // Check if expired
                if session.is_expired() {
                    self.delete(session_id).await?;
                    return Ok(None);
                }

                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    async fn save(&self, session: &Session) -> SessionResult<()> {
        let key = self.session_key(&session.id);
        let mut conn = self.conn.clone();

        let json = serde_json::to_string(session)
            .map_err(|e| SessionError::Serialization(e.to_string()))?;

        // Calculate remaining TTL
        let now = chrono::Utc::now();
        let remaining = (session.expires_at - now).num_seconds().max(0) as u64;

        // A non-positive TTL means the session is already expired, so there is
        // nothing sensible to write. Returning `Ok(())` for it made `save` a
        // silent no-op — `create()` would hand back a session that was never
        // persisted, and the caller would only find out when the next request
        // came back unauthenticated.
        if remaining == 0 {
            return Err(SessionError::Expired(session.id.clone()));
        }

        let _: () = conn.set_ex(&key, json, remaining).await?;

        Ok(())
    }

    async fn delete(&self, session_id: &str) -> SessionResult<()> {
        let key = self.session_key(session_id);
        let mut conn = self.conn.clone();

        let _: () = conn.del(&key).await?;

        Ok(())
    }

    async fn exists(&self, session_id: &str) -> SessionResult<bool> {
        let key = self.session_key(session_id);
        let mut conn = self.conn.clone();

        // `save()` always writes the key with a TTL, so a present key is a
        // live (non-expired) session. EXISTS alone is authoritative here;
        // there is no need for a second GET + deserialize round-trip.
        let exists: bool = conn.exists(&key).await?;

        Ok(exists)
    }

    async fn extend(&self, session_id: &str, ttl: Duration) -> SessionResult<()> {
        if let Some(mut session) = self.get(session_id).await? {
            // Enforce max TTL
            let ttl = if ttl > self.config.max_ttl {
                self.config.max_ttl
            } else {
                ttl
            };

            session.extend(ttl);
            self.save(&session).await?;
        }

        Ok(())
    }

    async fn touch(&self, session_id: &str) -> SessionResult<()> {
        if let Some(mut session) = self.get(session_id).await? {
            session.touch();
            self.save(&session).await?;
        }

        Ok(())
    }

    async fn clear_all(&self) -> SessionResult<()> {
        let mut conn = self.conn.clone();
        let pattern = format!("{}:*", self.config.namespace);

        // Cursored SCAN instead of the blocking KEYS command. Each batch of
        // matching keys is removed with UNLINK (non-blocking, reclaims memory
        // in a background thread) rather than the synchronous DEL.
        let mut cursor: u64 = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(SCAN_COUNT)
                .query_async(&mut conn)
                .await?;

            if !keys.is_empty() {
                let _: () = redis::cmd("UNLINK")
                    .arg(&keys)
                    .query_async(&mut conn)
                    .await?;
            }

            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        Ok(())
    }

    async fn count(&self) -> SessionResult<usize> {
        let mut conn = self.conn.clone();
        let pattern = format!("{}:*", self.config.namespace);

        // Cursored SCAN that accumulates only a running count; keys from each
        // batch are counted and discarded rather than materialized into one
        // large Vec, keeping memory bounded regardless of session volume.
        let mut cursor: u64 = 0;
        let mut total: usize = 0;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(SCAN_COUNT)
                .query_async(&mut conn)
                .await?;

            total += keys.len();

            cursor = next;
            if cursor == 0 {
                break;
            }
        }

        Ok(total)
    }

    async fn cleanup_expired(&self) -> SessionResult<usize> {
        // Redis automatically expires keys with TTL
        // This method is a no-op for Redis but returns 0 for consistency
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_key_generation() {
        let config = SessionConfig::redis("redis://localhost:6379").unwrap();
        assert!(config.session_key("test-id").starts_with("session:"));
    }

    /// Pure model of the cursored SCAN loop used by `count`/`clear_all`.
    ///
    /// Redis returns keys in unspecified batches and signals completion with a
    /// zero cursor. This verifies the driving logic (accumulate batch lengths,
    /// terminate on cursor 0, never materialize all keys at once) independently
    /// of a live backend.
    fn scan_count_model(batches: &[(u64, Vec<&str>)]) -> usize {
        let mut total = 0usize;
        let mut idx = 0usize;
        loop {
            let (next, keys) = &batches[idx];
            total += keys.len();
            idx += 1;
            if *next == 0 {
                break;
            }
        }
        total
    }

    #[test]
    fn test_scan_loop_accumulates_across_batches() {
        // Non-zero cursors chain batches; a final zero cursor ends the sweep.
        let batches = vec![
            (42u64, vec!["session:a", "session:b"]),
            (7u64, vec!["session:c"]),
            (0u64, vec!["session:d", "session:e"]),
        ];
        assert_eq!(scan_count_model(&batches), 5);
    }

    #[test]
    fn test_scan_loop_single_empty_batch() {
        // An immediately-complete sweep (cursor 0, no keys) counts zero.
        let batches = vec![(0u64, vec![])];
        assert_eq!(scan_count_model(&batches), 0);
    }
}
