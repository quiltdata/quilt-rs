//! CRC64-NVMe checksum implementation

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;

use crate::Error;

/// Multihash code for CRC64-NVMe
pub const MULTIHASH_CRC64_NVME: u64 = 0x0165;

/// CRC64-NVMe checksum wrapper
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crc64Hash(Multihash<256>);

impl Crc64Hash {
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

// From/TryFrom conversions for Crc64Hash
impl From<Crc64Hash> for Multihash<256> {
    fn from(crc64: Crc64Hash) -> Self {
        crc64.0
    }
}

impl TryFrom<Multihash<256>> for Crc64Hash {
    type Error = Error;

    fn try_from(hash: Multihash<256>) -> Result<Self, Self::Error> {
        if hash.code() == MULTIHASH_CRC64_NVME {
            Ok(Self(hash))
        } else {
            Err(Error::InvalidMultihash(format!(
                "Expected CRC64-NVMe hash (code {:#06x}), got code {:#06x}",
                MULTIHASH_CRC64_NVME,
                hash.code()
            )))
        }
    }
}

impl TryFrom<&str> for Crc64Hash {
    type Error = Error;

    fn try_from(base64_str: &str) -> Result<Self, Self::Error> {
        let hash_bytes = BASE64_STANDARD
            .decode(base64_str)
            .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
        let multihash = Multihash::wrap(MULTIHASH_CRC64_NVME, &hash_bytes)
            .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
        Ok(Self(multihash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc64_hash_algorithm() {
        let crc64_hash = multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test").unwrap();
        let crc64 = Crc64Hash::try_from(crc64_hash).unwrap();
        assert_eq!(crc64.algorithm(), MULTIHASH_CRC64_NVME);
    }

    #[test]
    fn test_crc64_hash_try_from_str() {
        // Test valid base64 string
        let base64_str = "dGVzdCBkYXRh"; // "test data" in base64
        let hash = Crc64Hash::try_from(base64_str).unwrap();
        assert_eq!(hash.algorithm(), MULTIHASH_CRC64_NVME);

        // Test that we can convert back to base64
        let encoded_back = BASE64_STANDARD.encode(hash.digest());
        assert_eq!(encoded_back, base64_str);

        // Test invalid base64 string
        let invalid_base64 = "invalid base64!";
        let result = Crc64Hash::try_from(invalid_base64);
        assert!(result.is_err());
    }

    #[test]
    fn test_crc64_hash_conversions() {
        // Create a CRC64-NVMe hash and test conversions
        let original_hash = multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test_data").unwrap();
        let crc64 = Crc64Hash::try_from(original_hash.clone()).unwrap();
        let converted_back: Multihash<256> = crc64.into();
        assert_eq!(original_hash, converted_back);
    }

    #[test]
    fn test_crc64_hash_conversion_error() {
        // Try to convert a SHA256 hash to CRC64Hash (should fail)
        let sha256_hash = multihash::Multihash::wrap(0x12, b"test").unwrap(); // SHA256 code
        let result = Crc64Hash::try_from(sha256_hash);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected CRC64-NVMe hash"));
    }
}
