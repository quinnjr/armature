//! # Armature Core
//!
//! Core library for the Armature HTTP framework - a modern, type-safe web framework for Rust
//! inspired by Angular and NestJS.
//!
//! This crate provides the foundational types, traits, and runtime components
//! for building web applications with Armature.
//!
//! ## Features
//!
//! - **HTTP Handling**: Request/Response types with fluent builders
//! - **Routing**: Path parameters, query strings, and constraints
//! - **Dependency Injection**: Type-safe DI container
//! - **Middleware**: Composable request/response processing
//! - **Guards**: Authentication and authorization
//! - **Resilience**: Circuit breakers, retries, bulkheads, and timeouts
//! - **Logging**: Structured logging with tracing integration
//! - **Health Checks**: Readiness and liveness probes
//! - **WebSocket & SSE**: Real-time communication support
//!
//! ## Quick Start
//!
//! ### HTTP Request Handling
//!
//! ```
//! use armature_core::HttpRequest;
//!
//! // Create an HTTP request
//! let request = HttpRequest::new("GET".to_string(), "/api/users".to_string());
//!
//! assert_eq!(request.method, "GET");
//! assert_eq!(request.path, "/api/users");
//!
//! // Access path and query parameters
//! let mut post = HttpRequest::new("POST".to_string(), "/api/users/123".to_string());
//! post.path_params.insert("id".to_string(), "123".to_string());
//! post.query_params.insert("format".to_string(), "json".to_string());
//! post.body = b"{\"name\":\"John\"}".to_vec();
//!
//! assert_eq!(post.param("id"), Some(&"123".to_string()));
//! assert_eq!(post.query("format"), Some(&"json".to_string()));
//! ```
//!
//! ### HTTP Response Builder
//!
//! ```
//! use armature_core::HttpResponse;
//! use serde_json::json;
//!
//! // JSON response (shorthand)
//! let response = HttpResponse::json(&json!({"message": "Hello"})).unwrap();
//! assert_eq!(response.status, 200);
//!
//! // HTML response
//! let html = HttpResponse::html("<h1>Welcome</h1>");
//! assert_eq!(html.status, 200);
//!
//! // Redirect
//! let redirect = HttpResponse::redirect("https://example.com");
//! assert_eq!(redirect.status, 302);
//!
//! // With fluent builder
//! let custom = HttpResponse::ok()
//!     .content_type("application/xml")
//!     .cache_control("max-age=3600")
//!     .with_body(b"<xml/>".to_vec());
//! ```
//!
//! ### Dependency Injection
//!
//! ```
//! use armature_core::Container;
//!
//! #[derive(Clone, Default)]
//! struct Config { debug: bool }
//!
//! #[derive(Clone)]
//! struct UserService { config: std::sync::Arc<Config> }
//!
//! let container = Container::new();
//!
//! // Register services
//! container.register(Config { debug: true });
//!
//! // Resolve services
//! let config = container.require::<Config>();
//! assert!(config.debug);
//!
//! // Get or use default
//! let config2 = container.get_or_default::<Config>();
//! ```
//!
//! ### Error Handling
//!
//! ```
//! use armature_core::Error;
//!
//! // Create errors with convenience methods
//! let err = Error::not_found("User not found");
//! assert_eq!(err.status_code(), 404);
//! assert!(err.is_client_error());
//!
//! let err = Error::validation("Email is required");
//! assert_eq!(err.status_code(), 400);
//!
//! // Get help suggestions
//! let err = Error::unauthorized("Invalid token");
//! if let Some(help) = err.help() {
//!     println!("Help: {}", help);
//! }
//! ```
//!
//! ## Module Overview
//!
//! | Module | Description |
//! |--------|-------------|
//! | [`application`] | Application bootstrap and lifecycle |
//! | [`container`] | Dependency injection container |
//! | [`routing`] | Request routing and handlers |
//! | [`middleware`] | Middleware chain processing |
//! | [`guard`] | Route guards for authorization |
//! | [`resilience`] | Circuit breaker, retry, bulkhead patterns |
//! | [`health`] | Health check endpoints |
//! | [`logging`] | Structured logging |
//! | [`websocket`] | WebSocket support |
//! | [`sse`] | Server-Sent Events |

pub mod application;
pub mod arena;
pub mod batch;
pub mod body;
pub mod body_limits;
pub mod body_parser;
pub mod buffer_pool;
pub mod cache_local;
pub mod connection;
pub mod connection_manager;
pub mod connection_tuning;
pub mod container;
pub mod cow_state;
pub mod epoll_tuning;
pub mod error;
pub mod extensions;
pub mod extractors;
pub mod fast_response;
pub mod form;
pub mod guard;
pub mod handler;
pub mod headers;
pub mod health;
pub mod hmr;
pub mod http;
pub mod http2;
pub mod http3;
pub mod interceptor;
pub mod io_uring;
pub mod json;
pub mod lifecycle;
pub mod load_balancer;
pub mod logging;
pub mod memory_opt;
pub mod middleware;
pub mod module;
pub mod numa;
pub mod pagination;
pub mod pipeline;
pub mod read_buffer;
pub mod read_state;
pub mod resilience;
pub mod response_buffer;
pub mod response_pipeline;
pub mod route_cache;
pub mod route_constraint;
pub mod route_group;
pub mod route_params;
pub mod route_registry;
pub mod routing;
pub mod runtime_config;
pub mod serialization_pool;
pub mod shutdown;
pub mod simd_parser;
pub mod small_vec;
pub mod socket_batch;
pub mod sse;
pub mod static_assets;
pub mod status;
pub mod streaming;
pub mod timeout;
pub mod tls;
pub mod tower_compat;
pub mod traits;
pub mod vectored_io;
pub mod websocket;
pub mod worker;
pub mod write_coalesce;
pub mod zero_cost;

/// Micro-framework API for lightweight applications
///
/// Provides an Actix-style API without the full module/controller system.
pub mod micro;

// Re-export commonly used types
pub use application::*;
pub use body_limits::*;
pub use connection::{
    Connection, ConnectionConfig, ConnectionEvent, ConnectionPool, ConnectionRecycler,
    ConnectionState, ConnectionStats, PoolHandle, Recyclable, RecyclableConnection, RecyclePool,
    RecyclePoolConfig, RecycleStats, RecyclerStats, StateMachineExecutor, TransitionAction,
    TransitionError, connection_stats, recycle_stats,
};
pub use container::*;
pub use error::*;
pub use extensions::Extensions;
pub use extractors::{
    Body, ContentType, Form, FromRequest, FromRequestNamed, Header, Headers, Method, Path,
    PathParams, Query, RawBody, State,
};
pub use form::*;
pub use guard::*;
pub use handler::{BoxedHandler, Handler, IntoHandler, OptimizedHandlerFn};
pub use headers::{Header as HeaderEntry, HeaderMap, INLINE_HEADERS};
pub use health::*;
pub use hmr::*;
pub use http::*;
pub use http2::*;
pub use http3::*;
pub use interceptor::*;
pub use lifecycle::*;
pub use logging::*;
pub use middleware::*;
pub use module::*;
pub use numa::{
    GlobalNumaStats, NumaAllocStats, NumaAllocator, NumaBuffer, NumaConfig, NumaError, NumaNode,
    NumaPolicy, bind_to_local_node, bind_to_node, cached_numa_config, current_numa_node,
    init_worker_numa, num_numa_nodes, numa_available, numa_stats,
};
pub use pagination::*;
pub use read_buffer::{
    AdaptiveBufferSizer, BufferSizingStats, ContentCategory, DEFAULT_INITIAL_BUFFER, HUGE_BUFFER,
    LARGE_BUFFER, MAX_BUFFER, MEDIUM_BUFFER, MIN_BUFFER, PayloadTracker, ReadBufferConfig,
    SMALL_BUFFER, TINY_BUFFER, buffer_sizing_stats,
};
pub use resilience::{
    BackoffStrategy, Bulkhead, BulkheadConfig, BulkheadError, BulkheadStats, CircuitBreaker,
    CircuitBreakerConfig, CircuitBreakerError, CircuitBreakerStats, CircuitState, Fallback,
    FallbackBuilder, FallbackChain, Retry, RetryConfig, RetryError, Timeout as ResilienceTimeout,
    TimeoutConfig, TimeoutError, fallback_default, fallback_value,
};
pub use response_buffer::{
    DEFAULT_RESPONSE_CAPACITY, LARGE_RESPONSE_CAPACITY, MEDIUM_RESPONSE_CAPACITY, ResponseBuffer,
    ResponseBuilder,
};
pub use response_pipeline::{
    ConnectionPipeline, GlobalPipelineStats, ResponseBatch, ResponseItem, ResponseQueue,
    ResponseQueueStats, ResponseWriterConfig, ResponseWriterStats, global_pipeline_stats,
    writer_stats,
};
pub use route_constraint::*;
pub use route_group::*;
pub use route_registry::{OptimizedRouteHandler, RouteEntry, RouteHandlerFn};
pub use routing::{OptimizedHandler, Route, Router}; // Explicit exports to avoid ambiguous HandlerFn
pub use shutdown::*;
pub use sse::*;
pub use static_assets::*;
pub use status::*;
pub use timeout::*;
pub use tls::*;
pub use traits::*;
pub use vectored_io::{
    MAX_IO_SLICES, ResponseChunks, VectoredIoStats, VectoredResponse, status_line, vectored_stats,
};
pub use websocket::*;
pub use worker::{
    AffinityConfig, AffinityError, AffinityMode, AffinityStats, StateFactory, WorkerCache,
    WorkerConfig, WorkerHandle, WorkerRouter, WorkerState, WorkerStateStats, WorkerStats,
    affinity_stats, affinity_supported, clear_worker_router, clear_worker_state,
    get_thread_affinity, has_worker_router, init_worker_router, init_worker_state,
    init_worker_with_affinity, next_worker_id, num_cpus, num_physical_cpus, set_thread_affinity,
    total_workers, worker_id, worker_state_stats, worker_stats,
};
pub use write_coalesce::{
    CoalesceConfig, CoalesceStats, ConnectionWriteBuffer, DEFAULT_COALESCE_CAPACITY,
    DEFAULT_FLUSH_THRESHOLD, DEFAULT_FLUSH_TIMEOUT_US, MAX_COALESCE_BUFFER, MIN_COALESCE_SIZE,
    MultiBufferCoalescer, WriteCoalescer, WriteResult, coalesce_stats,
};

// Re-export inventory for route registration macros
pub use inventory;
