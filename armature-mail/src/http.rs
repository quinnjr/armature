//! Shared HTTP concerns for the API-based providers (SendGrid, Mailgun).
//!
//! Both transports build a `reqwest::Client`, attach a long-lived API key to
//! every request, and have to interpret a `429`. Each did all three slightly
//! differently — one parsed `Retry-After` and the other hardcoded 60; both fell
//! back to a completely untimed client when the builder failed; neither checked
//! that the endpoint they were about to write the key to was encrypted.

use std::time::Duration;

use crate::{MailError, Result};

/// Default overall request timeout for a provider call.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default connect-phase timeout for a provider call.
///
/// Bounded separately from the overall timeout so an unreachable endpoint fails
/// quickly rather than consuming the whole request budget. Matches
/// `ApnsConfig::connect_timeout` in `armature-push`.
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Whether `host` names the local machine.
///
/// The same set is defined independently as `is_loopback_host` in
/// `armature-payments::provider` and `armature-push::host_guard`. There is no
/// dependency edge between the three crates, so this is a deliberate
/// duplication under a shared name: grepping for `is_loopback_host` finds all
/// three call sites when the set has to change.
fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// Reject an endpoint that is neither `https` nor a loopback `http` URL.
///
/// Both transports send a long-lived credential — SendGrid's
/// `Authorization: Bearer <api key>`, Mailgun's HTTP basic `api:<api key>` — on
/// *every* request. Pointing either at `http://collector.example.com/...`
/// therefore ships the key in cleartext to whoever is listening. Loopback is
/// allowed so a test can stand a plain HTTP server up locally.
///
/// This checks the *scheme* only. It is **not** an SSRF guard: it does not
/// resolve the host, and an `https` URL pointing at a private or link-local
/// address passes. `armature-push::web_push::validate_endpoint` is the function
/// that does the full address check; this one is named for what it actually
/// guarantees so the two are not confused.
pub(crate) fn require_encrypted_endpoint(endpoint: &str) -> Result<()> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|e| MailError::Config(format!("invalid endpoint {endpoint:?}: {e}")))?;

    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed.host_str().unwrap_or_default();
            if is_loopback_host(host) {
                Ok(())
            } else {
                Err(MailError::Config(format!(
                    "insecure endpoint {endpoint:?}: the API key is sent on every \
                     request, so plaintext http is only allowed for loopback hosts \
                     (localhost, 127.0.0.1, [::1]); use https"
                )))
            }
        }
        other => Err(MailError::Config(format!(
            "unsupported endpoint scheme {other:?} in {endpoint:?}: expected https"
        ))),
    }
}

/// Build a `reqwest::Client` carrying both timeouts, or fail.
///
/// The previous `…build().unwrap_or_else(|_| Client::new())` silently produced a
/// client with *no* request timeout and *no* connect timeout — precisely the
/// state the timeout config exists to prevent — in the one case where something
/// had already gone wrong. A construction failure is a configuration error and
/// must surface.
pub(crate) fn build_client(
    timeout: Duration,
    connect_timeout: Duration,
) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(connect_timeout)
        .build()
        .map_err(|e| MailError::Config(format!("failed to build HTTP client: {e}")))
}

/// Seconds to wait, from a response's `Retry-After` header.
///
/// Returns `None` when the header is absent or unparseable; the caller
/// applies its own default via `.unwrap_or(default)`. `Retry-After` may also
/// be an HTTP-date; only the delta-seconds form is honored, which is what
/// both providers send.
///
/// The retry schedule for a rate-limited job is keyed entirely off this value
/// (`EmailQueueWorker::calculate_backoff` returns it verbatim), so a provider
/// that told us to wait 300s and got retried after the local 5s default just
/// gets re-throttled. Mailgun hardcoded 60 and ignored the header outright.
pub(crate) fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_endpoints_are_accepted() {
        for ok in [
            "https://api.sendgrid.com/v3/mail/send",
            "https://api.eu.mailgun.net/v3/example.com/messages",
        ] {
            assert!(require_encrypted_endpoint(ok).is_ok(), "should allow {ok}");
        }
    }

    #[test]
    fn plaintext_http_is_allowed_only_on_loopback() {
        for ok in [
            "http://localhost:8080/v3/mail/send",
            "http://127.0.0.1:3000/v3/mail/send",
            "http://[::1]:9000/v3/mail/send",
        ] {
            assert!(require_encrypted_endpoint(ok).is_ok(), "should allow {ok}");
        }
    }

    #[test]
    fn plaintext_http_to_a_remote_host_is_rejected() {
        let err =
            require_encrypted_endpoint("http://collector.example.com/v3/mail/send").unwrap_err();
        assert!(matches!(err, MailError::Config(_)));
        assert!(err.to_string().contains("https"), "{err}");
    }

    #[test]
    fn non_http_schemes_and_garbage_are_rejected() {
        for bad in ["ftp://api.example.com", "file:///etc/passwd", "not a url"] {
            assert!(
                matches!(require_encrypted_endpoint(bad), Err(MailError::Config(_))),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn a_client_built_here_always_carries_both_timeouts() {
        // `Client` does not expose its timeouts, so the guarantee under test is
        // that construction is fallible rather than falling back to a bare
        // `Client::new()` with neither bound.
        assert!(build_client(DEFAULT_TIMEOUT, DEFAULT_CONNECT_TIMEOUT).is_ok());
    }

    #[test]
    fn retry_after_is_parsed_and_falls_back() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after_secs(&headers), None);

        headers.insert(reqwest::header::RETRY_AFTER, "30".parse().unwrap());
        assert_eq!(retry_after_secs(&headers), Some(30));

        // An HTTP-date form is not delta-seconds; fall back rather than guess.
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after_secs(&headers), None);
    }
}
