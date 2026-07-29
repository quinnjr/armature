//! ACME client implementation (RFC 8555).

use crate::{
    challenge::*,
    config::*,
    directory::*,
    error::*,
    jws::{self, AccountKey, KeyRef, Payload},
    order::{Order, OrderStatus},
};
use serde_json::{Value, json};
use std::path::Path;
use tokio::sync::Mutex;

/// Maximum time to poll a challenge/order for a terminal state.
const POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Delay between polls.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Captured pieces of an ACME HTTP response we care about.
struct AcmeResponse {
    location: Option<String>,
    body: Vec<u8>,
}

impl AcmeResponse {
    fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| {
            AcmeError::Internal(format!(
                "failed to parse ACME response as {}: {e}; body: {}",
                std::any::type_name::<T>(),
                String::from_utf8_lossy(&self.body)
            ))
        })
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

/// ACME client for certificate management.
///
/// This client implements the RFC 8555 flow for obtaining and renewing
/// SSL/TLS certificates from providers like Let's Encrypt.
///
/// # Challenge validation
///
/// The client drives account registration, ordering, challenge notification,
/// finalization and certificate download. For **HTTP-01** (the default) the
/// caller is responsible for serving each challenge's `key_authorization` at
/// its `.well-known/acme-challenge/<token>` path *before* calling
/// [`AcmeClient::notify_challenge_ready`] / [`AcmeClient::order_certificate`];
/// for **DNS-01** the caller must publish the TXT record
/// ([`jws::dns01_txt_value`]). The client does not run a challenge web server.
///
/// # Example
///
/// ```no_run
/// use armature_acme::{AcmeClient, AcmeConfig};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = AcmeConfig::lets_encrypt_staging(
///     vec!["admin@example.com".to_string()],
///     vec!["example.com".to_string()],
/// ).with_accept_tos(true);
///
/// let client = AcmeClient::new(config).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct AcmeClient {
    config: AcmeConfig,
    directory: Directory,
    http_client: reqwest::Client,
    account_key: Option<AccountKey>,
    account_url: Option<String>,
    /// The most recently observed replay nonce (interior-mutable so that
    /// `&self` request methods can consume and refresh it).
    nonce: Mutex<Option<String>>,
}

impl AcmeClient {
    /// Create a new ACME client and fetch the ACME directory.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use armature_acme::{AcmeClient, AcmeConfig};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = AcmeConfig::lets_encrypt_staging(
    ///     vec!["admin@example.com".to_string()],
    ///     vec!["example.com".to_string()],
    /// );
    ///
    /// let client = AcmeClient::new(config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(config: AcmeConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .user_agent(concat!("armature-acme/", env!("CARGO_PKG_VERSION")));

        if let Some(pem) = &config.ca_certificate {
            let cert = reqwest::Certificate::from_pem(pem)
                .map_err(|e| AcmeError::Internal(format!("invalid CA certificate: {e}")))?;
            builder = builder.add_root_certificate(cert);
        }
        let http_client = builder
            .build()
            .map_err(|e| AcmeError::Internal(e.to_string()))?;

        let directory = Self::fetch_directory(&http_client, &config.directory_url).await?;

        Ok(Self {
            config,
            directory,
            http_client,
            account_key: None,
            account_url: None,
            nonce: Mutex::new(None),
        })
    }

    /// Fetch the ACME directory.
    async fn fetch_directory(client: &reqwest::Client, url: &str) -> Result<Directory> {
        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(AcmeError::InvalidDirectory(format!(
                "Failed to fetch directory: {}",
                response.status()
            )));
        }

        let directory = response.json().await?;
        Ok(directory)
    }

    // ----- nonce management -------------------------------------------------

    fn nonce_from(response: &reqwest::Response) -> Option<String> {
        response
            .headers()
            .get("replay-nonce")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    /// Return a fresh nonce, reusing a stored one or fetching from `newNonce`.
    async fn take_nonce(&self) -> Result<String> {
        if let Some(n) = self.nonce.lock().await.take() {
            return Ok(n);
        }
        let response = self
            .http_client
            .head(&self.directory.new_nonce)
            .send()
            .await?;
        Self::nonce_from(&response)
            .ok_or_else(|| AcmeError::Internal("newNonce returned no Replay-Nonce header".into()))
    }

    // ----- signed requests --------------------------------------------------

    /// POST a signed JWS request, managing nonces and retrying once on badNonce.
    async fn post_signed(
        &self,
        url: &str,
        key_ref: KeyRef<'_>,
        payload: Payload<'_>,
    ) -> Result<AcmeResponse> {
        let key = self
            .account_key
            .as_ref()
            .ok_or_else(|| AcmeError::InvalidAccount("account key not initialized".into()))?;

        // Retry budget for the anti-replay nonce. A CA may reject a nonce
        // (Pebble deliberately rejects a fraction of good nonces); each retry
        // uses the fresh nonce captured from the rejecting response.
        const MAX_BADNONCE_RETRIES: u32 = 5;
        for attempt in 0..=MAX_BADNONCE_RETRIES {
            let nonce = self.take_nonce().await?;
            let body = jws::sign_jws(key, url, &nonce, key_ref, payload)?;

            let response = self
                .http_client
                .post(url)
                .header("content-type", "application/jose+json")
                .json(&body)
                .send()
                .await?;

            // Always capture the fresh nonce, even on error responses.
            let new_nonce = Self::nonce_from(&response);
            let status = response.status();
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = response.bytes().await?.to_vec();

            if let Some(n) = new_nonce {
                *self.nonce.lock().await = Some(n);
            }

            if status.is_success() {
                return Ok(AcmeResponse {
                    location,
                    body: bytes,
                });
            }

            // Parse the RFC 7807 problem document.
            let problem: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let problem_type = problem
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let detail = problem
                .get("detail")
                .and_then(|v| v.as_str())
                .unwrap_or("(no detail)")
                .to_string();

            if problem_type.ends_with(":badNonce") && attempt < MAX_BADNONCE_RETRIES {
                // Retry with the refreshed nonce captured above.
                continue;
            }

            return Err(Self::problem_error(problem_type, detail, status));
        }
        Err(AcmeError::Internal(
            "signed request failed after nonce retry".into(),
        ))
    }

    fn problem_error(problem_type: &str, detail: String, status: reqwest::StatusCode) -> AcmeError {
        if problem_type.ends_with(":rateLimited") {
            AcmeError::RateLimitExceeded(detail)
        } else if problem_type.ends_with(":accountDoesNotExist")
            || problem_type.ends_with(":unauthorized")
        {
            AcmeError::InvalidAccount(format!("{detail} ({problem_type})"))
        } else {
            AcmeError::OrderFailed(format!(
                "ACME error {status}: {detail} ({})",
                if problem_type.is_empty() {
                    "unknown"
                } else {
                    problem_type
                }
            ))
        }
    }

    /// POST-as-GET a resource and deserialize its JSON body.
    async fn post_as_get<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let kid = self.kid()?;
        let resp = self
            .post_signed(url, KeyRef::Kid(&kid), Payload::Empty)
            .await?;
        resp.json()
    }

    /// The account URL used as the JWS `kid`.
    fn kid(&self) -> Result<String> {
        self.account_url
            .clone()
            .ok_or_else(|| AcmeError::InvalidAccount("account not registered".into()))
    }

    // ----- account persistence ---------------------------------------------

    fn account_key_path(&self) -> std::path::PathBuf {
        self.config.account_dir.join("account_key.pem")
    }

    fn account_url_path(&self) -> std::path::PathBuf {
        self.config.account_dir.join("account_url.txt")
    }

    fn load_or_generate_key(&self) -> Result<AccountKey> {
        let path = self.account_key_path();
        if path.exists() {
            let pem = std::fs::read_to_string(&path)?;
            let der = jws::pem_to_pkcs8(&pem)?;
            return AccountKey::from_pkcs8(der);
        }
        let key = AccountKey::generate()?;
        std::fs::create_dir_all(&self.config.account_dir)?;
        set_dir_perms(&self.config.account_dir);
        write_secret_file(&path, jws::pkcs8_to_pem(key.pkcs8_der()).as_bytes())?;
        Ok(key)
    }

    // ----- protocol flow ----------------------------------------------------

    /// Register a new ACME account, or reuse the persisted key/account.
    ///
    /// The account key is loaded from `account_dir/account_key.pem` if present,
    /// otherwise generated and persisted. Re-registering with the same key is
    /// idempotent — the CA returns the existing account URL.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use armature_acme::{AcmeClient, AcmeConfig};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = AcmeConfig::lets_encrypt_staging(
    /// #     vec!["admin@example.com".to_string()],
    /// #     vec!["example.com".to_string()],
    /// # ).with_accept_tos(true);
    /// # let mut client = AcmeClient::new(config).await?;
    /// client.register_account().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_account(&mut self) -> Result<()> {
        // Fail closed: if the CA advertises Terms of Service and the caller has
        // not agreed, do not attempt registration.
        let tos_advertised = self
            .directory
            .meta
            .as_ref()
            .and_then(|m| m.terms_of_service.as_ref())
            .is_some();
        if !self.config.accept_tos && tos_advertised {
            return Err(AcmeError::InvalidAccount(
                "the CA requires agreeing to its Terms of Service; \
                 set AcmeConfig::with_accept_tos(true)"
                    .into(),
            ));
        }

        let key = self.load_or_generate_key()?;
        self.account_key = Some(key);

        // Build newAccount payload.
        let contact: Vec<String> = self
            .config
            .contact_email
            .iter()
            .filter(|e| !e.trim().is_empty())
            .map(|e| {
                if e.starts_with("mailto:") {
                    e.clone()
                } else {
                    format!("mailto:{e}")
                }
            })
            .collect();

        let mut payload = json!({ "termsOfServiceAgreed": self.config.accept_tos });
        if !contact.is_empty() {
            payload["contact"] = json!(contact);
        }

        // External Account Binding, if configured.
        if let (Some(kid), Some(hmac)) = (&self.config.eab_kid, &self.config.eab_hmac_key) {
            let jwk = self.account_key.as_ref().expect("key just set").jwk();
            let eab = jws::eab_jws(kid, hmac, &self.directory.new_account, &jwk)?;
            payload["externalAccountBinding"] = eab;
        }

        tracing::info!("Registering ACME account");
        let url = self.directory.new_account.clone();
        let resp = self
            .post_signed(&url, KeyRef::Jwk, Payload::Json(&payload))
            .await?;

        let account_url = resp.location.ok_or_else(|| {
            AcmeError::InvalidAccount("newAccount response had no Location header".into())
        })?;
        self.account_url = Some(account_url.clone());

        // Persist the account URL (best effort).
        if std::fs::create_dir_all(&self.config.account_dir).is_ok() {
            let _ = std::fs::write(self.account_url_path(), &account_url);
        }

        Ok(())
    }

    /// Create a new certificate order for the configured domains.
    ///
    /// Returns the order URL.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use armature_acme::{AcmeClient, AcmeConfig};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = AcmeConfig::lets_encrypt_staging(
    /// #     vec!["admin@example.com".to_string()],
    /// #     vec!["example.com".to_string()],
    /// # ).with_accept_tos(true);
    /// # let mut client = AcmeClient::new(config).await?;
    /// # client.register_account().await?;
    /// let order_url = client.create_order().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_order(&self) -> Result<String> {
        let kid = self.kid()?;

        if self.config.domains.is_empty() {
            return Err(AcmeError::OrderFailed("no domains configured".into()));
        }

        tracing::info!(
            "Creating certificate order for domains: {:?}",
            self.config.domains
        );

        let identifiers: Vec<Value> = self
            .config
            .domains
            .iter()
            .map(|d| json!({ "type": "dns", "value": d }))
            .collect();
        let payload = json!({ "identifiers": identifiers });

        let url = self.directory.new_order.clone();
        let resp = self
            .post_signed(&url, KeyRef::Kid(&kid), Payload::Json(&payload))
            .await?;
        resp.location.ok_or_else(|| {
            AcmeError::OrderFailed("newOrder response had no Location header".into())
        })
    }

    /// Fetch the challenges for an order, selecting the configured type.
    ///
    /// Returns one [`Http01Challenge`] per authorization. The returned
    /// `key_authorization` is the value to serve for HTTP-01; for DNS-01 pass it
    /// through [`jws::dns01_txt_value`] to derive the TXT record value.
    pub async fn get_challenges(&self, order_url: &str) -> Result<Vec<Http01Challenge>> {
        let key = self
            .account_key
            .as_ref()
            .ok_or_else(|| AcmeError::InvalidAccount("account key not initialized".into()))?;

        tracing::info!("Fetching challenges for order {order_url}");
        let order: Order = self.post_as_get(order_url).await?;

        let wanted = challenge_type_id(self.config.challenge_type);
        let mut out = Vec::new();

        for auth_url in &order.authorizations {
            let auth: Authorization = self.post_as_get(auth_url).await?;
            let challenge = auth
                .challenges
                .iter()
                .find(|c| c.challenge_type == wanted)
                .ok_or_else(|| {
                    AcmeError::ChallengeFailed(format!(
                        "authorization {auth_url} has no {wanted} challenge"
                    ))
                })?;

            out.push(Http01Challenge {
                token: challenge.token.clone(),
                key_authorization: key.key_authorization(&challenge.token),
                url: challenge.url.clone(),
            });
        }

        Ok(out)
    }

    /// Tell the CA a challenge is ready, then poll it to a terminal state.
    ///
    /// Returns `Ok(())` when the challenge becomes `valid`, or an error carrying
    /// the CA's challenge error when it becomes `invalid` or the poll times out.
    pub async fn notify_challenge_ready(&self, challenge_url: &str) -> Result<()> {
        let kid = self.kid()?;

        tracing::info!("Notifying challenge ready: {challenge_url}");
        // An empty JSON object triggers validation (RFC 8555 §7.5.1).
        let empty = json!({});
        self.post_signed(challenge_url, KeyRef::Kid(&kid), Payload::Json(&empty))
            .await?;

        let deadline = std::time::Instant::now() + POLL_TIMEOUT;
        loop {
            let challenge: Challenge = self.post_as_get(challenge_url).await?;
            match challenge.status {
                ChallengeStatus::Valid => return Ok(()),
                ChallengeStatus::Invalid => {
                    let detail = challenge
                        .error
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "challenge failed".into());
                    return Err(AcmeError::ChallengeFailed(detail));
                }
                _ => {
                    if std::time::Instant::now() >= deadline {
                        return Err(AcmeError::ChallengeFailed(
                            "timed out waiting for challenge validation".into(),
                        ));
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }
        }
    }

    /// Finalize an order: generate a CSR, submit it, and download the cert.
    ///
    /// Returns `(certificate_pem_chain, certificate_private_key_pem)`. The
    /// private key is a freshly generated ECDSA P-256 key matching the CSR.
    /// Returns an error (never empty strings) if finalization fails.
    pub async fn finalize_order(&self, order_url: &str) -> Result<(String, String)> {
        let kid = self.kid()?;

        if self.config.domains.is_empty() {
            return Err(AcmeError::OrderFailed("no domains configured".into()));
        }

        tracing::info!("Finalizing order {order_url}");

        // Generate the certificate keypair + CSR with a SAN per domain.
        let params = rcgen::CertificateParams::new(self.config.domains.clone())
            .map_err(|e| AcmeError::CertificateError(format!("CSR params: {e}")))?;
        let cert_key = rcgen::KeyPair::generate()
            .map_err(|e| AcmeError::CertificateError(format!("keygen: {e}")))?;
        let csr = params
            .serialize_request(&cert_key)
            .map_err(|e| AcmeError::CertificateError(format!("serialize CSR: {e}")))?;
        let csr_der = csr.der();
        let cert_key_pem = cert_key.serialize_pem();

        // Look up the order's finalize URL.
        let order: Order = self.post_as_get(order_url).await?;
        let finalize_url = order.finalize.clone();

        let payload = json!({ "csr": jws::b64url(csr_der) });
        self.post_signed(&finalize_url, KeyRef::Kid(&kid), Payload::Json(&payload))
            .await?;

        // Poll the order until it is valid (certificate ready) or fails.
        let deadline = std::time::Instant::now() + POLL_TIMEOUT;
        let order = loop {
            let order: Order = self.post_as_get(order_url).await?;
            match order.status {
                OrderStatus::Valid => break order,
                OrderStatus::Invalid => {
                    return Err(AcmeError::OrderFailed(
                        "order became invalid during finalization".into(),
                    ));
                }
                _ => {
                    if std::time::Instant::now() >= deadline {
                        return Err(AcmeError::OrderFailed(
                            "timed out waiting for order finalization".into(),
                        ));
                    }
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }
        };

        let cert_url = order
            .certificate
            .ok_or_else(|| AcmeError::OrderFailed("valid order has no certificate URL".into()))?;

        // Download the PEM certificate chain (POST-as-GET returns raw PEM).
        let resp = self
            .post_signed(&cert_url, KeyRef::Kid(&kid), Payload::Empty)
            .await?;
        let cert_pem = resp.text();
        if !cert_pem.contains("-----BEGIN CERTIFICATE-----") {
            return Err(AcmeError::CertificateError(
                "downloaded certificate chain was empty or not PEM".into(),
            ));
        }

        Ok((cert_pem, cert_key_pem))
    }

    /// Obtain a certificate end-to-end.
    ///
    /// Orchestrates register → order → challenges → notify → finalize and
    /// returns `(certificate_pem_chain, private_key_pem)`.
    ///
    /// **Challenge validation must be satisfied out-of-band.** For HTTP-01 the
    /// caller must be serving each challenge's `key_authorization` at its
    /// `.well-known/acme-challenge/<token>` path before this call reaches the
    /// notify step; for DNS-01 the TXT record must be published. To interleave
    /// serving with the flow, drive the steps manually instead:
    /// [`create_order`](Self::create_order) →
    /// [`get_challenges`](Self::get_challenges) → serve/publish →
    /// [`notify_challenge_ready`](Self::notify_challenge_ready) →
    /// [`finalize_order`](Self::finalize_order).
    ///
    /// Returns a real [`AcmeError`] on any failure — never empty strings.
    pub async fn order_certificate(&mut self) -> Result<(String, String)> {
        if self.account_url.is_none() {
            self.register_account().await?;
        }

        let order_url = self.create_order().await?;

        let challenges = self.get_challenges(&order_url).await?;
        for challenge in &challenges {
            self.notify_challenge_ready(&challenge.url).await?;
        }

        self.finalize_order(&order_url).await
    }

    /// Check whether the certificate at `cert_path` should be renewed.
    ///
    /// Returns `true` when the leaf certificate expires within
    /// `renew_before_days`, or when the file is missing/unparseable.
    pub async fn should_renew(&self, cert_path: impl AsRef<Path>) -> Result<bool> {
        let cert_path = cert_path.as_ref();
        if !cert_path.exists() {
            return Ok(true);
        }

        let pem = std::fs::read(cert_path)?;
        let mut reader = std::io::BufReader::new(pem.as_slice());
        // A corrupt/truncated/empty cert file is treated the same as a
        // missing one: renew rather than silently skip (fail-open-to-outage
        // would leave a renewal daemon never renewing a damaged cert).
        let leaf = match rustls_pemfile::certs(&mut reader).next().transpose() {
            Ok(Some(leaf)) => leaf,
            Ok(None) | Err(_) => return Ok(true),
        };

        let cert = match x509_parser::parse_x509_certificate(leaf.as_ref()) {
            Ok((_, cert)) => cert,
            Err(_) => return Ok(true),
        };
        let not_after = cert.validity().not_after.timestamp();

        let renew_secs = i64::from(self.config.renew_before_days) * 86_400;
        let threshold = chrono::Utc::now().timestamp() + renew_secs;
        Ok(threshold >= not_after)
    }

    /// Validate and write a certificate chain and private key to `cert_dir`.
    ///
    /// Returns `(cert_path, key_path)`. Rejects empty/whitespace or non-PEM
    /// input with an [`AcmeError`] so an empty result can never be written as a
    /// valid certificate.
    pub async fn save_certificate(
        &self,
        cert_pem: &str,
        key_pem: &str,
    ) -> Result<(String, String)> {
        use std::fs;

        if cert_pem.trim().is_empty() || !cert_pem.contains("-----BEGIN CERTIFICATE-----") {
            return Err(AcmeError::CertificateError(
                "refusing to save empty or non-PEM certificate".into(),
            ));
        }
        if key_pem.trim().is_empty() || !key_pem.contains("-----BEGIN") {
            return Err(AcmeError::CertificateError(
                "refusing to save empty or non-PEM private key".into(),
            ));
        }

        fs::create_dir_all(&self.config.cert_dir)?;

        let cert_path = self.config.cert_dir.join("cert.pem");
        let key_path = self.config.cert_dir.join("key.pem");

        fs::write(&cert_path, cert_pem)?;
        write_secret_file(&key_path, key_pem.as_bytes())?;

        Ok((
            cert_path.to_string_lossy().to_string(),
            key_path.to_string_lossy().to_string(),
        ))
    }

    /// Get the ACME directory.
    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    /// Get the configuration.
    pub fn config(&self) -> &AcmeConfig {
        &self.config
    }

    /// The registered account URL, if `register_account` has succeeded.
    pub fn account_url(&self) -> Option<&str> {
        self.account_url.as_deref()
    }
}

/// The ACME challenge type identifier string (e.g. `http-01`).
fn challenge_type_id(challenge_type: ChallengeType) -> &'static str {
    match challenge_type {
        ChallengeType::Http01 => "http-01",
        ChallengeType::Dns01 => "dns-01",
        ChallengeType::TlsAlpn01 => "tls-alpn-01",
    }
}

/// Write private key material, creating the file `0600` **before** any bytes
/// are written.
///
/// Writing first and `chmod`-ing after leaves a umask-dependent window in which
/// the key is group/world-readable, so the mode is set at create time instead.
#[cfg(unix)]
fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    // An existing file keeps its old mode through `open`, so re-assert it.
    set_file_perms(path);
    Ok(())
}

#[cfg(not(unix))]
fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

#[cfg(unix)]
fn set_file_perms(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_file_perms(_path: &Path) {}

#[cfg(unix)]
fn set_dir_perms(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_dir_perms(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_directory_invalid_url() {
        let client = reqwest::Client::new();
        let result = AcmeClient::fetch_directory(&client, "https://invalid.example.com").await;
        assert!(result.is_err());
    }

    #[test]
    fn test_client_config_access() {
        let config = AcmeConfig::lets_encrypt_staging(
            vec!["admin@example.com".to_string()],
            vec!["example.com".to_string()],
        );
        assert_eq!(config.domains, vec!["example.com"]);
    }

    #[test]
    fn challenge_type_ids() {
        assert_eq!(challenge_type_id(ChallengeType::Http01), "http-01");
        assert_eq!(challenge_type_id(ChallengeType::Dns01), "dns-01");
        assert_eq!(challenge_type_id(ChallengeType::TlsAlpn01), "tls-alpn-01");
    }

    /// A client without a real CA connection, for testing pure-local behavior.
    fn offline_client(config: AcmeConfig) -> AcmeClient {
        AcmeClient {
            config,
            directory: Directory {
                new_account: "https://ca.example/acme/new-acct".into(),
                new_order: "https://ca.example/acme/new-order".into(),
                new_nonce: "https://ca.example/acme/new-nonce".into(),
                revoke_cert: "https://ca.example/acme/revoke".into(),
                key_change: None,
                meta: None,
            },
            http_client: reqwest::Client::new(),
            account_key: None,
            account_url: None,
            nonce: Mutex::new(None),
        }
    }

    #[tokio::test]
    async fn save_certificate_rejects_empty() {
        let dir = std::env::temp_dir().join(format!("acme-save-{}", std::process::id()));
        let config = AcmeConfig::default().with_cert_dir(dir.clone());
        let client = offline_client(config);

        assert!(client.save_certificate("", "key").await.is_err());
        assert!(client.save_certificate("   \n  ", "key").await.is_err());
        assert!(
            client
                .save_certificate("not a pem", "-----BEGIN PRIVATE KEY-----")
                .await
                .is_err()
        );
        assert!(
            client
                .save_certificate(
                    "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----",
                    ""
                )
                .await
                .is_err()
        );

        // A valid-looking pair writes files.
        let ok = client
            .save_certificate(
                "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----\n",
                "-----BEGIN PRIVATE KEY-----\nxyz\n-----END PRIVATE KEY-----\n",
            )
            .await;
        assert!(ok.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csr_contains_configured_sans() {
        // Mirrors the CSR construction in `finalize_order`: a CSR built for the
        // configured domains must carry them as SubjectAltName DNS entries.
        let domains = vec!["a.example.com".to_string(), "b.example.com".to_string()];
        let params = rcgen::CertificateParams::new(domains.clone()).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let csr = params.serialize_request(&key).unwrap();
        let der = csr.der();

        use x509_parser::prelude::FromDer;
        let (_, parsed) =
            x509_parser::certification_request::X509CertificationRequest::from_der(der.as_ref())
                .unwrap();
        let exts = parsed
            .requested_extensions()
            .expect("CSR requested extensions");
        let mut found: Vec<String> = Vec::new();
        for ext in exts {
            if let x509_parser::extensions::ParsedExtension::SubjectAlternativeName(san) = ext {
                for gn in &san.general_names {
                    if let x509_parser::extensions::GeneralName::DNSName(n) = gn {
                        found.push(n.to_string());
                    }
                }
            }
        }
        for d in &domains {
            assert!(found.contains(d), "CSR SANs {found:?} must contain {d}");
        }
    }

    #[tokio::test]
    async fn should_renew_missing_file_is_true() {
        let client = offline_client(AcmeConfig::default());
        assert!(
            client
                .should_renew("/nonexistent/path/cert.pem")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn should_renew_honors_boundary() {
        use rcgen::{CertificateParams, KeyPair};
        use time::{Duration, OffsetDateTime};

        // Build a self-signed cert expiring `days` from now, write it, return path.
        fn write_cert_expiring_in(days: i64, tag: &str) -> std::path::PathBuf {
            let mut params = CertificateParams::new(vec!["renew.example".to_string()]).unwrap();
            params.not_before = OffsetDateTime::now_utc() - Duration::days(1);
            params.not_after = OffsetDateTime::now_utc() + Duration::days(days);
            let key = KeyPair::generate().unwrap();
            let pem = params.self_signed(&key).unwrap().pem();
            let path = std::env::temp_dir().join(format!(
                "acme-renew-{tag}-{}-{days}.pem",
                std::process::id()
            ));
            std::fs::write(&path, pem).unwrap();
            path
        }

        // Cert expiring in 40 days, renew window 30 days -> NOT due for renewal.
        let far = write_cert_expiring_in(40, "far");
        let config = AcmeConfig::default().with_renew_before_days(30);
        let client = offline_client(config);
        assert!(
            !client.should_renew(&far).await.unwrap(),
            "cert 40d out with 30d window should not renew"
        );

        // Cert expiring in 10 days, renew window 30 days -> due for renewal.
        let near = write_cert_expiring_in(10, "near");
        assert!(
            client.should_renew(&near).await.unwrap(),
            "cert 10d out with 30d window should renew"
        );

        let _ = std::fs::remove_file(&far);
        let _ = std::fs::remove_file(&near);
    }
}
