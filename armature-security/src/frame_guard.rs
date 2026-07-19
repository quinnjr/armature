//! Frame Guard (X-Frame-Options)
//!
//! Mitigates clickjacking attacks.

/// Frame Guard options
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameGuard {
    /// Deny all framing
    Deny,
    /// Allow framing from same origin
    SameOrigin,
    /// Allow framing from specific origin
    ///
    /// # Deprecated / legacy
    ///
    /// `ALLOW-FROM` was never standardized in the Fetch/HTML spec and modern browsers
    /// (Chrome, Firefox, Safari, Edge) ignore this directive entirely — it is a no-op in
    /// practice. It has been superseded by the `frame-ancestors` directive of
    /// Content-Security-Policy (see [`crate::content_security_policy::CspConfig`]), which
    /// is what you should use to restrict framing to specific origins.
    #[deprecated(
        note = "ALLOW-FROM is unstandardized and ignored by modern browsers; use CSP frame-ancestors instead."
    )]
    AllowFrom(String),
}

impl FrameGuard {
    pub fn to_header_value(&self) -> String {
        match self {
            Self::Deny => "DENY".to_string(),
            Self::SameOrigin => "SAMEORIGIN".to_string(),
            #[allow(deprecated)]
            Self::AllowFrom(origin) => format!("ALLOW-FROM {}", origin),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(deprecated)]
    fn test_frame_guard() {
        assert_eq!(FrameGuard::Deny.to_header_value(), "DENY");
        assert_eq!(FrameGuard::SameOrigin.to_header_value(), "SAMEORIGIN");
        assert_eq!(
            FrameGuard::AllowFrom("https://example.com".to_string()).to_header_value(),
            "ALLOW-FROM https://example.com"
        );
    }
}
