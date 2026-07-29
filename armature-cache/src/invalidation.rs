//! Tag-based cache invalidation

use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// Cache with tag-based invalidation support.
///
/// The tag -> member-key index (and its reverse, key -> tags) is persisted in
/// the backing [`CacheStore`] itself under reserved keys — it is NOT kept in
/// a local, per-process map. That means the index is visible to every
/// instance sharing the same backing store (e.g. every app process pointed at
/// the same Redis), so a key tagged by one instance can be looked up and
/// invalidated by another.
///
/// # Atomicity caveat (concurrent race)
///
/// Index updates go through [`CacheStore::set_add`] / [`CacheStore::set_remove`] /
/// [`CacheStore::set_members`]. Only backends that override these with a
/// native set type — `RedisCache` does, via `SADD`/`SREM`/`SMEMBERS` — update
/// the index atomically (see [`CacheStore::supports_atomic_sets`]). Other
/// backends (e.g. `InMemoryCache`, `MemcachedCache`) fall back to the
/// trait's default, non-atomic read-modify-write, so concurrent
/// `set_with_tags`/`invalidate_tag` calls against the SAME tag from
/// different instances can race and lose an update. For distributed
/// deployments, wrap a `RedisCache` (or another backend that overrides the
/// set primitives) if you need that guarantee. [`Self::new`] logs a warning
/// once, at construction time, when the backing store doesn't support
/// atomic sets.
///
/// # Partial-failure caveat (sequential, non-transactional updates)
///
/// Separately from the concurrent-race caveat above: [`Self::set_with_tags`],
/// [`Self::delete`], and [`Self::invalidate_tags`] each issue several
/// independent `set_add`/`set_remove`/`delete` calls in sequence. If an
/// early call in one of these sequences succeeds and a later one fails, the
/// tag index (and its reverse index) can end up **partially updated** —
/// inconsistent with the cached value, or inconsistent with itself (e.g. a
/// key's reverse-index tag set can end up not matching the forward tag ->
/// keys sets it's actually a member of). There is currently no automatic
/// rollback or reconciliation for this case: a failed call may need to be
/// retried or the affected tag(s)/key(s) reconciled manually. A best-effort
/// warning is logged when this happens (see the calls guarded by
/// `warn_on_partial_failure` in the implementation) so operators at least
/// get a signal, but the index itself is not repaired automatically.
///
/// # Reserved key namespace
///
/// The tag index's bookkeeping keys (`tag_set_key`/`key_tags_set_key`/
/// `TAG_INDEX_KEY`) all begin with the reserved prefix
/// `"__armature_"` and live in the SAME keyspace as caller-supplied keys —
/// both go through the same backing [`CacheStore`], with only `key_prefix`
/// (from `CacheConfig`) applied identically to both. A caller-supplied key
/// that happens to start with `"__armature_"` would therefore collide with
/// this reserved bookkeeping keyspace (e.g. writing to
/// `__armature_tag__:users` would clobber the "users" tag's member set).
/// This prefix is forbidden for caller-supplied keys: [`Self::set_with_tags`],
/// [`Self::get`], and [`Self::delete`] all reject a key starting with
/// `"__armature_"` with a `CacheError::Config`, rather than silently
/// allowing the collision.
pub struct TaggedCache<C: CacheStore> {
    /// Underlying cache store. Tag bookkeeping lives here too (see
    /// `tag_set_key`/`key_tags_set_key`/`TAG_INDEX_KEY`), not in a local map.
    cache: Arc<C>,
}

impl<C: CacheStore> TaggedCache<C> {
    /// Reserved key holding a tag's member-key set (`tag -> {keys}`).
    fn tag_set_key(tag: &str) -> String {
        format!("__armature_tag__:{tag}")
    }

    /// Reserved key holding a key's tag set (`key -> {tags}`).
    fn key_tags_set_key(key: &str) -> String {
        format!("__armature_keytags__:{key}")
    }

    /// Reserved key holding the set of every tag name that currently has at
    /// least one member (backs [`Self::list_tags`]).
    const TAG_INDEX_KEY: &'static str = "__armature_tag_index__";

    /// Prefix reserved for `TaggedCache`'s own tag-bookkeeping keys — every
    /// key produced by [`Self::tag_set_key`], [`Self::key_tags_set_key`], and
    /// [`Self::TAG_INDEX_KEY`] starts with it. Forbidden for caller-supplied
    /// keys; see [`Self::validate_caller_key`] and the struct-level
    /// "Reserved key namespace" docs.
    const RESERVED_KEY_PREFIX: &'static str = "__armature_";

    /// Reject a caller-supplied key that collides with `TaggedCache`'s
    /// reserved bookkeeping keyspace (see [`Self::RESERVED_KEY_PREFIX`]).
    ///
    /// Called at the top of every `TaggedCache` method that accepts a raw,
    /// caller-supplied key ([`Self::set_with_tags`], [`Self::get`],
    /// [`Self::delete`]) so a key that happens to start with
    /// `"__armature_"` is rejected with a clear `CacheError::Config` instead
    /// of silently colliding with (and potentially corrupting) the tag
    /// index's own reserved keys.
    fn validate_caller_key(key: &str) -> CacheResult<()> {
        if key.starts_with(Self::RESERVED_KEY_PREFIX) {
            Err(CacheError::Config(format!(
                "cache key {key:?} is reserved for TaggedCache's internal tag index \
                 (the {:?} prefix is forbidden for caller-supplied keys)",
                Self::RESERVED_KEY_PREFIX
            )))
        } else {
            Ok(())
        }
    }

    /// Best-effort observability hook for the "Partial-failure caveat"
    /// described on the struct docs: logs a warning when a step in a
    /// multi-step index update ([`Self::set_with_tags`], [`Self::delete`],
    /// [`Self::invalidate_tags`]) fails after one or more earlier steps in
    /// the same call already succeeded, since the tag index may now be left
    /// partially updated with no automatic rollback.
    fn warn_on_partial_failure(op: &str, err: &CacheError) {
        armature_log::warn!(
            "TaggedCache::{op} failed partway through a multi-step tag-index update; \
             the tag index may now be inconsistent with the cached value or with itself \
             (no automatic rollback): {err}"
        );
    }

    /// Create new tagged cache
    ///
    /// Checks [`CacheStore::supports_atomic_sets`] on `cache` and logs a
    /// warning once, here at construction time, when the backing store does
    /// NOT support atomic sets — see the struct-level "Atomicity caveat"
    /// docs for what that means for concurrent tag-index updates.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_cache::*;
    ///
    /// let cache = RedisCache::new(config).await?;
    /// let tagged = TaggedCache::new(Arc::new(cache));
    /// ```
    pub fn new(cache: Arc<C>) -> Self {
        if !cache.supports_atomic_sets() {
            armature_log::warn!(
                "TaggedCache backing store does not support atomic set operations \
                 (SADD/SREM/SMEMBERS-equivalent); concurrent set_with_tags/invalidate_tag \
                 calls against the same tag from different instances can race and lose an \
                 update. Wrap a backend that overrides CacheStore::set_add/set_remove/\
                 set_members atomically (e.g. RedisCache) if you need that guarantee."
            );
        }
        Self { cache }
    }

    /// Set a value with tags
    ///
    /// A repeated call for the same `key` REPLACES its tag membership with
    /// `tags` (it does not union with whatever tags the key carried before).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// tagged.set_with_tags(
    ///     "user:123",
    ///     user_json,
    ///     &["users", "active-users"],
    ///     Some(Duration::from_secs(3600)),
    /// ).await?;
    /// ```
    pub async fn set_with_tags(
        &self,
        key: &str,
        value: String,
        tags: &[&str],
        ttl: Option<Duration>,
    ) -> CacheResult<()> {
        Self::validate_caller_key(key)?;

        // Set in cache
        self.cache.set_json(key, value, ttl).await?;

        // Replace this key's persisted tag membership: drop it from any
        // previously associated tag that is no longer in `tags`, then (re)add
        // it to the current set.
        let previous_tags = self.get_tags_for_key(key).await?;
        let new_tags: HashSet<String> = tags.iter().map(|t| t.to_string()).collect();
        let key_tags_key = Self::key_tags_set_key(key);

        for old_tag in &previous_tags {
            if !new_tags.contains(old_tag) {
                self.cache
                    .set_remove(&Self::tag_set_key(old_tag), key)
                    .await
                    .inspect_err(|e| Self::warn_on_partial_failure("set_with_tags", e))?;
                self.cache
                    .set_remove(&key_tags_key, old_tag)
                    .await
                    .inspect_err(|e| Self::warn_on_partial_failure("set_with_tags", e))?;
                self.prune_tag_index_if_empty(old_tag).await?;
            }
        }

        for tag in &new_tags {
            self.cache
                .set_add(&Self::tag_set_key(tag), key)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("set_with_tags", e))?;
            self.cache
                .set_add(&key_tags_key, tag)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("set_with_tags", e))?;
            self.cache
                .set_add(Self::TAG_INDEX_KEY, tag)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("set_with_tags", e))?;
        }

        Ok(())
    }

    /// Get value from cache
    pub async fn get(&self, key: &str) -> CacheResult<Option<String>> {
        Self::validate_caller_key(key)?;
        self.cache.get_json(key).await
    }

    /// Delete a specific key
    pub async fn delete(&self, key: &str) -> CacheResult<()> {
        Self::validate_caller_key(key)?;

        // Delete from cache
        self.cache.delete(key).await?;

        // Remove from the persisted tag mappings.
        let key_tags_key = Self::key_tags_set_key(key);
        let tags = self.cache.set_members(&key_tags_key).await?;

        for tag in &tags {
            self.cache
                .set_remove(&Self::tag_set_key(tag), key)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("delete", e))?;
            self.prune_tag_index_if_empty(tag).await?;
        }
        if !tags.is_empty() {
            self.cache
                .delete(&key_tags_key)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("delete", e))?;
        }

        Ok(())
    }

    /// Invalidate all keys with a specific tag
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Invalidate all user-related cache entries
    /// tagged.invalidate_tag("users").await?;
    /// ```
    pub async fn invalidate_tag(&self, tag: &str) -> CacheResult<()> {
        self.invalidate_tags(&[tag]).await
    }

    /// Invalidate all keys with any of the specified tags
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Invalidate all user and session data
    /// tagged.invalidate_tags(&["users", "sessions"]).await?;
    /// ```
    pub async fn invalidate_tags(&self, tags: &[&str]) -> CacheResult<()> {
        // Gather the union of keys across all requested tags, reading each
        // tag's persisted member set from the backing store.
        let mut victims: HashSet<String> = HashSet::new();
        for &tag in tags {
            let members = self.cache.set_members(&Self::tag_set_key(tag)).await?;
            victims.extend(members);
        }

        // One batch delete for the whole union — coalesced into a single
        // backend round-trip regardless of how many tags were requested.
        if !victims.is_empty() {
            let key_refs: Vec<&str> = victims.iter().map(|s| s.as_str()).collect();
            self.cache
                .delete_many(&key_refs)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("invalidate_tags", e))?;
        }

        // Update each victim's reverse index: drop only the tags being
        // invalidated (a key may carry tags beyond those requested).
        let removed: HashSet<&str> = tags.iter().copied().collect();
        for key in &victims {
            let key_tags_key = Self::key_tags_set_key(key);
            let current_tags = self.cache.set_members(&key_tags_key).await?;
            for tag in current_tags.iter().filter(|t| removed.contains(t.as_str())) {
                self.cache
                    .set_remove(&key_tags_key, tag)
                    .await
                    .inspect_err(|e| Self::warn_on_partial_failure("invalidate_tags", e))?;
            }
        }

        // Drop the invalidated tags' member sets entirely and prune them from
        // the tag index, rather than leaving emptied sets behind.
        for &tag in tags {
            self.cache
                .delete(&Self::tag_set_key(tag))
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("invalidate_tags", e))?;
            self.cache
                .set_remove(Self::TAG_INDEX_KEY, tag)
                .await
                .inspect_err(|e| Self::warn_on_partial_failure("invalidate_tags", e))?;
        }

        Ok(())
    }

    /// Get all keys with a specific tag
    pub async fn get_keys_by_tag(&self, tag: &str) -> CacheResult<Vec<String>> {
        self.cache.set_members(&Self::tag_set_key(tag)).await
    }

    /// Get all tags for a specific key
    pub async fn get_tags_for_key(&self, key: &str) -> CacheResult<Vec<String>> {
        self.cache.set_members(&Self::key_tags_set_key(key)).await
    }

    /// Get all registered tags (tags currently carrying at least one member).
    pub async fn list_tags(&self) -> CacheResult<Vec<String>> {
        self.cache.set_members(Self::TAG_INDEX_KEY).await
    }

    /// Drop `tag` from the global tag index once it has no members left.
    async fn prune_tag_index_if_empty(&self, tag: &str) -> CacheResult<()> {
        let members = self.cache.set_members(&Self::tag_set_key(tag)).await?;
        if members.is_empty() {
            self.cache.set_remove(Self::TAG_INDEX_KEY, tag).await?;
        }
        Ok(())
    }
}

impl<C: CacheStore> Clone for TaggedCache<C> {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CacheResult;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    use std::sync::atomic::{AtomicUsize, Ordering};

    // Mock cache for testing. Counts batch-delete round-trips so tests can
    // assert that `invalidate_tags` coalesces into a single backend call.
    #[derive(Clone)]
    struct MockCache {
        data: Arc<RwLock<HashMap<String, String>>>,
        mdel_calls: Arc<AtomicUsize>,
    }

    impl MockCache {
        fn new() -> Self {
            Self {
                data: Arc::new(RwLock::new(HashMap::new())),
                mdel_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn mdel_calls(&self) -> usize {
            self.mdel_calls.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl CacheStore for MockCache {
        async fn get_json(&self, key: &str) -> CacheResult<Option<String>> {
            Ok(self.data.read().await.get(key).cloned())
        }

        async fn set_json(
            &self,
            key: &str,
            value: String,
            _ttl: Option<Duration>,
        ) -> CacheResult<()> {
            self.data.write().await.insert(key.to_string(), value);
            Ok(())
        }

        async fn delete(&self, key: &str) -> CacheResult<()> {
            self.data.write().await.remove(key);
            Ok(())
        }

        async fn exists(&self, key: &str) -> CacheResult<bool> {
            Ok(self.data.read().await.contains_key(key))
        }

        async fn clear(&self) -> CacheResult<()> {
            self.data.write().await.clear();
            Ok(())
        }

        async fn mdel(&self, keys: &[&str]) -> CacheResult<()> {
            self.mdel_calls.fetch_add(1, Ordering::Relaxed);
            let mut data = self.data.write().await;
            for key in keys {
                data.remove(*key);
            }
            Ok(())
        }

        async fn ttl(&self, _key: &str) -> CacheResult<Option<Duration>> {
            Ok(None)
        }

        async fn expire(&self, _key: &str, _ttl: Duration) -> CacheResult<()> {
            Ok(())
        }

        async fn increment(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
            Ok(0)
        }

        async fn decrement(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn test_tagged_cache() {
        let cache = Arc::new(MockCache::new());
        let tagged = TaggedCache::new(cache);

        // Set with tags
        tagged
            .set_with_tags("user:1", "Alice".to_string(), &["users", "active"], None)
            .await
            .unwrap();

        tagged
            .set_with_tags("user:2", "Bob".to_string(), &["users"], None)
            .await
            .unwrap();

        // Get value
        let value = tagged.get("user:1").await.unwrap();
        assert_eq!(value, Some("Alice".to_string()));

        // Get keys by tag
        let user_keys = tagged.get_keys_by_tag("users").await.unwrap();
        assert_eq!(user_keys.len(), 2);

        // Invalidate by tag
        tagged.invalidate_tag("users").await.unwrap();

        // Verify deletion
        let value = tagged.get("user:1").await.unwrap();
        assert_eq!(value, None);
    }

    #[tokio::test]
    async fn test_multiple_tags() {
        let cache = Arc::new(MockCache::new());
        let tagged = TaggedCache::new(cache);

        tagged
            .set_with_tags("key1", "value1".to_string(), &["tag1", "tag2"], None)
            .await
            .unwrap();

        let tags = tagged.get_tags_for_key("key1").await.unwrap();
        assert_eq!(tags.len(), 2);

        tagged.invalidate_tag("tag1").await.unwrap();

        let value = tagged.get("key1").await.unwrap();
        assert_eq!(value, None);
    }

    /// Regression: `invalidate_tags` must coalesce into a single backend batch
    /// delete instead of one `delete_many` round-trip per tag.
    #[tokio::test]
    async fn test_invalidate_tags_coalesces_into_single_roundtrip() {
        let cache = Arc::new(MockCache::new());
        let tagged = TaggedCache::new(cache.clone());

        // Distinct keys across three tags; "shared" carries two of them so we
        // also verify a shared key is deleted exactly once.
        tagged
            .set_with_tags("k1", "a".to_string(), &["t1"], None)
            .await
            .unwrap();
        tagged
            .set_with_tags("k2", "b".to_string(), &["t2"], None)
            .await
            .unwrap();
        tagged
            .set_with_tags("shared", "c".to_string(), &["t1", "t3"], None)
            .await
            .unwrap();

        tagged.invalidate_tags(&["t1", "t2", "t3"]).await.unwrap();

        // All matching keys gone.
        assert_eq!(tagged.get("k1").await.unwrap(), None);
        assert_eq!(tagged.get("k2").await.unwrap(), None);
        assert_eq!(tagged.get("shared").await.unwrap(), None);

        // Exactly one batch delete round-trip, regardless of tag count.
        assert_eq!(
            cache.mdel_calls(),
            1,
            "invalidate_tags must issue a single coalesced batch delete"
        );

        // Reverse index fully cleaned up.
        assert!(tagged.list_tags().await.unwrap().is_empty());
        assert!(tagged.get_tags_for_key("shared").await.unwrap().is_empty());
    }

    /// Regression for Finding 2: the tag index must be visible across
    /// independent `TaggedCache` instances that share the same backing store
    /// — e.g. two app processes both wrapping the same `RedisCache`. The old
    /// implementation kept `tags`/`key_tags` in a local, per-process
    /// `HashMap`, so a key tagged on instance A was invisible to
    /// `invalidate_tag` called on instance B. Tag state is now persisted in
    /// the backing `CacheStore` itself, so a second `TaggedCache` wrapping
    /// the same store observes and can invalidate tags set by the first.
    #[tokio::test]
    async fn test_tag_index_visible_across_instances_sharing_backend() {
        let shared_backend = Arc::new(MockCache::new());

        // Two independent `TaggedCache` "instances" (simulating two app
        // processes) wrapping the SAME backing store.
        let instance_a = TaggedCache::new(shared_backend.clone());
        let instance_b = TaggedCache::new(shared_backend.clone());

        instance_a
            .set_with_tags("user:1", "Alice".to_string(), &["users"], None)
            .await
            .unwrap();

        // Instance B never called `set_with_tags` itself but must still see
        // the tag membership through the shared backend.
        let keys = instance_b.get_keys_by_tag("users").await.unwrap();
        assert_eq!(keys, vec!["user:1".to_string()]);

        // ...and must be able to invalidate it.
        instance_b.invalidate_tag("users").await.unwrap();

        // The key is gone via the shared backend, observable from instance A.
        assert_eq!(instance_a.get("user:1").await.unwrap(), None);
        assert!(instance_a.list_tags().await.unwrap().is_empty());
    }

    /// Regression for Finding 2: a caller-supplied key that collides with
    /// `TaggedCache`'s reserved bookkeeping prefix must be rejected by
    /// `set_with_tags`, not silently allowed to corrupt the tag index.
    #[tokio::test]
    async fn test_set_with_tags_rejects_reserved_key_prefix() {
        let cache = Arc::new(MockCache::new());
        let tagged = TaggedCache::new(cache);

        let err = tagged
            .set_with_tags(
                "__armature_tag__:users",
                "corrupt".to_string(),
                &["users"],
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, CacheError::Config(_)),
            "expected CacheError::Config for a reserved-prefixed key, got: {err:?}"
        );

        // The real "users" tag index must be unaffected: no keys tagged yet.
        assert!(tagged.get_keys_by_tag("users").await.unwrap().is_empty());
    }

    /// `get` and `delete` must reject reserved-prefixed keys too, since both
    /// accept a raw caller-supplied key that is passed straight through to
    /// the same backing store the tag index's bookkeeping keys live in.
    #[tokio::test]
    async fn test_get_and_delete_reject_reserved_key_prefix() {
        let cache = Arc::new(MockCache::new());
        let tagged = TaggedCache::new(cache);

        let get_err = tagged.get("__armature_keytags__:foo").await.unwrap_err();
        assert!(matches!(get_err, CacheError::Config(_)));

        let delete_err = tagged.delete("__armature_tag_index__").await.unwrap_err();
        assert!(matches!(delete_err, CacheError::Config(_)));
    }

    /// A caller key that merely CONTAINS the reserved prefix (not as a
    /// leading substring) is a perfectly ordinary key and must be accepted —
    /// only keys that actually *start with* the reserved prefix collide with
    /// the bookkeeping keyspace.
    #[tokio::test]
    async fn test_key_containing_but_not_starting_with_reserved_prefix_is_allowed() {
        let cache = Arc::new(MockCache::new());
        let tagged = TaggedCache::new(cache);

        tagged
            .set_with_tags(
                "user:__armature_tag__:not-a-prefix-collision",
                "fine".to_string(),
                &["users"],
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            tagged
                .get("user:__armature_tag__:not-a-prefix-collision")
                .await
                .unwrap(),
            Some("fine".to_string())
        );
    }

    /// Regression for Finding 3: `TaggedCache::new` must not panic/error when
    /// wrapping a backend that doesn't support atomic sets (it only logs a
    /// warning) — functionality is unaffected either way.
    #[tokio::test]
    async fn test_new_does_not_fail_on_non_atomic_backend() {
        let cache = Arc::new(MockCache::new());
        assert!(!cache.supports_atomic_sets());
        let tagged = TaggedCache::new(cache);

        // Still fully functional.
        tagged
            .set_with_tags("k", "v".to_string(), &["t"], None)
            .await
            .unwrap();
        assert_eq!(tagged.get("k").await.unwrap(), Some("v".to_string()));
    }
}
