# ACME Certificate Management

Armature's ACME module provides automatic SSL/TLS certificate management using the ACME protocol (Automatic Certificate Management Environment), commonly used with Let's Encrypt.

---

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Certificate Providers](#certificate-providers)
- [Challenge Types](#challenge-types)
- [Configuration](#configuration)
- [Integration with Armature](#integration-with-armature)
- [Automatic Renewal](#automatic-renewal)
- [Production Deployment](#production-deployment)
- [Troubleshooting](#troubleshooting)
- [API Reference](#api-reference)

---

## Overview

ACME (Automatic Certificate Management Environment) is a protocol for automating domain validation and certificate issuance. Armature's ACME module implements this protocol to automatically obtain and renew SSL/TLS certificates from ACME-compliant providers.

### What is ACME?

- **Automated**: No manual CSR generation or email verification
- **Secure**: Domain validation through cryptographic challenges
- **Free**: Most providers (like Let's Encrypt) offer free certificates
- **Short-lived**: Certificates valid for 90 days, encouraging automation

---

## Features

### ✅ Supported Features

- **Multiple Providers** - Let's Encrypt, ZeroSSL, BuyPass, Google Trust Services
- **Challenge Types** - HTTP-01, DNS-01, TLS-ALPN-01
- **Account Management** - Register and manage ACME accounts
- **External Account Binding** - Support for providers requiring EAB
- **Automatic Renewal** - Check and renew before expiration
- **Wildcard Certificates** - Using DNS-01 challenges
- **Multi-domain Certificates** - Single cert for multiple domains (SAN)

---

## Quick Start

### 1. Add Dependency

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["acme"] }
tokio = { version = "1.35", features = ["full"] }
```

### 2. Basic Usage

```rust
use armature_acme::{AcmeClient, AcmeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure for Let's Encrypt staging (testing)
    let config = AcmeConfig::lets_encrypt_staging(
        vec!["admin@example.com".to_string()],
        vec!["example.com".to_string()],
    ).with_accept_tos(true);

    // Create client and order certificate
    let mut client = AcmeClient::new(config).await?;
    let (cert_pem, key_pem) = client.order_certificate().await?;

    // Save certificate and key
    client.save_certificate(&cert_pem, &key_pem).await?;

    Ok(())
}
```

---

## Certificate Providers

### Let's Encrypt (Recommended)

**Production:**

```rust
let config = AcmeConfig::lets_encrypt_production(
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
);
```

**Staging (Testing):**

```rust
let config = AcmeConfig::lets_encrypt_staging(
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
);
```

**Features:**
- ✅ Free
- ✅ Widely trusted
- ✅ 90-day certificates
- ✅ Rate limits: 50 certificates/domain/week

### ZeroSSL

```rust
let config = AcmeConfig::zerossl(
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
    "your_eab_kid".to_string(),
    "your_eab_hmac_key".to_string(),
);
```

**Features:**
- ✅ Free (with account)
- ✅ 90-day certificates
- ⚠️ Requires External Account Binding (EAB)

### BuyPass

```rust
let config = AcmeConfig::new(
    "https://api.buypass.com/acme/directory",
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
);
```

### Google Trust Services

```rust
let config = AcmeConfig::new(
    "https://dv.acme-v02.api.pki.goog/directory",
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
);
```

---

## Challenge Types

ACME uses challenges to prove you control the domain before issuing a certificate.

### HTTP-01 Challenge (Recommended for single domains)

**How it works:**
1. ACME server provides a token
2. Your server serves the token at `http://yourdomain.com/.well-known/acme-challenge/{token}`
3. ACME server verifies the response

**Configuration:**

```rust
let config = AcmeConfig::lets_encrypt_production(...)
    .with_challenge_type(ChallengeType::Http01);
```

**Requirements:**
- Port 80 must be accessible
- HTTP server running
- Cannot be used for wildcard certificates

**Example Integration:**

```rust
use armature_framework::prelude::*;
use armature_acme::*;

#[controller("/.well-known/acme-challenge")]
struct AcmeController {
    challenges: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
}

impl AcmeController {
    #[get("/:token")]
    fn serve_challenge(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        let token = req.param("token")?;
        let challenges = self.challenges.lock().unwrap();

        if let Some(key_auth) = challenges.get(token) {
            Ok(HttpResponse::ok()
                .with_header("Content-Type", "text/plain")
                .with_body(key_auth.as_bytes().to_vec()))
        } else {
            Err(Error::NotFound)
        }
    }
}
```

### DNS-01 Challenge (Required for wildcards)

**How it works:**
1. ACME server provides a token
2. You create a TXT record: `_acme-challenge.yourdomain.com`
3. ACME server queries DNS to verify

**Configuration:**

```rust
let config = AcmeConfig::lets_encrypt_production(...)
    .with_challenge_type(ChallengeType::Dns01);
```

**Requirements:**
- DNS provider API access
- Ability to create TXT records programmatically

**Wildcard Certificate Example:**

```rust
let config = AcmeConfig::lets_encrypt_production(
    vec!["admin@example.com".to_string()],
    vec!["*.example.com".to_string(), "example.com".to_string()],
)
.with_challenge_type(ChallengeType::Dns01)
.with_accept_tos(true);
```

### TLS-ALPN-01 Challenge

**How it works:**
1. ACME server initiates TLS connection on port 443
2. Your server presents special certificate with ACME validation data
3. ACME server validates the certificate

**Configuration:**

```rust
let config = AcmeConfig::lets_encrypt_production(...)
    .with_challenge_type(ChallengeType::TlsAlpn01);
```

**Requirements:**
- Port 443 must be accessible
- TLS server with ALPN support

---

## Configuration

### Basic Configuration

```rust
use armature_acme::{AcmeConfig, ChallengeType};
use std::path::PathBuf;

let config = AcmeConfig::new(
    "https://acme-v02.api.letsencrypt.org/directory",
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
);
```

### Builder Pattern

```rust
let config = AcmeConfig::lets_encrypt_production(
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string(), "www.example.com".to_string()],
)
.with_challenge_type(ChallengeType::Http01)
.with_cert_dir(PathBuf::from("/etc/letsencrypt/live/example.com"))
.with_account_dir(PathBuf::from("/etc/letsencrypt/accounts"))
.with_renew_before_days(30)
.with_accept_tos(true);
```

### Configuration Options

| Option | Description | Default |
|--------|-------------|---------|
| `directory_url` | ACME directory URL | - |
| `contact_email` | Contact emails | - |
| `domains` | Domains to certify | - |
| `challenge_type` | Challenge type | HTTP-01 |
| `cert_dir` | Certificate storage | `./certs` |
| `account_dir` | Account credentials | `./accounts` |
| `renew_before_days` | Renewal threshold | 30 days |
| `accept_tos` | Accept ToS | `false` |

---

## Integration with Armature

### Complete HTTPS Server with ACME

```rust
use armature_framework::prelude::*;
use armature_acme::{AcmeClient, AcmeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Obtain certificate
    let acme_config = AcmeConfig::lets_encrypt_production(
        vec!["admin@example.com".to_string()],
        vec!["example.com".to_string()],
    ).with_accept_tos(true);

    let mut acme_client = AcmeClient::new(acme_config).await?;
    let (cert_pem, key_pem) = acme_client.order_certificate().await?;
    let (cert_path, key_path) = acme_client.save_certificate(&cert_pem, &key_pem).await?;

    // 2. Configure TLS
    let tls_config = TlsConfig::from_pem_files(&cert_path, &key_path)?;

    // 3. Start HTTPS server
    let app = Application::create::<AppModule>().await;
    app.listen_https(443, tls_config).await?;

    Ok(())
}
```

### With HTTP to HTTPS Redirect

```rust
use armature_core::{Application, HttpsConfig, TlsConfig};

let https_config = HttpsConfig::new("0.0.0.0:443", tls_config)
    .with_http_redirect("0.0.0.0:80");

app.listen_with_config(https_config).await?;
```

---

## Automatic Renewal

Certificates from Let's Encrypt expire after 90 days. Armature's ACME client can check and renew certificates automatically.

### Check if Renewal Needed

```rust
if acme_client.should_renew("certs/cert.pem").await? {
    let (cert_pem, key_pem) = acme_client.order_certificate().await?;
    acme_client.save_certificate(&cert_pem, &key_pem).await?;
    println!("Certificate renewed!");
}
```

### Background Renewal Task

```rust
use tokio::time::{interval, Duration};

tokio::spawn(async move {
    let mut ticker = interval(Duration::from_secs(86400)); // Check daily

    loop {
        ticker.tick().await;

        if acme_client.should_renew("certs/cert.pem").await.unwrap_or(false) {
            match acme_client.order_certificate().await {
                Ok((cert_pem, key_pem)) => {
                    acme_client.save_certificate(&cert_pem, &key_pem).await.ok();
                    // TODO: Reload TLS configuration
                }
                Err(e) => eprintln!("Renewal failed: {}", e),
            }
        }
    }
});
```

---

## Production Deployment

### 1. Test with Staging First

Always test with Let's Encrypt staging to avoid rate limits:

```rust
let config = AcmeConfig::lets_encrypt_staging(...);
```

### 2. Production Configuration

```rust
let config = AcmeConfig::lets_encrypt_production(
    vec!["admin@yourdomain.com".to_string()],
    vec!["yourdomain.com".to_string(), "www.yourdomain.com".to_string()],
)
.with_cert_dir(PathBuf::from("/etc/letsencrypt/live/yourdomain.com"))
.with_renew_before_days(30)
.with_accept_tos(true);
```

### 3. Certificate Storage

Store certificates securely:

```
/etc/letsencrypt/
├── accounts/          # Account credentials
│   └── acme-v02.api.letsencrypt.org/
└── live/             # Certificates
    └── yourdomain.com/
        ├── cert.pem
        ├── key.pem
        └── fullchain.pem
```

Set appropriate permissions:

```bash
chmod 700 /etc/letsencrypt
chmod 600 /etc/letsencrypt/live/yourdomain.com/key.pem
```

### 4. Systemd Timer for Renewal

Create `/etc/systemd/system/cert-renewal.service`:

```ini
[Unit]
Description=Renew ACME Certificates

[Service]
Type=oneshot
ExecStart=/usr/local/bin/your-renewal-script
```

Create `/etc/systemd/system/cert-renewal.timer`:

```ini
[Unit]
Description=Daily certificate renewal check

[Timer]
OnCalendar=daily
Persistent=true

[Install]
WantedBy=timers.target
```

Enable:

```bash
systemctl enable cert-renewal.timer
systemctl start cert-renewal.timer
```

---

## Troubleshooting

### Rate Limits

**Problem:** `Rate limit exceeded`

**Solution:**
- Use staging environment for testing
- Let's Encrypt production limits:
  - 50 certificates per domain per week
  - 5 failed validations per hour
- Wait or use a different domain for testing

### Challenge Validation Failed

**Problem:** `Challenge failed: Connection refused`

**HTTP-01:**
- Ensure port 80 is accessible from internet
- Check firewall rules
- Verify DNS points to your server
- Test: `curl http://yourdomain.com/.well-known/acme-challenge/test`

**DNS-01:**
- Verify DNS record propagation: `dig TXT _acme-challenge.yourdomain.com`
- Wait for DNS propagation (can take minutes to hours)
- Check DNS provider API credentials

### Certificate Not Trusted

**Problem:** Browser shows "Certificate not trusted"

**Cause:** Using Let's Encrypt staging certificates

**Solution:** Switch to production:

```rust
let config = AcmeConfig::lets_encrypt_production(...);
```

### Renewal Failures

**Problem:** Automatic renewal failing

**Solutions:**
- Check logs for specific errors
- Verify challenge setup still works
- Ensure sufficient disk space
- Check account credentials haven't expired

---

## API Reference

### `AcmeConfig`

Configuration for ACME client.

**Methods:**
- `new(directory_url, contact_email, domains)` - Create config
- `lets_encrypt_production(email, domains)` - Let's Encrypt production
- `lets_encrypt_staging(email, domains)` - Let's Encrypt staging
- `zerossl(email, domains, eab_kid, eab_hmac_key)` - ZeroSSL with EAB
- `with_challenge_type(type)` - Set challenge type
- `with_cert_dir(path)` - Set certificate directory
- `with_account_dir(path)` - Set account directory
- `with_renew_before_days(days)` - Set renewal threshold
- `with_accept_tos(accept)` - Accept terms of service

### `AcmeClient`

ACME protocol client.

**Methods:**
- `new(config)` - Create client
- `register_account()` - Register ACME account
- `create_order()` - Create certificate order
- `get_challenges(order_url)` - Get validation challenges
- `notify_challenge_ready(challenge_url)` - Notify challenge ready
- `finalize_order(order_url)` - Finalize and get certificate
- `order_certificate()` - Complete ordering process
- `should_renew(cert_path)` - Check if renewal needed
- `save_certificate(cert_pem, key_pem)` - Save to files

### `ChallengeType`

Challenge type enumeration.

**Variants:**
- `Http01` - HTTP-01 challenge (port 80)
- `Dns01` - DNS-01 challenge (DNS TXT record)
- `TlsAlpn01` - TLS-ALPN-01 challenge (port 443)

---

## Summary

Armature's ACME module provides:

✅ **Automatic certificate management** with ACME protocol
✅ **Multiple providers** (Let's Encrypt, ZeroSSL, etc.)
✅ **All challenge types** (HTTP-01, DNS-01, TLS-ALPN-01)
✅ **Account management** and External Account Binding
✅ **Automatic renewal** before expiration
✅ **Seamless integration** with Armature HTTPS servers
✅ **Production-ready** with error handling and retry logic

For more examples, see:
- `examples/acme_certificate.rs`
- `examples/https_server.rs`


