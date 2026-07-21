//! Tag-based cache invalidation

use crate::error::CacheResult;
use crate::traits::CacheStore;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Cache with tag-based invalidation support
pub struct TaggedCache<C: CacheStore> {
    /// Underlying cache store
    cache: Arc<C>,

    /// Tag to keys mapping
    tags: Arc<RwLock<HashMap<String, HashSet<String>>>>,

    /// Key to tags mapping
    key_tags: Arc<RwLock<HashMap<String, HashSet<String>>>>,
}

impl<C: CacheStore> TaggedCache<C> {
    /// Create new tagged cache
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
        Self {
            cache,
            tags: Arc::new(RwLock::new(HashMap::new())),
            key_tags: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set a value with tags
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
        // Set in cache
        self.cache.set_json(key, value, ttl).await?;

        // Update tag mappings
        let mut tags_map = self.tags.write().await;
        let mut key_tags_map = self.key_tags.write().await;

        for tag in tags {
            tags_map
                .entry(tag.to_string())
                .or_insert_with(HashSet::new)
                .insert(key.to_string());
        }

        let tag_set: HashSet<String> = tags.iter().map(|t| t.to_string()).collect();
        key_tags_map.insert(key.to_string(), tag_set);

        Ok(())
    }

    /// Get value from cache
    pub async fn get(&self, key: &str) -> CacheResult<Option<String>> {
        self.cache.get_json(key).await
    }

    /// Delete a specific key
    pub async fn delete(&self, key: &str) -> CacheResult<()> {
        // Delete from cache
        self.cache.delete(key).await?;

        // Remove from tag mappings
        let mut tags_map = self.tags.write().await;
        let mut key_tags_map = self.key_tags.write().await;

        if let Some(tag_set) = key_tags_map.remove(key) {
            for tag in tag_set {
                if let Some(keys) = tags_map.get_mut(&tag) {
                    keys.remove(key);
                    if keys.is_empty() {
                        tags_map.remove(&tag);
                    }
                }
            }
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
        let mut tags_map = self.tags.write().await;
        let mut key_tags_map = self.key_tags.write().await;

        if let Some(keys) = tags_map.remove(tag) {
            // Delete all keys with this tag
            let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            self.cache.delete_many(&key_refs).await?;

            // Remove from key_tags mapping
            for key in keys {
                if let Some(tag_set) = key_tags_map.get_mut(&key) {
                    tag_set.remove(tag);
                    if tag_set.is_empty() {
                        key_tags_map.remove(&key);
                    }
                }
            }
        }

        Ok(())
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
        // Coalesce into a single acquisition of each lock and a single backend
        // round-trip, instead of re-locking and issuing a separate `delete_many`
        // per tag (the old per-tag `invalidate_tag` loop). Keys shared across
        // several of the requested tags are deleted exactly once.
        let mut tags_map = self.tags.write().await;
        let mut key_tags_map = self.key_tags.write().await;

        // Gather the union of keys across all requested tags, removing those
        // tags from the tag->keys index as we go.
        let mut victims: HashSet<String> = HashSet::new();
        for tag in tags {
            if let Some(keys) = tags_map.remove(*tag) {
                victims.extend(keys);
            }
        }

        if victims.is_empty() {
            return Ok(());
        }

        // One batch delete for the whole union.
        let key_refs: Vec<&str> = victims.iter().map(|s| s.as_str()).collect();
        self.cache.delete_many(&key_refs).await?;

        // Update the reverse index once. A key may carry tags beyond those being
        // invalidated; drop only the invalidated tags, and remove the key entry
        // entirely once it has no tags left.
        let removed: HashSet<&str> = tags.iter().copied().collect();
        for key in &victims {
            if let Some(tag_set) = key_tags_map.get_mut(key) {
                tag_set.retain(|t| !removed.contains(t.as_str()));
                if tag_set.is_empty() {
                    key_tags_map.remove(key);
                }
            }
        }

        Ok(())
    }

    /// Get all keys with a specific tag
    pub async fn get_keys_by_tag(&self, tag: &str) -> Vec<String> {
        let tags_map = self.tags.read().await;
        tags_map
            .get(tag)
            .map(|keys| keys.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all tags for a specific key
    pub async fn get_tags_for_key(&self, key: &str) -> Vec<String> {
        let key_tags_map = self.key_tags.read().await;
        key_tags_map
            .get(key)
            .map(|tags| tags.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all registered tags
    pub async fn list_tags(&self) -> Vec<String> {
        let tags_map = self.tags.read().await;
        tags_map.keys().cloned().collect()
    }
}

impl<C: CacheStore> Clone for TaggedCache<C> {
    fn clone(&self) -> Self {
        Self {
            cache: self.cache.clone(),
            tags: self.tags.clone(),
            key_tags: self.key_tags.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CacheResult;
    use async_trait::async_trait;

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
        let user_keys = tagged.get_keys_by_tag("users").await;
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

        let tags = tagged.get_tags_for_key("key1").await;
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
        assert!(tagged.list_tags().await.is_empty());
        assert!(tagged.get_tags_for_key("shared").await.is_empty());
    }
}
