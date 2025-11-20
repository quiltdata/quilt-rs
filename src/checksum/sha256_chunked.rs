//! SHA256 chunked checksum implementation

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;

use crate::Error;

/// Multihash code for chunksums
pub const MULTIHASH_SHA256_CHUNKED: u64 = 0xb510;

/// SHA256 chunked checksum wrapper
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sha256ChunkedHash(Multihash<256>);

impl Sha256ChunkedHash {
    /// Get the inner multihash
    pub fn multihash(&self) -> &Multihash<256> {
        &self.0
    }

    /// Get the algorithm code
    pub fn algorithm(&self) -> u64 {
        self.0.code()
    }

    /// Get the digest bytes
    pub fn digest(&self) -> &[u8] {
        self.0.digest()
    }
}

// From/TryFrom conversions for Sha256ChunkedHash
impl From<Sha256ChunkedHash> for Multihash<256> {
    fn from(sha256_chunked: Sha256ChunkedHash) -> Self {
        sha256_chunked.0
    }
}

impl TryFrom<Multihash<256>> for Sha256ChunkedHash {
    type Error = Error;

    fn try_from(hash: Multihash<256>) -> Result<Self, Self::Error> {
        if hash.code() == MULTIHASH_SHA256_CHUNKED {
            Ok(Self(hash))
        } else {
            Err(Error::InvalidMultihash(format!(
                "Expected SHA256 chunked hash (code {:#06x}), got code {:#06x}",
                MULTIHASH_SHA256_CHUNKED,
                hash.code()
            )))
        }
    }
}

impl TryFrom<&str> for Sha256ChunkedHash {
    type Error = Error;

    fn try_from(base64_str: &str) -> Result<Self, Self::Error> {
        let hash_bytes = BASE64_STANDARD
            .decode(base64_str)
            .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
        let multihash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &hash_bytes)
            .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
        Ok(Self(multihash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn test_sha256_chunked_hash_algorithm() {
        let sha256_chunked_hash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test").unwrap();
        let sha256_chunked = Sha256ChunkedHash::try_from(sha256_chunked_hash).unwrap();
        assert_eq!(sha256_chunked.algorithm(), MULTIHASH_SHA256_CHUNKED);
    }

    #[test]
    fn test_sha256_chunked_hash_conversions() {
        // Create a SHA256 chunked hash and test conversions
        let original_hash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test_data").unwrap();
        let sha256_chunked = Sha256ChunkedHash::try_from(original_hash.clone()).unwrap();
        let converted_back: Multihash<256> = sha256_chunked.into();
        assert_eq!(original_hash, converted_back);
    }

    #[test]
    fn test_sha256_chunked_hash_conversion_error() {
        // Try to convert a SHA256 hash to Sha256ChunkedHash (should fail)
        let sha256_hash = multihash::Multihash::wrap(0x12, b"test").unwrap(); // SHA256 code
        let result = Sha256ChunkedHash::try_from(sha256_hash);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected SHA256 chunked hash"));
    }

    #[test]
    fn test_sha256_chunked_hash_try_from_str() {
        // Test valid base64 string
        let base64_str = "EfrtXWeClWPJ/IVKjQeAmMKhJV45/GcpjDm1IhvhJAY=";
        let hash = Sha256ChunkedHash::try_from(base64_str).unwrap();
        assert_eq!(hash.algorithm(), MULTIHASH_SHA256_CHUNKED);

        // Test that we can convert back to base64
        let encoded_back = BASE64_STANDARD.encode(hash.digest());
        assert_eq!(encoded_back, base64_str);

        // Test invalid base64 string
        let invalid_base64 = "invalid base64!";
        let result = Sha256ChunkedHash::try_from(invalid_base64);
        assert!(result.is_err());
    }
}
