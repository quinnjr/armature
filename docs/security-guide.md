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

✅ **Content Security Policy (CSP)** - Prevent XSS attacks
✅ **HTTP Strict Transport Security (HSTS)** - Force HTTPS
✅ **X-Frame-Options** - Prevent clickjacking
✅ **X-Content-Type-Options** - Prevent MIME sniffing
✅ **X-XSS-Protection** - Enable browser XSS filters
✅ **Referrer Policy** - Control referrer information
✅ **DNS Prefetch Control** - Control DNS prefetching
✅ **Expect-CT** - Certificate Transparency
✅ **X-Download-Options** - Prevent IE download execution
✅ **X-Permitted-Cross-Domain-Policies** - Control Flash/PDF policies
✅ **Hide X-Powered-By** - Remove server fingerprinting

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["security"] }
```

## Quick Start

### Adding to Middleware Chain (Recommended)

The preferred way to use security middleware is to add it to your application's middleware chain:

```rust
use armature_framework::prelude::*;
use armature_framework::{MiddlewareChain, SecurityHeadersMiddleware};
use armature_security::SecurityMiddleware;

// Create your middleware chain
let mut middleware_chain = MiddlewareChain::new();

// Add security middleware (recommended to be early in the chain)
middleware_chain.use_middleware(SecurityMiddleware::default());

// Add other middleware
middleware_chain.use_middleware(SecurityHeadersMiddleware::new());

// Define your module
#[module(
    providers: [MyService],
    controllers: [MyController]
)]
#[derive(Default)]
struct AppModule;
```

### Full Application Example

```rust
use armature_framework::prelude::*;
use armature_framework::{MiddlewareChain, LoggerMiddleware, CorsMiddleware};
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
async fn main() {
    // Build middleware chain with security
    let mut middleware_chain = MiddlewareChain::new();
    
    // 1. Logging (first to capture all requests)
    middleware_chain.use_middleware(LoggerMiddleware::new());
    
    // 2. CORS (handle preflight early)
    middleware_chain.use_middleware(
        CorsMiddleware::new()
            .allow_origin("https://example.com")
            .allow_credentials(true)
    );
    
    // 3. Security headers (apply to all responses)
    middleware_chain.use_middleware(
        SecurityMiddleware::new()
            .with_hsts(HstsConfig::new(31536000).include_subdomains(true))
            .with_frame_guard(FrameGuard::Deny)
            .with_csp(CspConfig::new().default_src(vec!["'self'".to_string()]))
            .hide_powered_by(true)
    );
    
    println!("Server running with security middleware enabled");
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
    .with_csp(CspConfig::default())
    .with_hsts(HstsConfig::new(31536000)) // 1 year
    .with_frame_guard(FrameGuard::Deny)
    .with_referrer_policy(ReferrerPolicy::NoReferrer)
    .hide_powered_by(true);

// Add to middleware chain
middleware_chain.use_middleware(security);
```

## Configuration

### Enable All Features (Recommended)

```rust
// All security features with 1-year HSTS
let security = SecurityMiddleware::enable_all(31536000);
```

### Start from Scratch

```rust
// No protections - configure manually
let security = SecurityMiddleware::new()
    .with_frame_guard(FrameGuard::SameOrigin)
    .hide_powered_by(true);
```

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
    .style_src(vec![
        "'self'".to_string(),
        "'unsafe-inline'".to_string()
    ])
    .img_src(vec![
        "'self'".to_string(),
        "data:".to_string(),
        "https:".to_string()
    ]);

let security = SecurityMiddleware::new().with_csp(csp);
```

**Output Header:**
```
Content-Security-Policy: default-src 'self'; script-src 'self' https://cdn.example.com; ...
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

Enables the browser's XSS filtering.

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

**Output Header:**
```
X-XSS-Protection: 1; mode=block
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

Helps detect misissued certificates.

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
    )
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

- `new()` - Create with no protections
- `default()` - Create with recommended settings
- `enable_all(max_age: u64)` - Enable all protections
- `with_csp(config: CspConfig)` - Add CSP
- `with_hsts(config: HstsConfig)` - Add HSTS
- `with_frame_guard(guard: FrameGuard)` - Set frame options
- `with_referrer_policy(policy: ReferrerPolicy)` - Set referrer policy
- `with_xss_filter(filter: XssFilter)` - Set XSS filter
- `with_dns_prefetch_control(control: DnsPrefetchControl)` - Control DNS prefetch
- `with_expect_ct(config: ExpectCtConfig)` - Add Expect-CT
- `hide_powered_by(hide: bool)` - Hide X-Powered-By header
- `apply(response: HttpResponse) -> HttpResponse` - Apply headers to response

### Submodules

- `content_security_policy` - CSP configuration
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
use armature_framework::prelude::*;
use armature_framework::{
    MiddlewareChain, LoggerMiddleware, CorsMiddleware, 
    RequestIdMiddleware, TimeoutMiddleware
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
async fn main() {
    println!("🔒 Secure API Server");
    println!("====================\n");
    
    // Build comprehensive middleware stack
    let mut middleware = MiddlewareChain::new();
    
    // Request tracking
    middleware.use_middleware(RequestIdMiddleware);
    middleware.use_middleware(LoggerMiddleware::new());
    
    // CORS for cross-origin requests
    middleware.use_middleware(
        CorsMiddleware::new()
            .allow_origin("https://myapp.com")
            .allow_methods("GET, POST, PUT, DELETE")
            .allow_headers("Content-Type, Authorization")
            .allow_credentials(true)
    );
    
    // Security headers - comprehensive protection
    middleware.use_middleware(
        SecurityMiddleware::new()
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
                    .style_src(vec!["'self'".to_string(), "'unsafe-inline'".to_string()])
                    .img_src(vec!["'self'".to_string(), "data:".to_string(), "https:".to_string()])
                    .connect_src(vec!["'self'".to_string(), "https://api.example.com".to_string()])
            )
            .hide_powered_by(true)
    );
    
    // Request timeout
    middleware.use_middleware(TimeoutMiddleware::new(30));
    
    println!("Middleware stack configured:");
    println!("  ✓ Request ID tracking");
    println!("  ✓ Request logging");
    println!("  ✓ CORS protection");
    println!("  ✓ Security headers (HSTS, CSP, Frame-Options, etc.)");
    println!("  ✓ 30 second timeout");
    println!();
    println!("Server running on http://localhost:3000");
}
```

### API-Only Configuration (Minimal)

```rust
use armature_framework::prelude::*;
use armature_framework::MiddlewareChain;
use armature_security::{SecurityMiddleware, frame_guard::FrameGuard};

// For API servers that don't serve HTML, use a minimal config
let mut middleware = MiddlewareChain::new();

middleware.use_middleware(
    SecurityMiddleware::new()
        .with_frame_guard(FrameGuard::Deny)  // Prevent embedding
        .hide_powered_by(true)                // Hide server info
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
middleware_chain.use_middleware(create_security_middleware(is_prod));
```

## Summary

The Armature Security middleware provides comprehensive protection against common web vulnerabilities:

- ✅ Easy to use - `SecurityMiddleware::default()` for most cases
- ✅ Highly customizable - Configure each feature individually
- ✅ Production-ready - Based on industry best practices
- ✅ Well-tested - 21+ unit tests covering all features
- ✅ Type-safe - Full Rust type safety

For more examples, see `examples/security_example.rs` in the repository.

