//! Behavioral test for the tagged branch of `#[cache]`.
//!
//! `#[cache(tag = "...")]` must route through `__tagged_cache` and attach the
//! declared tags so tag-based invalidation is possible. Under the old code the
//! tag list was always empty, so the tagged branch was dead and only `__cache`
//! was ever referenced.

use armature_proc_macro::cache;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// (key, tags, ttl) recorded for each tagged write.
type TaggedSet = (String, Vec<String>, Option<Duration>);

#[derive(Default)]
struct MockTaggedCache {
    store: Mutex<HashMap<String, String>>,
    sets: Mutex<Vec<TaggedSet>>,
}

impl MockTaggedCache {
    fn new() -> Self {
        Self::default()
    }

    async fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.store.lock().unwrap().get(key).cloned())
    }

    async fn set_with_tags(
        &self,
        key: &str,
        value: String,
        tags: &[&str],
        ttl: Option<Duration>,
    ) -> Result<(), String> {
        self.store.lock().unwrap().insert(key.to_string(), value);
        self.sets.lock().unwrap().push((
            key.to_string(),
            tags.iter().map(|s| s.to_string()).collect(),
            ttl,
        ));
        Ok(())
    }
}

#[allow(non_upper_case_globals)]
static __tagged_cache: LazyLock<MockTaggedCache> = LazyLock::new(MockTaggedCache::new);

#[cache(ttl = 10, tag = "users")]
async fn get_all_users() -> Result<String, String> {
    Ok("all".to_string())
}

#[tokio::test]
async fn tags_route_through_tagged_cache() {
    let _ = get_all_users().await.unwrap();
    let sets = __tagged_cache.sets.lock().unwrap();
    assert_eq!(sets.len(), 1, "tagged cache must be written");
    assert_eq!(sets[0].1, vec!["users".to_string()]);
    assert_eq!(sets[0].2, Some(Duration::from_secs(10)));
}
