//! Testing utilities for Armature framework.
//!
//! This crate provides comprehensive testing tools for Armature applications.
//!
//! ## Features
//!
//! - 🧪 **TestApp** - Integration test builder
//! - 📡 **TestClient** - HTTP test client
//! - 🎭 **MockService** - Service mocking
//! - 👁️ **Spy** - Manual call recording (see [`Spy`] - it records what you tell
//!   it to; it does not intercept calls to the value it wraps)
//! - ✅ **Assertions** - Fluent test assertions
//! - 🗄️ **Integration Helpers** - Database setup/teardown
//! - 🐳 **Docker Containers** - Docker-based testing
//! - 📊 **Load Testing** - Performance test utilities
//! - 📜 **Contract Testing** - Pact/consumer-driven contracts
//!
//! ## Quick Start
//!
//! ```no_run
//! use armature_testing::*;
//! use armature_core::{HttpRequest, HttpResponse, Error};
//!
//! # tokio_test::block_on(async {
//! // Create a test app
//! let app = TestAppBuilder::new()
//!     .with_route("/hello", |_req| async {
//!         Ok(HttpResponse::ok().with_body(b"Hello!".to_vec()))
//!     })
//!     .build();
//!
//! // Make test requests
//! let client = app.client();
//! let response = client.get("/hello").await;
//! assert_eq!(response.status(), Some(200));
//! assert_eq!(response.body_string(), Some("Hello!".to_string()));
//! # });
//! ```
//!
//! ## Testing Controllers
//!
//! ```no_run
//! use armature_testing::*;
//! use armature_core::{HttpRequest, HttpResponse, Error};
//!
//! # tokio_test::block_on(async {
//! let app = TestAppBuilder::new()
//!     .with_route("/api/users", |_req| async {
//!         Ok(HttpResponse::ok().with_json(&serde_json::json!({
//!             "users": ["Alice", "Bob"]
//!         }))?)
//!     })
//!     .build();
//!
//! let client = app.client();
//! let response = client.get("/api/users").await;
//! let json: serde_json::Value = response.body_json().unwrap();
//! assert_eq!(json["users"][0], "Alice");
//! # });
//! ```
//!
//! ## Mock Services
//!
//! ```
//! use armature_testing::MockService;
//!
//! // Create a mock service
//! let mock = MockService::<String>::new();
//! mock.record_call("get_user");
//!
//! // Verify calls
//! assert_eq!(mock.call_count(), 1);
//! assert!(mock.was_called("get_user"));
//! ```
//!
//! ## Assertions
//!
//! ```
//! use armature_testing::*;
//! use armature_core::HttpResponse;
//!
//! let response = HttpResponse::ok()
//!     .with_header("Content-Type".to_string(), "application/json".to_string())
//!     .with_body(b"{\"status\":\"ok\"}".to_vec());
//!
//! let test_response = TestResponse::Success(response);
//!
//! // Assert status
//! assert_status(&test_response, 200);
//!
//! // Assert headers
//! assert_header(&test_response, "Content-Type", "application/json");
//! ```
//!
//! ## Integration Testing with Database
//!
//! ```rust,ignore
//! use armature_testing::integration::*;
//! use std::sync::Arc;
//!
//! // Implement DatabaseTestHelper for your database
//! struct MyDbHelper;
//!
//! #[async_trait]
//! impl DatabaseTestHelper for MyDbHelper {
//!     async fn setup(&self) -> Result<(), IntegrationTestError> {
//!         // Create tables, seed data
//!         Ok(())
//!     }
//!
//!     async fn teardown(&self) -> Result<(), IntegrationTestError> {
//!         // Clean up
//!         Ok(())
//!     }
//! }
//!
//! // Use in tests
//! let fixture = TestFixture::new(Arc::new(MyDbHelper));
//! fixture.run_test(|| async {
//!     // Your test code
//!     Ok(())
//! }).await?;
//! ```
//!
//! ## Docker Test Containers
//!
//! ```rust,ignore
//! use armature_testing::docker::*;
//!
//! // Start Postgres container
//! let config = PostgresContainer::config("testdb", "user", "pass");
//! let mut container = DockerContainer::new(config);
//! container.start().await?;
//!
//! // Run tests...
//!
//! // Container automatically stops on drop
//! ```
//!
//! ## Load Testing
//!
//! ```rust,ignore
//! use armature_testing::load::*;
//!
//! let config = LoadTestConfig::new(10, 1000); // 10 concurrent, 1000 requests
//! let runner = LoadTestRunner::new(config, || async {
//!     // Your test code
//!     Ok(())
//! });
//!
//! let stats = runner.run().await?;
//! stats.print();
//! ```
//!
//! ## Contract Testing
//!
//! ```rust,ignore
//! use armature_testing::contract::*;
//!
//! // Create a contract
//! let request = ContractRequest::new(ContractMethod::Get, "/api/users/1");
//! let response = ContractResponse::new(200)
//!     .with_body(serde_json::json!({"id": 1, "name": "Alice"}));
//!
//! let mut builder = ContractBuilder::new("Frontend", "Backend");
//! builder.add_interaction(ContractInteraction::new(
//!     "get user by ID",
//!     request,
//!     response,
//! ));
//!
//! let contract = builder.build();
//!
//! // Save contract
//! let manager = ContractManager::new(PathBuf::from("./pacts"));
//! manager.save(&contract)?;
//! ```

mod assertions;
mod mock;
mod test_app;
mod test_client;
mod test_container;

// New modules
pub mod contract;
pub mod docker;
pub mod integration;
pub mod load;

pub use assertions::{assert_header, assert_json, assert_status};
pub use mock::{MockController, MockProvider, MockService, Spy};
pub use test_app::{TestApp, TestAppBuilder};
pub use test_client::{TestClient, TestResponse};
pub use test_container::TestContainer;

// Re-export common testing utilities
pub use tokio::test as tokio_test;

/// Prelude for common imports.
///
/// ```
/// use armature_testing::prelude::*;
/// ```
pub mod prelude {
    pub use crate::assertions::{assert_header, assert_json, assert_status};
    pub use crate::mock::{MockController, MockProvider, MockService};
    pub use crate::test_app::{TestApp, TestAppBuilder};
    pub use crate::test_client::{TestClient, TestResponse};
    pub use crate::test_container::TestContainer;

    // Integration testing
    pub use crate::integration::{DatabaseTestHelper, IntegrationTestError, TestFixture};

    // Docker testing
    pub use crate::docker::{ContainerConfig, DockerContainer};

    // Load testing
    pub use crate::load::{LoadTestConfig, LoadTestRunner, LoadTestStats};

    // Contract testing
    pub use crate::contract::{
        Contract, ContractBuilder, ContractInteraction, ContractManager, ContractMethod,
        ContractRequest, ContractResponse,
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_exports() {
        // Ensure module compiles
    }
}
