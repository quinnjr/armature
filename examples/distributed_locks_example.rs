//! Distributed Locks Example
//!
//! Demonstrates Redis-based distributed locks (`armature_distributed::RedisLock`
//! and `LockBuilder`) coordinating work across multiple instances.
//!
//! Several concurrent Tokio tasks stand in for separate application
//! instances. All of them contend for the *same* named Redis lock; the
//! example proves mutual exclusion empirically — by tracking how many
//! instances are inside the critical section at once (it must never exceed
//! one) — and shows the non-blocking `try_acquire()` API observing the lock
//! as busy while another instance holds it.
//!
//! Note: This example requires Redis to be running
//! Start Redis: docker run -p 6379:6379 redis
//!
//! If Redis isn't reachable, the example prints instructions and exits
//! cleanly instead of failing.

use armature_distributed::{DistributedLock, LockBuilder, RedisLock};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const REDIS_URL: &str = "redis://127.0.0.1:6379/";
const LOCK_KEY: &str = "armature-example:order-processor";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Distributed Locks Example ===\n");

    let client = redis::Client::open(REDIS_URL)?;
    let mut conn = match client.get_connection_manager().await {
        Ok(conn) => conn,
        Err(e) => {
            println!("⚠️  Could not connect to Redis at {REDIS_URL}: {e}");
            println!("   Start Redis with: docker run -p 6379:6379 redis\n");
            println!("   Skipping example — no Redis available.\n");
            return Ok(());
        }
    };
    println!("✅ Connected to Redis at {REDIS_URL}\n");

    // Clear any leftover state from a previous (possibly interrupted) run.
    let _: () = redis::cmd("DEL")
        .arg(LOCK_KEY)
        .query_async(&mut conn)
        .await?;

    mutual_exclusion_demo(conn.clone()).await?;
    try_acquire_demo(conn.clone()).await?;
    builder_demo(conn.clone()).await?;

    println!("=== Distributed Locks Example Complete ===\n");
    println!("💡 Key Features Demonstrated:");
    println!("   - Real mutual exclusion across concurrently racing instances");
    println!("   - Blocking acquire_timeout() and non-blocking try_acquire()");
    println!("   - RAII auto-release on drop (LockBuilder + guard)");
    println!("   - LockGuard::is_held() fencing check before committing work");
    println!();

    Ok(())
}

/// Spawns several tasks — standing in for separate application instances —
/// that all race to acquire the *same* lock via the blocking
/// `acquire_timeout()` API. Only one may hold it at a time; this is proven
/// with a shared counter that must never exceed 1 while any instance is
/// inside its critical section, and a total wall-clock time that shows the
/// work was serialized rather than run in parallel.
async fn mutual_exclusion_demo(
    conn: redis::aio::ConnectionManager,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- 1. Mutual Exclusion Across Simulated Instances ---\n");

    const INSTANCES: usize = 4;
    println!(
        "Spawning {INSTANCES} concurrent tasks (simulated instances) contending for '{LOCK_KEY}'...\n"
    );

    let concurrent_holders = Arc::new(AtomicUsize::new(0));
    let max_observed = Arc::new(AtomicUsize::new(0));
    let start = Instant::now();

    let mut handles = Vec::new();
    for id in 0..INSTANCES {
        let conn = conn.clone();
        let concurrent_holders = concurrent_holders.clone();
        let max_observed = max_observed.clone();

        handles.push(tokio::spawn(async move {
            // Each simulated instance builds its own lock handle over its
            // own connection but targets the same Redis key, exactly as
            // independent processes would.
            let lock = RedisLock::new(LOCK_KEY, Duration::from_secs(10), conn);

            println!("  [instance {id}] waiting to acquire lock...");
            let guard = lock
                .acquire_timeout(Duration::from_secs(15))
                .await
                .expect("acquire_timeout should succeed within 15s");
            println!("  [instance {id}] acquired lock at {:?}", start.elapsed());

            // Track how many instances hold the lock concurrently; this
            // must never rise above 1 if mutual exclusion actually holds.
            let now_holding = concurrent_holders.fetch_add(1, Ordering::SeqCst) + 1;
            max_observed.fetch_max(now_holding, Ordering::SeqCst);
            assert_eq!(
                now_holding, 1,
                "mutual exclusion violated: {now_holding} instances held the lock simultaneously"
            );

            // Simulated critical-section work.
            tokio::time::sleep(Duration::from_millis(150)).await;

            // Per LockGuard::is_held's contract: re-check the fence
            // immediately before committing any externally-visible effect.
            assert!(guard.is_held(), "lease was lost mid-critical-section");
            println!("  [instance {id}] critical work done");

            concurrent_holders.fetch_sub(1, Ordering::SeqCst);
            guard.release().await.expect("release should succeed");
            println!("  [instance {id}] released lock at {:?}", start.elapsed());
        }));
    }

    for handle in handles {
        handle.await?;
    }

    println!(
        "\n  Max concurrent holders observed: {} (must be 1 to prove mutual exclusion)",
        max_observed.load(Ordering::SeqCst)
    );
    println!(
        "  Total wall time: {:?} (serialized rather than parallel confirms real contention)\n",
        start.elapsed()
    );

    Ok(())
}

/// Demonstrates the non-blocking `try_acquire()` API: one instance holds
/// the lock while a second instance's `try_acquire()` observes it as busy
/// (`None`), then succeeds once the first instance releases.
async fn try_acquire_demo(
    mut conn: redis::aio::ConnectionManager,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- 2. Non-Blocking try_acquire() ---\n");

    let key = "armature-example:try-acquire-demo";
    let _: () = redis::cmd("DEL").arg(key).query_async(&mut conn).await?;

    let holder = RedisLock::new(key, Duration::from_secs(5), conn.clone());
    let contender = RedisLock::new(key, Duration::from_secs(5), conn.clone());

    let guard = holder
        .try_acquire()
        .await?
        .expect("first try_acquire should succeed on an unheld lock");
    println!("  [holder]    acquired '{key}'");

    let contended = contender.try_acquire().await?;
    println!(
        "  [contender] try_acquire while held -> {}",
        if contended.is_some() {
            "Some (unexpected!)"
        } else {
            "None (lock busy, as expected)"
        }
    );
    assert!(
        contended.is_none(),
        "try_acquire must fail while the lock is held elsewhere"
    );

    guard.release().await?;
    println!("  [holder]    released '{key}'");

    let now_free = contender.try_acquire().await?;
    println!(
        "  [contender] try_acquire after release -> {}",
        if now_free.is_some() {
            "Some (acquired!)"
        } else {
            "None (unexpected!)"
        }
    );
    assert!(
        now_free.is_some(),
        "try_acquire must succeed once the lock is released"
    );
    now_free.unwrap().release().await?;

    println!();
    Ok(())
}

/// Demonstrates constructing a lock via `LockBuilder` and the RAII guard:
/// dropping the guard without an explicit `release()` call still frees the
/// lock (best-effort release spawned from `Drop`).
async fn builder_demo(
    mut conn: redis::aio::ConnectionManager,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- 3. LockBuilder + RAII Auto-Release ---\n");

    let key = "armature-example:builder-demo";
    let _: () = redis::cmd("DEL").arg(key).query_async(&mut conn).await?;

    let lock = LockBuilder::new(key)
        .with_ttl(Duration::from_secs(10))
        .build(conn.clone());

    {
        let guard = lock.acquire().await?;
        println!("  acquired '{key}' via LockBuilder");
        assert!(guard.is_held());
        // `guard` drops here with no explicit `release()` call; `Drop`
        // performs a best-effort release in the background.
    }

    // Give the drop's spawned release task a moment to run, then confirm
    // the lock is actually free.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let reacquired = lock.try_acquire().await?;
    println!(
        "  lock free after guard dropped (no explicit release) -> {}",
        if reacquired.is_some() { "yes" } else { "no" }
    );
    assert!(
        reacquired.is_some(),
        "RAII drop should have released the lock"
    );
    reacquired.unwrap().release().await?;

    println!();
    Ok(())
}
