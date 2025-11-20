//! SHA256 checksum implementation

use multihash::Multihash;

use crate::Error;

/// Multihash code for legacy or single-chunked checksums
pub const MULTIHASH_SHA256: u64 = 0x12;

/// SHA256 (legacy) checksum wrapper
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sha256Hash(Multihash<256>);

impl Sha256Hash {
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

// From/TryFrom conversions for Sha256Hash
impl From<Sha256Hash> for Multihash<256> {
    fn from(sha256: Sha256Hash) -> Self {
        sha256.0
    }
}

impl TryFrom<Multihash<256>> for Sha256Hash {
    type Error = Error;

    fn try_from(hash: Multihash<256>) -> Result<Self, Self::Error> {
        if hash.code() == MULTIHASH_SHA256 {
            Ok(Self(hash))
        } else {
            Err(Error::InvalidMultihash(format!(
                "Expected SHA256 hash (code {:#06x}), got code {:#06x}",
                MULTIHASH_SHA256,
                hash.code()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hash_algorithm() {
        let sha256_hash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"test").unwrap();
        let sha256 = Sha256Hash::try_from(sha256_hash).unwrap();
        assert_eq!(sha256.algorithm(), MULTIHASH_SHA256);
    }

    #[test]
    fn test_sha256_hash_conversions() {
        // Create a SHA256 hash and test conversions
        let original_hash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"test_data").unwrap();
        let sha256 = Sha256Hash::try_from(original_hash.clone()).unwrap();
        let converted_back: Multihash<256> = sha256.into();
        assert_eq!(original_hash, converted_back);
    }

    #[test]
    fn test_sha256_hash_conversion_error() {
        // Try to convert a SHA256Chunked hash to Sha256Hash (should fail)
        let sha256_chunked_hash = multihash::Multihash::wrap(0xb510, b"test").unwrap(); // SHA256_CHUNKED code
        let result = Sha256Hash::try_from(sha256_chunked_hash);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected SHA256 hash"));
    }
}
