//! Memcached cache implementation.

use crate::config::CacheConfig;
use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// Largest expiration value the memcached protocol interprets as a *relative*
/// number of seconds (30 days). Anything strictly greater is interpreted by the
/// server as an **absolute Unix timestamp** instead — see
/// [`MemcachedCache::expiration_from_secs`].
const MEMCACHED_RELATIVE_TTL_MAX_SECS: u64 = 2_592_000;

/// Memcached cache store.
///
/// Note: The `memcache` crate doesn't have native async support,
/// so we wrap it with tokio's Mutex and use spawn_blocking for operations.
///
/// # Limitations
///
/// This backend deliberately does **not** honour several [`CacheConfig`]
/// fields. They are accepted (so one `CacheConfig` can describe either
/// backend) but ignored here; `RedisCache` wires all three.
///
/// * **`connection_timeout` — ignored.** The initial connect is performed by
///   `memcache::connect` inside `spawn_blocking` with no timeout applied, so a
///   hung server can block the connect for as long as the OS TCP timeout
///   allows.
/// * **`operation_timeout` — ignored.** Individual gets/sets/deletes are not
///   bounded by a deadline, so `CacheError::Timeout` is never produced by this
///   backend. A stalled memcached server stalls the calling task.
/// * **`max_connections` — ignored, and structurally unhonourable as
///   implemented.** The whole backend holds a *single* `memcache::Client`
///   behind one `Arc<Mutex<..>>`, and every operation takes that mutex inside
///   `spawn_blocking`. All memcached traffic from a given `MemcachedCache`
///   (including every clone of it, since the `Arc` is shared) therefore
///   serializes onto one lock and one socket. Batch operations that are
///   "parallel" at the future level still execute one at a time underneath;
///   `mget` sidesteps this only because it uses memcached's native multi-get,
///   which is a single round-trip.
///
/// [`Self::new`] logs a warning when `max_connections > 1` is configured, so
/// this is visible at runtime and not only in these docs.
///
/// Two further protocol-level limitations are documented on the methods
/// themselves: [`CacheStore::clear`] cannot be scoped to `key_prefix`, and
/// [`CacheStore::ttl`] always returns `None`.
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

        // Surface the ignored pool setting at runtime rather than only in the
        // type's `# Limitations` rustdoc: an operator who configured a pool is
        // otherwise given no signal that every operation still serializes onto
        // a single client behind a single mutex.
        if config.max_connections > 1 {
            armature_log::warn!(
                "MemcachedCache ignores CacheConfig::max_connections (configured: {}); \
                 this backend holds a single memcache::Client behind one mutex, so all \
                 operations serialize onto one connection. connection_timeout and \
                 operation_timeout are ignored by this backend as well.",
                config.max_connections
            );
        }

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

    /// Convert a `Duration` to the expiration value memcached expects.
    ///
    /// See [`Self::expiration_from_secs`] for the 30-day rule this implements.
    /// `None` maps to `0`, memcached's "never expires".
    fn duration_to_expiration(ttl: Option<Duration>) -> u32 {
        match ttl {
            Some(d) => Self::expiration_from_secs(d.as_secs(), Self::unix_now()),
            None => 0,
        }
    }

    /// Seconds since the Unix epoch, saturating at 0 if the clock is somehow
    /// before the epoch.
    fn unix_now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Encode `secs` of desired lifetime as a memcached expiration value,
    /// given the current Unix time `now`.
    ///
    /// The memcached protocol overloads this single field: a value of at most
    /// 2,592,000 (30 days) is a *relative* offset from now, but anything larger
    /// is read as an *absolute* Unix timestamp. Passing a raw 40-day lifetime
    /// (3,456,000) therefore does not store the item for 40 days — the server
    /// reads it as a date in early 1970, so the item is written already
    /// expired, is never readable again, and the write still reports success.
    /// Longer lifetimes must consequently be converted to `now + secs`.
    ///
    /// The result is clamped to `u32::MAX`, so an absurdly long lifetime
    /// degrades to "expires at the far end of the 32-bit epoch" rather than
    /// wrapping around into a timestamp in the past.
    fn expiration_from_secs(secs: u64, now: u64) -> u32 {
        if secs <= MEMCACHED_RELATIVE_TTL_MAX_SECS {
            // Fits in u32 by construction (2_592_000 < u32::MAX).
            secs as u32
        } else {
            now.saturating_add(secs).min(u32::MAX as u64) as u32
        }
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

        // The `memcache` crate's own ascii/binary protocol implementations of
        // `get` already distinguish a genuine cache miss from an operational
        // failure: both return `Ok(None)` for an absent key (there is no
        // "NOT_FOUND" error response for `get`, unlike `delete`/`incr`/`touch`)
        // and only ever return `Err` for real I/O, parse, client, or server
        // errors. So there is nothing to narrow-match here — every `Err` is a
        // genuine failure and must propagate, not collapse into `Ok(None)`
        // indistinguishable from a miss.
        result.map_err(CacheError::from)
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

    /// Store with memcached's `0` expiration ("never expires"), skipping the
    /// `default_ttl` fallback that `set_json` applies to a `None` TTL. See
    /// [`CacheStore::set_json_forever`].
    async fn set_json_forever(&self, key: &str, value: String) -> CacheResult<()> {
        let key = self.build_key(key);
        let client = self.client.clone();

        tokio::task::spawn_blocking(move || {
            let client = client.blocking_lock();
            client.set(&key, value, 0)
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

    /// Clear this cache.
    ///
    /// **Protocol limitation, unscoped:** unlike `RedisCache::clear()` (which
    /// scopes to `key_prefix` via `SCAN`+`UNLINK`), this always issues
    /// memcached's `flush_all`, which invalidates **every** key on the
    /// memcached server/pool — `key_prefix` is not, and cannot be, applied
    /// here. The memcached text/binary protocols expose no key-enumeration
    /// primitive (no `SCAN`/`KEYS` equivalent; `stats cachedump` is a
    /// non-standard admin extension that isn't reliably available across
    /// servers and isn't exposed by the `memcache` crate this backend uses),
    /// so there is no way to discover "just this cache's keys" to delete
    /// individually. A `MemcachedCache` sharing a memcached instance with
    /// other services/tenants should not call `clear()`.
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

    /// A lifetime at or below memcached's 30-day threshold is a relative
    /// offset and must be passed through untouched.
    #[test]
    fn test_expiration_under_threshold_passes_through() {
        let now = 1_700_000_000;
        assert_eq!(MemcachedCache::expiration_from_secs(0, now), 0);
        assert_eq!(MemcachedCache::expiration_from_secs(60, now), 60);
        assert_eq!(
            MemcachedCache::expiration_from_secs(MEMCACHED_RELATIVE_TTL_MAX_SECS - 1, now),
            (MEMCACHED_RELATIVE_TTL_MAX_SECS - 1) as u32
        );
    }

    /// Exactly 30 days is still relative — the absolute-timestamp
    /// interpretation only kicks in *above* the threshold.
    #[test]
    fn test_expiration_at_threshold_passes_through() {
        let now = 1_700_000_000;
        assert_eq!(
            MemcachedCache::expiration_from_secs(MEMCACHED_RELATIVE_TTL_MAX_SECS, now),
            MEMCACHED_RELATIVE_TTL_MAX_SECS as u32
        );
    }

    /// Regression: a lifetime above the threshold used to be sent raw, which
    /// memcached read as a 1970 timestamp and stored already-expired. It must
    /// now be converted to an absolute timestamp in the future.
    #[test]
    fn test_expiration_over_threshold_becomes_future_absolute_timestamp() {
        let now = 1_700_000_000;
        let forty_days = 40 * 24 * 60 * 60; // 3_456_000 > 2_592_000
        let expiration = MemcachedCache::expiration_from_secs(forty_days, now);

        assert_eq!(expiration as u64, now + forty_days);
        assert!(
            (expiration as u64) > now,
            "an over-threshold TTL must land in the future, not 1970"
        );
    }

    /// A lifetime large enough to overflow the 32-bit expiration field
    /// saturates at `u32::MAX` instead of wrapping into the past.
    #[test]
    fn test_expiration_overflow_saturates() {
        let now = 1_700_000_000;
        assert_eq!(
            MemcachedCache::expiration_from_secs(u64::MAX, now),
            u32::MAX
        );
        assert_eq!(
            MemcachedCache::expiration_from_secs(u32::MAX as u64, now),
            u32::MAX
        );
        assert_eq!(
            MemcachedCache::duration_to_expiration(Some(Duration::from_secs(u64::MAX))),
            u32::MAX
        );
    }
}
