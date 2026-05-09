# HTTPS/TLS Guide

Complete guide to adding HTTPS/TLS support to your Armature applications.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Certificate Management](#certificate-management)
- [Production Deployment](#production-deployment)
- [HTTP to HTTPS Redirect](#http-to-https-redirect)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

## Overview

Armature provides built-in HTTPS/TLS support using `rustls`, a modern TLS library written in Rust. This enables secure communication between clients and your server.

### Features

- ✅ TLS 1.2 and TLS 1.3 support
- ✅ HTTP/2 and HTTP/1.1 ALPN
- ✅ Certificate loading from PEM files
- ✅ Self-signed certificates for development
- ✅ HTTP to HTTPS automatic redirect
- ✅ Zero-copy TLS with tokio-rustls

## Quick Start

### Development (Self-Signed Certificates)

For local development, you can use automatically generated self-signed certificates:

```rust
use armature_framework::prelude::*;

#[module()]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<()> {
    let app = Application::create::<AppModule>().await;

    // Generate self-signed certificate (development only!)
    let tls_config = TlsConfig::self_signed(&["localhost", "127.0.0.1"])?;

    // Start HTTPS server
    app.listen_https(8443, tls_config).await?;

    Ok(())
}
```

**Build with the `self-signed-certs` feature:**

```bash
cargo run --features self-signed-certs
```

**Test:**

```bash
curl -k https://localhost:8443/
```

> ⚠️ **Warning**: Self-signed certificates should NEVER be used in production!

### Production (Real Certificates)

For production, use certificates from a trusted Certificate Authority:

```rust
use armature_framework::prelude::*;

#[module()]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<()> {
    let app = Application::create::<AppModule>().await;

    // Load real certificates
    let tls_config = TlsConfig::from_pem_files(
        "/etc/ssl/certs/your-cert.pem",
        "/etc/ssl/private/your-key.pem"
    )?;

    // Start HTTPS server
    app.listen_https(443, tls_config).await?;

    Ok(())
}
```

## Certificate Management

### Loading from Files

The most common approach is to load certificates from PEM files:

```rust
use armature_core::TlsConfig;

// Load from file paths
let tls_config = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
```

### Loading from Memory

You can also load certificates from byte arrays:

```rust
use armature_core::TlsConfig;

let cert_pem = include_bytes!("../certs/cert.pem");
let key_pem = include_bytes!("../certs/key.pem");

let tls_config = TlsConfig::from_pem_bytes(cert_pem, key_pem)?;
```

### Certificate Formats

Armature accepts certificates in **PEM format**:

- **Certificate**: `cert.pem` or `fullchain.pem`
- **Private Key**: `key.pem` or `privkey.pem`

**Example PEM Certificate:**

```
-----BEGIN CERTIFICATE-----
MIIDXTCCAkWgAwIBAgIJAKJ...
...
-----END CERTIFICATE-----
```

**Example PEM Private Key:**

```
-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0...
...
-----END PRIVATE KEY-----
```

### Generating Development Certificates

#### Using OpenSSL

```bash
openssl req -x509 -newkey rsa:4096 \
  -keyout key.pem -out cert.pem \
  -days 365 -nodes \
  -subj "/CN=localhost"
```

#### Using mkcert (Recommended for Development)

[mkcert](https://github.com/FiloSottile/mkcert) automatically creates and installs a local CA:

```bash
# Install mkcert
brew install mkcert  # macOS
# or: choco install mkcert  # Windows
# or: apt install mkcert  # Ubuntu

# Install local CA
mkcert -install

# Generate certificate
mkcert localhost 127.0.0.1 ::1
```

This creates `localhost+2.pem` and `localhost+2-key.pem`.

## Production Deployment

### Let's Encrypt (Recommended)

[Let's Encrypt](https://letsencrypt.org/) provides free, automated TLS certificates.

#### Using Certbot

```bash
# Install certbot
sudo apt install certbot  # Ubuntu/Debian

# Get certificate
sudo certbot certonly --standalone -d yourdomain.com

# Certificates will be in:
# /etc/letsencrypt/live/yourdomain.com/fullchain.pem
# /etc/letsencrypt/live/yourdomain.com/privkey.pem
```

#### Using Armature

```rust
use armature_framework::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let app = Application::create::<AppModule>().await;

    let tls_config = TlsConfig::from_pem_files(
        "/etc/letsencrypt/live/yourdomain.com/fullchain.pem",
        "/etc/letsencrypt/live/yourdomain.com/privkey.pem"
    )?;

    app.listen_https(443, tls_config).await?;

    Ok(())
}
```

### Certificate Renewal

Let's Encrypt certificates expire every 90 days. Set up automatic renewal:

```bash
# Test renewal
sudo certbot renew --dry-run

# Add to crontab for automatic renewal
sudo crontab -e
# Add: 0 0 * * * certbot renew --quiet && systemctl restart your-app
```

### File Permissions

Ensure proper permissions for certificate files:

```bash
# Certificate (public) - readable by all
chmod 644 /etc/ssl/certs/your-cert.pem

# Private key - readable only by owner
chmod 600 /etc/ssl/private/your-key.pem
chown root:root /etc/ssl/private/your-key.pem
```

## HTTP to HTTPS Redirect

Automatically redirect HTTP traffic to HTTPS:

```rust
use armature_framework::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let app = Application::create::<AppModule>().await;

    let tls_config = TlsConfig::from_pem_files("cert.pem", "key.pem")?;

    // Configure HTTPS with HTTP redirect
    let https_config = HttpsConfig::new("0.0.0.0:443", tls_config)
        .with_http_redirect("0.0.0.0:80");

    // This starts both:
    // - HTTPS server on port 443
    // - HTTP server on port 80 (redirects to HTTPS)
    app.listen_with_config(https_config).await?;

    Ok(())
}
```

**How it works:**

1. HTTP server listens on port 80
2. All requests receive a `301 Moved Permanently` response
3. `Location` header points to the HTTPS URL
4. Client automatically follows redirect to HTTPS

**Example redirect response:**

```http
HTTP/1.1 301 Moved Permanently
Location: https://example.com/path
```

## Best Practices

### Security

1. **Never Use Self-Signed Certs in Production**
   ```rust
   #[cfg(debug_assertions)]
   let tls = TlsConfig::self_signed(&["localhost"])?;

   #[cfg(not(debug_assertions))]
   let tls = TlsConfig::from_pem_files("cert.pem", "key.pem")?;
   ```

2. **Protect Private Keys**
   - Store in secure locations (e.g., `/etc/ssl/private/`)
   - Use `chmod 600` to restrict access
   - Never commit to version control
   - Consider using secrets management (Vault, AWS Secrets Manager)

3. **Use Strong Certificates**
   - RSA 2048-bit minimum (4096-bit recommended)
   - Or ECDSA P-256 or P-384
   - From trusted Certificate Authorities

4. **Keep Certificates Updated**
   - Monitor expiration dates
   - Automate renewal
   - Test renewal process

### Configuration

1. **Environment-Based Config**
   ```rust
   use std::env;

   let cert_path = env::var("TLS_CERT_PATH")
       .unwrap_or_else(|_| "/etc/ssl/certs/cert.pem".to_string());

   let key_path = env::var("TLS_KEY_PATH")
       .unwrap_or_else(|_| "/etc/ssl/private/key.pem".to_string());

   let tls_config = TlsConfig::from_pem_files(cert_path, key_path)?;
   ```

2. **Graceful Error Handling**
   ```rust
   let tls_config = match TlsConfig::from_pem_files("cert.pem", "key.pem") {
       Ok(config) => config,
       Err(e) => {
           eprintln!("Failed to load TLS certificates: {}", e);
           eprintln!("Make sure cert.pem and key.pem exist and are readable");
           return Err(e);
       }
   };
   ```

3. **Use Standard Ports**
   - HTTPS: port 443
   - HTTP: port 80 (for redirects only)

### Performance

1. **HTTP/2 is Enabled by Default**
   - Armature automatically negotiates HTTP/2 via ALPN
   - Falls back to HTTP/1.1 if needed

2. **TLS Session Resumption**
   - rustls handles session resumption automatically
   - Reduces handshake overhead for repeat connections

3. **Connection Pooling**
   - Use keep-alive connections
   - Let clients reuse TLS sessions

## Troubleshooting

### Certificate Errors

**Problem**: `Failed to create TLS config: invalid certificate`

**Solutions:**
- Verify certificate is in PEM format
- Check certificate is not expired: `openssl x509 -in cert.pem -noout -dates`
- Ensure certificate matches private key:
  ```bash
  openssl x509 -noout -modulus -in cert.pem | openssl md5
  openssl rsa -noout -modulus -in key.pem | openssl md5
  # MD5 hashes should match
  ```

### Permission Errors

**Problem**: `Failed to open key file: Permission denied`

**Solutions:**
```bash
# Check file permissions
ls -l key.pem

# Fix permissions
chmod 600 key.pem

# Or run with appropriate user
sudo -u www-data ./your-app
```

### Port Binding Errors

**Problem**: `Address already in use (os error 98)`

**Solutions:**
```bash
# Check what's using the port
sudo lsof -i :443

# Kill the process
sudo kill <PID>

# Or use a different port for testing
# app.listen_https(8443, tls_config).await?;
```

### Browser Warnings

**Problem**: Browser shows "Your connection is not private"

**Solutions:**
- **Development**: Expected with self-signed certs, click "Advanced" → "Proceed"
- **Production**: Use certificates from a trusted CA (Let's Encrypt)
- **Testing**: Import self-signed cert into browser's trusted certificates

### TLS Handshake Failures

**Problem**: `TLS handshake failed: ...`

**Solutions:**
- Check client supports TLS 1.2/1.3
- Verify certificate chain is complete (use `fullchain.pem`, not just `cert.pem`)
- Test with OpenSSL:
  ```bash
  openssl s_client -connect localhost:443 -servername localhost
  ```

## Examples

### Basic HTTPS Server

```rust
use armature_framework::prelude::*;

#[derive(Default)]
pub struct ApiService;

#[injectable]
impl ApiService {
    pub fn get_data(&self) -> String {
        "Secure data".to_string()
    }
}

pub struct ApiController {
    api_service: std::sync::Arc<ApiService>,
}

#[controller("/api")]
impl ApiController {
    pub fn new(api_service: std::sync::Arc<ApiService>) -> Self {
        Self { api_service }
    }

    #[get("/data")]
    pub async fn get_data(&self, _req: HttpRequest) -> Result<HttpResponse> {
        let data = self.api_service.get_data();
        Ok(HttpResponse::ok().with_json(&serde_json::json!({
            "data": data,
            "secure": true
        }))?)
    }
}

#[module({
    providers: [ApiService],
    controllers: [ApiController],
})]
pub struct AppModule {}

#[tokio::main]
async fn main() -> Result<()> {
    let app = Application::create::<AppModule>().await;

    #[cfg(feature = "self-signed-certs")]
    let tls_config = TlsConfig::self_signed(&["localhost"])?;

    #[cfg(not(feature = "self-signed-certs"))]
    let tls_config = TlsConfig::from_pem_files("cert.pem", "key.pem")?;

    app.listen_https(8443, tls_config).await?;

    Ok(())
}
```

### HTTPS with Environment Config

```rust
use armature_framework::prelude::*;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let app = Application::create::<AppModule>().await;

    // Load config from environment
    let cert_path = env::var("TLS_CERT_PATH")?;
    let key_path = env::var("TLS_KEY_PATH")?;
    let port: u16 = env::var("HTTPS_PORT")
        .unwrap_or_else(|_| "443".to_string())
        .parse()?;

    let tls_config = TlsConfig::from_pem_files(cert_path, key_path)?;

    println!("Starting HTTPS server on port {}", port);
    app.listen_https(port, tls_config).await?;

    Ok(())
}
```

### Full Production Setup

```rust
use armature_framework::prelude::*;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    // Load environment variables
    dotenv::dotenv().ok();

    let app = Application::create::<AppModule>().await;

    let domain = env::var("DOMAIN")?;
    let cert_dir = env::var("CERT_DIR").unwrap_or_else(|_| "/etc/letsencrypt/live".to_string());

    let cert_path = format!("{}/{}/fullchain.pem", cert_dir, domain);
    let key_path = format!("{}/{}/privkey.pem", cert_dir, domain);

    let tls_config = TlsConfig::from_pem_files(cert_path, key_path)?;

    let https_config = HttpsConfig::new("0.0.0.0:443", tls_config)
        .with_http_redirect("0.0.0.0:80");

    println!("🔒 Starting production HTTPS server");
    println!("   Domain: {}", domain);
    println!("   HTTPS: https://{}", domain);
    println!("   HTTP redirect enabled");

    app.listen_with_config(https_config).await?;

    Ok(())
}
```

## Summary

**Key Takeaways:**

1. ✅ Use `TlsConfig::self_signed()` for development
2. ✅ Use `TlsConfig::from_pem_files()` for production
3. ✅ Get free certificates from Let's Encrypt
4. ✅ Enable HTTP to HTTPS redirect with `HttpsConfig`
5. ✅ Protect private keys with proper permissions
6. ✅ Automate certificate renewal
7. ✅ Use environment variables for configuration

**Never:**
- ❌ Use self-signed certificates in production
- ❌ Commit private keys to version control
- ❌ Use weak keys (< 2048 bits)
- ❌ Ignore certificate expiration

HTTPS is essential for modern web applications. With Armature's built-in support, securing your application is straightforward and follows Rust best practices.

