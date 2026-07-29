//! Idempotent installation of rustls's `ring` `CryptoProvider`.
//!
//! `tonic`'s `tls-ring` feature makes `ring` available, but does not itself
//! call `rustls::crypto::CryptoProvider::install_default()` — rustls falls
//! back to auto-detecting a process-wide default the first time TLS is
//! actually used. That auto-detection only succeeds when exactly one
//! provider (`ring` or `aws-lc-rs`) is compiled into the binary. In a real
//! application — or in this workspace's own test suite, which links this
//! crate alongside AWS SDK crates that default to `aws-lc-rs` — Cargo
//! unifies both providers into one binary, and rustls refuses to guess:
//! every TLS handshake panics with "Could not automatically determine the
//! process-level CryptoProvider". Installing `ring` explicitly, once, before
//! any TLS config is built removes the ambiguity regardless of what else the
//! final binary links in.
use std::sync::Once;

static INIT: Once = Once::new();

/// Install the `ring` `CryptoProvider` as the process default, if one has not
/// already been installed. Safe to call from every TLS entry point — the
/// underlying `install_default` is idempotent-safe here because `Once`
/// ensures it runs at most one time per process, and its `Result` is
/// intentionally discarded: a prior, unrelated installation (by this crate
/// or another) having already won is not an error condition.
pub(crate) fn ensure_installed() {
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
