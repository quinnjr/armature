//! Request/Response logging middleware

use crate::{AuditEvent, AuditLogger, AuditSeverity, AuditStatus};
use armature_auth::UserContext;
use armature_core::{Error, HttpRequest, HttpResponse, Middleware};
use std::sync::Arc;
use std::time::Instant;

/// Request/Response audit logging middleware
///
/// Automatically logs HTTP requests and responses.
pub struct AuditMiddleware {
    logger: Arc<AuditLogger>,
    log_request_body: bool,
    log_response_body: bool,
    max_body_size: usize,
    /// JWT claim to read the principal from (defaults to `sub`).
    subject_claim: String,
    /// Whether to fall back to reading the principal from an **unverified**,
    /// unsigned JWT payload when no verified [`UserContext`] is present.
    ///
    /// **Default: `false` (safe).** When `false`, the audit principal is only
    /// ever derived from a signature-verified identity attached to the request
    /// by real auth middleware; if none is present the principal is `None`.
    ///
    /// **SECURITY:** setting this to `true` is DANGEROUS. A bearer token's
    /// payload is base64-decoded with NO signature verification, so any client
    /// can forge `header.base64({"sub":"victim"}).x` and every action will be
    /// logged under `victim`'s identity — destroying the audit trail's
    /// non-repudiation guarantee. Only enable this in an environment where the
    /// token was already verified upstream and cannot be attacker-supplied.
    trust_unverified_jwt_subject: bool,
}

impl AuditMiddleware {
    /// Create a new audit middleware
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_audit::*;
    /// use std::sync::Arc;
    ///
    /// let logger = Arc::new(AuditLogger::builder()
    ///     .backend(FileBackend::new("audit.log"))
    ///     .build());
    ///
    /// let middleware = AuditMiddleware::new(logger);
    /// ```
    pub fn new(logger: Arc<AuditLogger>) -> Self {
        Self {
            logger,
            log_request_body: true,
            log_response_body: true,
            max_body_size: 10_000, // 10KB default
            subject_claim: "sub".to_string(),
            trust_unverified_jwt_subject: false,
        }
    }

    /// Set which JWT claim identifies the principal (defaults to `sub`).
    pub fn subject_claim(mut self, claim: impl Into<String>) -> Self {
        self.subject_claim = claim.into();
        self
    }

    /// Opt in to deriving the audit principal from an **unverified** JWT payload
    /// when no verified [`UserContext`] is attached to the request.
    ///
    /// **SECURITY:** this is spoofable and defaults to `false`. See
    /// [`AuditMiddleware::trust_unverified_jwt_subject`] (the field docs) for
    /// the full warning. Leave it off unless the token cannot be
    /// attacker-supplied.
    pub fn trust_unverified_jwt_subject(mut self, trust: bool) -> Self {
        self.trust_unverified_jwt_subject = trust;
        self
    }

    /// Set whether to log request bodies
    pub fn log_request_body(mut self, log: bool) -> Self {
        self.log_request_body = log;
        self
    }

    /// Set whether to log response bodies
    pub fn log_response_body(mut self, log: bool) -> Self {
        self.log_response_body = log;
        self
    }

    /// Set maximum body size to log (in bytes)
    pub fn max_body_size(mut self, size: usize) -> Self {
        self.max_body_size = size;
        self
    }

    /// Determine the audit principal for a request.
    ///
    /// Resolution order:
    /// 1. A signature-**verified** [`UserContext`] attached to the request
    ///    extensions by real auth middleware (e.g. `armature-auth`'s
    ///    `JwtAuthMiddleware`). This is the only trustworthy source and is
    ///    always preferred.
    /// 2. Only if [`Self::trust_unverified_jwt_subject`] was explicitly enabled,
    ///    the subject claim decoded from the **unverified** bearer-token payload.
    ///
    /// Under the safe default (no verified identity, unverified fallback off)
    /// this returns `None` — the audit trail records no principal rather than a
    /// forgeable one, preserving non-repudiation.
    fn extract_user_id(&self, request: &HttpRequest) -> Option<String> {
        // 1. Verified identity from auth middleware — the trustworthy source.
        if let Some(subject) = self.verified_subject(request) {
            return Some(subject);
        }

        // 2. Opt-in, spoofable fallback. Disabled by default.
        if self.trust_unverified_jwt_subject {
            let auth = request.headers.get("authorization")?;
            let token = auth.strip_prefix("Bearer ").map(str::trim)?;
            return Self::subject_from_jwt(token, &self.subject_claim);
        }

        None
    }

    /// Read the principal from a verified [`UserContext`] extension, if present.
    ///
    /// When the configured [`Self::subject_claim`] is the default `sub`, the
    /// context's verified `user_id` is used. For a custom claim, the value is
    /// read from the verified claim set preserved in `UserContext::metadata`.
    fn verified_subject(&self, request: &HttpRequest) -> Option<String> {
        let ctx = request.extension::<UserContext>()?;

        if self.subject_claim == "sub" {
            return (!ctx.user_id.is_empty()).then(|| ctx.user_id.clone());
        }

        ctx.metadata
            .get(&self.subject_claim)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// Decode a JWT's payload segment and pull out the named subject claim.
    fn subject_from_jwt(token: &str, subject_claim: &str) -> Option<String> {
        use base64::Engine;

        // header.payload.signature — the claims live in the middle segment.
        let payload_b64 = token.split('.').nth(1)?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

        claims
            .get(subject_claim)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// Extract IP address from request
    fn extract_ip(&self, request: &HttpRequest) -> Option<String> {
        // Try X-Forwarded-For first
        if let Some(forwarded) = request.headers.get("x-forwarded-for") {
            return Some(
                forwarded
                    .split(',')
                    .next()
                    .unwrap_or(forwarded)
                    .trim()
                    .to_string(),
            );
        }

        // Try X-Real-IP
        if let Some(real_ip) = request.headers.get("x-real-ip") {
            return Some(real_ip.clone());
        }

        None
    }

    /// Extract user agent from request
    fn extract_user_agent(&self, request: &HttpRequest) -> Option<String> {
        request.headers.get("user-agent").cloned()
    }

    /// Truncate body if too large
    fn truncate_body(&self, body: &[u8]) -> Option<String> {
        if body.is_empty() {
            return None;
        }

        if body.len() > self.max_body_size {
            let truncated = &body[..self.max_body_size];
            let mut text = String::from_utf8_lossy(truncated).to_string();
            text.push_str("... [TRUNCATED]");
            Some(text)
        } else {
            Some(String::from_utf8_lossy(body).to_string())
        }
    }
}

#[async_trait::async_trait]
impl Middleware for AuditMiddleware {
    async fn handle(
        &self,
        request: HttpRequest,
        next: armature_core::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        let start = Instant::now();

        // Extract request information
        let method = request.method.clone();
        let path = request.path.clone();
        let user_id = self.extract_user_id(&request);
        let ip_address = self.extract_ip(&request);
        let user_agent = self.extract_user_agent(&request);

        // Optionally log request body
        let request_body = if self.log_request_body {
            self.truncate_body(&request.body)
        } else {
            None
        };

        // Process request
        let result = next(request).await;

        // Calculate duration
        let duration_ms = start.elapsed().as_millis() as u64;

        // Create audit event based on result
        let event = match &result {
            Ok(response) => {
                let status_code = response.status;
                let response_body = if self.log_response_body {
                    self.truncate_body(&response.body)
                } else {
                    None
                };

                let status = if status_code < 400 {
                    AuditStatus::Success
                } else if status_code < 500 {
                    AuditStatus::Denied
                } else {
                    AuditStatus::Error
                };

                let severity = if status_code < 400 {
                    AuditSeverity::Info
                } else if status_code < 500 {
                    AuditSeverity::Warning
                } else {
                    AuditSeverity::Error
                };

                let mut event = AuditEvent::new("http.request")
                    .action("http_request")
                    .method(method)
                    .path(path)
                    .status_code(status_code)
                    .status(status)
                    .severity(severity)
                    .duration_ms(duration_ms);

                if let Some(user) = user_id {
                    event = event.user(user);
                }
                if let Some(ip) = ip_address {
                    event = event.ip(ip);
                }
                if let Some(ua) = user_agent {
                    event = event.user_agent(ua);
                }
                if let Some(body) = request_body {
                    event = event.request_body(body);
                }
                if let Some(body) = response_body {
                    event = event.response_body(body);
                }

                event
            }
            Err(err) => {
                let status_code = err.status_code();

                let mut event = AuditEvent::new("http.request")
                    .action("http_request")
                    .method(method)
                    .path(path)
                    .status_code(status_code)
                    .status(AuditStatus::Error)
                    .severity(AuditSeverity::Error)
                    .duration_ms(duration_ms)
                    .error(err.to_string());

                if let Some(user) = user_id {
                    event = event.user(user);
                }
                if let Some(ip) = ip_address {
                    event = event.ip(ip);
                }
                if let Some(ua) = user_agent {
                    event = event.user_agent(ua);
                }
                if let Some(body) = request_body {
                    event = event.request_body(body);
                }

                event
            }
        };

        // Log audit event (don't fail request if logging fails)
        if let Err(e) = self.logger.log(event).await {
            tracing::error!("Failed to log audit event: {}", e);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditLogger, MemoryBackend};

    #[test]
    fn test_audit_middleware_creation() {
        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());

        let middleware = AuditMiddleware::new(logger);
        assert!(middleware.log_request_body);
        assert!(middleware.log_response_body);
    }

    #[test]
    fn test_audit_middleware_configuration() {
        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());

        let middleware = AuditMiddleware::new(logger)
            .log_request_body(false)
            .log_response_body(false)
            .max_body_size(5000);

        assert!(!middleware.log_request_body);
        assert!(!middleware.log_response_body);
        assert_eq!(middleware.max_body_size, 5000);
    }

    #[test]
    fn test_extract_user_id_reads_jwt_subject() {
        use base64::Engine;

        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());
        // This exercises the OPT-IN unverified fallback explicitly.
        let middleware = AuditMiddleware::new(logger).trust_unverified_jwt_subject(true);

        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = engine.encode(br#"{"sub":"alice-42","role":"admin"}"#);
        let token = format!("{header}.{payload}.signature");

        let mut req = HttpRequest::new("GET".to_string(), "/".to_string());
        req.headers
            .insert("authorization", format!("Bearer {token}"));

        // The recorded principal must be the token's real subject, not a
        // constant placeholder.
        assert_eq!(
            middleware.extract_user_id(&req),
            Some("alice-42".to_string())
        );
        assert_ne!(
            middleware.extract_user_id(&req),
            Some("authenticated_user".to_string())
        );
    }

    #[test]
    fn test_extract_user_id_custom_subject_claim() {
        use base64::Engine;

        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());
        let middleware = AuditMiddleware::new(logger)
            .subject_claim("user_id")
            .trust_unverified_jwt_subject(true);

        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = engine.encode(br#"{"user_id":"bob-7"}"#);
        let token = format!("{header}.{payload}.sig");

        let mut req = HttpRequest::new("GET".to_string(), "/".to_string());
        req.headers
            .insert("authorization", format!("Bearer {token}"));

        assert_eq!(middleware.extract_user_id(&req), Some("bob-7".to_string()));
    }

    #[test]
    fn test_extract_user_id_no_bearer() {
        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());
        let middleware = AuditMiddleware::new(logger);

        let req = HttpRequest::new("GET".to_string(), "/".to_string());
        assert_eq!(middleware.extract_user_id(&req), None);
    }

    #[test]
    fn test_forged_unsigned_token_is_ignored_by_default() {
        use base64::Engine;

        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());
        // Safe default: unverified fallback is OFF.
        let middleware = AuditMiddleware::new(logger);

        // An attacker forges header.base64({"sub":"victim"}).garbage — no valid
        // signature. Under the safe default this must NOT set the principal.
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"none"}"#);
        let payload = engine.encode(br#"{"sub":"victim"}"#);
        let forged = format!("{header}.{payload}.not-a-real-signature");

        let mut req = HttpRequest::new("GET".to_string(), "/".to_string());
        req.headers
            .insert("authorization", format!("Bearer {forged}"));

        // No verified UserContext is attached, so the forged subject is dropped.
        assert_eq!(
            middleware.extract_user_id(&req),
            None,
            "a forged unsigned token must not spoof the audit principal"
        );
    }

    #[test]
    fn test_verified_user_context_is_preferred() {
        use base64::Engine;

        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());
        // Even with the spoofable fallback enabled, a verified identity wins.
        let middleware = AuditMiddleware::new(logger).trust_unverified_jwt_subject(true);

        // Forged token claims to be "victim".
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let header = engine.encode(br#"{"alg":"none"}"#);
        let payload = engine.encode(br#"{"sub":"victim"}"#);
        let forged = format!("{header}.{payload}.sig");

        let mut req = HttpRequest::new("GET".to_string(), "/".to_string());
        req.headers
            .insert("authorization", format!("Bearer {forged}"));
        // Real auth middleware attached a verified principal.
        req.insert_extension(UserContext::new("real-user".to_string()));

        assert_eq!(
            middleware.extract_user_id(&req),
            Some("real-user".to_string()),
            "the verified UserContext must override any token claim"
        );
    }

    #[test]
    fn test_verified_user_context_custom_claim_from_metadata() {
        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());
        let middleware = AuditMiddleware::new(logger).subject_claim("account_id");

        let ctx = UserContext::new("sub-value".to_string())
            .with_metadata(serde_json::json!({ "account_id": "acct-9" }));

        let mut req = HttpRequest::new("GET".to_string(), "/".to_string());
        req.insert_extension(ctx);

        // Custom claim is read from the verified claim set (metadata).
        assert_eq!(middleware.extract_user_id(&req), Some("acct-9".to_string()));
    }

    #[test]
    fn test_truncate_body() {
        let logger = Arc::new(AuditLogger::builder().backend(MemoryBackend::new()).build());

        let middleware = AuditMiddleware::new(logger).max_body_size(10);

        let body = b"This is a very long body that should be truncated";
        let truncated = middleware.truncate_body(body).unwrap();

        assert!(truncated.len() <= 30); // 10 + "... [TRUNCATED]"
        assert!(truncated.contains("[TRUNCATED]"));
    }
}
