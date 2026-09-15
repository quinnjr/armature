#![allow(clippy::all)]
#![allow(clippy::needless_question_mark)]
//! Load Testing Example
//!
//! Drives `armature_testing::load` against synthetic workloads - `sleep`s
//! standing in for I/O - so the runner itself can be demonstrated without a
//! live server. The numbers printed describe the sleeps, not any real system.
//!
//! The "real API" section at the end is a template: its HTTP call is commented
//! out because this example has no server to talk to, and adding a `reqwest`
//! dependency to demonstrate one line of client code is not worth it. Uncomment
//! it and point it at your own server to use it.
//!
//! Each section asserts on the stats it produced, so a regression that broke
//! request accounting - the wrong number of requests, failures miscounted,
//! quantiles out of order - fails this example rather than printing a plausible
//! wrong table.
//!
//! ```bash
//! cargo run --example testing_load
//! ```

use armature_testing::load::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Load Testing Example ===\n");

    // 1. Basic load test
    println!("1. Basic Load Test (10 concurrent, 100 requests):");
    println!("   Starting load test...");

    let basic_config = LoadTestConfig::new(10, 100).with_timeout(Duration::from_secs(5));

    let basic_runner = LoadTestRunner::new(basic_config, || async {
        // Simulate API call
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(())
    });

    let stats = basic_runner.run().await?;
    stats.print();

    // Every request was accounted for, and none of them failed: a workload
    // that only sleeps has nothing to fail on.
    assert_eq!(
        stats.total_requests, 100,
        "all 100 requests must be counted"
    );
    assert_eq!(stats.successful, 100);
    assert_eq!(stats.failed, 0);
    // Quantiles must be ordered, whatever the machine's timing noise.
    assert!(stats.min_response_time <= stats.median_response_time);
    assert!(stats.median_response_time <= stats.p95_response_time);
    assert!(stats.p95_response_time <= stats.p99_response_time);
    assert!(stats.p99_response_time <= stats.max_response_time);

    // 2. Duration-based load test
    println!("\n2. Duration-Based Load Test (5 concurrent, 3 seconds):");
    println!("   Starting duration-based test...");

    let duration_config = LoadTestConfig::new(5, u64::MAX)
        .with_duration(Duration::from_secs(3))
        .with_timeout(Duration::from_secs(5));

    let duration_runner = LoadTestRunner::new(duration_config, || async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        Ok(())
    });

    let stats = duration_runner.run().await?;
    stats.print();

    // A duration-based run has no request target, but it must have run for
    // roughly the configured window and completed at least one request.
    assert!(stats.total_requests > 0, "duration-based run must do work");
    assert!(stats.duration >= Duration::from_secs(3));

    // 3. Load test with failures
    println!("\n3. Load Test with Some Failures:");
    println!("   Starting load test with 20% failure rate...");

    let failure_count = Arc::new(AtomicU32::new(0));
    let failure_config = LoadTestConfig::new(5, 50).with_timeout(Duration::from_secs(5));

    let failure_count_clone = failure_count.clone();
    let failure_runner = LoadTestRunner::new(failure_config, move || {
        let count = failure_count_clone.clone();
        async move {
            let current = count.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(40)).await;

            // Fail 20% of requests
            if current % 5 == 0 {
                Err(LoadTestError::TestFailed("Simulated failure".to_string()))
            } else {
                Ok(())
            }
        }
    });

    let stats = failure_runner.run().await?;
    stats.print();

    // Exactly one request in five is made to fail, so of 50 requests exactly
    // 10 must be counted as failures and 40 as successes. This is the
    // assertion that catches failure accounting silently reporting zero.
    assert_eq!(stats.total_requests, 50);
    assert_eq!(
        stats.failed, 10,
        "20% of 50 requests must be recorded failed"
    );
    assert_eq!(stats.successful, 40);

    // 4. Stress test (gradually increasing load)
    println!("\n4. Stress Test (1 → 20 concurrent, step by 5, 2 seconds per step):");
    println!("   Starting stress test...");

    let stress_runner = StressTestRunner::new(
        1,                      // Initial concurrency
        20,                     // Max concurrency
        5,                      // Step size
        Duration::from_secs(2), // Duration per step
        || async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(())
        },
    );

    let stress_results = stress_runner.run().await?;

    // 1, 6, 11, 16 - stepping by 5 from 1 up to (not past) 20.
    assert!(
        !stress_results.is_empty(),
        "a stress test must produce at least one step"
    );
    let concurrencies: Vec<_> = stress_results.iter().map(|(c, _)| *c).collect();
    assert!(
        concurrencies.windows(2).all(|w| w[0] < w[1]),
        "each step must raise concurrency: {concurrencies:?}"
    );
    assert!(
        concurrencies.iter().all(|c| *c <= 20),
        "no step may exceed the configured maximum: {concurrencies:?}"
    );

    // Print stress test summary
    println!("\nStress Test Summary:");
    println!("┌─────────────┬──────────┬────────────┬────────────┐");
    println!("│ Concurrency │ RPS      │ Avg (ms)   │ p95 (ms)   │");
    println!("├─────────────┼──────────┼────────────┼────────────┤");
    for (concurrency, stats) in stress_results {
        println!(
            "│ {:11} │ {:8.2} │ {:10.2} │ {:10.2} │",
            concurrency,
            stats.rps,
            stats.avg_response_time.as_millis(),
            stats.p95_response_time.as_millis()
        );
    }
    println!("└─────────────┴──────────┴────────────┴────────────┘");

    // 5. Real-world example: Testing an API endpoint
    println!("\n5. Real-World Example: API Endpoint Load Test");
    println!("   (Simulated - replace with actual HTTP client)");

    let api_config = LoadTestConfig::new(20, 200).with_timeout(Duration::from_secs(10));

    let api_runner = LoadTestRunner::new(api_config, || async {
        // In a real scenario, you would use reqwest or similar:
        // let response = reqwest::get("http://localhost:3000/api/users").await?;
        // if !response.status().is_success() {
        //     return Err(LoadTestError::TestFailed("Request failed".to_string()));
        // }

        // Simulated API call
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    });

    let stats = api_runner.run().await?;
    stats.print();

    println!("=== Load Testing Complete ===\n");
    println!("💡 Tips:");
    println!("   - Use LoadTestRunner for fixed request counts");
    println!("   - Use duration-based tests for sustained load");
    println!("   - Use StressTestRunner to find breaking points");
    println!("   - Monitor p95/p99 latencies for SLA compliance");
    println!();

    Ok(())
}
