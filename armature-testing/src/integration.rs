//! Integration Test Helpers
//!
//! Provides database setup/teardown and integration test utilities.

use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;

/// Type alias for async test hook function.
pub type AsyncTestHookFn = Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Integration test errors
#[derive(Debug, Error)]
pub enum IntegrationTestError {
    #[error("Setup failed: {0}")]
    SetupFailed(String),

    #[error("Teardown failed: {0}")]
    TeardownFailed(String),

    #[error("Database error: {0}")]
    Database(String),
}

/// Database test helper trait
///
/// Implement this trait for your database to provide setup/teardown.
#[async_trait]
pub trait DatabaseTestHelper: Send + Sync {
    /// Setup database for testing (create tables, seed data, etc.)
    async fn setup(&self) -> Result<(), IntegrationTestError>;

    /// Teardown database after testing (drop tables, clean data, etc.)
    async fn teardown(&self) -> Result<(), IntegrationTestError>;

    /// Reset database to clean state
    async fn reset(&self) -> Result<(), IntegrationTestError> {
        self.teardown().await?;
        self.setup().await
    }

    /// Run migrations
    async fn migrate(&self) -> Result<(), IntegrationTestError> {
        Ok(())
    }

    /// Seed test data
    async fn seed(&self) -> Result<(), IntegrationTestError> {
        Ok(())
    }
}

/// Test fixture
///
/// Manages test lifecycle with automatic setup/teardown.
pub struct TestFixture<T: DatabaseTestHelper> {
    db_helper: Arc<T>,
    auto_cleanup: bool,
}

impl<T: DatabaseTestHelper> TestFixture<T> {
    /// Create new test fixture
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let fixture = TestFixture::new(Arc::new(MyDbHelper::new()));
    /// ```
    pub fn new(db_helper: Arc<T>) -> Self {
        Self {
            db_helper,
            auto_cleanup: true,
        }
    }

    /// Disable automatic cleanup
    pub fn without_auto_cleanup(mut self) -> Self {
        self.auto_cleanup = false;
        self
    }

    /// Run setup
    pub async fn setup(&self) -> Result<(), IntegrationTestError> {
        self.db_helper.setup().await
    }

    /// Run teardown
    pub async fn teardown(&self) -> Result<(), IntegrationTestError> {
        self.db_helper.teardown().await
    }

    /// Run test with automatic setup/teardown
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// fixture.run_test(|| async {
    ///     // Your test code here
    ///     // Database is automatically set up before and torn down after
    ///     Ok(())
    /// }).await?;
    /// ```
    pub async fn run_test<F, Fut>(&self, test_fn: F) -> Result<(), IntegrationTestError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), IntegrationTestError>>,
    {
        // Setup
        self.setup().await?;

        // Run test
        let result = test_fn().await;

        // Teardown (even if test failed)
        if self.auto_cleanup
            && let Err(e) = self.teardown().await
        {
            eprintln!("Warning: Teardown failed: {}", e);
        }

        result
    }
}

/// Integration test builder
pub struct IntegrationTestBuilder {
    #[allow(dead_code)]
    name: String,
    before_each: Vec<AsyncTestHookFn>,
    after_each: Vec<AsyncTestHookFn>,
}

impl IntegrationTestBuilder {
    /// Create new integration test builder
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            before_each: Vec::new(),
            after_each: Vec::new(),
        }
    }

    /// Add before_each hook
    pub fn before_each<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.before_each.push(Box::new(move || Box::pin(f())));
        self
    }

    /// Add after_each hook
    pub fn after_each<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        self.after_each.push(Box::new(move || Box::pin(f())));
        self
    }

    /// Run `body` with the collected hooks around it: every `before_each`
    /// hook is awaited (in registration order), then `body`, then every
    /// `after_each` hook (in registration order). `after_each` hooks run
    /// whenever `body` returns normally; if `body` panics, the panic
    /// unwinds through this function and `after_each` hooks do *not* run
    /// (matching `TestFixture::run_test`'s non-catching behavior).
    ///
    /// # Examples
    ///
    /// ```
    /// use armature_testing::integration::IntegrationTestBuilder;
    /// use std::sync::Arc;
    /// use std::sync::atomic::{AtomicUsize, Ordering};
    ///
    /// # tokio_test::block_on(async {
    /// let order = Arc::new(AtomicUsize::new(0));
    /// let before_slot = order.clone();
    /// let after_slot = order.clone();
    ///
    /// let builder = IntegrationTestBuilder::new("example")
    ///     .before_each(move || {
    ///         let before_slot = before_slot.clone();
    ///         async move { before_slot.store(1, Ordering::SeqCst); }
    ///     })
    ///     .after_each(move || {
    ///         let after_slot = after_slot.clone();
    ///         async move { after_slot.store(3, Ordering::SeqCst); }
    ///     });
    ///
    /// let seen_during_body = builder
    ///     .run_test(|| async { order.load(Ordering::SeqCst) })
    ///     .await;
    ///
    /// assert_eq!(seen_during_body, 1); // before_each already ran
    /// assert_eq!(order.load(Ordering::SeqCst), 3); // after_each ran too
    /// # });
    /// ```
    pub async fn run_test<F, Fut, T>(&self, body: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        for hook in &self.before_each {
            hook().await;
        }

        let result = body().await;

        for hook in &self.after_each {
            hook().await;
        }

        result
    }
}

/// Database seeder
pub struct DatabaseSeeder {
    fixtures: Vec<String>,
}

impl DatabaseSeeder {
    /// Create new database seeder
    pub fn new() -> Self {
        Self {
            fixtures: Vec::new(),
        }
    }

    /// Add fixture
    pub fn add_fixture(mut self, fixture: impl Into<String>) -> Self {
        self.fixtures.push(fixture.into());
        self
    }

    /// Get all fixtures
    pub fn fixtures(&self) -> &[String] {
        &self.fixtures
    }
}

impl Default for DatabaseSeeder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockDbHelper;

    #[async_trait]
    impl DatabaseTestHelper for MockDbHelper {
        async fn setup(&self) -> Result<(), IntegrationTestError> {
            Ok(())
        }

        async fn teardown(&self) -> Result<(), IntegrationTestError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_fixture_setup_teardown() {
        let helper = Arc::new(MockDbHelper);
        let fixture = TestFixture::new(helper);

        fixture.setup().await.unwrap();
        fixture.teardown().await.unwrap();
    }

    #[tokio::test]
    async fn test_fixture_run_test() {
        let helper = Arc::new(MockDbHelper);
        let fixture = TestFixture::new(helper);

        fixture
            .run_test(|| async {
                // Test code
                Ok(())
            })
            .await
            .unwrap();
    }

    #[test]
    fn test_database_seeder() {
        let seeder = DatabaseSeeder::new()
            .add_fixture("users")
            .add_fixture("posts");

        assert_eq!(seeder.fixtures().len(), 2);
    }

    #[tokio::test]
    async fn run_test_invokes_hooks_in_order_around_body() {
        let log = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));

        let before_log = log.clone();
        let before_log_2 = log.clone();
        let after_log = log.clone();
        let after_log_2 = log.clone();

        let builder = IntegrationTestBuilder::new("ordering")
            .before_each(move || {
                let log = before_log.clone();
                async move { log.lock().unwrap().push("before_1") }
            })
            .before_each(move || {
                let log = before_log_2.clone();
                async move { log.lock().unwrap().push("before_2") }
            })
            .after_each(move || {
                let log = after_log.clone();
                async move { log.lock().unwrap().push("after_1") }
            })
            .after_each(move || {
                let log = after_log_2.clone();
                async move { log.lock().unwrap().push("after_2") }
            });

        let body_log = log.clone();
        builder
            .run_test(|| async move {
                body_log.lock().unwrap().push("body");
            })
            .await;

        assert_eq!(
            *log.lock().unwrap(),
            vec!["before_1", "before_2", "body", "after_1", "after_2"]
        );
    }

    #[tokio::test]
    async fn run_test_returns_the_body_result() {
        let builder = IntegrationTestBuilder::new("passthrough");
        let value = builder.run_test(|| async { 42 }).await;
        assert_eq!(value, 42);
    }

    #[tokio::test]
    async fn run_test_runs_after_each_even_when_no_before_each_hooks_registered() {
        let ran_after = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ran_after.clone();

        let builder = IntegrationTestBuilder::new("no-before").after_each(move || {
            let flag = flag.clone();
            async move {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        });

        builder.run_test(|| async {}).await;
        assert!(ran_after.load(std::sync::atomic::Ordering::SeqCst));
    }
}
