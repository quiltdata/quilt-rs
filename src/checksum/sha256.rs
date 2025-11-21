//! SHA256 checksum implementation

use aws_smithy_checksums::ChecksumAlgorithm;
use multihash::Multihash;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

use crate::{Error, Res};

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

    /// Calculates legacy or single-chunk checksum from file or from single chunk
    pub async fn from_file<F: AsyncRead + Unpin>(file: F) -> Res<Self> {
        let mut hasher = ChecksumAlgorithm::Sha256.into_impl();
        let mut reader = BufReader::new(file);
        let mut buf = [0; 4096];
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[0..n]);
        }
        Ok(Self(Multihash::wrap(MULTIHASH_SHA256, &hasher.finalize())?))
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

impl TryFrom<&str> for Sha256Hash {
    type Error = Error;

    fn try_from(hex_str: &str) -> Result<Self, Self::Error> {
        // Add multibase prefix to plain hex and decode with multibase
        let prefixed_value = format!("{}{}", multibase::Base::Base16Lower.code(), hex_str);
        let (_, hash_bytes) = multibase::decode(&prefixed_value)?;
        let multihash = Multihash::wrap(MULTIHASH_SHA256, &hash_bytes)?;
        Ok(Self(multihash))
    }
}

impl fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use multibase encoding but strip the prefix to get plain hex
        let multibase_encoded = multibase::encode(multibase::Base::Base16Lower, self.digest());
        let hex_value = &multibase_encoded[1..]; // Remove the multibase prefix
        write!(f, "{}", hex_value)
    }
}

impl Serialize for Sha256Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "SHA256")?;
        map.serialize_entry("value", &self.to_string())?;
        map.end()
    }
}

#[derive(Deserialize)]
struct Sha256HashJson {
    #[serde(rename = "type")]
    hash_type: String,
    value: String,
}

impl<'de> Deserialize<'de> for Sha256Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        use serde::de::Unexpected;

        let json = Sha256HashJson::deserialize(deserializer)?;

        if json.hash_type != "SHA256" {
            return Err(Error::invalid_value(
                Unexpected::Str(&json.hash_type),
                &"SHA256",
            ));
        }

        Sha256Hash::try_from(json.value.as_str())
            .map_err(|_| Error::invalid_value(Unexpected::Str(&json.value), &"valid hex string"))
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

    #[test]
    fn test_sha256_hash_serde() {
        let original_hash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"test_data").unwrap();
        let sha256 = Sha256Hash::try_from(original_hash).unwrap();

        // Test serialization
        let serialized = serde_json::to_string(&sha256).unwrap();

        // Test deserialization
        let deserialized: Sha256Hash = serde_json::from_str(&serialized).unwrap();
        assert_eq!(sha256, deserialized);

        // Test specific format (plain hex)
        let test_json = r#"{"type":"SHA256","value":"deadbeef"}"#;
        let parsed: Sha256Hash = serde_json::from_str(test_json).unwrap();
        let multibase_with_prefix =
            multibase::encode(multibase::Base::Base16Lower, parsed.digest());
        assert_eq!(multibase_with_prefix, "fdeadbeef");

        // Test serialized format (should be hex without prefix)
        let expected_multibase = multibase::encode(multibase::Base::Base16Lower, sha256.digest());
        let expected_hex = &expected_multibase[1..]; // Remove 'f' prefix
        assert!(serialized.contains("\"type\":\"SHA256\""));
        assert!(serialized.contains(&format!("\"value\":\"{}\"", expected_hex)));
    }

    #[tokio::test]
    async fn test_sha256_hash_from_file() {
        let test_data = b"test file content";
        let cursor = std::io::Cursor::new(test_data);

        // Test from_file method
        let hash_from_method = Sha256Hash::from_file(cursor).await.unwrap();

        // Compare with manual creation
        let mut manual_hasher = ChecksumAlgorithm::Sha256.into_impl();
        manual_hasher.update(test_data);
        let expected_digest = manual_hasher.finalize();
        let expected_hash =
            Sha256Hash::try_from(Multihash::wrap(MULTIHASH_SHA256, &expected_digest).unwrap())
                .unwrap();

        assert_eq!(hash_from_method, expected_hash);
        assert_eq!(hash_from_method.algorithm(), MULTIHASH_SHA256);
        assert_eq!(hash_from_method.digest(), &expected_digest);
    }

    #[test]
    fn test_sha256_hash_serde_errors() {
        // Test invalid type
        let invalid_type = r#"{"type":"INVALID","value":"deadbeef"}"#;
        let result: Result<Sha256Hash, _> = serde_json::from_str(invalid_type);
        assert!(result.is_err());

        // Test invalid hex
        let invalid_hex = r#"{"type":"SHA256","value":"invalid_hex"}"#;
        let result: Result<Sha256Hash, _> = serde_json::from_str(invalid_hex);
        assert!(result.is_err());

        // Test missing fields
        let missing_type = r#"{"value":"deadbeef"}"#;
        let result: Result<Sha256Hash, _> = serde_json::from_str(missing_type);
        assert!(result.is_err());

        let missing_value = r#"{"type":"SHA256"}"#;
        let result: Result<Sha256Hash, _> = serde_json::from_str(missing_value);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_calculate_sha256_checksum() -> crate::Res {
        let bytes = crate::fixtures::objects::less_than_8mb();
        let hash = Sha256Hash::from_file(bytes).await?;

        assert_eq!(hash.multihash().code(), MULTIHASH_SHA256);

        let mut double_hasher = ChecksumAlgorithm::Sha256.into_impl();
        double_hasher.update(hash.digest());
        let double_hash = double_hasher.finalize();
        assert_eq!(
            hex::encode(double_hash),
            crate::fixtures::objects::LESS_THAN_8MB_HASH_HEX
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_from_bytes() -> crate::Res {
        let bytes = crate::fixtures::objects::less_than_8mb();
        let hash = Sha256Hash::from_file(&bytes[..]).await?;

        assert_eq!(hash.algorithm(), MULTIHASH_SHA256);
        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_hash_conversions_from_file() -> crate::Res {
        let bytes = crate::fixtures::objects::less_than_8mb();

        // Test Sha256Hash conversions
        let sha256 = Sha256Hash::from_file(&bytes[..]).await?;
        let multihash: Multihash<256> = sha256.clone().into();
        let back_to_sha256 = Sha256Hash::try_from(multihash)?;
        assert_eq!(sha256, back_to_sha256);

        Ok(())
    }

    #[test]
    fn test_sha256_hash_display() {
        let original_hash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"test_data").unwrap();
        let sha256 = Sha256Hash::try_from(original_hash).unwrap();

        // Test Display implementation
        let display_string = format!("{}", sha256);

        // Should be hex without multibase prefix
        let expected_hex = hex::encode(b"test_data");
        assert_eq!(display_string, expected_hex);

        // Test that to_string() works (provided by Display)
        assert_eq!(sha256.to_string(), expected_hex);
    }

    #[test]
    fn test_sha256_hash_try_from_str() {
        // Test valid hex string
        let hex_str = "deadbeef";
        let hash = Sha256Hash::try_from(hex_str).unwrap();
        assert_eq!(hash.algorithm(), MULTIHASH_SHA256);

        // Test that we can convert back to string (round-trip)
        let back_to_string = hash.to_string();
        assert_eq!(back_to_string, hex_str);

        // Test invalid hex string (contains non-hex characters)
        let invalid_hex = "invalid_hex_string!";
        let result = Sha256Hash::try_from(invalid_hex);
        assert!(result.is_err());

        // Test odd-length hex string (multibase might handle this differently)
        let odd_hex = "abc";
        let _result = Sha256Hash::try_from(odd_hex);
        // This might succeed or fail depending on multibase implementation
        // Let's just check that it doesn't panic
    }
}
