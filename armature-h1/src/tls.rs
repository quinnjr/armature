//! Protocol dispatch: TLS with ALPN, and h2c prior-knowledge detection.
//!
//! This crate serves HTTP/1.1 only. A peer that insists on HTTP/2 — by
//! negotiating `h2` over ALPN, or by opening with the h2c prior-knowledge preface
//! — is handed to an [`H2Fallback`], which `armature-core` implements with hyper.
//! Without a fallback configured, such a connection is closed rather than
//! mis-served as HTTP/1.1.

use crate::service::Transport;
use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;

/// The HTTP/2 connection preface sent by a client using prior knowledge
/// (RFC 9113 section 3.4).
pub const H2C_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// What the first bytes of a plaintext connection indicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preface {
    /// Not HTTP/2: serve as HTTP/1.
    Http1,
    /// The h2c prior-knowledge preface.
    Http2,
    /// Consistent with the preface so far, but too short to decide.
    NeedMore,
}

/// Classify the start of a plaintext connection.
///
/// Returns [`Preface::NeedMore`] for input that is a strict prefix of the preface,
/// so dispatch never guesses on a partial first read. Anything that diverges from
/// the preface is HTTP/1 — including input that merely starts with `P`, since
/// `PROPFIND` and `PATCH` are perfectly good HTTP/1 requests.
#[inline]
pub fn is_h2c_preface(buf: &[u8]) -> Preface {
    let n = buf.len().min(H2C_PREFACE.len());
    if buf[..n] != H2C_PREFACE[..n] {
        return Preface::Http1;
    }
    if buf.len() >= H2C_PREFACE.len() {
        Preface::Http2
    } else {
        Preface::NeedMore
    }
}

/// Somewhere to send connections this crate will not serve.
///
/// Not gated behind the `tls` feature: plaintext h2c dispatch needs it with no
/// TLS involved.
pub trait H2Fallback {
    /// Take over `io`.
    ///
    /// `buffered` holds bytes already read from the transport — for h2c, the
    /// preface itself. The implementation **must** process them before reading
    /// `io`, or it will see a stream that appears to be missing its opening
    /// frames.
    fn handle(&self, io: Box<dyn Transport>, buffered: Bytes) -> Pin<Box<dyn Future<Output = ()>>>;
}

/// TLS configuration.
#[cfg(feature = "tls")]
#[derive(Clone, Debug)]
pub struct TlsConfig {
    /// DER-encoded certificate chain, leaf first.
    pub cert_chain: Vec<Vec<u8>>,
    /// DER-encoded private key.
    pub key_der: Vec<u8>,
    /// ALPN protocols to offer, in preference order.
    ///
    /// Defaults to `http/1.1` alone. Add `h2` only when an [`H2Fallback`] is
    /// configured, or a negotiated `h2` connection has nowhere to go but closed.
    pub alpn: Vec<Vec<u8>>,
}

#[cfg(feature = "tls")]
impl TlsConfig {
    /// A configuration offering `http/1.1` only.
    pub fn new(cert_chain: Vec<Vec<u8>>, key_der: Vec<u8>) -> Self {
        Self {
            cert_chain,
            key_der,
            alpn: vec![b"http/1.1".to_vec()],
        }
    }

    /// Also offer `h2`, for use with an [`H2Fallback`].
    pub fn with_h2(mut self) -> Self {
        if !self.alpn.iter().any(|p| p == b"h2") {
            // Before http/1.1: a client that supports both should get the
            // protocol the fallback is there to serve.
            self.alpn.insert(0, b"h2".to_vec());
        }
        self
    }

    /// Build a rustls server configuration.
    ///
    /// Built once and shared across workers as an `Arc`. It is read-only after
    /// construction, so sharing it costs one refcount at worker startup rather
    /// than any per-connection synchronization.
    pub fn server_config(&self) -> Result<std::sync::Arc<rustls::ServerConfig>, TlsError> {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};

        if self.cert_chain.is_empty() {
            return Err(TlsError::NoCertificate);
        }

        let certs: Vec<CertificateDer<'static>> = self
            .cert_chain
            .iter()
            .map(|c| CertificateDer::from(c.clone()))
            .collect();

        let key = PrivateKeyDer::try_from(self.key_der.clone())
            .map_err(|_| TlsError::InvalidPrivateKey)?;

        // Name the provider explicitly rather than relying on a process-global
        // default. A library that installs or requires one imposes a choice on
        // every other rustls user in the binary, and rustls panics rather than
        // guessing when more than one is compiled in.
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Rustls(e.to_string()))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| TlsError::Rustls(e.to_string()))?;
        cfg.alpn_protocols = self.alpn.clone();
        Ok(std::sync::Arc::new(cfg))
    }
}

/// A TLS configuration error.
#[cfg(feature = "tls")]
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    /// No certificate was supplied.
    #[error("no certificate supplied")]
    NoCertificate,
    /// The private key could not be parsed.
    #[error("invalid private key")]
    InvalidPrivateKey,
    /// rustls rejected the configuration.
    #[error("rustls: {0}")]
    Rustls(String),
}

/// Whether a completed TLS handshake negotiated HTTP/2.
#[cfg(feature = "tls")]
#[inline]
pub fn negotiated_h2(conn: &rustls::ServerConnection) -> bool {
    conn.alpn_protocol() == Some(b"h2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_http1() {
        assert_eq!(is_h2c_preface(b"GET / HTTP/1.1\r\n"), Preface::Http1);
        assert_eq!(is_h2c_preface(b"POST /x HTTP/1.1\r\n"), Preface::Http1);
    }

    #[test]
    fn detects_the_preface() {
        assert_eq!(is_h2c_preface(H2C_PREFACE), Preface::Http2);
        // Trailing frame bytes do not change the classification.
        let mut extended = H2C_PREFACE.to_vec();
        extended.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 0, 0]);
        assert_eq!(is_h2c_preface(&extended), Preface::Http2);
    }

    /// A strict prefix must never be guessed at in either direction.
    #[test]
    fn needs_more_on_a_short_prefix() {
        assert_eq!(is_h2c_preface(b""), Preface::NeedMore);
        assert_eq!(is_h2c_preface(b"P"), Preface::NeedMore);
        assert_eq!(is_h2c_preface(b"PRI * HTTP/2"), Preface::NeedMore);
        // One byte short.
        assert_eq!(
            is_h2c_preface(&H2C_PREFACE[..H2C_PREFACE.len() - 1]),
            Preface::NeedMore
        );
    }

    /// `PATCH` and `PROPFIND` start with `P` and are ordinary HTTP/1 requests.
    #[test]
    fn a_divergent_prefix_is_http1() {
        assert_eq!(is_h2c_preface(b"PRX"), Preface::Http1);
        assert_eq!(is_h2c_preface(b"PATCH / HTTP/1.1\r\n"), Preface::Http1);
        assert_eq!(is_h2c_preface(b"PROPFIND / HTTP/1.1\r\n"), Preface::Http1);
        // Diverging at the very last byte still decides.
        let mut nearly = H2C_PREFACE.to_vec();
        let last = nearly.len() - 1;
        nearly[last] = b'X';
        assert_eq!(is_h2c_preface(&nearly), Preface::Http1);
    }

    /// Every prefix length is classified consistently: NeedMore until complete,
    /// then Http2. A single off-by-one here would either misroute HTTP/2 to the
    /// HTTP/1 parser or stall an HTTP/1 client waiting for more bytes.
    #[test]
    fn every_prefix_length_is_classified_consistently() {
        for i in 0..H2C_PREFACE.len() {
            assert_eq!(
                is_h2c_preface(&H2C_PREFACE[..i]),
                Preface::NeedMore,
                "prefix of length {i} must be NeedMore"
            );
        }
        assert_eq!(is_h2c_preface(H2C_PREFACE), Preface::Http2);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn tls_config_defaults_to_http11_alpn() {
        let c = TlsConfig::new(vec![vec![1, 2, 3]], vec![4, 5, 6]);
        assert_eq!(c.alpn, vec![b"http/1.1".to_vec()]);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn with_h2_prefers_h2_and_is_idempotent() {
        let c = TlsConfig::new(vec![vec![1]], vec![2]).with_h2();
        assert_eq!(c.alpn, vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
        let c = c.with_h2();
        assert_eq!(
            c.alpn,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()],
            "calling it twice must not duplicate the protocol"
        );
    }

    #[cfg(feature = "tls")]
    #[test]
    fn server_config_rejects_an_empty_chain() {
        let c = TlsConfig::new(vec![], vec![1, 2, 3]);
        assert!(matches!(c.server_config(), Err(TlsError::NoCertificate)));
    }

    #[cfg(feature = "tls")]
    #[test]
    fn server_config_rejects_a_bogus_key() {
        let c = TlsConfig::new(vec![vec![1, 2, 3]], vec![]);
        assert!(c.server_config().is_err());
    }
}
