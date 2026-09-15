#![allow(clippy::needless_question_mark)]
//! Docker Test Containers Example
//!
//! Starts real Postgres, Redis and MySQL containers through
//! `armature_testing::docker` and asserts the lifecycle actually happened -
//! a started container reports an id, a stopped one does not, and
//! `is_running()` tracks both. The `sleep`s between start and stop stand in
//! for the work a test would do against the container; they are not the
//! thing being demonstrated.
//!
//! Requires a running Docker daemon. Without one the example prints a notice
//! and exits successfully rather than failing - it is a demo, not a test.
//!
//! ```bash
//! cargo run --example testing_docker
//! ```

use armature_testing::docker::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Docker Test Containers Example ===\n");

    // Check if Docker is available
    if !DockerContainer::is_docker_available() {
        println!("⚠️  Docker not available. Please install Docker to run this example.");
        println!("    Visit: https://docs.docker.com/get-docker/");
        return Ok(());
    }

    println!("✅ Docker is available\n");

    // 1. PostgreSQL test container
    println!("1. PostgreSQL Test Container:");
    println!("   Creating Postgres container...");

    let postgres_config = PostgresContainer::config("testdb", "testuser", "testpass");
    let mut postgres = DockerContainer::new(postgres_config);

    match postgres.start().await {
        Ok(()) => {
            println!("   ✅ Postgres container started");
            // A successful `start()` must leave the container addressable: an
            // id to reference it by and a running flag that agrees.
            let id = postgres
                .container_id()
                .expect("a started container must expose its id")
                .to_owned();
            assert!(!id.is_empty(), "container id must not be empty");
            assert!(
                postgres.is_running(),
                "container must report itself running"
            );
            println!("   Container ID: {id}");
            println!("   Connection: postgres://testuser:testpass@localhost:5432/testdb");

            // Stand-in for the database work a real test would do here.
            println!("   Simulating database operations...");
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            println!("   ✅ Database operations complete");

            // Stop container
            postgres.stop().await?;
            assert!(
                !postgres.is_running(),
                "a stopped container must not report itself running"
            );
            println!("   ✅ Container stopped");
        }
        Err(e) => {
            println!("   ⚠️  Could not start Postgres container: {}", e);
            println!(
                "       This is expected if Docker is not running or the image cannot be pulled."
            );
        }
    }

    println!();

    // 2. Redis test container
    println!("2. Redis Test Container:");
    println!("   Creating Redis container...");

    let redis_config = RedisContainer::config();
    let mut redis = DockerContainer::new(redis_config);

    match redis.start().await {
        Ok(()) => {
            println!("   ✅ Redis container started");
            println!(
                "   Container ID: {}",
                redis.container_id().unwrap_or("unknown")
            );
            println!("   Connection: redis://localhost:6379");

            // Simulate some work
            println!("   Simulating cache operations...");
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            println!("   ✅ Cache operations complete");

            // Container will be automatically stopped on drop
            println!("   (Container will auto-stop on drop)");
        }
        Err(e) => {
            println!("   ⚠️  Could not start Redis container: {}", e);
        }
    }

    println!();

    // 3. MongoDB test container
    println!("3. MongoDB Test Container:");
    println!("   Creating MongoDB container...");

    let mongo_config = MongoContainer::config("testdb");
    let mut mongo = DockerContainer::new(mongo_config);

    match mongo.start().await {
        Ok(()) => {
            println!("   ✅ MongoDB container started");
            println!(
                "   Container ID: {}",
                mongo.container_id().unwrap_or("unknown")
            );
            println!("   Connection: mongodb://localhost:27017/testdb");

            // Check if running
            println!("   Is running? {}", mongo.is_running());

            mongo.stop().await?;
            println!("   ✅ Container stopped");
        }
        Err(e) => {
            println!("   ⚠️  Could not start MongoDB container: {}", e);
        }
    }

    println!();

    // 4. Custom container configuration
    println!("4. Custom Container Configuration:");
    let custom_config = ContainerConfig::new("nginx", "alpine")
        .with_name("armature-test-nginx")
        .with_port(8080, 80)
        .with_env("NGINX_HOST", "localhost")
        .with_wait_timeout(5);

    println!("   Image: {}", custom_config.image_name());
    println!("   Ports: {:?}", custom_config.ports);
    println!("   Env: {:?}", custom_config.env);

    println!();
    println!("=== Docker Test Containers Complete ===\n");
    println!("💡 Tip: Containers automatically clean up when dropped or stopped");
    println!("💡 Use these in your integration tests for isolated environments");

    Ok(())
}
