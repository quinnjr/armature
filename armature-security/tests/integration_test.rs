//! Integration tests for armature-security

use armature_core::HttpResponse;
use armature_security::SecurityMiddleware;
use armature_security::content_security_policy::CspConfig;
use armature_security::content_type_options::ContentTypeOptions;
use armature_security::dns_prefetch_control::DnsPrefetchControl;
use armature_security::download_options::DownloadOptions;
#[allow(deprecated)]
use armature_security::expect_ct::ExpectCtConfig;
use armature_security::frame_guard::FrameGuard;
use armature_security::hsts::HstsConfig;
use armature_security::permitted_cross_domain_policies::PermittedCrossDomainPolicies;
use armature_security::referrer_policy::ReferrerPolicy;
use armature_security::xss_filter::XssFilter;

#[test]
fn test_security_middleware_default() {
    let security = SecurityMiddleware::default();
    let response = HttpResponse::ok();
    let secured = security.apply(response);

    // Should have security headers
    assert!(secured.headers.contains_key("Strict-Transport-Security"));
    assert!(secured.headers.contains_key("X-Frame-Options"));
    assert!(secured.headers.contains_key("X-Content-Type-Options"));
}

#[test]
fn test_security_middleware_custom() {
    let security = SecurityMiddleware::new()
        .with_hsts(HstsConfig::new(31536000))
        .with_frame_guard(FrameGuard::Deny)
        .hide_powered_by(true);

    let response = HttpResponse::ok();
    let secured = security.apply(response);

    assert_eq!(
        secured.headers.get("X-Frame-Options"),
        Some(&"DENY".to_string())
    );
    assert!(!secured.headers.contains_key("X-Powered-By"));
}

#[test]
fn test_hsts_config() {
    let hsts = HstsConfig::new(31536000)
        .include_subdomains(true)
        .preload(true);

    let header = hsts.to_header_value();
    assert!(header.contains("max-age=31536000"));
    assert!(header.contains("includeSubDomains"));
    assert!(header.contains("preload"));
}

#[test]
#[allow(deprecated)]
fn test_frame_guard_variants() {
    assert_eq!(FrameGuard::Deny.to_header_value(), "DENY");
    assert_eq!(FrameGuard::SameOrigin.to_header_value(), "SAMEORIGIN");
    assert_eq!(
        FrameGuard::AllowFrom("https://example.com".to_string()).to_header_value(),
        "ALLOW-FROM https://example.com"
    );
}

#[test]
fn test_csp_config() {
    let csp = CspConfig::new()
        .default_src(vec!["'self'".to_string()])
        .script_src(vec!["'self'".to_string(), "'unsafe-inline'".to_string()])
        .style_src(vec!["'self'".to_string()]);

    let header = csp.to_header_value();
    assert!(header.contains("default-src 'self'"));
    assert!(header.contains("script-src 'self' 'unsafe-inline'"));
}

#[test]
fn test_referrer_policy() {
    assert_eq!(ReferrerPolicy::NoReferrer.to_header_value(), "no-referrer");
    assert_eq!(
        ReferrerPolicy::StrictOrigin.to_header_value(),
        "strict-origin"
    );
    assert_eq!(ReferrerPolicy::SameOrigin.to_header_value(), "same-origin");
}

#[test]
fn test_xss_filter() {
    assert_eq!(XssFilter::Enabled.to_header_value(), "1");
    assert_eq!(XssFilter::Disabled.to_header_value(), "0");
    assert_eq!(XssFilter::EnabledBlock.to_header_value(), "1; mode=block");
}

#[test]
fn test_content_type_options() {
    assert_eq!(ContentTypeOptions::NoSniff.to_header_value(), "nosniff");
}

#[test]
fn test_dns_prefetch_control() {
    assert_eq!(DnsPrefetchControl::On.to_header_value(), "on");
    assert_eq!(DnsPrefetchControl::Off.to_header_value(), "off");
}

#[test]
#[allow(deprecated)]
fn test_expect_ct_config() {
    let expect_ct = ExpectCtConfig::new(86400)
        .enforce(true)
        .report_uri("https://example.com/report".to_string());

    let header = expect_ct.to_header_value();
    assert!(header.contains("max-age=86400"));
    assert!(header.contains("enforce"));
    assert!(header.contains("report-uri="));
}

#[test]
fn test_download_options() {
    assert_eq!(DownloadOptions::NoOpen.to_header_value(), "noopen");
}

#[test]
fn test_permitted_cross_domain_policies() {
    assert_eq!(PermittedCrossDomainPolicies::None.to_header_value(), "none");
    assert_eq!(
        PermittedCrossDomainPolicies::MasterOnly.to_header_value(),
        "master-only"
    );
}

#[test]
fn test_security_middleware_all_headers() {
    let security = SecurityMiddleware::new()
        .with_hsts(HstsConfig::new(31536000))
        .with_frame_guard(FrameGuard::Deny)
        .with_xss_filter(XssFilter::EnabledBlock)
        .with_referrer_policy(ReferrerPolicy::StrictOrigin)
        .with_dns_prefetch_control(DnsPrefetchControl::Off)
        .hide_powered_by(true);

    let response = HttpResponse::ok();
    let secured = security.apply(response);

    // Verify headers are present
    assert!(secured.headers.contains_key("Strict-Transport-Security"));
    assert!(secured.headers.contains_key("X-Frame-Options"));
    assert!(secured.headers.contains_key("X-XSS-Protection"));
    assert!(secured.headers.contains_key("Referrer-Policy"));
    assert!(secured.headers.contains_key("X-DNS-Prefetch-Control"));
}
