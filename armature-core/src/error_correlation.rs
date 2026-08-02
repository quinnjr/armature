//! Error Correlation Module
//!
//! Provides error correlation and distributed tracing capabilities for tracking
//! errors across services and request chains.
//!
//! # Features
//!
//! - ✅ Correlation ID generation and propagation
//! - ✅ Trace/Span ID support for distributed tracing
//! - ✅ Error chain tracking (parent-child relationships)
//! - ✅ Causation chain for root cause analysis
//! - ✅ Correlation context propagation
//! - ✅ Middleware for automatic correlation
//! - ✅ OpenTelemetry-compatible trace context
//!
//! # Example
//!
//! ```rust,ignore
//! use armature_core::error_correlation::*;
//!
//! // Create correlation context
//! let ctx = CorrelationContext::new()
//!     .with_user_id("user-123")
//!     .with_service("auth-service");
//!
//! // Track a correlated error
//! let error = CorrelatedError::new("Database connection failed")
//!     .with_context(ctx)
//!     .caused_by("Connection timeout");
//! ```

use crate::middleware::{Middleware, Next};
use crate::{Error, HttpRequest, HttpResponse};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

// ============================================================================
// Correlation ID Generation
// ============================================================================

/// Unique ID generator with different strategies.
#[derive(Debug, Clone, Copy, Default)]
pub enum IdGenerationStrategy {
    /// UUID v4 (random)
    #[default]
    UuidV4,
    /// UUID v7 (time-ordered)
    UuidV7,
    /// Snowflake-style ID (timestamp + machine + sequence)
    Snowflake,
    /// ULID (Universally Unique Lexicographically Sortable Identifier)
    Ulid,
    /// Short ID (8 characters, base62)
    Short,
}

/// Counter for snowflake IDs
static SEQUENCE_COUNTER: AtomicU64 = AtomicU64::new(0);

impl IdGenerationStrategy {
    /// Generate a new ID using this strategy.
    pub fn generate(&self) -> String {
        match self {
            IdGenerationStrategy::UuidV4 => uuid::Uuid::new_v4().to_string(),
            IdGenerationStrategy::UuidV7 => {
                // UUID v7 - time-ordered UUID
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let random_bytes: [u8; 10] = rand_bytes();
                let mut bytes = [0u8; 16];

                // First 6 bytes: timestamp (48 bits)
                bytes[0..6].copy_from_slice(&timestamp.to_be_bytes()[2..8]);
                // Set version 7
                bytes[6] = (random_bytes[0] & 0x0F) | 0x70;
                bytes[7] = random_bytes[1];
                // Set variant
                bytes[8] = (random_bytes[2] & 0x3F) | 0x80;
                bytes[9..16].copy_from_slice(&random_bytes[3..10]);

                uuid::Uuid::from_bytes(bytes).to_string()
            }
            IdGenerationStrategy::Snowflake => {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let seq = SEQUENCE_COUNTER.fetch_add(1, Ordering::SeqCst) & 0xFFF;
                let machine_id = std::process::id() as u64 & 0x3FF;

                // 41 bits timestamp + 10 bits machine + 12 bits sequence
                let id = ((timestamp & 0x1FFFFFFFFFF) << 22) | (machine_id << 12) | seq;
                format!("{:016x}", id)
            }
            IdGenerationStrategy::Ulid => {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                let random: [u8; 10] = rand_bytes();

                // Encode timestamp (6 bytes) + random (10 bytes) in Crockford base32
                let mut result = String::with_capacity(26);
                let alphabet = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

                // Encode timestamp (10 chars)
                for i in (0..10).rev() {
                    let shift = i * 5;
                    if shift < 48 {
                        let idx = ((timestamp >> shift) & 0x1F) as usize;
                        result.push(alphabet[idx] as char);
                    }
                }

                // Encode random (16 chars)
                let mut bits: u128 = 0;
                for &b in &random {
                    bits = (bits << 8) | b as u128;
                }
                for i in (0..16).rev() {
                    let idx = ((bits >> (i * 5)) & 0x1F) as usize;
                    result.push(alphabet[idx] as char);
                }

                result
            }
            IdGenerationStrategy::Short => {
                let random: [u8; 6] = rand_bytes::<6>();
                let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
                let mut result = String::with_capacity(8);

                for b in random {
                    result.push(alphabet[(b % 62) as usize] as char);
                }
                // Add 2 more chars from timestamp
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                result.push(alphabet[(ts % 62) as usize] as char);
                result.push(alphabet[((ts / 62) % 62) as usize] as char);

                result
            }
        }
    }
}

/// Generate random bytes
/// Generate a random lowercase-hex identifier of exactly `hex_len` characters.
fn random_hex_id(hex_len: usize) -> String {
    let mut s = String::with_capacity(hex_len);
    while s.len() < hex_len {
        let bytes: [u8; 16] = rand_bytes();
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{:02x}", b);
        }
    }
    s.truncate(hex_len);
    s
}

fn rand_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    // Use simple PRNG based on timestamp and address
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let mut state = seed ^ 0xDEADBEEF;
    for b in bytes.iter_mut() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *b = (state >> 33) as u8;
    }
    bytes
}

// ============================================================================
// Correlation Context
// ============================================================================

/// Standard HTTP headers for correlation.
pub mod headers {
    /// Correlation ID header (custom)
    pub const CORRELATION_ID: &str = "X-Correlation-ID";
    /// Request ID header (custom)
    pub const REQUEST_ID: &str = "X-Request-ID";
    /// Trace ID header (W3C Trace Context)
    pub const TRACE_PARENT: &str = "traceparent";
    /// Trace state header (W3C Trace Context)
    pub const TRACE_STATE: &str = "tracestate";
    /// B3 trace ID (Zipkin)
    pub const B3_TRACE_ID: &str = "X-B3-TraceId";
    /// B3 span ID (Zipkin)
    pub const B3_SPAN_ID: &str = "X-B3-SpanId";
    /// B3 parent span ID (Zipkin)
    pub const B3_PARENT_SPAN_ID: &str = "X-B3-ParentSpanId";
    /// B3 sampled (Zipkin)
    pub const B3_SAMPLED: &str = "X-B3-Sampled";
    /// Causation ID header
    pub const CAUSATION_ID: &str = "X-Causation-ID";
    /// Session ID header
    pub const SESSION_ID: &str = "X-Session-ID";
}

/// Correlation context that can be propagated across services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationContext {
    /// Correlation ID - groups related requests/errors
    pub correlation_id: String,
    /// Request ID - unique per request
    pub request_id: String,
    /// Trace ID (for distributed tracing)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Span ID (current span in trace)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    /// Parent span ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Causation ID (what caused this request)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    /// Session ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Service name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Service version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_version: Option<String>,
    /// User ID (if authenticated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Tenant ID (for multi-tenant systems)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Custom baggage items
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub baggage: HashMap<String, String>,
    /// Sampling decision for traces
    pub sampled: bool,
    /// Timestamp when context was created
    pub created_at: u64,
}

impl Default for CorrelationContext {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrelationContext {
    /// Create a new correlation context with generated IDs.
    pub fn new() -> Self {
        let strategy = IdGenerationStrategy::UuidV4;
        Self {
            correlation_id: strategy.generate(),
            request_id: strategy.generate(),
            trace_id: None,
            span_id: None,
            parent_span_id: None,
            causation_id: None,
            session_id: None,
            service: None,
            service_version: None,
            user_id: None,
            tenant_id: None,
            baggage: HashMap::new(),
            sampled: true,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    /// Create context with a specific ID strategy.
    pub fn with_strategy(strategy: IdGenerationStrategy) -> Self {
        Self {
            correlation_id: strategy.generate(),
            request_id: strategy.generate(),
            ..Default::default()
        }
    }

    /// Create a child context (for downstream calls).
    pub fn child(&self) -> Self {
        let strategy = IdGenerationStrategy::UuidV4;
        Self {
            correlation_id: self.correlation_id.clone(),
            request_id: strategy.generate(),
            trace_id: self.trace_id.clone(),
            span_id: Some(strategy.generate()),
            parent_span_id: self.span_id.clone(),
            causation_id: Some(self.request_id.clone()),
            session_id: self.session_id.clone(),
            service: self.service.clone(),
            service_version: self.service_version.clone(),
            user_id: self.user_id.clone(),
            tenant_id: self.tenant_id.clone(),
            baggage: self.baggage.clone(),
            sampled: self.sampled,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    /// Set the correlation ID.
    pub fn correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = id.into();
        self
    }

    /// Set the trace ID.
    pub fn trace_id(mut self, id: impl Into<String>) -> Self {
        self.trace_id = Some(id.into());
        self
    }

    /// Set the span ID.
    pub fn span_id(mut self, id: impl Into<String>) -> Self {
        self.span_id = Some(id.into());
        self
    }

    /// Set the causation ID.
    pub fn with_causation(mut self, id: impl Into<String>) -> Self {
        self.causation_id = Some(id.into());
        self
    }

    /// Set the session ID.
    pub fn with_session(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Set the service name.
    pub fn with_service(mut self, service: impl Into<String>) -> Self {
        self.service = Some(service.into());
        self
    }

    /// Set the service version.
    pub fn with_service_version(mut self, version: impl Into<String>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// Set the user ID.
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set the tenant ID.
    pub fn with_tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Add a baggage item.
    pub fn with_baggage(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.baggage.insert(key.into(), value.into());
        self
    }

    /// Set sampling decision.
    pub fn with_sampled(mut self, sampled: bool) -> Self {
        self.sampled = sampled;
        self
    }

    /// Extract correlation context from HTTP request headers.
    pub fn from_request(req: &HttpRequest) -> Self {
        let mut ctx = Self::new();

        // Extract correlation ID
        if let Some(id) = req.headers.get(headers::CORRELATION_ID).or_else(|| {
            req.headers
                .get(headers::CORRELATION_ID.to_lowercase().as_str())
        }) {
            ctx.correlation_id = id.to_owned();
        }

        // Extract request ID
        if let Some(id) = req
            .headers
            .get(headers::REQUEST_ID)
            .or_else(|| req.headers.get(headers::REQUEST_ID.to_lowercase().as_str()))
        {
            ctx.request_id = id.to_owned();
        }

        // Extract W3C trace context
        if let Some(traceparent) = req.headers.get(headers::TRACE_PARENT)
            && let Some((trace_id, span_id, sampled)) = parse_traceparent(traceparent)
        {
            ctx.trace_id = Some(trace_id);
            ctx.parent_span_id = Some(span_id);
            ctx.sampled = sampled;
            // Generate new span ID for this service
            ctx.span_id = Some(IdGenerationStrategy::Short.generate());
        }

        // Fall back to B3 headers (Zipkin)
        if ctx.trace_id.is_none()
            && let Some(id) = req.headers.get(headers::B3_TRACE_ID)
        {
            ctx.trace_id = Some(id.to_owned());
        }
        if ctx.span_id.is_none()
            && let Some(id) = req.headers.get(headers::B3_SPAN_ID)
        {
            ctx.parent_span_id = Some(id.to_owned());
            ctx.span_id = Some(IdGenerationStrategy::Short.generate());
        }

        // Extract causation ID
        if let Some(id) = req.headers.get(headers::CAUSATION_ID) {
            ctx.causation_id = Some(id.to_owned());
        }

        // Extract session ID
        if let Some(id) = req.headers.get(headers::SESSION_ID) {
            ctx.session_id = Some(id.to_owned());
        }

        ctx
    }

    /// Inject correlation context into HTTP request headers.
    pub fn inject_into_request(&self, req: &mut HttpRequest) {
        // Names are `&str` and values are borrowed where they can be: the header
        // map interns the name and copies the value once, so a `to_string` on
        // either side would be a second allocation for nothing.
        req.headers
            .insert(headers::CORRELATION_ID, self.correlation_id.as_str());
        req.headers
            .insert(headers::REQUEST_ID, self.request_id.as_str());

        if let Some(ref trace_id) = self.trace_id {
            let span_id = self.span_id.as_deref().unwrap_or("0000000000000000");
            let sampled = if self.sampled { "01" } else { "00" };
            let traceparent = format!("00-{trace_id}-{span_id}-{sampled}");
            req.headers.insert(headers::TRACE_PARENT, traceparent);

            // Also add B3 headers for Zipkin compatibility
            req.headers.insert(headers::B3_TRACE_ID, trace_id.as_str());
            req.headers.insert(headers::B3_SPAN_ID, span_id);
            if let Some(ref parent) = self.parent_span_id {
                req.headers
                    .insert(headers::B3_PARENT_SPAN_ID, parent.as_str());
            }
            req.headers
                .insert(headers::B3_SAMPLED, if self.sampled { "1" } else { "0" });
        }

        if let Some(ref causation_id) = self.causation_id {
            req.headers
                .insert(headers::CAUSATION_ID, causation_id.as_str());
        }

        if let Some(ref session_id) = self.session_id {
            req.headers.insert(headers::SESSION_ID, session_id.as_str());
        }
    }

    /// Inject correlation context into HTTP response headers.
    pub fn inject_into_response(&self, res: &mut HttpResponse) {
        res.headers.insert(
            headers::CORRELATION_ID.to_string(),
            self.correlation_id.clone(),
        );
        res.headers
            .insert(headers::REQUEST_ID.to_string(), self.request_id.clone());

        if let Some(ref trace_id) = self.trace_id {
            let span_id = self.span_id.as_deref().unwrap_or("0000000000000000");
            let sampled = if self.sampled { "01" } else { "00" };
            let traceparent = format!("00-{}-{}-{}", trace_id, span_id, sampled);
            res.headers
                .insert(headers::TRACE_PARENT.to_string(), traceparent);
        }
    }

    /// Convert to W3C traceparent header format.
    pub fn to_traceparent(&self) -> Option<String> {
        let trace_id = self.trace_id.as_ref()?;
        let span_id = self.span_id.as_deref().unwrap_or("0000000000000000");
        let sampled = if self.sampled { "01" } else { "00" };
        Some(format!("00-{}-{}-{}", trace_id, span_id, sampled))
    }
}

/// Parse W3C traceparent header.
fn parse_traceparent(value: &str) -> Option<(String, String, bool)> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() >= 4 && parts[0] == "00" {
        let trace_id = parts[1].to_string();
        let span_id = parts[2].to_string();
        let sampled = parts[3] == "01";
        Some((trace_id, span_id, sampled))
    } else {
        None
    }
}

// ============================================================================
// Correlated Error
// ============================================================================

/// An error with full correlation information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelatedError {
    /// Error ID (unique to this error occurrence)
    pub error_id: String,
    /// Correlation context
    pub context: CorrelationContext,
    /// Error message
    pub message: String,
    /// Error code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Error category/type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// HTTP status code
    pub status: u16,
    /// Source service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_service: Option<String>,
    /// Source location (file:line)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_location: Option<String>,
    /// Causation chain (list of error IDs that led to this error)
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub causation_chain: Vec<String>,
    /// Related error IDs
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub related_errors: Vec<String>,
    /// Stack trace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,
    /// Additional metadata
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Timestamp
    pub timestamp: u64,
    /// Retry information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_info: Option<RetryInfo>,
}

/// Retry information for recoverable errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryInfo {
    /// Whether the error is retryable
    pub retryable: bool,
    /// Suggested retry delay in milliseconds
    pub retry_delay_ms: Option<u64>,
    /// Maximum retry attempts
    pub max_retries: Option<u32>,
    /// Current retry attempt
    pub current_attempt: u32,
}

impl CorrelatedError {
    /// Create a new correlated error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error_id: IdGenerationStrategy::UuidV4.generate(),
            context: CorrelationContext::new(),
            message: message.into(),
            code: None,
            error_type: None,
            status: 500,
            source_service: None,
            source_location: None,
            causation_chain: Vec::new(),
            related_errors: Vec::new(),
            stack_trace: None,
            metadata: HashMap::new(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            retry_info: None,
        }
    }

    /// Create from an existing Error with context.
    pub fn from_error(error: &Error, context: CorrelationContext) -> Self {
        let status = error.status_code();
        let error_type = match error {
            Error::BadRequest(_) => "BAD_REQUEST",
            Error::Unauthorized(_) => "UNAUTHORIZED",
            Error::Forbidden(_) => "FORBIDDEN",
            Error::NotFound(_) => "NOT_FOUND",
            Error::Validation(_) => "VALIDATION_ERROR",
            Error::Internal(_) => "INTERNAL_ERROR",
            Error::Conflict(_) => "CONFLICT",
            Error::TooManyRequests(_) => "RATE_LIMITED",
            Error::ServiceUnavailable(_) => "SERVICE_UNAVAILABLE",
            Error::RequestTimeout(_) => "TIMEOUT",
            _ => "ERROR",
        };

        Self {
            error_id: IdGenerationStrategy::UuidV4.generate(),
            context,
            message: error.to_string(),
            code: None,
            error_type: Some(error_type.to_string()),
            status,
            source_service: None,
            source_location: None,
            causation_chain: Vec::new(),
            related_errors: Vec::new(),
            stack_trace: None,
            metadata: HashMap::new(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            retry_info: None,
        }
    }

    /// Set the correlation context.
    pub fn with_context(mut self, context: CorrelationContext) -> Self {
        self.context = context;
        self
    }

    /// Set the error code.
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Set the error type.
    pub fn with_type(mut self, error_type: impl Into<String>) -> Self {
        self.error_type = Some(error_type.into());
        self
    }

    /// Set the HTTP status.
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Set the source service.
    pub fn with_source_service(mut self, service: impl Into<String>) -> Self {
        self.source_service = Some(service.into());
        self
    }

    /// Set the source location.
    pub fn with_source_location(mut self, location: impl Into<String>) -> Self {
        self.source_location = Some(location.into());
        self
    }

    /// Add a causing error to the chain.
    pub fn caused_by(mut self, cause: impl Into<String>) -> Self {
        self.causation_chain.push(cause.into());
        self
    }

    /// Add a related error.
    pub fn related_to(mut self, error_id: impl Into<String>) -> Self {
        self.related_errors.push(error_id.into());
        self
    }

    /// Add a stack trace.
    pub fn with_stack_trace(mut self, trace: impl Into<String>) -> Self {
        self.stack_trace = Some(trace.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        if let Ok(json_value) = serde_json::to_value(value) {
            self.metadata.insert(key.into(), json_value);
        }
        self
    }

    /// Set retry information.
    pub fn with_retry_info(mut self, info: RetryInfo) -> Self {
        self.retry_info = Some(info);
        self
    }

    /// Mark as retryable.
    pub fn retryable(mut self, delay_ms: u64, max_retries: u32) -> Self {
        self.retry_info = Some(RetryInfo {
            retryable: true,
            retry_delay_ms: Some(delay_ms),
            max_retries: Some(max_retries),
            current_attempt: 0,
        });
        self
    }

    /// Convert to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| {
            format!(
                r#"{{"error_id":"{}","message":"{}","status":{}}}"#,
                self.error_id, self.message, self.status
            )
        })
    }
}

// ============================================================================
// Error Registry
// ============================================================================

/// Registry for tracking correlated errors.
pub struct ErrorRegistry {
    /// Maximum number of errors to keep in memory
    max_size: usize,
    /// Errors indexed by error ID
    errors: RwLock<HashMap<String, CorrelatedError>>,
    /// Errors grouped by correlation ID
    by_correlation: RwLock<HashMap<String, Vec<String>>>,
    /// Errors grouped by trace ID
    by_trace: RwLock<HashMap<String, Vec<String>>>,
}

impl ErrorRegistry {
    /// Create a new error registry.
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            errors: RwLock::new(HashMap::new()),
            by_correlation: RwLock::new(HashMap::new()),
            by_trace: RwLock::new(HashMap::new()),
        }
    }

    /// Register an error.
    pub async fn register(&self, error: CorrelatedError) {
        let error_id = error.error_id.clone();
        let correlation_id = error.context.correlation_id.clone();
        let trace_id = error.context.trace_id.clone();

        // Check size limit
        let mut errors = self.errors.write().await;
        let evicted = if errors.len() >= self.max_size {
            // Remove oldest error (simple LRU would be better)
            errors
                .keys()
                .next()
                .cloned()
                .and_then(|id| errors.remove(&id).map(|e| (id, e)))
        } else {
            None
        };
        errors.insert(error_id.clone(), error);
        drop(errors);

        // Drop the evicted error's index entries so the indexes stay bounded
        // by max_size alongside the error store itself.
        if let Some((evicted_id, evicted)) = evicted {
            let mut by_correlation = self.by_correlation.write().await;
            if let Some(ids) = by_correlation.get_mut(&evicted.context.correlation_id) {
                ids.retain(|id| id != &evicted_id);
                if ids.is_empty() {
                    by_correlation.remove(&evicted.context.correlation_id);
                }
            }
            drop(by_correlation);

            if let Some(ref evicted_trace) = evicted.context.trace_id {
                let mut by_trace = self.by_trace.write().await;
                if let Some(ids) = by_trace.get_mut(evicted_trace) {
                    ids.retain(|id| id != &evicted_id);
                    if ids.is_empty() {
                        by_trace.remove(evicted_trace);
                    }
                }
            }
        }

        // Index by correlation ID
        let mut by_correlation = self.by_correlation.write().await;
        by_correlation
            .entry(correlation_id)
            .or_insert_with(Vec::new)
            .push(error_id.clone());
        drop(by_correlation);

        // Index by trace ID
        if let Some(trace_id) = trace_id {
            let mut by_trace = self.by_trace.write().await;
            by_trace
                .entry(trace_id)
                .or_insert_with(Vec::new)
                .push(error_id);
        }
    }

    /// Get an error by ID.
    pub async fn get(&self, error_id: &str) -> Option<CorrelatedError> {
        self.errors.read().await.get(error_id).cloned()
    }

    /// Get all errors for a correlation ID.
    pub async fn get_by_correlation(&self, correlation_id: &str) -> Vec<CorrelatedError> {
        let by_correlation = self.by_correlation.read().await;
        let error_ids = by_correlation.get(correlation_id);

        if let Some(ids) = error_ids {
            let errors = self.errors.read().await;
            ids.iter()
                .filter_map(|id| errors.get(id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all errors for a trace ID.
    pub async fn get_by_trace(&self, trace_id: &str) -> Vec<CorrelatedError> {
        let by_trace = self.by_trace.read().await;
        let error_ids = by_trace.get(trace_id);

        if let Some(ids) = error_ids {
            let errors = self.errors.read().await;
            ids.iter()
                .filter_map(|id| errors.get(id).cloned())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Build the causation tree for an error.
    pub async fn build_causation_tree(&self, error_id: &str) -> Option<ErrorTree> {
        let error = self.get(error_id).await?;

        let mut children = Vec::new();
        for child_id in &error.related_errors {
            if let Some(child_tree) = Box::pin(self.build_causation_tree(child_id)).await {
                children.push(child_tree);
            }
        }

        Some(ErrorTree { error, children })
    }

    /// Clear all errors.
    pub async fn clear(&self) {
        self.errors.write().await.clear();
        self.by_correlation.write().await.clear();
        self.by_trace.write().await.clear();
    }

    /// Get error count.
    pub async fn len(&self) -> usize {
        self.errors.read().await.len()
    }

    /// Check if registry is empty.
    pub async fn is_empty(&self) -> bool {
        self.errors.read().await.is_empty()
    }
}

impl Default for ErrorRegistry {
    fn default() -> Self {
        Self::new(10000)
    }
}

/// Tree structure for error causation.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorTree {
    /// The error at this node
    pub error: CorrelatedError,
    /// Child errors (caused by this error)
    pub children: Vec<ErrorTree>,
}

// ============================================================================
// Correlation Middleware
// ============================================================================

/// Configuration for correlation middleware.
#[derive(Debug, Clone)]
pub struct CorrelationConfig {
    /// ID generation strategy
    pub id_strategy: IdGenerationStrategy,
    /// Service name to tag in context
    pub service_name: Option<String>,
    /// Service version
    pub service_version: Option<String>,
    /// Whether to generate trace IDs if not present
    pub generate_trace_id: bool,
    /// Whether to propagate context in response headers
    pub propagate_in_response: bool,
    /// Header name for correlation ID (customize if needed)
    pub correlation_header: String,
    /// Header name for request ID
    pub request_header: String,
}

impl Default for CorrelationConfig {
    fn default() -> Self {
        Self {
            id_strategy: IdGenerationStrategy::UuidV4,
            service_name: None,
            service_version: None,
            generate_trace_id: true,
            propagate_in_response: true,
            correlation_header: headers::CORRELATION_ID.to_string(),
            request_header: headers::REQUEST_ID.to_string(),
        }
    }
}

impl CorrelationConfig {
    /// Create new configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set service name.
    pub fn service(mut self, name: impl Into<String>) -> Self {
        self.service_name = Some(name.into());
        self
    }

    /// Set service version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.service_version = Some(version.into());
        self
    }

    /// Set ID generation strategy.
    pub fn strategy(mut self, strategy: IdGenerationStrategy) -> Self {
        self.id_strategy = strategy;
        self
    }

    /// Enable/disable trace ID generation.
    pub fn generate_traces(mut self, enabled: bool) -> Self {
        self.generate_trace_id = enabled;
        self
    }

    /// Enable/disable response header propagation.
    pub fn propagate_response(mut self, enabled: bool) -> Self {
        self.propagate_in_response = enabled;
        self
    }
}

/// Middleware that handles correlation context.
pub struct CorrelationMiddleware {
    config: CorrelationConfig,
    registry: Option<Arc<ErrorRegistry>>,
}

impl CorrelationMiddleware {
    /// Create new correlation middleware.
    pub fn new(config: CorrelationConfig) -> Self {
        Self {
            config,
            registry: None,
        }
    }

    /// Create with default configuration.
    pub fn default_config() -> Self {
        Self::new(CorrelationConfig::default())
    }

    /// Attach an error registry.
    pub fn with_registry(mut self, registry: Arc<ErrorRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }
}

#[async_trait]
impl Middleware for CorrelationMiddleware {
    async fn handle(&self, mut req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        // Extract or create correlation context
        let mut ctx = CorrelationContext::from_request(&req);

        // Generate new IDs if not present
        if !req.headers.contains_key(&self.config.correlation_header) {
            ctx.correlation_id = self.config.id_strategy.generate();
        }
        ctx.request_id = self.config.id_strategy.generate();

        // Generate trace ID if configured and not present. W3C trace/span IDs
        // are fixed-length lowercase hex, independent of the configured ID
        // strategy (whose output may be shorter than the required length).
        if self.config.generate_trace_id && ctx.trace_id.is_none() {
            ctx.trace_id = Some(random_hex_id(32));
            ctx.span_id = Some(random_hex_id(16));
        }

        // Add service info
        if let Some(ref service) = self.config.service_name {
            ctx.service = Some(service.clone());
        }
        if let Some(ref version) = self.config.service_version {
            ctx.service_version = Some(version.clone());
        }

        // Inject context into request
        ctx.inject_into_request(&mut req);

        // Process request
        let result = next(req).await;

        match result {
            Ok(mut response) => {
                // Inject correlation headers into response
                if self.config.propagate_in_response {
                    ctx.inject_into_response(&mut response);
                }
                Ok(response)
            }
            Err(error) => {
                // Register error if registry is attached
                if let Some(ref registry) = self.registry {
                    let correlated_error = CorrelatedError::from_error(&error, ctx.clone());
                    registry.register(correlated_error).await;
                }
                Err(error)
            }
        }
    }
}

// ============================================================================
// Extension Traits
// ============================================================================

/// Extension trait for HttpRequest to access correlation context.
pub trait CorrelatedRequest {
    /// Get the correlation context from the request.
    fn correlation_context(&self) -> CorrelationContext;
    /// Get the correlation ID.
    fn correlation_id(&self) -> Option<String>;
    /// Get the request ID.
    fn request_id(&self) -> Option<String>;
    /// Get the trace ID.
    fn trace_id(&self) -> Option<String>;
    /// Get the span ID.
    fn span_id(&self) -> Option<String>;
}

impl CorrelatedRequest for HttpRequest {
    fn correlation_context(&self) -> CorrelationContext {
        CorrelationContext::from_request(self)
    }

    fn correlation_id(&self) -> Option<String> {
        // One lookup: header names intern case-insensitively, so the
        // lowercased retry was always redundant.
        self.headers.get(headers::CORRELATION_ID).map(str::to_owned)
    }

    fn request_id(&self) -> Option<String> {
        self.headers.get(headers::REQUEST_ID).map(str::to_owned)
    }

    fn trace_id(&self) -> Option<String> {
        // Try W3C traceparent first
        if let Some(traceparent) = self.headers.get(headers::TRACE_PARENT)
            && let Some((trace_id, _, _)) = parse_traceparent(traceparent)
        {
            return Some(trace_id);
        }
        // Fall back to B3
        self.headers.get(headers::B3_TRACE_ID).map(str::to_owned)
    }

    fn span_id(&self) -> Option<String> {
        // Try W3C traceparent first
        if let Some(traceparent) = self.headers.get(headers::TRACE_PARENT)
            && let Some((_, span_id, _)) = parse_traceparent(traceparent)
        {
            return Some(span_id);
        }
        // Fall back to B3
        self.headers.get(headers::B3_SPAN_ID).map(str::to_owned)
    }
}

/// Extension trait for Error to create correlated errors.
pub trait CorrelatedErrorExt {
    /// Convert to a correlated error with context.
    fn correlate(&self, context: CorrelationContext) -> CorrelatedError;
    /// Convert to a correlated error from request.
    fn correlate_with_request(&self, request: &HttpRequest) -> CorrelatedError;
}

impl CorrelatedErrorExt for Error {
    fn correlate(&self, context: CorrelationContext) -> CorrelatedError {
        CorrelatedError::from_error(self, context)
    }

    fn correlate_with_request(&self, request: &HttpRequest) -> CorrelatedError {
        let context = CorrelationContext::from_request(request);
        CorrelatedError::from_error(self, context)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_generation_uuid_v4() {
        let id1 = IdGenerationStrategy::UuidV4.generate();
        let id2 = IdGenerationStrategy::UuidV4.generate();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 36); // UUID format
    }

    #[test]
    fn test_id_generation_snowflake() {
        let id1 = IdGenerationStrategy::Snowflake.generate();
        let id2 = IdGenerationStrategy::Snowflake.generate();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 16); // 16 hex chars
    }

    #[test]
    fn test_id_generation_short() {
        let id = IdGenerationStrategy::Short.generate();
        assert_eq!(id.len(), 8);
    }

    #[test]
    fn test_correlation_context_new() {
        let ctx = CorrelationContext::new();
        assert!(!ctx.correlation_id.is_empty());
        assert!(!ctx.request_id.is_empty());
        assert!(ctx.sampled);
    }

    #[test]
    fn test_correlation_context_child() {
        let parent = CorrelationContext::new()
            .trace_id("trace-123")
            .span_id("span-456")
            .with_user_id("user-1");

        let child = parent.child();

        assert_eq!(child.correlation_id, parent.correlation_id);
        assert_ne!(child.request_id, parent.request_id);
        assert_eq!(child.trace_id, parent.trace_id);
        assert_eq!(child.parent_span_id, parent.span_id);
        assert_eq!(child.causation_id, Some(parent.request_id.clone()));
        assert_eq!(child.user_id, parent.user_id);
    }

    #[test]
    fn test_correlation_context_from_request() {
        let mut req = HttpRequest::new("GET".to_string(), "/test".to_string());
        req.headers
            .insert(headers::CORRELATION_ID, "corr-123".to_string());
        req.headers
            .insert(headers::REQUEST_ID, "req-456".to_string());
        req.headers.insert(
            headers::TRACE_PARENT,
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".to_string(),
        );

        let ctx = CorrelationContext::from_request(&req);

        assert_eq!(ctx.correlation_id, "corr-123");
        assert_eq!(ctx.request_id, "req-456");
        assert_eq!(
            ctx.trace_id,
            Some("4bf92f3577b34da6a3ce929d0e0e4736".to_string())
        );
        assert_eq!(ctx.parent_span_id, Some("00f067aa0ba902b7".to_string()));
        assert!(ctx.sampled);
    }

    #[test]
    fn test_traceparent_format() {
        let ctx = CorrelationContext::new()
            .trace_id("4bf92f3577b34da6a3ce929d0e0e4736")
            .span_id("00f067aa0ba902b7")
            .with_sampled(true);

        let traceparent = ctx.to_traceparent().unwrap();
        assert!(traceparent.starts_with("00-"));
        assert!(traceparent.ends_with("-01"));
    }

    #[test]
    fn test_correlated_error() {
        let ctx = CorrelationContext::new()
            .with_service("test-service")
            .with_user_id("user-123");

        let error = CorrelatedError::new("Something went wrong")
            .with_context(ctx)
            .with_code("ERR_001")
            .with_type("VALIDATION_ERROR")
            .with_status(400)
            .caused_by("Invalid input")
            .with_metadata("field", "email");

        assert_eq!(error.message, "Something went wrong");
        assert_eq!(error.status, 400);
        assert_eq!(error.code, Some("ERR_001".to_string()));
        assert_eq!(error.causation_chain.len(), 1);
        assert!(error.metadata.contains_key("field"));
    }

    #[test]
    fn test_retry_info() {
        let error = CorrelatedError::new("Temporary failure").retryable(1000, 3);

        let retry = error.retry_info.unwrap();
        assert!(retry.retryable);
        assert_eq!(retry.retry_delay_ms, Some(1000));
        assert_eq!(retry.max_retries, Some(3));
    }

    #[tokio::test]
    async fn test_error_registry() {
        let registry = ErrorRegistry::new(100);

        let ctx = CorrelationContext::new();
        let correlation_id = ctx.correlation_id.clone();

        let error1 = CorrelatedError::new("Error 1").with_context(ctx.clone());
        let error2 = CorrelatedError::new("Error 2").with_context(ctx.child());

        registry.register(error1.clone()).await;
        registry.register(error2.clone()).await;

        // Get by ID
        let retrieved = registry.get(&error1.error_id).await.unwrap();
        assert_eq!(retrieved.message, "Error 1");

        // Get by correlation ID
        let errors = registry.get_by_correlation(&correlation_id).await;
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_correlated_request_extension() {
        let mut req = HttpRequest::new("GET".to_string(), "/test".to_string());
        req.headers
            .insert(headers::CORRELATION_ID, "corr-123".to_string());
        req.headers
            .insert(headers::REQUEST_ID, "req-456".to_string());

        assert_eq!(req.correlation_id(), Some("corr-123".to_string()));
        assert_eq!(req.request_id(), Some("req-456".to_string()));
    }

    #[test]
    fn test_correlation_config() {
        let config = CorrelationConfig::new()
            .service("my-service")
            .version("1.0.0")
            .strategy(IdGenerationStrategy::UuidV7)
            .generate_traces(true)
            .propagate_response(true);

        assert_eq!(config.service_name, Some("my-service".to_string()));
        assert_eq!(config.service_version, Some("1.0.0".to_string()));
        assert!(config.generate_trace_id);
        assert!(config.propagate_in_response);
    }

    #[test]
    fn test_inject_headers() {
        let ctx = CorrelationContext::new()
            .correlation_id("corr-123")
            .trace_id("trace-456")
            .span_id("span-789")
            .with_session("session-abc");

        let mut req = HttpRequest::new("POST".to_string(), "/api".to_string());
        ctx.inject_into_request(&mut req);

        assert_eq!(req.headers.get(headers::CORRELATION_ID), Some("corr-123"));
        assert!(req.headers.get(headers::TRACE_PARENT).is_some());
        assert_eq!(req.headers.get(headers::SESSION_ID), Some("session-abc"));
    }

    #[test]
    fn test_parse_traceparent() {
        let (trace_id, span_id, sampled) =
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").unwrap();

        assert_eq!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(span_id, "00f067aa0ba902b7");
        assert!(sampled);

        // Not sampled
        let (_, _, sampled) =
            parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00").unwrap();
        assert!(!sampled);
    }

    #[test]
    fn test_correlated_error_to_json() {
        let error = CorrelatedError::new("Test error")
            .with_code("TEST_001")
            .with_status(400);

        let json = error.to_json();
        assert!(json.contains("Test error"));
        assert!(json.contains("TEST_001"));
        assert!(json.contains("400"));
    }
}
