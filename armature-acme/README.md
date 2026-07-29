# armature-acme

RFC 8555 ACME client for automatic TLS certificate issuance and renewal
(Let's Encrypt, ZeroSSL, BuyPass, Google Trust Services, or any ACME CA).

## Features

- **RFC 8555 flow** — account registration, ordering, challenge validation,
  finalization, and certificate download
- **ES256 account keys** — ECDSA P-256 (`ring`), persisted as PKCS#8 and reused
- **Challenges** — HTTP-01 (default) and DNS-01 challenge data; TLS-ALPN-01
  selection (serving is the caller's responsibility)
- **External Account Binding (EAB)** — HS256 binding for CAs that require it
- **Renewal** — `should_renew` parses the leaf certificate's `notAfter`
- **Private / test CAs** — trust a custom root, or (test only) accept invalid
  certs for local CAs such as Pebble

## Installation

```toml
[dependencies]
armature-acme = "0.1"
```

## Quick start

```rust,no_run
use armature_acme::{AcmeClient, AcmeConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Contacts and domains are `Vec<String>`.
    let config = AcmeConfig::lets_encrypt_staging(
        vec!["admin@example.com".to_string()],
        vec!["example.com".to_string(), "www.example.com".to_string()],
    )
    .with_accept_tos(true);

    let mut client = AcmeClient::new(config).await?;

    // Obtain a certificate. For HTTP-01 you must be serving each challenge's
    // key authorization before this call reaches validation (see below).
    let (cert_pem, key_pem) = client.order_certificate().await?;

    // `save_certificate` refuses to write empty / non-PEM input.
    let (cert_path, key_path) = client.save_certificate(&cert_pem, &key_pem).await?;
    println!("wrote {cert_path} and {key_path}");
    Ok(())
}
```

## Challenge validation

`order_certificate` drives the whole flow but does **not** run a challenge web
server. Validation must be satisfied out of band:

- **HTTP-01** (default): serve each challenge's `key_authorization` at
  `/.well-known/acme-challenge/<token>` before the notify step.
- **DNS-01**: publish a `_acme-challenge.<domain>` TXT record with
  `armature_acme::dns01_txt_value(&key_authorization)`.

To interleave serving with the protocol, drive the steps manually:

```rust,no_run
use armature_acme::{AcmeClient, AcmeConfig};

# async fn example(mut client: AcmeClient) -> Result<(), Box<dyn std::error::Error>> {
client.register_account().await?;
let order_url = client.create_order().await?;

let challenges = client.get_challenges(&order_url).await?;
for ch in &challenges {
    // Serve `ch.key_authorization` at `ch.path()` (HTTP-01), then:
    client.notify_challenge_ready(&ch.url).await?;
}

let (cert_pem, key_pem) = client.finalize_order(&order_url).await?;
# let _ = (cert_pem, key_pem);
# Ok(())
# }
```

## Configuration

```rust
use armature_acme::{AcmeConfig, ChallengeType};
use std::path::PathBuf;

let config = AcmeConfig::lets_encrypt_production(
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
)
.with_accept_tos(true)
.with_challenge_type(ChallengeType::Http01)
.with_cert_dir(PathBuf::from("/etc/ssl/armature"))   // where cert.pem / key.pem are written
.with_account_dir(PathBuf::from("/var/lib/armature/acme")) // account_key.pem persistence
.with_renew_before_days(30);
```

### ZeroSSL / EAB

```rust
use armature_acme::AcmeConfig;

let config = AcmeConfig::zerossl(
    vec!["admin@example.com".to_string()],
    vec!["example.com".to_string()],
    "your_eab_kid".to_string(),
    "your_eab_hmac_key".to_string(), // base64url-encoded MAC key
);
```

### Private or test CAs

```rust
use armature_acme::AcmeConfig;

// Trust a private CA's root for the ACME endpoint.
let root_pem: Vec<u8> = std::fs::read("private-ca-root.pem").unwrap();
let config = AcmeConfig::new(
    "https://acme.internal/directory",
    vec!["admin@example.com".to_string()],
    vec!["service.internal".to_string()],
)
.with_accept_tos(true)
.with_ca_certificate(root_pem);
```

TLS verification is always enforced. To talk to a private or test CA (Pebble,
Boulder, an internal ACME server), fetch its root certificate and pass it to
`with_ca_certificate` — there is deliberately no option to disable certificate
validation.

## Renewal

```rust,no_run
# use armature_acme::AcmeClient;
# async fn example(mut client: AcmeClient) -> Result<(), Box<dyn std::error::Error>> {
if client.should_renew("/etc/ssl/armature/cert.pem").await? {
    let (cert_pem, key_pem) = client.order_certificate().await?;
    client.save_certificate(&cert_pem, &key_pem).await?;
}
# Ok(())
# }
```

`should_renew` returns `true` when the leaf certificate expires within
`renew_before_days`, or when the file is missing.

## License

MIT OR Apache-2.0
