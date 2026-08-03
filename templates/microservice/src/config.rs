//! Service configuration

use std::env;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub service_name: String,
    pub host: String,
    pub port: u16,
    pub redis_url: String,
    pub queue_name: String,
    pub concurrency: usize,
    pub retry_attempts: u32,
    pub retry_delay_ms: u64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            service_name: "microservice".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8080,
            redis_url: "redis://localhost:6379".to_string(),
            queue_name: "jobs".to_string(),
            concurrency: 4,
            retry_attempts: 3,
            retry_delay_ms: 1000,
        }
    }
}

impl ServiceConfig {
    pub fn from_env() -> Self {
        Self {
            service_name: env::var("SERVICE_NAME").unwrap_or_else(|_| "microservice".to_string()),
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .unwrap_or(8080),
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            queue_name: env::var("QUEUE_NAME").unwrap_or_else(|_| "jobs".to_string()),
            concurrency: env::var("CONCURRENCY")
                .unwrap_or_else(|_| "4".to_string())
                .parse()
                .unwrap_or(4),
            retry_attempts: env::var("RETRY_ATTEMPTS")
                .unwrap_or_else(|_| "3".to_string())
                .parse()
                .unwrap_or(3),
            retry_delay_ms: env::var("RETRY_DELAY_MS")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()
                .unwrap_or(1000),
        }
    }
}
