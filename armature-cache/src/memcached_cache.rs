//! Memcached cache implementation.

use crate::config::CacheConfig;
use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Memcached cache store.
///
/// Note: The `memcache` crate doesn't have native async support,
/// so we wrap it with tokio's Mutex and use spawn_blocking for operations.
#[derive(Clone)]
pub struct MemcachedCache {
    client: Arc<Mutex<memcache::Client>>,
    config: CacheConfig,
}

impl MemcachedCache {
    /// Create a new Memcached cache instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Cache configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), CacheError> {
    ///     let config = CacheConfig::memcached("memcache://localhost:11211")?;
    ///     let cache = MemcachedCache::new(config).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn new(config: CacheConfig) -> CacheResult<Self> {
        // Parse the URL to extract the server address
        let url = config.url.clone();
        let server_url = Self::parse_memcached_url(&url)?;

        // Create client in blocking context
        let client = tokio::task::spawn_blocking(move || memcache::connect(server_url.as_str()))
            .await
            .map_err(|e| CacheError::Connection(format!("Failed to spawn task: {}", e)))?
            .map_err(|e| CacheError::Connection(format!("Failed to connect: {}", e)))?;

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            config,
        })
    }

    /// Parse Memcached URL to extract server address.
    ///
    /// Converts "memcache://localhost:11211" to "memcache://localhost:11211"
    /// or handles plain "localhost:11211" format.
    fn parse_memcached_url(url: &str) -> CacheResult<String> {
        if url.starts_with("memcache://") {
            Ok(url.to_string())
        } else if url.contains(':') {
            Ok(format!("memcache://{}", url))
        } else {
            Err(CacheError::InvalidUrl(format!(
                "Invalid Memcached URL: {}. Expected format: 'memcache://host:port' or 'host:port'",
                url
            )))
        }
    }

    /// Build the full key with prefix.
    fn build_key(&self, key: &str) -> String {
        self.config.build_key(key)
    }

    /// Convert Duration to Memcached expiration (in seconds).
    fn duration_to_expiration(ttl: Option<Duration>) -> u32 {
        ttl.map(|d| d.as_secs() as u32).unwrap_or(0)
    }
}

#[async_trait]
impl CacheStore for MemcachedCache {
    async fn get_json(&self, key: &str) -> CacheResult<Option<String>> {
        let key = self.build_key(key);
        let client = self.client.clone();

        let result = tokio::task::spawn_blocking(move || {
            let client = client.blocking_lock();
            client.get::<String>(&key)
        })
        .await
        .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))?;

        match result {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) | Err(_) => Ok(None),
        }
    }

    async fn mget(&self, keys: &[&str]) -> CacheResult<Vec<Option<String>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        // Prefix-map every key, then fetch them all with memcached's native
        // multi-get (`gets`) in a single round-trip inside one `spawn_blocking`.
        // The default `mget` (traits.rs) issues one `get_json` per key, and
        // although those futures are joined, they all contend on the single
        // `Arc<Mutex<Client>>`, degrading to N serial round-trips. This is N->1.
        let prefixed: Vec<String> = keys.iter().map(|k| self.build_key(k)).collect();
        let client = self.client.clone();

        let found: std::collections::HashMap<String, String> =
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = prefixed.iter().map(|s| s.as_str()).collect();
                let client = client.blocking_lock();
                client.gets::<String>(&refs)
            })
            .await
            .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))?
            .map_err(|e| CacheError::Other(format!("memcached mget failed: {}", e)))?;

        // Reassemble in input order; absent keys become `None`.
        Ok(keys
            .iter()
            .map(|k| found.get(&self.build_key(k)).cloned())
            .collect())
    }

    async fn set_json(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()> {
        let key = self.build_key(key);
        let client = self.client.clone();
        let ttl = ttl.or(self.config.default_ttl);
        let expiration = Self::duration_to_expiration(ttl);

        tokio::task::spawn_blocking(move || {
            let client = client.blocking_lock();
            client.set(&key, value, expiration)
        })
        .await
        .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))??;

        Ok(())
    }

    async fn delete(&self, key: &str) -> CacheResult<()> {
        let key = self.build_key(key);
        let client = self.client.clone();

        tokio::task::spawn_blocking(move || {
            let client = client.blocking_lock();
            client.delete(&key)
        })
        .await
        .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))??;

        Ok(())
    }

    async fn exists(&self, key: &str) -> CacheResult<bool> {
        // Memcached doesn't have a native "exists" command
        // We check by trying to get the key
        let result = self.get_json(key).await?;
        Ok(result.is_some())
    }

    async fn clear(&self) -> CacheResult<()> {
        let client = self.client.clone();

        tokio::task::spawn_blocking(move || {
            let client = client.blocking_lock();
            client.flush()
        })
        .await
        .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))??;

        Ok(())
    }

    async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>> {
        // Protocol limitation, stated honestly: the memcached text/binary
        // protocols expose no way to read an item's remaining TTL. `GET`
        // returns only the value (and flags/CAS), never the expiry, and there
        // is no `TTL`/`PTTL` equivalent. We therefore always return `Ok(None)`
        // — "no known expiration" — rather than pretending to have queried it.
        // Callers needing TTL visibility must track expirations out-of-band or
        // use a backend (e.g. Redis) that supports `TTL`.
        let _ = key;
        Ok(None)
    }

    async fn expire(&self, key: &str, ttl: Duration) -> CacheResult<()> {
        // Use memcached's native `touch`, which updates an item's expiration in
        // place: one round-trip, no payload transfer, and no get->set race. The
        // old read-then-write did two round-trips and re-uploaded the full
        // value. `touch` returns Ok(false) when the key is absent -> NotFound.
        let full_key = self.build_key(key);
        let client = self.client.clone();
        let expiration = Self::duration_to_expiration(Some(ttl));

        let touched = tokio::task::spawn_blocking(move || {
            let client = client.blocking_lock();
            client.touch(&full_key, expiration)
        })
        .await
        .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))??;

        if touched {
            Ok(())
        } else {
            Err(CacheError::NotFound(key.to_string()))
        }
    }

    async fn increment(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let key = self.build_key(key);
        let client = self.client.clone();
        // Apply the configured default TTL to keys we create at zero, matching
        // `set_json`'s expiry semantics.
        let expiration = Self::duration_to_expiration(self.config.default_ttl);
        let magnitude = delta.unsigned_abs();
        let is_increment = delta >= 0;

        // Perform the whole read-modify-write on the memcached server via its
        // native atomic `incr`/`decr`, returning the authoritative new value
        // directly — no lossy second `GET`, and crucially no `delta.abs()`
        // fabrication when a re-read fails to parse.
        //
        // memcached's create-at-zero semantics: the binary protocol
        // auto-creates a missing counter at 0 (the delta is not applied on
        // creation) and returns 0. The ASCII protocol instead returns
        // `KeyNotFound`; we mirror the binary behaviour there by adding the key
        // at 0 and returning 0, retrying once if we lose the create race.
        let new_value =
            tokio::task::spawn_blocking(move || -> Result<u64, memcache::MemcacheError> {
                let client = client.blocking_lock();

                let apply = |client: &memcache::Client| -> Result<u64, memcache::MemcacheError> {
                    if is_increment {
                        client.increment(&key, magnitude)
                    } else {
                        client.decrement(&key, magnitude)
                    }
                };

                match apply(&client) {
                    Ok(value) => Ok(value),
                    Err(memcache::MemcacheError::CommandError(
                        memcache::CommandError::KeyNotFound,
                    )) => {
                        // Create the counter at zero (matching the binary protocol),
                        // returning 0.
                        match client.add(&key, 0u64, expiration) {
                            Ok(()) => Ok(0),
                            // Lost the create race: another client added it first.
                            // Retry the atomic op against the now-present key.
                            Err(memcache::MemcacheError::CommandError(
                                memcache::CommandError::KeyExists,
                            )) => apply(&client),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            })
            .await
            .map_err(|e| CacheError::Other(format!("Task join error: {}", e)))??;

        // Preserve the exact server value across the u64 -> i64 boundary. Note
        // this is a lossless bit-cast: counters above `i64::MAX` become
        // negative, but never the old `delta.abs()` fabrication.
        Ok(new_value as i64)
    }

    async fn decrement(&self, key: &str, delta: i64) -> CacheResult<i64> {
        self.increment(key, -delta).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_memcached_url() {
        assert_eq!(
            MemcachedCache::parse_memcached_url("memcache://localhost:11211").unwrap(),
            "memcache://localhost:11211"
        );

        assert_eq!(
            MemcachedCache::parse_memcached_url("localhost:11211").unwrap(),
            "memcache://localhost:11211"
        );

        assert!(MemcachedCache::parse_memcached_url("invalid").is_err());
    }

    #[test]
    fn test_duration_to_expiration() {
        assert_eq!(MemcachedCache::duration_to_expiration(None), 0);
        assert_eq!(
            MemcachedCache::duration_to_expiration(Some(Duration::from_secs(60))),
            60
        );
    }
}
