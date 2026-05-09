//! Load Testing Utilities
//!
//! Provides performance testing and load generation capabilities.

use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::Mutex;

/// Load testing errors
#[derive(Debug, Error)]
pub enum LoadTestError {
    #[error("Test failed: {0}")]
    TestFailed(String),

    #[error("Timeout: {0}")]
    Timeout(String),
}

/// Load test statistics
#[derive(Debug, Clone)]
pub struct LoadTestStats {
    /// Total number of requests
    pub total_requests: u64,

    /// Successful requests
    pub successful: u64,

    /// Failed requests
    pub failed: u64,

    /// Total duration
    pub duration: Duration,

    /// Min response time
    pub min_response_time: Duration,

    /// Max response time
    pub max_response_time: Duration,

    /// Average response time
    pub avg_response_time: Duration,

    /// Median response time (p50)
    pub median_response_time: Duration,

    /// 95th percentile response time
    pub p95_response_time: Duration,

    /// 99th percentile response time
    pub p99_response_time: Duration,

    /// Requests per second
    pub rps: f64,
}

impl LoadTestStats {
    /// Calculate statistics from response times
    pub fn from_response_times(
        response_times: &[Duration],
        failed: u64,
        total_duration: Duration,
    ) -> Self {
        let mut sorted = response_times.to_vec();
        sorted.sort();

        let total = response_times.len() as u64;
        let sum: Duration = response_times.iter().sum();

        let min = sorted.first().copied().unwrap_or_default();
        let max = sorted.last().copied().unwrap_or_default();
        let avg = if !sorted.is_empty() {
            sum / sorted.len() as u32
        } else {
            Duration::default()
        };

        let median = if !sorted.is_empty() {
            sorted[sorted.len() / 2]
        } else {
            Duration::default()
        };

        let p95_idx = (sorted.len() as f64 * 0.95) as usize;
        let p95 = sorted.get(p95_idx).copied().unwrap_or(max);

        let p99_idx = (sorted.len() as f64 * 0.99) as usize;
        let p99 = sorted.get(p99_idx).copied().unwrap_or(max);

        let rps = if total_duration.as_secs_f64() > 0.0 {
            total as f64 / total_duration.as_secs_f64()
        } else {
            0.0
        };

        Self {
            total_requests: total + failed,
            successful: total,
            failed,
            duration: total_duration,
            min_response_time: min,
            max_response_time: max,
            avg_response_time: avg,
            median_response_time: median,
            p95_response_time: p95,
            p99_response_time: p99,
            rps,
        }
    }

    /// Print statistics
    pub fn print(&self) {
        println!("\n========== Load Test Results ==========");
        println!("Total Requests:     {}", self.total_requests);
        println!("Successful:         {}", self.successful);
        println!("Failed:             {}", self.failed);
        println!("Duration:           {:.2}s", self.duration.as_secs_f64());
        println!("Requests/sec:       {:.2}", self.rps);
        println!("\nResponse Times:");
        println!(
            "  Min:              {:.2}ms",
            self.min_response_time.as_millis()
        );
        println!(
            "  Avg:              {:.2}ms",
            self.avg_response_time.as_millis()
        );
        println!(
            "  Median (p50):     {:.2}ms",
            self.median_response_time.as_millis()
        );
        println!(
            "  p95:              {:.2}ms",
            self.p95_response_time.as_millis()
        );
        println!(
            "  p99:              {:.2}ms",
            self.p99_response_time.as_millis()
        );
        println!(
            "  Max:              {:.2}ms",
            self.max_response_time.as_millis()
        );
        println!("=======================================\n");
    }
}

/// Load test configuration
#[derive(Debug, Clone)]
pub struct LoadTestConfig {
    /// Number of concurrent requests
    pub concurrency: usize,

    /// Total number of requests
    pub total_requests: u64,

    /// Duration of test (alternative to total_requests)
    pub duration: Option<Duration>,

    /// Requests per second limit
    pub rate_limit: Option<f64>,

    /// Timeout per request
    pub timeout: Duration,
}

impl LoadTestConfig {
    /// Create new load test config
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::load::LoadTestConfig;
    ///
    /// let config = LoadTestConfig::new(10, 1000);
    /// ```
    pub fn new(concurrency: usize, total_requests: u64) -> Self {
        Self {
            concurrency,
            total_requests,
            duration: None,
            rate_limit: None,
            timeout: Duration::from_secs(30),
        }
    }

    /// Set test duration instead of request count
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Set rate limit (requests per second)
    pub fn with_rate_limit(mut self, rps: f64) -> Self {
        self.rate_limit = Some(rps);
        self
    }

    /// Set timeout per request
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Default for LoadTestConfig {
    fn default() -> Self {
        Self::new(10, 100)
    }
}

/// Load test runner
pub struct LoadTestRunner<F, Fut>
where
    F: Fn() -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<(), LoadTestError>> + Send + 'static,
{
    config: LoadTestConfig,
    test_fn: Arc<F>,
}

impl<F, Fut> LoadTestRunner<F, Fut>
where
    F: Fn() -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<(), LoadTestError>> + Send + 'static,
{
    /// Create new load test runner
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_testing::load::*;
    ///
    /// let config = LoadTestConfig::new(10, 100);
    /// let runner = LoadTestRunner::new(config, || async {
    ///     // Your test code here
    ///     Ok(())
    /// });
    /// ```
    pub fn new(config: LoadTestConfig, test_fn: F) -> Self {
        Self {
            config,
            test_fn: Arc::new(test_fn),
        }
    }

    /// Run load test
    pub async fn run(&self) -> Result<LoadTestStats, LoadTestError> {
        let start_time = Instant::now();
        let response_times = Arc::new(Mutex::new(Vec::new()));
        let failed_count = Arc::new(Mutex::new(0u64));

        let mut handles = vec![];

        // Determine number of requests per worker
        let requests_per_worker = if let Some(_duration) = self.config.duration {
            // Duration-based test
            None
        } else {
            // Request count-based test
            Some(self.config.total_requests / self.config.concurrency as u64)
        };

        for _ in 0..self.config.concurrency {
            let test_fn = self.test_fn.clone();
            let response_times = response_times.clone();
            let failed_count = failed_count.clone();
            let timeout = self.config.timeout;
            let duration = self.config.duration;

            let handle = tokio::spawn(async move {
                let worker_start = Instant::now();
                let mut request_count = 0u64;

                loop {
                    // Check if we should stop
                    if let Some(duration) = duration {
                        if worker_start.elapsed() >= duration {
                            break;
                        }
                    } else if let Some(max_requests) = requests_per_worker
                        && request_count >= max_requests
                    {
                        break;
                    }

                    // Execute test function
                    let req_start = Instant::now();
                    let result = tokio::time::timeout(timeout, test_fn()).await;

                    match result {
                        Ok(Ok(())) => {
                            let elapsed = req_start.elapsed();
                            response_times.lock().await.push(elapsed);
                        }
                        _ => {
                            *failed_count.lock().await += 1;
                        }
                    }

                    request_count += 1;
                }
            });

            handles.push(handle);
        }

        // Wait for all workers to complete
        for handle in handles {
            let _ = handle.await;
        }

        let total_duration = start_time.elapsed();
        let response_times = response_times.lock().await;
        let failed = *failed_count.lock().await;

        Ok(LoadTestStats::from_response_times(
            &response_times,
            failed,
            total_duration,
        ))
    }
}

/// Stress test runner (gradually increases load)
pub struct StressTestRunner<F, Fut>
where
    F: Fn() -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<(), LoadTestError>> + Send + 'static,
{
    initial_concurrency: usize,
    max_concurrency: usize,
    step_size: usize,
    step_duration: Duration,
    test_fn: Arc<F>,
}

impl<F, Fut> StressTestRunner<F, Fut>
where
    F: Fn() -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<(), LoadTestError>> + Send + 'static,
{
    /// Create new stress test runner
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_testing::load::*;
    /// use std::time::Duration;
    ///
    /// let runner = StressTestRunner::new(1, 100, 10, Duration::from_secs(10), || async {
    ///     // Your test code
    ///     Ok(())
    /// });
    /// ```
    pub fn new(
        initial_concurrency: usize,
        max_concurrency: usize,
        step_size: usize,
        step_duration: Duration,
        test_fn: F,
    ) -> Self {
        Self {
            initial_concurrency,
            max_concurrency,
            step_size,
            step_duration,
            test_fn: Arc::new(test_fn),
        }
    }

    /// Run stress test
    pub async fn run(&self) -> Result<Vec<(usize, LoadTestStats)>, LoadTestError> {
        let mut results = vec![];
        let mut concurrency = self.initial_concurrency;

        println!("\n========== Stress Test Starting ==========");
        println!("Initial Concurrency: {}", self.initial_concurrency);
        println!("Max Concurrency:     {}", self.max_concurrency);
        println!("Step Size:           {}", self.step_size);
        println!(
            "Step Duration:       {:.0}s",
            self.step_duration.as_secs_f64()
        );
        println!("==========================================\n");

        while concurrency <= self.max_concurrency {
            println!("Testing with {} concurrent requests...", concurrency);

            let config =
                LoadTestConfig::new(concurrency, u64::MAX).with_duration(self.step_duration);

            let runner = LoadTestRunner::new(config, self.test_fn.as_ref().clone());
            let stats = runner.run().await?;

            println!(
                "  RPS: {:.2}, Avg: {:.2}ms, p95: {:.2}ms",
                stats.rps,
                stats.avg_response_time.as_millis(),
                stats.p95_response_time.as_millis()
            );

            results.push((concurrency, stats));
            concurrency += self.step_size;
        }

        println!("\n========== Stress Test Complete ==========\n");

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_test_config() {
        let config = LoadTestConfig::new(10, 100)
            .with_rate_limit(50.0)
            .with_timeout(Duration::from_secs(5));

        assert_eq!(config.concurrency, 10);
        assert_eq!(config.total_requests, 100);
        assert_eq!(config.rate_limit, Some(50.0));
    }

    #[test]
    fn test_load_test_stats() {
        let response_times = vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(150),
            Duration::from_millis(300),
            Duration::from_millis(250),
        ];

        let stats = LoadTestStats::from_response_times(&response_times, 0, Duration::from_secs(1));

        assert_eq!(stats.total_requests, 5);
        assert_eq!(stats.successful, 5);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.min_response_time, Duration::from_millis(100));
        assert_eq!(stats.max_response_time, Duration::from_millis(300));
    }

    #[tokio::test]
    async fn test_load_test_runner() {
        let config = LoadTestConfig::new(2, 10);

        let runner = LoadTestRunner::new(config, || async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            Ok(())
        });

        let stats = runner.run().await.unwrap();
        assert_eq!(stats.total_requests, 10);
        assert_eq!(stats.successful, 10);
    }
}
