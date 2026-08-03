//! Configuring the ACME client, plus a printed walkthrough of the rest.
//!
//! This runs the parts of the flow that need no network and no real domain:
//! building an [`AcmeConfig`] against the Let's Encrypt *staging* directory,
//! constructing an [`AcmeClient`], and setting up an HTTP-01 challenge
//! responder. It stops there.
//!
//! It does **not** obtain a certificate. Ordering one requires a publicly
//! resolvable domain whose HTTP-01 challenge path Let's Encrypt can reach, so
//! everything past challenge setup - `order_certificate`, `save_certificate`,
//! renewal, wiring the result into `listen_https` - is *printed* as the code
//! you would write, not executed. Nothing here writes to `./certs`.
//!
//! ```bash
//! cargo run --example acme_certificate --features acme
//! ```

#![allow(dead_code)]
use armature_acme::{AcmeClient, AcmeConfig, ChallengeType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔐 Armature ACME Certificate Management Example\n");

    // 1. Configuration with Let's Encrypt Staging (for testing)
    println!("📋 Configuring ACME client...");
    let config = AcmeConfig::lets_encrypt_staging(
        vec!["admin@example.com".to_string()],
        vec!["example.com".to_string(), "www.example.com".to_string()],
    )
    .with_challenge_type(ChallengeType::Http01)
    .with_cert_dir("./certs".into())
    .with_renew_before_days(30)
    .with_accept_tos(true);

    println!("   Directory: {}", config.directory_url);
    println!("   Domains: {:?}", config.domains);
    println!("   Challenge: {:?}", config.challenge_type);
    println!("   Certificate directory: {}", config.cert_dir.display());
    println!();

    // 2. Create ACME client
    println!("🌐 Creating ACME client...");
    let mut client = AcmeClient::new(config).await?;
    println!("   ✓ Client created");
    println!("   ✓ Directory fetched");
    println!();

    // 3. Register account
    println!("👤 Registering ACME account...");
    client.register_account().await?;
    println!("   ✓ Account registered");
    println!();

    // 4. Create order
    println!("📦 Creating certificate order...");
    let order_url = client.create_order().await?;
    println!("   ✓ Order created");
    println!("   Order URL: {}", order_url);
    println!();

    // 5. Get challenges
    println!("🔍 Fetching challenges...");
    let challenges = client.get_challenges(&order_url).await?;
    println!(
        "   ✓ Challenges retrieved: {} challenge(s)",
        challenges.len()
    );
    println!();

    // 6. Display challenge setup instructions
    if !challenges.is_empty() {
        println!("⚠️  Challenge Setup Required:");
        println!();
        for (i, challenge) in challenges.iter().enumerate() {
            println!("   Challenge #{}", i + 1);
            println!("   Path: {}", challenge.path());
            println!("   Content: {}", challenge.content());
            println!();
            println!("   Setup HTTP server to serve the challenge:");
            println!("   GET {}", challenge.path());
            println!("   Response: {}", challenge.content());
            println!();
        }

        println!("   Once HTTP server is ready, the challenge can be validated.");
        println!();
    }

    // 7. In a real scenario, you would:
    //    - Set up HTTP server for HTTP-01 challenges
    //    - Add DNS records for DNS-01 challenges
    //    - Configure TLS for TLS-ALPN-01 challenges
    //    - Then notify ACME server and wait for validation

    println!("📝 Summary:");
    println!();
    println!("   This example demonstrates the ACME certificate ordering process:");
    println!();
    println!("   1. ✓ Configure ACME client (Let's Encrypt Staging)");
    println!("   2. ✓ Fetch ACME directory endpoints");
    println!("   3. ✓ Register account");
    println!("   4. ✓ Create certificate order");
    println!("   5. ✓ Retrieve challenges");
    println!();
    println!("   Next steps (not shown in this example):");
    println!();
    println!("   6. Set up challenge validation (HTTP-01, DNS-01, or TLS-ALPN-01)");
    println!("   7. Notify ACME server that challenge is ready");
    println!("   8. Wait for validation");
    println!("   9. Finalize order with CSR");
    println!("   10. Download signed certificate");
    println!("   11. Save certificate and private key");
    println!("   12. Configure HTTPS server with certificate");
    println!();

    // Production usage example
    println!("🚀 Production Usage:");
    println!();
    println!("   For production, use Let's Encrypt production:");
    println!();
    println!("   let config = AcmeConfig::lets_encrypt_production(");
    println!("       vec![\"admin@yourdomain.com\".to_string()],");
    println!("       vec![\"yourdomain.com\".to_string()],");
    println!("   ).with_accept_tos(true);");
    println!();
    println!("   // Complete flow");
    println!("   let mut client = AcmeClient::new(config).await?;");
    println!("   let (cert_pem, key_pem) = client.order_certificate().await?;");
    println!("   let (cert_path, key_path) = client.save_certificate(&cert_pem, &key_pem).await?;");
    println!();

    // Integration with Armature
    println!("🔗 Integration with Armature:");
    println!();
    println!("   use armature::prelude::*;");
    println!("   use armature_acme::{{AcmeClient, AcmeConfig}};");
    println!();
    println!("   // Obtain certificate");
    println!("   let config = AcmeConfig::lets_encrypt_production(...);");
    println!("   let mut client = AcmeClient::new(config).await?;");
    println!("   let (cert_pem, key_pem) = client.order_certificate().await?;");
    println!("   client.save_certificate(&cert_pem, &key_pem).await?;");
    println!();
    println!("   // Use with Armature HTTPS server");
    println!("   let tls = TlsConfig::from_pem_files(\"certs/cert.pem\", \"certs/key.pem\")?;");
    println!("   app.listen_https(443, tls).await?;");
    println!();

    // Certificate renewal
    println!("🔄 Automatic Renewal:");
    println!();
    println!("   // Check if certificate needs renewal");
    println!("   if client.should_renew(\"certs/cert.pem\").await? {{");
    println!("       let (cert_pem, key_pem) = client.order_certificate().await?;");
    println!("       client.save_certificate(&cert_pem, &key_pem).await?;");
    println!("       println!(\"Certificate renewed!\");");
    println!("   }}");
    println!();

    println!("✅ Example complete!");

    Ok(())
}
