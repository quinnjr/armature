#![allow(clippy::needless_question_mark)]
//! Cache Improvements Example
//!
//! Demonstrates tag-based invalidation and multi-tier caching.

use armature_cache::*;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Cache Improvements Example ===\n");

    // 1. Tag-based Cache Invalidation
    println!("1. Tag-Based Cache Invalidation:");
    println!("   Creating tagged cache...\n");

    let base_cache = Arc::new(InMemoryCache::new());
    let tagged_cache = TaggedCache::new(base_cache.clone());

    // Set cache entries with tags
    tagged_cache
        .set_with_tags(
            "user:1",
            r#"{"id":1,"name":"Alice","role":"admin"}"#.to_string(),
            &["users", "admins"],
            Some(Duration::from_secs(3600)),
        )
        .await?;

    tagged_cache
        .set_with_tags(
            "user:2",
            r#"{"id":2,"name":"Bob","role":"user"}"#.to_string(),
            &["users"],
            Some(Duration::from_secs(3600)),
        )
        .await?;

    tagged_cache
        .set_with_tags(
            "post:1",
            r#"{"id":1,"title":"Hello World","author_id":1}"#.to_string(),
            &["posts", "user:1:posts"],
            Some(Duration::from_secs(1800)),
        )
        .await?;

    println!("   ✅ Cached 2 users and 1 post with tags");

    // Query by tags
    let user_keys = tagged_cache.get_keys_by_tag("users").await?;
    println!("   📊 Keys with 'users' tag: {} entries", user_keys.len());
    for key in &user_keys {
        println!("      - {}", key);
    }
    println!();

    let admin_keys = tagged_cache.get_keys_by_tag("admins").await?;
    println!("   📊 Keys with 'admins' tag: {} entries", admin_keys.len());
    for key in &admin_keys {
        println!("      - {}", key);
    }
    println!();

    // Verify cached data
    let user1 = tagged_cache.get("user:1").await?;
    println!("   ✅ Retrieved user:1: {}", user1.is_some());

    // Invalidate by tag
    println!("\n   🔥 Invalidating all 'users' tag entries...");
    tagged_cache.invalidate_tag("users").await?;

    // Verify invalidation
    let user1_after = tagged_cache.get("user:1").await?;
    let user2_after = tagged_cache.get("user:2").await?;
    let post1_after = tagged_cache.get("post:1").await?;

    println!(
        "   ✅ user:1 exists: {} (should be false)",
        user1_after.is_some()
    );
    println!(
        "   ✅ user:2 exists: {} (should be false)",
        user2_after.is_some()
    );
    println!(
        "   ✅ post:1 exists: {} (should be true)",
        post1_after.is_some()
    );
    println!();

    // Invalidate multiple tags
    println!("\n   🔥 Invalidating 'posts' and 'user:1:posts' tags...");
    tagged_cache
        .invalidate_tags(&["posts", "user:1:posts"])
        .await?;

    let post1_final = tagged_cache.get("post:1").await?;
    println!(
        "   ✅ post:1 exists: {} (should be false)",
        post1_final.is_some()
    );
    println!();

    // 2. Multi-Tier Caching
    println!("2. Multi-Tier Caching (L1/L2):");
    println!("   Creating tiered cache with L1 (memory) and L2 (memory simulating Redis)...\n");

    let l1 = Arc::new(InMemoryCache::new());
    let l2 = Arc::new(InMemoryCache::new());
    let tiered = TieredCache::new(l1.clone(), l2.clone());

    // Set value (writes to both L1 and L2)
    println!("   ✍️  Setting 'product:1' in tiered cache...");
    tiered
        .set(
            "product:1",
            r#"{"id":1,"name":"Laptop","price":999.99}"#.to_string(),
            Some(Duration::from_secs(300)),
        )
        .await?;

    // Check both tiers
    let in_l1 = l1.exists("product:1").await?;
    let in_l2 = l2.exists("product:1").await?;
    println!("   ✅ Exists in L1 (fast): {}", in_l1);
    println!("   ✅ Exists in L2 (distributed): {}", in_l2);
    println!();

    // Get from tiered cache (should hit L1)
    println!("   🔍 Getting 'product:1' (should hit L1)...");
    let product = tiered.get("product:1").await?;
    println!("   ✅ Retrieved: {}", product.is_some());
    println!();

    // Clear L1, data should still be in L2
    println!("   🗑️  Clearing L1 cache...");
    l1.clear().await?;
    println!("   ✅ L1 cleared");

    // Get again (should hit L2 and promote to L1)
    println!("\n   🔍 Getting 'product:1' again (should hit L2 and promote to L1)...");
    let product2 = tiered.get("product:1").await?;
    println!("   ✅ Retrieved from L2: {}", product2.is_some());

    // Check if promoted to L1
    let promoted = l1.exists("product:1").await?;
    println!("   ✅ Promoted to L1: {}", promoted);
    println!();

    // Custom tiered configuration
    println!("3. Custom Tiered Configuration:");
    let custom_config = TieredCacheConfig {
        enable_l1: true,
        enable_l2: true,
        write_through: true,
        promote_to_l1: true,
        l1_ttl_fraction: 0.1, // L1 lives 10% as long as L2
        ..Default::default()
    };

    let l1_custom = Arc::new(InMemoryCache::new());
    let l2_custom = Arc::new(InMemoryCache::new());
    let tiered_custom = TieredCache::with_config(l1_custom, l2_custom, custom_config);

    println!("   ✅ Created tiered cache with custom config");
    println!("      - Write-through: enabled");
    println!("      - Promote to L1: enabled");
    println!("      - L1 TTL: 10% of L2 TTL");
    println!();

    tiered_custom
        .set(
            "config:1",
            "test".to_string(),
            Some(Duration::from_secs(1000)),
        )
        .await?;
    println!("   ✅ Set value with 1000s TTL (L2), L1 TTL ~100s");
    println!();

    // 4. Combined: Tagged + Tiered
    println!("4. Combined Approach (Tagged + Tiered):");
    println!("   You can use TaggedCache with TieredCache as the backend!\n");

    let l1_combined = Arc::new(InMemoryCache::new());
    let l2_combined = Arc::new(InMemoryCache::new());
    let _tiered_backend = TieredCache::new(l1_combined, l2_combined);

    // Note: This would require wrapping TieredCache to implement CacheStore
    println!("   💡 Tip: Wrap TieredCache with CacheStore impl for tagged support");
    println!();

    println!("=== Cache Improvements Example Complete ===\n");
    println!("💡 Key Features Demonstrated:");
    println!("   ✅ Tag-based cache invalidation");
    println!("   ✅ Query cache entries by tag");
    println!("   ✅ Bulk invalidation by multiple tags");
    println!("   ✅ Multi-tier caching (L1 + L2)");
    println!("   ✅ Automatic L2 → L1 promotion");
    println!("   ✅ Custom tier configuration");
    println!("   ✅ Write-through caching");
    println!();
    println!("💡 Use Cases:");
    println!("   - Tag Invalidation:");
    println!("     • Invalidate all user-related caches on profile update");
    println!("     • Clear product caches when inventory changes");
    println!("     • Bulk cache busting for related entities");
    println!();
    println!("   - Multi-tier Caching:");
    println!("     • Fast L1 (in-memory) for hot data");
    println!("     • Persistent L2 (Redis) for shared data");
    println!("     • Reduced Redis traffic with local cache");
    println!("     • Automatic fallback and promotion");
    println!();
    println!("💡 Production Example:");
    println!(
        r#"
   // L1: In-memory (per-instance)
   let l1 = Arc::new(InMemoryCache::new());

   // L2: Redis (shared across instances)
   let redis_config = CacheConfig::redis("redis://localhost:6379")?;
   let l2 = Arc::new(RedisCache::new(redis_config).await?);

   // Tiered cache with tag support
   let tiered = TieredCache::new(l1, l2);
   let cache = TaggedCache::new(Arc::new(tiered));

   // Set with tags, cached in both L1 and L2
   cache.set_with_tags(
       "user:123:profile",
       user_json,
       &["users", "user:123"],
       Some(Duration::from_secs(3600))
   ).await?;

   // Invalidate all user:123 related caches
   cache.invalidate_tag("user:123").await?;
"#
    );

    Ok(())
}
