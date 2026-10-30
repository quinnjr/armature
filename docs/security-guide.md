# Security Middleware

Comprehensive security middleware for Armature - inspired by Helmet for Express.js.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Security Headers](#security-headers)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)

## Overview

The Armature Security middleware provides a collection of security headers and protections that help secure your web applications against common vulnerabilities. It's inspired by [Helmet](https://helmetjs.github.io/) for Express.js and provides similar functionality for Rust/Armature applications.

## Features

✅ **Content Security Policy (CSP)** - Prevent XSS attacks (validated, strict default)
✅ **HTTP Strict Transport Security (HSTS)** - Force HTTPS
✅ **X-Frame-Options** - Prevent clickjacking
✅ **X-Content-Type-Options** - Prevent MIME sniffing
✅ **Cross-Origin-Opener-Policy / Cross-Origin-Resource-Policy** - Cross-origin isolation (`same-origin` by default)
✅ **Permissions-Policy** - Disable powerful browser features (restrictive by default)
✅ **X-XSS-Protection** - Legacy XSS auditor control (disabled by default)
✅ **Referrer Policy** - Control referrer information
✅ **DNS Prefetch Control** - Control DNS prefetching
✅ **Expect-CT** - Certificate Transparency (deprecated; off by default)
✅ **X-Download-Options** - Prevent IE download execution
✅ **X-Permitted-Cross-Domain-Policies** - Control Flash/PDF policies
✅ **Hide X-Powered-By** - Remove server fingerprinting

The crate also ships CSRF protection (`armature_security::csrf`), CORS
configuration and HMAC request signing; see the
[Advanced Security Guide](security-advanced-guide.md).

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature = { package = "armature-framework", version = "0.9", features = ["security"] }
```

## Quick Start

### Registering as Global Middleware (Recommended)

Register the middleware on the application with `Application::use_middleware`.
Global middleware runs in registration order, before any guard and handler,
so the headers are applied to every response, including guard rejections:

```rust
use armature::prelude::*;
use armature_security::SecurityMiddleware;

#[module(
    providers: [MyService],
    controllers: [MyController]
)]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::create::<AppModule>()
        .await
        .use_middleware(SecurityMiddleware::default())
        .listen(3000)
        .await?;
    Ok(())
}
```

### Full Application Example

```rust
use armature::prelude::*;
use armature::{LoggerMiddleware, CorsMiddleware};
use armature_security::{
    SecurityMiddleware,
    content_security_policy::CspConfig,
    hsts::HstsConfig,
    frame_guard::FrameGuard,
};

#[injectable]
#[derive(Clone, Default)]
struct ApiService;

#[controller("/api")]
#[derive(Default, Clone)]
struct ApiController;

impl ApiController {
    #[get("/data")]
    async fn get_data() -> Result<Json<serde_json::Value>, Error> {
        Ok(Json(serde_json::json!({ "status": "ok" })))
    }
}

#[module(
    providers: [ApiService],
    controllers: [ApiController]
)]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Security headers (apply to all responses). `with_csp` validates the
    // policy and returns `Result<_, CspError>`.
    let security = SecurityMiddleware::new()
        .with_hsts(HstsConfig::new(31536000).include_subdomains(true))
        .with_frame_guard(FrameGuard::Deny)
        .with_csp(CspConfig::new().default_src(vec!["'self'".to_string()]))?
        .hide_powered_by(true);

    Application::create::<AppModule>()
        .await
        // 1. Logging (first to capture all requests)
        .use_middleware(LoggerMiddleware::new())
        // 2. CORS (handle preflight early)
        .use_middleware(
            CorsMiddleware::new()
                .allow_origin("https://example.com")
                .allow_credentials(true),
        )
        // 3. Security headers
        .use_middleware(security)
        .listen(3000)
        .await?;
    Ok(())
}
```

### Custom Configuration

```rust
use armature_security::{
    SecurityMiddleware,
    content_security_policy::CspConfig,
    hsts::HstsConfig,
    frame_guard::FrameGuard,
    referrer_policy::ReferrerPolicy,
};

let security = SecurityMiddleware::new()
    .with_csp(CspConfig::default())?
    .with_hsts(HstsConfig::new(31536000)) // 1 year
    .with_frame_guard(FrameGuard::Deny)
    .with_referrer_policy(ReferrerPolicy::NoReferrer)
    .hide_powered_by(true);

// Register as global middleware
let app = app.use_middleware(security);
```

## Configuration

### Enable All Features (Recommended)

```rust
// All security features with 1-year HSTS and the strict default CSP
let security = SecurityMiddleware::enable_all(31536000);
```

### Start from the Always-Safe Baseline

`SecurityMiddleware::new()` turns on only the headers that are safe for every
deployment: `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`,
`X-Content-Type-Options: nosniff`, `X-Download-Options: noopen`,
`X-Permitted-Cross-Domain-Policies: none`, `X-DNS-Prefetch-Control: off`,
`Cross-Origin-Opener-Policy: same-origin`,
`Cross-Origin-Resource-Policy: same-origin` and a restrictive
`Permissions-Policy`. CSP, HSTS and Expect-CT stay off until configured:

```rust
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::SameOrigin)
    .hide_powered_by(true);
```

### Handler-Set Headers Win

`SecurityMiddleware::apply` only adds a security header the response does not
already carry under any letter case, so a handler that sets its own
route-specific policy keeps it. `hide_powered_by(true)` removes every spelling
of `X-Powered-By`.

## Security Headers

### Content Security Policy (CSP)

Prevents XSS attacks by declaring which dynamic resources are allowed to load.

```rust
use armature_security::content_security_policy::CspConfig;

let csp = CspConfig::new()
    .default_src(vec!["'self'".to_string()])
    .script_src(vec![
        "'self'".to_string(),
        "https://cdn.example.com".to_string()
    ])
    .style_src(vec!["'self'".to_string()])
    .img_src(vec!["'self'".to_string(), "data:".to_string()])
    .object_src(vec!["'none'".to_string()])
    .frame_ancestors(vec!["'none'".to_string()])
    .base_uri(vec!["'none'".to_string()])
    .form_action(vec!["'self'".to_string()]);

// Directive names and values are validated: `;`, `,`, whitespace and
// control characters (CR/LF) are rejected with a `CspError`.
let security = SecurityMiddleware::new().with_csp(csp)?;
```

**Output Header** (directives in name order):
```
Content-Security-Policy: base-uri 'none'; default-src 'self'; form-action 'self'; frame-ancestors 'none'; ...
```

`CspConfig::default()` is a strict baseline: `'self'` for scripts, styles,
fonts and connections, `'self' data:` for images, and `'none'` for
`object-src`, `frame-ancestors` and `base-uri`. It does **not** allow
`'unsafe-inline'` styles or arbitrary `https:` images; add them explicitly if
you need them. `CspConfig::to_header_value()` returns
`Result<String, CspError>`.

### Cross-Origin Isolation and Permissions-Policy

```rust
use armature_security::cross_origin::{CrossOriginOpenerPolicy, CrossOriginResourcePolicy};
use armature_security::permissions_policy::PermissionsPolicy;

let security = SecurityMiddleware::default()
    // Needed by some OAuth / payment popup flows
    .with_coop(Some(CrossOriginOpenerPolicy::SameOriginAllowPopups))
    // Assets embedded by other origins
    .with_corp(Some(CrossOriginResourcePolicy::CrossOrigin))
    // `None` stops sending the header
    .with_permissions_policy(Some(PermissionsPolicy::restrictive().allow_self("fullscreen")));
```

**Output Headers** (defaults):
```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
Permissions-Policy: accelerometer=(), autoplay=(), camera=(), ...
```

### HTTP Strict Transport Security (HSTS)

Forces browsers to use HTTPS.

```rust
use armature_security::hsts::HstsConfig;

let hsts = HstsConfig::new(31536000) // 1 year in seconds
    .include_subdomains(true)
    .preload(true);

let security = SecurityMiddleware::new().with_hsts(hsts);
```

**Output Header:**
```
Strict-Transport-Security: max-age=31536000; includeSubDomains; preload
```

### X-Frame-Options

Prevents clickjacking by controlling if your site can be framed.

```rust
use armature_security::frame_guard::FrameGuard;

// Deny all framing
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::Deny);

// Allow same origin
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::SameOrigin);

// Allow specific origin
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::AllowFrom("https://example.com".to_string()));
```

**Output Header:**
```
X-Frame-Options: DENY
X-Frame-Options: SAMEORIGIN
X-Frame-Options: ALLOW-FROM https://example.com
```

### Referrer Policy

Controls how much referrer information is sent with requests.

```rust
use armature_security::referrer_policy::ReferrerPolicy;

let security = SecurityMiddleware::new()
    .with_referrer_policy(ReferrerPolicy::NoReferrer);

// Other options:
// - NoReferrer
// - NoReferrerWhenDowngrade
// - Origin
// - OriginWhenCrossOrigin
// - SameOrigin
// - StrictOrigin
// - StrictOriginWhenCrossOrigin
// - UnsafeUrl
```

**Output Header:**
```
Referrer-Policy: no-referrer
```

### X-XSS-Protection

Controls the legacy browser XSS auditor. Modern browsers have removed it (and
it could itself introduce vulnerabilities), so it is `Disabled` by default;
rely on CSP instead.

```rust
use armature_security::xss_filter::XssFilter;

// Enable with blocking
let security = SecurityMiddleware::new()
    .with_xss_filter(XssFilter::EnabledBlock);

// Just enable (don't block)
let security = SecurityMiddleware::new()
    .with_xss_filter(XssFilter::Enabled);

// Disable
let security = SecurityMiddleware::new()
    .with_xss_filter(XssFilter::Disabled);
```

**Output Header** (default):
```
X-XSS-Protection: 0
```

### DNS Prefetch Control

Controls browser DNS prefetching.

```rust
use armature_security::dns_prefetch_control::DnsPrefetchControl;

// Disable DNS prefetching (more privacy)
let security = SecurityMiddleware::new()
    .with_dns_prefetch_control(DnsPrefetchControl::Off);

// Enable DNS prefetching (better performance)
let security = SecurityMiddleware::new()
    .with_dns_prefetch_control(DnsPrefetchControl::On);
```

**Output Header:**
```
X-DNS-Prefetch-Control: off
```

### Expect-CT

Deprecated: the header has been removed from all major browsers, so it is
never enabled by default and `ExpectCtConfig` is marked `#[deprecated]`.
Configure it only for a specific legacy requirement.

```rust
use armature_security::expect_ct::ExpectCtConfig;

let expect_ct = ExpectCtConfig::new(86400) // 1 day
    .enforce(true)
    .report_uri("https://example.com/report".to_string());

let security = SecurityMiddleware::new().with_expect_ct(expect_ct);
```

**Output Header:**
```
Expect-CT: max-age=86400, enforce, report-uri="https://example.com/report"
```

### Hide X-Powered-By

Removes the `X-Powered-By` header to prevent server fingerprinting.

```rust
let security = SecurityMiddleware::new()
    .hide_powered_by(true);
```

**Result:** `X-Powered-By` header is removed from responses.

## Best Practices

### 1. Use Default Settings for Production

```rust
// Recommended for most applications
let security = SecurityMiddleware::default();
```

### 2. Customize for Your Needs

```rust
// Example: API server with custom CSP
let security = SecurityMiddleware::new()
    .with_csp(
        CspConfig::new()
            .default_src(vec!["'self'".to_string()])
            .connect_src(vec!["'self'".to_string(), "https://api.example.com".to_string()])
    )?
    .with_hsts(HstsConfig::new(31536000))
    .hide_powered_by(true);
```

### 3. Test in Development

Use browser developer tools to verify headers are applied:

```bash
# Chrome/Firefox: Network tab → Select request → Headers section
```

### 4. HSTS Considerations

- Start with a shorter `max-age` (e.g., 300 seconds) in testing
- Gradually increase to 1 year (31536000 seconds) in production
- Only enable `preload` after testing thoroughly

### 5. CSP Development

- Start with `report-only` mode:
  ```rust
  let csp = CspConfig::default().report_only(true);
  ```
- Monitor violations before enforcing
- Gradually tighten policies

## API Reference

### `SecurityMiddleware`

Main security middleware struct.

#### Methods

- `new()` - Create with the always-safe headers (frame guard, nosniff, referrer policy, COOP/CORP, Permissions-Policy, ...); no CSP or HSTS
- `default()` - Same as `enable_all(31536000)`
- `enable_all(max_age: u64)` - Enable all protections, including the default CSP and HSTS
- `with_csp(config: CspConfig) -> Result<Self, CspError>` - Add a validated CSP
- `with_coop(policy: Option<CrossOriginOpenerPolicy>)` - Set or disable COOP
- `with_corp(policy: Option<CrossOriginResourcePolicy>)` - Set or disable CORP
- `with_permissions_policy(policy: Option<PermissionsPolicy>)` - Set or disable Permissions-Policy
- `with_hsts(config: HstsConfig)` - Add HSTS
- `with_frame_guard(guard: FrameGuard)` - Set frame options
- `with_referrer_policy(policy: ReferrerPolicy)` - Set referrer policy
- `with_xss_filter(filter: XssFilter)` - Set XSS filter
- `with_dns_prefetch_control(control: DnsPrefetchControl)` - Control DNS prefetch
- `with_expect_ct(config: ExpectCtConfig)` - Add Expect-CT (deprecated header)
- `hide_powered_by(hide: bool)` - Hide X-Powered-By header
- `apply(response: HttpResponse) -> HttpResponse` - Add any security header the response does not already carry

### Submodules

- `content_security_policy` - CSP configuration
- `cross_origin` - Cross-Origin-Opener-Policy / Cross-Origin-Resource-Policy
- `permissions_policy` - Permissions-Policy
- `csrf` - CSRF protection (`CsrfMiddleware`, `CsrfConfig`)
- `cors` - CORS configuration
- `request_signing` - HMAC request signing
- `hsts` - HSTS configuration
- `frame_guard` - X-Frame-Options
- `referrer_policy` - Referrer-Policy
- `xss_filter` - X-XSS-Protection
- `dns_prefetch_control` - X-DNS-Prefetch-Control
- `expect_ct` - Expect-CT
- `content_type_options` - X-Content-Type-Options
- `download_options` - X-Download-Options
- `permitted_cross_domain_policies` - X-Permitted-Cross-Domain-Policies

## Examples

### Complete Application with Middleware Chain

```rust
use armature::prelude::*;
use armature::{
    LoggerMiddleware, CorsMiddleware, RequestIdMiddleware, TimeoutMiddleware
};
use armature_security::{
    SecurityMiddleware,
    content_security_policy::CspConfig,
    hsts::HstsConfig,
    frame_guard::FrameGuard,
    referrer_policy::ReferrerPolicy,
};

#[injectable]
#[derive(Clone, Default)]
struct UserService;

impl UserService {
    fn get_users(&self) -> Vec<String> {
        vec!["Alice".to_string(), "Bob".to_string()]
    }
}

#[controller("/api/users")]
#[derive(Default, Clone)]
struct UserController;

impl UserController {
    #[get("/")]
    async fn list_users() -> Result<Json<Vec<String>>, Error> {
        let service = UserService::default();
        Ok(Json(service.get_users()))
    }
    
    #[get("/:id")]
    async fn get_user(req: HttpRequest) -> Result<Json<String>, Error> {
        let id = req.params.get("id").unwrap_or(&"0".to_string()).clone();
        Ok(Json(format!("User {}", id)))
    }
}

#[module(
    providers: [UserService],
    controllers: [UserController]
)]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Security headers - comprehensive protection
    let security = SecurityMiddleware::new()
        .with_hsts(
            HstsConfig::new(31536000)
                .include_subdomains(true)
                .preload(true)
        )
        .with_frame_guard(FrameGuard::Deny)
        .with_referrer_policy(ReferrerPolicy::StrictOriginWhenCrossOrigin)
        .with_csp(
            CspConfig::new()
                .default_src(vec!["'self'".to_string()])
                .script_src(vec!["'self'".to_string()])
                .style_src(vec!["'self'".to_string()])
                .img_src(vec!["'self'".to_string(), "data:".to_string()])
                .connect_src(vec!["'self'".to_string(), "https://api.example.com".to_string()])
                .frame_ancestors(vec!["'none'".to_string()])
        )?
        .hide_powered_by(true);

    // Global middleware runs in registration order, before guards and handlers
    Application::create::<AppModule>()
        .await
        // Request tracking
        .use_middleware(RequestIdMiddleware)
        .use_middleware(LoggerMiddleware::new())
        // CORS for cross-origin requests
        .use_middleware(
            CorsMiddleware::new()
                .allow_origin("https://myapp.com")
                .allow_methods("GET, POST, PUT, DELETE")
                .allow_headers("Content-Type, Authorization")
                .allow_credentials(true)
        )
        .use_middleware(security)
        // Request timeout
        .use_middleware(TimeoutMiddleware::new(30))
        .listen(3000)
        .await?;
    Ok(())
}
```

### API-Only Configuration (Minimal)

```rust
use armature::prelude::*;
use armature_security::{SecurityMiddleware, frame_guard::FrameGuard};

// For API servers that don't serve HTML, the always-safe baseline is enough
let app = Application::create::<AppModule>()
    .await
    .use_middleware(
        SecurityMiddleware::new()
            .with_frame_guard(FrameGuard::Deny)  // Prevent embedding
            .hide_powered_by(true)               // Hide server info
            // Skip CSP for API-only servers
    );
```

### Development vs Production

```rust
use armature_security::{SecurityMiddleware, hsts::HstsConfig};

fn create_security_middleware(is_production: bool) -> SecurityMiddleware {
    if is_production {
        // Full security for production
        SecurityMiddleware::new()
            .with_hsts(HstsConfig::new(31536000).preload(true))
            .hide_powered_by(true)
    } else {
        // Relaxed for development
        SecurityMiddleware::new()
            .with_hsts(HstsConfig::new(300)) // Short HSTS for testing
            .hide_powered_by(false)          // Keep for debugging
    }
}

// Usage
let is_prod = std::env::var("ENVIRONMENT").map(|v| v == "production").unwrap_or(false);
let app = app.use_middleware(create_security_middleware(is_prod));
```

## Summary

The Armature Security middleware provides comprehensive protection against common web vulnerabilities:

- ✅ Easy to use - `SecurityMiddleware::default()` for most cases
- ✅ Highly customizable - Configure each feature individually
- ✅ Strict defaults - Based on current industry guidance (OWASP, Helmet)
- ✅ Non-destructive - Never overwrites headers your handlers set
- ✅ Type-safe - Full Rust type safety

For more examples, see `examples/security_example.rs` in the repository.

