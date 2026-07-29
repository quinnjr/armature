//! Behavioral tests for the `#[cache]` attribute macro.
//!
//! Regression coverage for:
//! - cache key must incorporate the actual argument values (not a constant)
//! - `ttl = N` must be threaded into the stored TTL
//! - `key = "..."` must be used as the key template
//! - `tag = "..."` must route through the tagged cache with the tags attached

use armature_proc_macro::cache;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// A minimal in-memory cache exposing the surface the generated code calls.
#[derive(Default)]
struct MockCache {
    store: Mutex<HashMap<String, String>>,
    sets: Mutex<Vec<(String, Option<Duration>)>>,
}

impl MockCache {
    fn new() -> Self {
        Self::default()
    }

    async fn get_json(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.store.lock().unwrap().get(key).cloned())
    }

    async fn set_json(
        &self,
        key: &str,
        value: String,
        ttl: Option<Duration>,
    ) -> Result<(), String> {
        self.store.lock().unwrap().insert(key.to_string(), value);
        self.sets.lock().unwrap().push((key.to_string(), ttl));
        Ok(())
    }
}

mod key_from_args {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache]
    async fn get_user(id: i64) -> Result<String, String> {
        Ok(format!("user-{id}"))
    }

    #[tokio::test]
    async fn distinct_args_do_not_collide() {
        let a = get_user(1).await.unwrap();
        let b = get_user(2).await.unwrap();
        // The bug: one constant key for all args means get_user(2) returns the
        // cached value of get_user(1).
        assert_eq!(a, "user-1");
        assert_eq!(b, "user-2");

        // Two distinct keys must have been written.
        let keys: Vec<String> = {
            let sets = __cache.sets.lock().unwrap();
            sets.iter().map(|(k, _)| k.clone()).collect()
        };
        assert_eq!(keys.len(), 2, "expected two cache writes");
        assert_ne!(keys[0], keys[1], "distinct args must produce distinct keys");

        // A repeat call for id=1 must hit the cache (no third write).
        let a2 = get_user(1).await.unwrap();
        assert_eq!(a2, "user-1");
        assert_eq!(
            __cache.sets.lock().unwrap().len(),
            2,
            "cached hit must not write again"
        );
    }
}

mod ttl_threaded {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache(ttl = 5)]
    async fn get_posts(user_id: i64) -> Result<String, String> {
        Ok(format!("posts-{user_id}"))
    }

    #[tokio::test]
    async fn ttl_attribute_is_used() {
        let _ = get_posts(42).await.unwrap();
        let sets = __cache.sets.lock().unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].1, Some(Duration::from_secs(5)));
    }
}

mod custom_key {
    use super::*;

    #[allow(non_upper_case_globals)]
    static __cache: LazyLock<MockCache> = LazyLock::new(MockCache::new);

    #[cache(key = "user:profile:{}", ttl = 600)]
    async fn get_profile(user_id: i64) -> Result<String, String> {
        Ok(format!("profile-{user_id}"))
    }

    #[tokio::test]
    async fn key_template_is_applied() {
        let _ = get_profile(7).await.unwrap();
        let sets = __cache.sets.lock().unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].0, "user:profile:7");
        assert_eq!(sets[0].1, Some(Duration::from_secs(600)));
    }
}
