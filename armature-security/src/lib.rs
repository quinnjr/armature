//! Security middleware for Armature - inspired by Helmet for Express.js
//!
//! This module provides a comprehensive set of security headers and protections
//! to help secure your Armature applications against common web vulnerabilities.
//!
//! ## Features
//!
//! - 🛡️ **Content Security Policy** - Prevent XSS attacks
//! - 🔒 **HSTS** - Force HTTPS connections
//! - 🚫 **Frame Guard** - Prevent clickjacking
//! - 🎭 **XSS Filter** - Browser XSS protection
//! - 📝 **Content Type Options** - Prevent MIME sniffing
//! - 🌐 **Referrer Policy** - Control referrer information
//! - 🔐 **11 Security Headers** - Comprehensive protection
//!
//! ## Quick Start - Default Security
//!
//! ```
//! use armature_security::SecurityMiddleware;
//!
//! // Use default security settings (recommended for most apps)
//! let security = SecurityMiddleware::default();
//!
//! // Apply to a response
//! let mut response = armature_core::HttpResponse::ok();
//! let response = security.apply(response);
//!
//! // Now response has all default security headers
//! assert!(response.headers.contains_key("X-Frame-Options"));
//! assert!(response.headers.contains_key("X-Content-Type-Options"));
//! ```
//!
//! ## Custom Configuration
//!
//! ```
//! use armature_security::SecurityMiddleware;
//! use armature_security::hsts::HstsConfig;
//! use armature_security::frame_guard::FrameGuard;
//! use armature_security::referrer_policy::ReferrerPolicy;
//!
//! let security = SecurityMiddleware::new()
//!     .with_hsts(HstsConfig::new(31536000).include_subdomains(true))
//!     .with_frame_guard(FrameGuard::Deny)
//!     .with_referrer_policy(ReferrerPolicy::StrictOrigin)
//!     .hide_powered_by(true);
//!
//! let response = security.apply(armature_core::HttpResponse::ok());
//! assert_eq!(response.headers.get("X-Frame-Options"), Some(&"DENY".to_string()));
//! ```
//!
//! ## Content Security Policy
//!
//! ```
//! use armature_security::SecurityMiddleware;
//! use armature_security::content_security_policy::CspConfig;
//!
//! let csp = CspConfig::new()
//!     .default_src(vec!["'self'".to_string()])
//!     .script_src(vec!["'self'".to_string(), "'unsafe-inline'".to_string()])
//!     .style_src(vec!["'self'".to_string(), "https://fonts.googleapis.com".to_string()]);
//!
//! let security = SecurityMiddleware::new().with_csp(csp);
//! let response = security.apply(armature_core::HttpResponse::ok());
//!
//! assert!(response.headers.contains_key("Content-Security-Policy"));
//! ```
//!
//! ## HSTS (HTTP Strict Transport Security)
//!
//! ```
//! use armature_security::SecurityMiddleware;
//! use armature_security::hsts::HstsConfig;
//!
//! // HSTS for 1 year with subdomains
//! let hsts = HstsConfig::new(31536000)
//!     .include_subdomains(true)
//!     .preload(true);
//!
//! let security = SecurityMiddleware::new().with_hsts(hsts);
//! let response = security.apply(armature_core::HttpResponse::ok());
//!
//! let hsts_header = response.headers.get("Strict-Transport-Security").unwrap();
//! assert!(hsts_header.contains("max-age=31536000"));
//! assert!(hsts_header.contains("includeSubDomains"));
//! ```
//!
//! ## Frame Guard (Clickjacking Protection)
//!
//! ```
//! use armature_security::SecurityMiddleware;
//! use armature_security::frame_guard::FrameGuard;
//!
//! // Deny all framing
//! let security = SecurityMiddleware::new()
//!     .with_frame_guard(FrameGuard::Deny);
//!
//! let response = security.apply(armature_core::HttpResponse::ok());
//! assert_eq!(response.headers.get("X-Frame-Options"), Some(&"DENY".to_string()));
//!
//! // Allow framing from same origin
//! let security = SecurityMiddleware::new()
//!     .with_frame_guard(FrameGuard::SameOrigin);
//!
//! let response = security.apply(armature_core::HttpResponse::ok());
//! assert_eq!(response.headers.get("X-Frame-Options"), Some(&"SAMEORIGIN".to_string()));
//! ```

pub mod content_security_policy;
pub mod content_type_options;
pub mod cors;
pub mod dns_prefetch_control;
pub mod download_options;
pub mod expect_ct;
pub mod frame_guard;
pub mod hsts;
pub mod permitted_cross_domain_policies;
pub mod powered_by;
pub mod referrer_policy;
pub mod request_signing;
pub mod xss_filter;

use armature_core::HttpResponse;
use std::collections::HashMap;

/// Main security middleware that combines all security features
#[derive(Debug, Clone)]
pub struct SecurityMiddleware {
    /// Content Security Policy configuration
    pub csp: Option<content_security_policy::CspConfig>,

    /// DNS Prefetch Control
    pub dns_prefetch_control: dns_prefetch_control::DnsPrefetchControl,

    /// Expect-CT configuration
    pub expect_ct: Option<expect_ct::ExpectCtConfig>,

    /// Frame Guard (X-Frame-Options)
    pub frame_guard: frame_guard::FrameGuard,

    /// HSTS (Strict-Transport-Security)
    pub hsts: Option<hsts::HstsConfig>,

    /// Hide X-Powered-By header
    pub hide_powered_by: bool,

    /// Referrer Policy
    pub referrer_policy: referrer_policy::ReferrerPolicy,

    /// X-XSS-Protection
    pub xss_filter: xss_filter::XssFilter,

    /// X-Content-Type-Options
    pub content_type_options: content_type_options::ContentTypeOptions,

    /// X-Download-Options
    pub download_options: download_options::DownloadOptions,

    /// X-Permitted-Cross-Domain-Policies
    pub permitted_cross_domain_policies:
        permitted_cross_domain_policies::PermittedCrossDomainPolicies,
}

impl SecurityMiddleware {
    /// Create a new security middleware with no protections enabled
    pub fn new() -> Self {
        Self {
            csp: None,
            dns_prefetch_control: dns_prefetch_control::DnsPrefetchControl::Off,
            expect_ct: None,
            frame_guard: frame_guard::FrameGuard::Deny,
            hsts: None,
            hide_powered_by: false,
            referrer_policy: referrer_policy::ReferrerPolicy::NoReferrer,
            // Modern browsers have removed the legacy XSS auditor (it could itself introduce
            // vulnerabilities), so it is disabled by default in line with Helmet's current
            // recommendation. Prefer a strong Content-Security-Policy instead.
            xss_filter: xss_filter::XssFilter::Disabled,
            content_type_options: content_type_options::ContentTypeOptions::NoSniff,
            download_options: download_options::DownloadOptions::NoOpen,
            permitted_cross_domain_policies:
                permitted_cross_domain_policies::PermittedCrossDomainPolicies::None,
        }
    }

    /// Enable Content Security Policy
    pub fn with_csp(mut self, config: content_security_policy::CspConfig) -> Self {
        self.csp = Some(config);
        self
    }

    /// Enable DNS Prefetch Control
    pub fn with_dns_prefetch_control(
        mut self,
        control: dns_prefetch_control::DnsPrefetchControl,
    ) -> Self {
        self.dns_prefetch_control = control;
        self
    }

    /// Enable Expect-CT
    pub fn with_expect_ct(mut self, config: expect_ct::ExpectCtConfig) -> Self {
        self.expect_ct = Some(config);
        self
    }

    /// Set Frame Guard policy
    pub fn with_frame_guard(mut self, guard: frame_guard::FrameGuard) -> Self {
        self.frame_guard = guard;
        self
    }

    /// Enable HSTS
    pub fn with_hsts(mut self, config: hsts::HstsConfig) -> Self {
        self.hsts = Some(config);
        self
    }

    /// Hide X-Powered-By header
    pub fn hide_powered_by(mut self, hide: bool) -> Self {
        self.hide_powered_by = hide;
        self
    }

    /// Set Referrer Policy
    pub fn with_referrer_policy(mut self, policy: referrer_policy::ReferrerPolicy) -> Self {
        self.referrer_policy = policy;
        self
    }

    /// Set XSS Filter
    pub fn with_xss_filter(mut self, filter: xss_filter::XssFilter) -> Self {
        self.xss_filter = filter;
        self
    }

    /// Apply security headers to a response
    pub fn apply(&self, mut response: HttpResponse) -> HttpResponse {
        let mut headers = HashMap::new();

        // Content Security Policy
        if let Some(ref csp) = self.csp {
            let header_name = if csp.report_only {
                "Content-Security-Policy-Report-Only"
            } else {
                "Content-Security-Policy"
            };
            headers.insert(header_name.to_string(), csp.to_header_value());
        }

        // DNS Prefetch Control
        headers.insert(
            "X-DNS-Prefetch-Control".to_string(),
            self.dns_prefetch_control.to_header_value(),
        );

        // Expect-CT
        if let Some(ref expect_ct) = self.expect_ct {
            headers.insert("Expect-CT".to_string(), expect_ct.to_header_value());
        }

        // Frame Guard
        headers.insert(
            "X-Frame-Options".to_string(),
            self.frame_guard.to_header_value(),
        );

        // HSTS
        if let Some(ref hsts) = self.hsts {
            headers.insert(
                "Strict-Transport-Security".to_string(),
                hsts.to_header_value(),
            );
        }

        // Referrer Policy
        headers.insert(
            "Referrer-Policy".to_string(),
            self.referrer_policy.to_header_value(),
        );

        // XSS Filter
        headers.insert(
            "X-XSS-Protection".to_string(),
            self.xss_filter.to_header_value(),
        );

        // Content Type Options
        headers.insert(
            "X-Content-Type-Options".to_string(),
            self.content_type_options.to_header_value(),
        );

        // Download Options
        headers.insert(
            "X-Download-Options".to_string(),
            self.download_options.to_header_value(),
        );

        // Permitted Cross Domain Policies
        headers.insert(
            "X-Permitted-Cross-Domain-Policies".to_string(),
            self.permitted_cross_domain_policies.to_header_value(),
        );

        // Apply all headers
        for (key, value) in headers {
            response.headers.insert(key, value);
        }

        // Remove X-Powered-By if requested
        if self.hide_powered_by {
            response.headers.remove("X-Powered-By");
        }

        response
    }

    /// Convenience method to enable all common security features (recommended defaults)
    ///
    /// Note: Expect-CT is intentionally *not* enabled here. The `Expect-CT` header is
    /// deprecated and has been removed from all major browsers (since ~2023), so it is a
    /// no-op in practice; use it explicitly via [`Self::with_expect_ct`] only if you have a
    /// specific legacy requirement.
    pub fn enable_all(max_age_seconds: u64) -> Self {
        Self {
            csp: Some(content_security_policy::CspConfig::default()),
            dns_prefetch_control: dns_prefetch_control::DnsPrefetchControl::Off,
            expect_ct: None,
            frame_guard: frame_guard::FrameGuard::Deny,
            hsts: Some(hsts::HstsConfig::new(max_age_seconds)),
            hide_powered_by: true,
            referrer_policy: referrer_policy::ReferrerPolicy::NoReferrer,
            // See `new()` for why this defaults to `Disabled`.
            xss_filter: xss_filter::XssFilter::Disabled,
            content_type_options: content_type_options::ContentTypeOptions::NoSniff,
            download_options: download_options::DownloadOptions::NoOpen,
            permitted_cross_domain_policies:
                permitted_cross_domain_policies::PermittedCrossDomainPolicies::None,
        }
    }
}

impl Default for SecurityMiddleware {
    fn default() -> Self {
        Self::enable_all(31536000) // 1 year
    }
}

/// Prelude for common imports.
///
/// ```
/// use armature_security::prelude::*;
/// ```
pub mod prelude {
    pub use crate::SecurityMiddleware;
    pub use crate::content_security_policy::CspConfig;
    pub use crate::cors::CorsConfig;
    pub use crate::frame_guard::FrameGuard;
    pub use crate::hsts::HstsConfig;
    pub use crate::referrer_policy::ReferrerPolicy;
    pub use crate::request_signing::{RequestSigner, RequestSigningMiddleware, RequestVerifier};
}

/// Implement the core Middleware trait for SecurityMiddleware
/// This allows it to be used in a MiddlewareChain
#[async_trait::async_trait]
impl armature_core::Middleware for SecurityMiddleware {
    async fn handle(
        &self,
        req: armature_core::HttpRequest,
        next: Box<
            dyn FnOnce(
                    armature_core::HttpRequest,
                ) -> std::pin::Pin<
                    Box<
                        dyn std::future::Future<
                                Output = Result<armature_core::HttpResponse, armature_core::Error>,
                            > + Send,
                    >,
                > + Send,
        >,
    ) -> Result<armature_core::HttpResponse, armature_core::Error> {
        // Call the next handler first
        let response = next(req).await?;
        // Apply security headers to the response
        Ok(self.apply(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_middleware_new() {
        let middleware = SecurityMiddleware::new();
        assert!(middleware.csp.is_none());
        assert!(!middleware.hide_powered_by);
    }

    #[test]
    fn test_security_middleware_default() {
        let middleware = SecurityMiddleware::default();
        assert!(middleware.csp.is_some());
        assert!(middleware.hsts.is_some());
        assert!(middleware.hide_powered_by);
    }

    #[test]
    fn test_security_middleware_apply() {
        let middleware = SecurityMiddleware::default();
        let response = HttpResponse::ok();
        let secured = middleware.apply(response);

        assert!(secured.headers.contains_key("X-Frame-Options"));
        assert!(secured.headers.contains_key("X-Content-Type-Options"));
        assert!(secured.headers.contains_key("X-XSS-Protection"));
        assert!(secured.headers.contains_key("Strict-Transport-Security"));
        assert!(secured.headers.contains_key("Content-Security-Policy"));
    }

    #[test]
    fn test_xss_filter_defaults_to_disabled() {
        // Modern browsers dropped the legacy XSS auditor; both `new()` and the
        // recommended `enable_all`/`Default` configuration should disable it ("0")
        // rather than opting into the deprecated, sometimes-exploitable filter ("1").
        assert_eq!(
            SecurityMiddleware::new().xss_filter,
            xss_filter::XssFilter::Disabled
        );
        assert_eq!(
            SecurityMiddleware::default().xss_filter,
            xss_filter::XssFilter::Disabled
        );

        let secured = SecurityMiddleware::default().apply(HttpResponse::ok());
        assert_eq!(
            secured.headers.get("X-XSS-Protection"),
            Some(&"0".to_string())
        );
    }

    #[test]
    fn test_enable_all_does_not_enable_expect_ct() {
        // Expect-CT is deprecated and removed from browsers; the recommended defaults
        // should not send it.
        let middleware = SecurityMiddleware::default();
        assert!(middleware.expect_ct.is_none());

        let secured = middleware.apply(HttpResponse::ok());
        assert!(!secured.headers.contains_key("Expect-CT"));
    }

    #[test]
    fn test_csp_header_name_depends_on_report_only() {
        // Enforcing CSP uses the standard header name.
        let enforcing =
            SecurityMiddleware::new().with_csp(content_security_policy::CspConfig::default());
        let secured = enforcing.apply(HttpResponse::ok());
        assert!(secured.headers.contains_key("Content-Security-Policy"));
        assert!(
            !secured
                .headers
                .contains_key("Content-Security-Policy-Report-Only")
        );

        // report_only(true) must emit the header under the Report-Only name instead.
        let report_only = SecurityMiddleware::new()
            .with_csp(content_security_policy::CspConfig::default().report_only(true));
        let secured = report_only.apply(HttpResponse::ok());
        assert!(
            secured
                .headers
                .contains_key("Content-Security-Policy-Report-Only")
        );
        assert!(!secured.headers.contains_key("Content-Security-Policy"));
    }

    #[test]
    fn test_hide_powered_by() {
        let middleware = SecurityMiddleware::new().hide_powered_by(true);
        let mut response = HttpResponse::ok();
        response
            .headers
            .insert("X-Powered-By".to_string(), "Armature".to_string());

        let secured = middleware.apply(response);
        assert!(!secured.headers.contains_key("X-Powered-By"));
    }

    #[test]
    fn test_custom_configuration() {
        let middleware = SecurityMiddleware::new()
            .with_frame_guard(frame_guard::FrameGuard::SameOrigin)
            .with_referrer_policy(referrer_policy::ReferrerPolicy::StrictOriginWhenCrossOrigin)
            .hide_powered_by(true);

        let response = HttpResponse::ok();
        let secured = middleware.apply(response);

        assert_eq!(
            secured.headers.get("X-Frame-Options"),
            Some(&"SAMEORIGIN".to_string())
        );
        assert_eq!(
            secured.headers.get("Referrer-Policy"),
            Some(&"strict-origin-when-cross-origin".to_string())
        );
    }
}
