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
}
