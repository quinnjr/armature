# Advanced Security Guide

Comprehensive guide to advanced security features in Armature including CORS, CSP, HSTS, and request signing.

## Table of Contents

- [Overview](#overview)
- [CORS (Cross-Origin Resource Sharing)](#cors)
- [Content Security Policy (CSP)](#csp)
- [HSTS (HTTP Strict Transport Security)](#hsts)
- [Request Signing with HMAC](#request-signing)
- [Best Practices](#best-practices)
- [Security Checklist](#security-checklist)

## Overview

Armature provides enterprise-grade security features to protect your applications from common web vulnerabilities and attacks.

### Security Features

- ✅ **Granular CORS Control** - Origin patterns, method restrictions, credential handling
- ✅ **Content Security Policy** - Prevent XSS attacks with CSP directives
- ✅ **HSTS** - Force HTTPS with preload support
- ✅ **Request Signing** - HMAC-SHA256 verification with replay protection
- ✅ **Security Headers** - 11+ security headers automatically applied
- ✅ **Rate Limiting** - Token bucket and sliding window algorithms

## CORS

### Basic Configuration

```rust
use armature_security::cors::CorsConfig;

// Strict production CORS
let cors = CorsConfig::new()
    .allow_origin("https://app.example.com")
    .allow_origin("https://admin.example.com")
    .allow_methods(vec!["GET", "POST", "PUT", "DELETE"])
    .allow_headers(vec!["Content-Type", "Authorization"])
    .allow_credentials(true)
    .max_age(3600); // 1 hour preflight cache
```

### Origin Patterns (Regex)

Allow multiple subdomains with regex patterns:

```rust
let cors = CorsConfig::new()
    // Allow all subdomains of example.com
    .allow_origin_regex(r"https://.*\.example\.com").unwrap()
    // Allow multiple TLDs
    .allow_origin_regex(r"https://app\.(com|net|org)").unwrap();
```

### Development vs Production

```rust
// ❌ Development only - allows all origins
let cors = CorsConfig::permissive();

// ✅ Production - specific origins only
let cors = CorsConfig::new()
    .allow_origin("https://app.example.com")
    .allow_credentials(true);
```

## CSP

### Basic Configuration

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
        "https://fonts.googleapis.com".to_string()
    ])
    .img_src(vec![
        "'self'".to_string(),
        "data:".to_string(),
        "https:".to_string()
    ]);
```

### CSP Directives

Common directives:

- `default-src` - Fallback for all directives
- `script-src` - JavaScript sources
- `style-src` - CSS sources
- `img-src` - Image sources
- `font-src` - Font sources
- `connect-src` - AJAX, WebSocket, EventSource
- `frame-src` - iframe sources

## HSTS

### Basic Configuration

```rust
use armature_security::hsts::HstsConfig;

let hsts = HstsConfig::new(31536000) // 1 year in seconds
    .include_subdomains(true)
    .preload(true);
```

### Gradual Rollout

Start with a shorter max-age and increase:

```rust
// Week 1: 1 week
let hsts = HstsConfig::new(604800);

// Week 2: 1 month
let hsts = HstsConfig::new(2592000);

// Week 3+: 1 year
let hsts = HstsConfig::new(31536000);
```

## Request Signing

### Basic Setup

```rust
use armature_security::request_signing::{RequestSigner, RequestVerifier};

// Server-side: Verify incoming requests
let verifier = RequestVerifier::new("shared-secret")
    .with_max_age(300); // 5 minutes

// Client-side: Sign outgoing requests
let signer = RequestSigner::new("shared-secret");
```

### Verifying Requests

```rust
match verifier.verify(method, path, body, timestamp, signature) {
    Ok(true) => println!("Valid signature"),
    Ok(false) => println!("Invalid signature"),
    Err(e) => println!("Verification error: {}", e),
}
```

### Middleware

Automatically verify all requests:

```rust
use armature_security::request_signing::RequestSigningMiddleware;

let signing = RequestSigningMiddleware::new("shared-secret")
    .with_max_age(300)
    .skip_path("/health")
    .skip_path("/metrics");

let app = Application::new()
    .middleware(Arc::new(signing))
    .build();
```

## Best Practices

### Security Checklist

- [ ] Use HTTPS in production (required for HSTS)
- [ ] Configure strict CORS (no wildcards in production)
- [ ] Enable HSTS with `includeSubDomains` and `preload`
- [ ] Implement Content Security Policy
- [ ] Use request signing for API authentication
- [ ] Enable all security headers
- [ ] Set up rate limiting
- [ ] Keep secrets secure (use environment variables)
- [ ] Rotate secrets regularly
- [ ] Monitor CSP reports

### Production Security Stack

```rust
use armature_security::*;
use armature_ratelimit::*;

// 1. Security Headers
let security = SecurityMiddleware::default();

// 2. CORS
let cors = CorsConfig::new()
    .allow_origin("https://app.example.com")
    .allow_methods(vec!["GET", "POST", "PUT", "DELETE"])
    .allow_credentials(true);

// 3. Rate Limiting
let rate_limit = RateLimitMiddleware::new(100, 60); // 100 req/min

// 4. Request Signing
let signing = RequestSigningMiddleware::new(std::env::var("API_SECRET")?);

let app = Application::new()
    .middleware(Arc::new(security))
    .middleware(Arc::new(rate_limit))
    .middleware(Arc::new(signing))
    .build();
```

## Summary

### Key Takeaways

1. **CORS**: Use specific origins in production, never wildcards with credentials
2. **CSP**: Start with report-only mode, gradually tighten
3. **HSTS**: Start with short max-age, increase over time
4. **Request Signing**: Use unique secrets, rotate regularly
5. **Defense in Depth**: Combine multiple security layers

### Security Levels

**Basic** (Minimum):
- Security headers (default middleware)
- HTTPS in production
- Basic CORS

**Intermediate**:
- Custom CSP policy
- HSTS with subdomains
- Rate limiting

**Advanced**:
- Request signing
- CSP with nonces
- HSTS preload
- Origin patterns
- Comprehensive monitoring

---

**Security is not optional!** Start with defaults and customize as needed. 🔒

