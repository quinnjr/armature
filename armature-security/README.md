# armature-security

Security utilities for the Armature framework — HTTP security headers (inspired by
[Helmet](https://helmetjs.github.io/) for Express.js), CORS handling, and HMAC request
signing/verification.

## Features

- **`SecurityMiddleware`** - Applies a comprehensive set of security response headers
  (CSP, HSTS, X-Frame-Options, X-Content-Type-Options, Referrer-Policy, and more)
- **`CorsConfig`** - Cross-origin resource sharing: origin/method/header allow-lists,
  regex origin matching, preflight handling
- **Request signing** - `RequestSigner`, `RequestVerifier`, and
  `RequestSigningMiddleware` for HMAC-based request authentication/integrity

There is no CSRF middleware, response-body sanitization, or input-sanitization utility
in this crate — if you need those, they must come from elsewhere in your stack.

## Installation

```toml
[dependencies]
armature-security = "0.1"
```

## Quick Start - Security Headers

```
use armature_security::SecurityMiddleware;

// Recommended defaults (CSP, HSTS, frame-deny, etc.)
let security = SecurityMiddleware::default();

let response = security.apply(armature_core::HttpResponse::ok());
assert!(response.headers.contains_key("X-Frame-Options"));
assert!(response.headers.contains_key("X-Content-Type-Options"));
```

`SecurityMiddleware` also implements Armature's `Middleware` trait, so it can be added
directly to a middleware chain and it will apply these headers to every response.

## Custom Configuration

```
use armature_security::SecurityMiddleware;
use armature_security::hsts::HstsConfig;
use armature_security::frame_guard::FrameGuard;
use armature_security::referrer_policy::ReferrerPolicy;

let security = SecurityMiddleware::new()
    .with_hsts(HstsConfig::new(31536000).include_subdomains(true))
    .with_frame_guard(FrameGuard::Deny)
    .with_referrer_policy(ReferrerPolicy::StrictOrigin)
    .hide_powered_by(true);

let response = security.apply(armature_core::HttpResponse::ok());
assert_eq!(response.headers.get("X-Frame-Options"), Some(&"DENY".to_string()));
```

## Content Security Policy

```
use armature_security::SecurityMiddleware;
use armature_security::content_security_policy::CspConfig;

let csp = CspConfig::new()
    .default_src(vec!["'self'".to_string()])
    .script_src(vec!["'self'".to_string(), "'unsafe-inline'".to_string()])
    .style_src(vec!["'self'".to_string(), "https://fonts.googleapis.com".to_string()]);

let security = SecurityMiddleware::new().with_csp(csp);
let response = security.apply(armature_core::HttpResponse::ok());

assert!(response.headers.contains_key("Content-Security-Policy"));
```

Setting `.report_only(true)` on a `CspConfig` makes `SecurityMiddleware` emit the
policy under `Content-Security-Policy-Report-Only` instead, so violations are reported
without being enforced:

```
use armature_security::SecurityMiddleware;
use armature_security::content_security_policy::CspConfig;

let csp = CspConfig::default().report_only(true);
let security = SecurityMiddleware::new().with_csp(csp);
let response = security.apply(armature_core::HttpResponse::ok());

assert!(response.headers.contains_key("Content-Security-Policy-Report-Only"));
```

## HSTS (HTTP Strict Transport Security)

```
use armature_security::SecurityMiddleware;
use armature_security::hsts::HstsConfig;

// HSTS for 1 year with subdomains
let hsts = HstsConfig::new(31536000)
    .include_subdomains(true)
    .preload(true);

let security = SecurityMiddleware::new().with_hsts(hsts);
let response = security.apply(armature_core::HttpResponse::ok());

let hsts_header = response.headers.get("Strict-Transport-Security").unwrap();
assert!(hsts_header.contains("max-age=31536000"));
assert!(hsts_header.contains("includeSubDomains"));
```

## Frame Guard (Clickjacking Protection)

```
use armature_security::SecurityMiddleware;
use armature_security::frame_guard::FrameGuard;

// Deny all framing
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::Deny);

let response = security.apply(armature_core::HttpResponse::ok());
assert_eq!(response.headers.get("X-Frame-Options"), Some(&"DENY".to_string()));

// Allow framing from same origin
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::SameOrigin);

let response = security.apply(armature_core::HttpResponse::ok());
assert_eq!(response.headers.get("X-Frame-Options"), Some(&"SAMEORIGIN".to_string()));
```

`FrameGuard::AllowFrom(origin)` also exists, but the `ALLOW-FROM` directive was never
standardized and modern browsers ignore it — prefer restricting framing with a CSP
`frame-ancestors` directive via `CspConfig` instead.

## CORS

```
use armature_security::cors::CorsConfig;

// Development only - allow all origins
let cors = CorsConfig::permissive();

// Production - specific origins
let cors = CorsConfig::new()
    .allow_origin("https://example.com")
    .allow_methods(vec!["GET", "POST", "PUT", "DELETE"])
    .allow_headers(vec!["Content-Type", "Authorization"])
    .allow_credentials(true)
    .max_age(3600);
```

`CorsConfig::apply` adds CORS headers to a normal response, and
`CorsConfig::handle_preflight` builds the response for an `OPTIONS` preflight request.

## Request Signing

```
use armature_security::request_signing::{RequestSigner, RequestVerifier};

let signer = RequestSigner::new("shared-secret");
let verifier = RequestVerifier::new("shared-secret");
```

`RequestSigningMiddleware` wraps this in Armature's `Middleware` trait to verify
incoming request signatures automatically, with `skip_path` to exempt specific routes
(e.g. health checks).

## Deprecated / legacy options

A few settings exist for backwards compatibility but are no-ops or discouraged on
modern browsers, and are documented as such at the type level:

- `expect_ct` (`ExpectCtConfig`) - the `Expect-CT` header is deprecated and removed
  from all major browsers; it is **not** enabled by `SecurityMiddleware::default()` /
  `enable_all()` anymore.
- `xss_filter` (`XssFilter`) - defaults to `Disabled` (`X-XSS-Protection: 0`), matching
  current Helmet guidance, since the legacy browser XSS auditor could itself introduce
  vulnerabilities. Use a strong CSP instead.
- `FrameGuard::AllowFrom` - superseded by CSP's `frame-ancestors` directive.

## License

MIT OR Apache-2.0
