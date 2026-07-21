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

    /// Select the best algorithm based on Accept-Encoding header
    #[cfg_attr(
        not(any(feature = "gzip", feature = "brotli", feature = "zstd")),
        allow(unused_variables)
    )]
    pub fn select_from_accept_encoding(accept_encoding: &str) -> Self {
        let encodings: Vec<&str> = accept_encoding
            .split(',')
            .map(|s| s.split(';').next().unwrap_or("").trim())
            .collect();

        // Priority: br > zstd > gzip
        #[cfg(feature = "brotli")]
        if encodings.contains(&"br") {
            return Self::Brotli;
        }

        #[cfg(feature = "zstd")]
        if encodings.contains(&"zstd") {
            return Self::Zstd;
        }

        #[cfg(feature = "gzip")]
        if encodings.contains(&"gzip") {
            return Self::Gzip;
        }

        Self::None
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
