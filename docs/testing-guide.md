# Testing Guide

Comprehensive testing utilities for Armature framework applications.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Integration Test Helpers](#integration-test-helpers)
- [Docker Test Containers](#docker-test-containers)
- [Load Testing](#load-testing)
- [Contract Testing](#contract-testing)
- [Basic Test Utilities](#basic-test-utilities)
- [Best Practices](#best-practices)
- [Summary](#summary)

## Overview

The `armature-testing` crate provides a comprehensive suite of testing utilities:

- **Integration Helpers** - Database setup/teardown
- **Docker Containers** - Isolated test environments
- **Load Testing** - Performance and stress testing
- **Contract Testing** - Consumer-driven contracts (Pact)
- **Test App** - HTTP test client
- **Mocks** - Service mocking and spies
- **Assertions** - Fluent test assertions

## Features

✅ **Integration Test Helpers**
- Database setup/teardown automation
- Test fixtures with lifecycle management
- Database seeding utilities
- Migration support

✅ **Docker Test Containers**
- Automatic container lifecycle
- Built-in configurations for Postgres, Redis, MongoDB
- Custom container support
- Auto-cleanup on drop

✅ **Load Testing**
- Request count-based testing
- Duration-based testing
- Concurrent load generation
- Stress testing (gradual ramp-up)
- Detailed statistics (RPS, latency percentiles)

✅ **Contract Testing**
- Pact-compatible contracts
- Consumer-driven design
- Contract versioning
- Verification utilities

## Integration Test Helpers

### Database Setup/Teardown

```rust
use armature_testing::integration::*;
use async_trait::async_trait;

struct MyDbHelper {
    connection_string: String,
}

#[async_trait]
impl DatabaseTestHelper for MyDbHelper {
    async fn setup(&self) -> Result<(), IntegrationTestError> {
        // Connect to database
        // Run migrations
        // Seed test data
        Ok(())
    }

    async fn teardown(&self) -> Result<(), IntegrationTestError> {
        // Drop tables
        // Clean up test data
        Ok(())
    }
}
```

### Test Fixtures

```rust
use std::sync::Arc;

let helper = Arc::new(MyDbHelper::new("postgres://localhost/test"));
let fixture = TestFixture::new(helper);

// Automatic setup and teardown
fixture.run_test(|| async {
    // Your test code
    // Database is ready to use
    Ok(())
}).await?;
```

## Docker Test Containers

### PostgreSQL Container

```rust
use armature_testing::docker::*;

let config = PostgresContainer::config("testdb", "user", "pass");
let mut container = DockerContainer::new(config);

container.start().await?;
// Connection: postgres://user:pass@localhost:5432/testdb

container.stop().await?;
// Or let it drop for auto-cleanup
```

### Redis Container

```rust
let config = RedisContainer::config();
let mut container = DockerContainer::new(config);

container.start().await?;
// Connection: redis://localhost:6379
```

### MongoDB Container

```rust
let config = MongoContainer::config("testdb");
let mut container = DockerContainer::new(config);

container.start().await?;
// Connection: mongodb://localhost:27017/testdb
```

## Load Testing

### Basic Load Test

```rust
use armature_testing::load::*;
use std::time::Duration;

let config = LoadTestConfig::new(10, 1000); // 10 concurrent, 1000 requests

let runner = LoadTestRunner::new(config, || async {
    // Your test code (e.g., HTTP request)
    Ok(())
});

let stats = runner.run().await?;
stats.print();
```

### Duration-Based Load Test

```rust
let config = LoadTestConfig::new(20, u64::MAX)
    .with_duration(Duration::from_secs(60))  // Run for 60 seconds
    .with_timeout(Duration::from_secs(10));

let runner = LoadTestRunner::new(config, || async {
    Ok(())
});

let stats = runner.run().await?;
```

### Stress Test (Gradual Ramp-Up)

```rust
let stress_runner = StressTestRunner::new(
    1,                          // Start with 1 concurrent
    100,                        // Max 100 concurrent
    10,                         // Step by 10
    Duration::from_secs(10),    // 10 seconds per step
    || async {
        Ok(())
    },
);

let results = stress_runner.run().await?;

for (concurrency, stats) in results {
    println!("Concurrency {}: {} RPS", concurrency, stats.rps);
}
```

### Load Test Statistics

The `LoadTestStats` struct provides:

- `total_requests` - Total number of requests
- `successful` - Successful requests
- `failed` - Failed requests
- `duration` - Total test duration
- `rps` - Requests per second
- `min_response_time` - Minimum latency
- `max_response_time` - Maximum latency
- `avg_response_time` - Average latency
- `median_response_time` - Median (p50)
- `p95_response_time` - 95th percentile
- `p99_response_time` - 99th percentile

## Contract Testing

### Creating a Contract

```rust
use armature_testing::contract::*;

let mut builder = ContractBuilder::new("Frontend", "UserAPI");

// Define interaction
let request = ContractRequest::new(ContractMethod::Get, "/api/users/1")
    .with_header("Accept", "application/json");

let response = ContractResponse::new(200)
    .with_header("Content-Type", "application/json")
    .with_body(serde_json::json!({
        "id": 1,
        "name": "Alice"
    }));

builder.add_interaction(
    ContractInteraction::new(
        "get user by ID",
        request,
        response,
    )
    .with_provider_state("user with ID 1 exists")
);

let contract = builder.build();
```

### Saving Contracts

```rust
use std::path::PathBuf;

let manager = ContractManager::new(PathBuf::from("./pacts"));
manager.save(&contract)?;
// Saves to: ./pacts/frontend-userapi.json
```

### Verifying Contracts

```rust
let actual_response = ContractResponse::new(200)
    .with_body(serde_json::json!({"id": 1, "name": "Alice"}));

match ContractVerifier::verify_interaction(&interaction, &actual_response) {
    Ok(()) => println!("✅ Contract verified"),
    Err(e) => println!("❌ Verification failed: {}", e),
}
```

## Basic Test Utilities

### Test App

```rust
use armature_testing::*;

let app = TestAppBuilder::new()
    .with_route("/hello", |_req| async {
        Ok(HttpResponse::ok().with_body(b"Hello!".to_vec()))
    })
    .build();

let client = app.client();
let response = client.get("/hello").await;
assert_eq!(response.status(), Some(200));
```

### Mock Services

```rust
use armature_testing::MockService;

let mock = MockService::<String>::new();
mock.record_call("get_user");

assert_eq!(mock.call_count(), 1);
assert!(mock.was_called("get_user"));
```

### Assertions

```rust
use armature_testing::*;

// Assert status
assert_status(&response, 200);

// Assert header
assert_header(&response, "Content-Type", "application/json");

// Assert JSON
assert_json(&response, &serde_json::json!({"status": "ok"}));
```

## Best Practices

### Integration Testing

1. **Use Fixtures** - Automate setup/teardown
2. **Isolate Tests** - Each test should be independent
3. **Clean Up** - Always clean up test data
4. **Use Transactions** - Rollback after each test
5. **Seed Minimal Data** - Only what's needed for the test

### Docker Containers

1. **Check Availability** - Always check if Docker is available
2. **Use RAII** - Let containers auto-cleanup
3. **Wait for Ready** - Use wait timeouts
4. **Unique Names** - Use UUIDs for container names

### Load Testing

1. **Start Small** - Begin with low concurrency
2. **Gradual Increase** - Use stress tests to find limits
3. **Monitor Metrics** - Track p95/p99, not just average
4. **Realistic Tests** - Use production-like data

### Contract Testing

1. **Consumer-Driven** - Let consumers define contracts
2. **Version Contracts** - Track contract versions
3. **Share Contracts** - Use shared repository
4. **Verify Often** - Run verification in CI

## Summary

The `armature-testing` crate provides comprehensive testing utilities:

- ✅ **Integration Helpers** - Automate database setup/teardown
- ✅ **Docker Containers** - Isolated, reproducible environments
- ✅ **Load Testing** - Find performance limits
- ✅ **Contract Testing** - Consumer-driven API design
- ✅ **Test Utilities** - Mock, assert, test clients

**Key Benefits:**

- **Productivity** - Less boilerplate, more testing
- **Reliability** - Isolated, reproducible tests
- **Performance** - Find bottlenecks early
- **Confidence** - Comprehensive test coverage

Happy Testing! 🧪

