//! Compression algorithm implementations

use crate::{CompressionError, Result};
#[cfg(any(feature = "gzip", feature = "brotli", feature = "zstd"))]
use std::io::Write;

/// Supported compression algorithms
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionAlgorithm {
    /// Automatically select the best algorithm based on Accept-Encoding
    #[default]
    Auto,

    /// Gzip compression (widely supported)
    #[cfg(feature = "gzip")]
    Gzip,

    /// Brotli compression (best ratio for text)
    #[cfg(feature = "brotli")]
    Brotli,

    /// Zstd compression (fast with good ratio)
    #[cfg(feature = "zstd")]
    Zstd,

    /// No compression (pass-through)
    None,
}

impl CompressionAlgorithm {
    /// Get the Content-Encoding header value for this algorithm
    pub fn encoding_name(&self) -> Option<&'static str> {
        match self {
            Self::Auto => None, // Will be determined at runtime
            #[cfg(feature = "gzip")]
            Self::Gzip => Some("gzip"),
            #[cfg(feature = "brotli")]
            Self::Brotli => Some("br"),
            #[cfg(feature = "zstd")]
            Self::Zstd => Some("zstd"),
            Self::None => None,
        }
    }

    /// Check if this algorithm is available (feature enabled)
    pub fn is_available(&self) -> bool {
        match self {
            Self::Auto | Self::None => true,
            #[cfg(feature = "gzip")]
            Self::Gzip => true,
            #[cfg(feature = "brotli")]
            Self::Brotli => true,
            #[cfg(feature = "zstd")]
            Self::Zstd => true,
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    /// Parse an `Accept-Encoding` header into `(token, qvalue)` pairs.
    ///
    /// RFC 9110 §12.4.2: a missing `;q=` means `q=1`, and an unparsable qvalue
    /// is treated as 1 rather than dropping the entry, matching how browsers
    /// and other servers behave with malformed headers.
    fn parse_accept_encoding(accept_encoding: &str) -> Vec<(String, f32)> {
        accept_encoding
            .split(',')
            .filter_map(|entry| {
                let mut parts = entry.split(';');
                let token = parts.next()?.trim().to_ascii_lowercase();
                if token.is_empty() {
                    return None;
                }
                let q = parts
                    .find_map(|p| {
                        let p = p.trim();
                        let rest = p.strip_prefix("q=").or_else(|| p.strip_prefix("Q="))?;
                        rest.trim().parse::<f32>().ok()
                    })
                    .unwrap_or(1.0);
                Some((token, q))
            })
            .collect()
    }

    /// The qvalue a client assigned to `token`, honouring a `*` wildcard.
    ///
    /// Returns `None` when the client neither named the token nor covered it
    /// with `*` — which is different from naming it with `q=0` (explicitly
    /// unacceptable) and different again from `q>0` (acceptable).
    fn qvalue_for(entries: &[(String, f32)], token: &str) -> Option<f32> {
        entries
            .iter()
            .find(|(name, _)| name == token)
            .or_else(|| entries.iter().find(|(name, _)| name == "*"))
            .map(|(_, q)| *q)
    }

    /// Whether a client advertising `accept_encoding` will accept `self`.
    ///
    /// `None`/`Auto` mean "send it uncompressed", which is `identity`; a client
    /// may reject that too, with `identity;q=0`.
    pub fn is_accepted_by(&self, accept_encoding: &str) -> bool {
        let entries = Self::parse_accept_encoding(accept_encoding);
        let token = match self.encoding_name() {
            Some(name) => name,
            // No Content-Encoding on the wire is `identity`, which is
            // acceptable by default unless explicitly refused.
            None => return Self::qvalue_for(&entries, "identity").unwrap_or(1.0) > 0.0,
        };
        Self::qvalue_for(&entries, token).is_some_and(|q| q > 0.0)
    }

    /// Select the best algorithm based on `Accept-Encoding`.
    ///
    /// Follows RFC 9110 §12.5.3: `;q=0` means "not acceptable" and excludes a
    /// coding outright, a `*` covers codings the client did not name, and among
    /// acceptable codings the highest qvalue wins. Ties are broken by the
    /// server's own preference, br > zstd > gzip, which is what the qvalue
    /// rules leave to the server.
    #[cfg_attr(
        not(any(feature = "gzip", feature = "brotli", feature = "zstd")),
        allow(unused_variables, unused_mut)
    )]
    pub fn select_from_accept_encoding(accept_encoding: &str) -> Self {
        let entries = Self::parse_accept_encoding(accept_encoding);

        // Server preference order, most preferred first. Only used to break
        // ties between codings the client rated equally.
        let candidates: &[Self] = &[
            #[cfg(feature = "brotli")]
            Self::Brotli,
            #[cfg(feature = "zstd")]
            Self::Zstd,
            #[cfg(feature = "gzip")]
            Self::Gzip,
        ];

        let mut best: Option<(Self, f32)> = None;
        for candidate in candidates {
            let Some(name) = candidate.encoding_name() else {
                continue;
            };
            let Some(q) = Self::qvalue_for(&entries, name) else {
                continue;
            };
            if q <= 0.0 {
                continue;
            }
            // Strictly greater, so an equal qvalue leaves the earlier (more
            // preferred) candidate in place.
            if best.is_none_or(|(_, best_q)| q > best_q) {
                best = Some((*candidate, q));
            }
        }

        best.map(|(algo, _)| algo).unwrap_or(Self::None)
    }

    /// Get the minimum compression level for this algorithm
    pub fn min_level(&self) -> u32 {
        match self {
            #[cfg(feature = "gzip")]
            Self::Gzip => 1,
            #[cfg(feature = "brotli")]
            Self::Brotli => 0,
            #[cfg(feature = "zstd")]
            Self::Zstd => 1,
            _ => 0,
        }
    }

    /// Get the maximum compression level for this algorithm
    pub fn max_level(&self) -> u32 {
        match self {
            #[cfg(feature = "gzip")]
            Self::Gzip => 9,
            #[cfg(feature = "brotli")]
            Self::Brotli => 11,
            #[cfg(feature = "zstd")]
            Self::Zstd => 22,
            _ => 0,
        }
    }

    /// Get the default compression level for this algorithm
    pub fn default_level(&self) -> u32 {
        match self {
            #[cfg(feature = "gzip")]
            Self::Gzip => 6,
            #[cfg(feature = "brotli")]
            Self::Brotli => 4,
            #[cfg(feature = "zstd")]
            Self::Zstd => 3,
            _ => 0,
        }
    }

    /// Compress data using this algorithm
    #[cfg_attr(
        not(any(feature = "gzip", feature = "brotli", feature = "zstd")),
        allow(unused_variables)
    )]
    pub fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>> {
        match self {
            #[cfg(feature = "gzip")]
            Self::Gzip => compress_gzip(data, level),
            #[cfg(feature = "brotli")]
            Self::Brotli => compress_brotli(data, level),
            #[cfg(feature = "zstd")]
            Self::Zstd => compress_zstd(data, level),
            // `None` is an explicit pass-through and legitimately returns the
            // input unchanged.
            Self::None => Ok(data.to_vec()),
            // `Auto` is not a concrete algorithm: it must be resolved to one
            // (via `select_from_accept_encoding`) before compressing. Silently
            // returning uncompressed bytes with `Ok` and no encoding signal
            // would be a lie, so surface it as an error instead.
            Self::Auto => Err(CompressionError::UnsupportedAlgorithm(
                "Auto must be resolved to a concrete algorithm via \
                 select_from_accept_encoding before compressing"
                    .to_string(),
            )),
            #[allow(unreachable_patterns)]
            _ => Err(CompressionError::UnsupportedAlgorithm(format!(
                "{:?}",
                self
            ))),
        }
    }
}

impl std::fmt::Display for CompressionAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "auto"),
            #[cfg(feature = "gzip")]
            Self::Gzip => write!(f, "gzip"),
            #[cfg(feature = "brotli")]
            Self::Brotli => write!(f, "brotli"),
            #[cfg(feature = "zstd")]
            Self::Zstd => write!(f, "zstd"),
            Self::None => write!(f, "none"),
        }
    }
}

// ========== Gzip Implementation ==========

#[cfg(feature = "gzip")]
fn compress_gzip(data: &[u8], level: u32) -> Result<Vec<u8>> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(level));
    encoder
        .write_all(data)
        .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| CompressionError::CompressionFailed(e.to_string()))
}

// ========== Brotli Implementation ==========

#[cfg(feature = "brotli")]
fn compress_brotli(data: &[u8], level: u32) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let params = brotli::enc::BrotliEncoderParams {
        quality: level as i32,
        ..Default::default()
    };

    let mut reader = std::io::Cursor::new(data);
    brotli::BrotliCompress(&mut reader, &mut output, &params)
        .map_err(|e| CompressionError::CompressionFailed(e.to_string()))?;

    Ok(output)
}

// ========== Zstd Implementation ==========

#[cfg(feature = "zstd")]
fn compress_zstd(data: &[u8], level: u32) -> Result<Vec<u8>> {
    zstd::encode_all(std::io::Cursor::new(data), level as i32)
        .map_err(|e| CompressionError::CompressionFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "gzip")]
    use std::io::Read;

    #[test]
    fn test_algorithm_display() {
        assert_eq!(format!("{}", CompressionAlgorithm::Auto), "auto");
        assert_eq!(format!("{}", CompressionAlgorithm::None), "none");

        #[cfg(feature = "gzip")]
        assert_eq!(format!("{}", CompressionAlgorithm::Gzip), "gzip");

        #[cfg(feature = "brotli")]
        assert_eq!(format!("{}", CompressionAlgorithm::Brotli), "brotli");

        #[cfg(feature = "zstd")]
        assert_eq!(format!("{}", CompressionAlgorithm::Zstd), "zstd");
    }

    #[test]
    fn test_encoding_name() {
        assert_eq!(CompressionAlgorithm::Auto.encoding_name(), None);
        assert_eq!(CompressionAlgorithm::None.encoding_name(), None);

        #[cfg(feature = "gzip")]
        assert_eq!(CompressionAlgorithm::Gzip.encoding_name(), Some("gzip"));

        #[cfg(feature = "brotli")]
        assert_eq!(CompressionAlgorithm::Brotli.encoding_name(), Some("br"));

        #[cfg(feature = "zstd")]
        assert_eq!(CompressionAlgorithm::Zstd.encoding_name(), Some("zstd"));
    }

    /// Regression: `Auto` is not a concrete algorithm, so `compress` must not
    /// silently return uncompressed bytes with `Ok`. Against the pre-fix code
    /// this fails because `Auto` returned `Ok(data.to_vec())`.
    #[test]
    fn test_auto_compress_is_not_a_silent_noop() {
        let data = b"some data that a caller expects to be compressed";
        let result = CompressionAlgorithm::Auto.compress(data, 6);
        assert!(
            result.is_err(),
            "Auto must signal it cannot compress, not pass data through as Ok"
        );
    }

    /// `None` remains a legitimate explicit pass-through.
    #[test]
    fn test_none_compress_passes_through() {
        let data = b"unchanged";
        assert_eq!(
            CompressionAlgorithm::None.compress(data, 6).unwrap(),
            data.to_vec()
        );
    }

    #[test]
    fn test_select_from_accept_encoding() {
        // Test gzip selection
        #[cfg(feature = "gzip")]
        {
            let algo = CompressionAlgorithm::select_from_accept_encoding("gzip, deflate");
            assert_eq!(algo, CompressionAlgorithm::Gzip);
        }

        // Test brotli has priority
        #[cfg(all(feature = "gzip", feature = "brotli"))]
        {
            let algo = CompressionAlgorithm::select_from_accept_encoding("gzip, br");
            assert_eq!(algo, CompressionAlgorithm::Brotli);
        }

        // Test no match
        let algo = CompressionAlgorithm::select_from_accept_encoding("deflate");
        assert_eq!(algo, CompressionAlgorithm::None);
    }

    /// RFC 9110 §12.5.3: `q=0` means "not acceptable". Against the pre-fix
    /// code, which stripped qvalues and then ignored them, `br;q=0, gzip`
    /// selected Brotli - a coding the client had explicitly refused.
    #[cfg(all(feature = "gzip", feature = "brotli"))]
    #[test]
    fn test_q_zero_excludes_a_coding() {
        assert_eq!(
            CompressionAlgorithm::select_from_accept_encoding("br;q=0, gzip"),
            CompressionAlgorithm::Gzip
        );
    }

    /// Every acceptable coding refused leaves nothing to compress with.
    #[cfg(all(feature = "gzip", feature = "brotli"))]
    #[test]
    fn test_all_q_zero_selects_none() {
        assert_eq!(
            CompressionAlgorithm::select_from_accept_encoding("br;q=0, gzip;q=0"),
            CompressionAlgorithm::None
        );
    }

    /// The highest qvalue wins even when it is not the server's first choice.
    #[cfg(all(feature = "gzip", feature = "brotli"))]
    #[test]
    fn test_highest_q_wins_over_server_preference() {
        assert_eq!(
            CompressionAlgorithm::select_from_accept_encoding("br;q=0.1, gzip;q=0.9"),
            CompressionAlgorithm::Gzip
        );
        // Equal qvalues fall back to the server's own br > gzip preference.
        assert_eq!(
            CompressionAlgorithm::select_from_accept_encoding("br;q=0.5, gzip;q=0.5"),
            CompressionAlgorithm::Brotli
        );
    }

    /// A `*` covers codings the client did not name explicitly.
    #[cfg(feature = "gzip")]
    #[test]
    fn test_wildcard_is_honoured() {
        // Which coding wins depends on the enabled features; what matters is
        // that a bare `*` does not resolve to "no compression".
        assert_ne!(
            CompressionAlgorithm::select_from_accept_encoding("*"),
            CompressionAlgorithm::None
        );
        // An explicit entry still beats the wildcard for the coding it names.
        assert_eq!(
            CompressionAlgorithm::select_from_accept_encoding("gzip;q=0, *;q=0"),
            CompressionAlgorithm::None
        );
    }

    /// `identity;q=0` means the client will not take an uncompressed response.
    #[test]
    fn test_identity_q_zero_is_not_accepted() {
        assert!(!CompressionAlgorithm::None.is_accepted_by("gzip, identity;q=0"));
        assert!(CompressionAlgorithm::None.is_accepted_by("gzip"));
    }

    /// A configured algorithm must be checked against what the client accepts.
    #[cfg(all(feature = "gzip", feature = "brotli"))]
    #[test]
    fn test_is_accepted_by() {
        assert!(CompressionAlgorithm::Gzip.is_accepted_by("gzip, deflate"));
        assert!(!CompressionAlgorithm::Gzip.is_accepted_by("br"));
        assert!(!CompressionAlgorithm::Gzip.is_accepted_by("gzip;q=0"));
        assert!(CompressionAlgorithm::Brotli.is_accepted_by("*"));
    }

    #[cfg(feature = "gzip")]
    #[test]
    fn test_gzip_compression() {
        let data = b"Hello, World! This is a test string for compression.";
        let compressed = CompressionAlgorithm::Gzip.compress(data, 6).unwrap();

        // Compressed should be different from original
        assert_ne!(compressed, data.to_vec());

        // Decompress and verify
        use flate2::read::GzDecoder;
        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        assert_eq!(decompressed, data.to_vec());
    }

    #[cfg(feature = "brotli")]
    #[test]
    fn test_brotli_compression() {
        let data = b"Hello, World! This is a test string for compression.";
        let compressed = CompressionAlgorithm::Brotli.compress(data, 4).unwrap();

        // Compressed should be different from original
        assert_ne!(compressed, data.to_vec());

        // Decompress and verify
        let mut decompressed = Vec::new();
        brotli::BrotliDecompress(&mut std::io::Cursor::new(&compressed), &mut decompressed)
            .unwrap();
        assert_eq!(decompressed, data.to_vec());
    }

    #[cfg(feature = "zstd")]
    #[test]
    fn test_zstd_compression() {
        let data = b"Hello, World! This is a test string for compression.";
        let compressed = CompressionAlgorithm::Zstd.compress(data, 3).unwrap();

        // Compressed should be different from original
        assert_ne!(compressed, data.to_vec());

        // Decompress and verify
        let decompressed = zstd::decode_all(std::io::Cursor::new(&compressed)).unwrap();
        assert_eq!(decompressed, data.to_vec());
    }
}
