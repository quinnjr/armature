# armature-jwt

JWT authentication and authorization for the Armature framework.

## Features

- **Token Generation** - Create signed JWTs with custom claims
- **Token Verification** - Validate signatures and expiration
- **Multiple Algorithms** - HS256, HS384, HS512, RS256, RS384, RS512, ES256, ES384
- **Refresh Tokens** - Built-in token refresh flow
- **Custom Claims** - Extend with your own claim types

## Installation

```toml
[dependencies]
armature-jwt = "0.1"
```

## Quick Start

```rust
use armature_jwt::{JwtManager, JwtConfig, StandardClaims};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create JWT manager
    let config = JwtConfig::new("your-secret-key".to_string())
        .with_expiration(Duration::from_secs(3600));
    let manager = JwtManager::new(config)?;

    // Create claims
    let claims = StandardClaims::new()
        .with_subject("user123".to_string());
    let token = manager.sign(&claims)?;

    // Verify a token
    let verified: StandardClaims = manager.verify(&token)?;
    println!("User: {}", verified.sub.unwrap());

    Ok(())
}
```

## Token Refresh

```rust
use armature_jwt::{JwtManager, JwtConfig, StandardClaims};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let config = JwtConfig::new("secret".to_string());
# let manager = JwtManager::new(config)?;
# let claims = StandardClaims::new();
// Generate token pair (access + refresh)
let pair = manager.generate_token_pair(&claims)?;

// Refresh the access token using a refresh token
let new_pair = manager.refresh_token::<StandardClaims>(&pair.refresh_token)?;
# Ok(())
# }
```

## License

MIT OR Apache-2.0

