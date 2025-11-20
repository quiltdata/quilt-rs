//! SHA256 chunked checksum implementation

use multihash::Multihash;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::checksum::{get_checksum_chunksize_and_parts, Sha256Hash};
use crate::{Error, Res};

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

    /// Calculates chunksum from a file
    pub async fn from_file<F: AsyncRead + Unpin + Send>(file: F, length: u64) -> Res<Self> {
        let (chunksize, num_parts) = get_checksum_chunksize_and_parts(length);

        let mut sha256_hasher = Sha256::new();

        let mut chunk = file.take(0);
        for _ in 0..num_parts {
            chunk.set_limit(chunksize);
            sha256_hasher.update(Sha256Hash::from_file(&mut chunk).await?.digest());
        }

        Sha256ChunkedHash::try_from(Multihash::wrap(
            MULTIHASH_SHA256_CHUNKED,
            &sha256_hasher.finalize(),
        )?)
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
        // Add multibase prefix to plain base64 and decode with multibase
        let prefixed_value = format!("{}{}", multibase::Base::Base64Pad.code(), base64_str);
        let (_, hash_bytes) = multibase::decode(&prefixed_value)?;
        let multihash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &hash_bytes)?;
        Ok(Self(multihash))
    }
}

impl std::fmt::Display for Sha256ChunkedHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Use multibase encoding but strip the prefix to get plain base64
        let multibase_encoded = multibase::encode(multibase::Base::Base64Pad, self.digest());
        let base64_value = &multibase_encoded[1..]; // Remove the multibase prefix
        write!(f, "{}", base64_value)
    }
}

impl Serialize for Sha256ChunkedHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "sha2-256-chunked")?;
        map.serialize_entry("value", &self.to_string())?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Sha256ChunkedHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct Sha256ChunkedHashVisitor;

        impl<'de> Visitor<'de> for Sha256ChunkedHashVisitor {
            type Value = Sha256ChunkedHash;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map with type and value fields")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut type_field = None;
                let mut value_field = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "type" => {
                            if type_field.is_some() {
                                return Err(de::Error::duplicate_field("type"));
                            }
                            let type_value: String = map.next_value()?;
                            if type_value != "sha2-256-chunked" {
                                return Err(de::Error::custom(format!(
                                    "Expected type 'sha2-256-chunked', got '{}'",
                                    type_value
                                )));
                            }
                            type_field = Some(type_value);
                        }
                        "value" => {
                            if value_field.is_some() {
                                return Err(de::Error::duplicate_field("value"));
                            }
                            value_field = Some(map.next_value::<String>()?);
                        }
                        _ => {
                            let _: serde::de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                if type_field.is_none() {
                    return Err(de::Error::missing_field("type"));
                }
                let value_field = value_field.ok_or_else(|| de::Error::missing_field("value"))?;

                Sha256ChunkedHash::try_from(value_field.as_str())
                    .map_err(|e| de::Error::custom(format!("Invalid SHA256 chunked hash: {}", e)))
            }
        }

        deserializer.deserialize_map(Sha256ChunkedHashVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let encoded_back = &multibase::encode(multibase::Base::Base64Pad, hash.digest())[1..];
        assert_eq!(encoded_back, base64_str);

        // Test invalid base64 string
        let invalid_base64 = "invalid base64!";
        let result = Sha256ChunkedHash::try_from(invalid_base64);
        assert!(result.is_err());
    }

    #[test]
    fn test_sha256_chunked_hash_serde() {
        let original_hash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test_data").unwrap();
        let sha256_chunked = Sha256ChunkedHash::try_from(original_hash).unwrap();

        // Test serialization
        let serialized = serde_json::to_string(&sha256_chunked).unwrap();

        // Test deserialization
        let deserialized: Sha256ChunkedHash = serde_json::from_str(&serialized).unwrap();
        assert_eq!(sha256_chunked, deserialized);

        // Test specific format
        let test_json = r#"{"type":"sha2-256-chunked","value":"dGVzdCBkYXRh"}"#;
        let parsed: Sha256ChunkedHash = serde_json::from_str(test_json).unwrap();
        assert_eq!(
            &multibase::encode(multibase::Base::Base64Pad, parsed.digest())[1..],
            "dGVzdCBkYXRh"
        );

        // Test serialized format
        let expected_base64 =
            &multibase::encode(multibase::Base::Base64Pad, sha256_chunked.digest())[1..];
        assert!(serialized.contains("\"type\":\"sha2-256-chunked\""));
        assert!(serialized.contains(&format!("\"value\":\"{}\"", expected_base64)));
    }

    #[test]
    fn test_sha256_chunked_hash_display() {
        let original_hash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test_data").unwrap();
        let sha256_chunked = Sha256ChunkedHash::try_from(original_hash).unwrap();

        // Test Display implementation
        let display_string = format!("{}", sha256_chunked);

        // Should be base64 without multibase prefix
        let expected_base64 = &multibase::encode(multibase::Base::Base64Pad, b"test_data")[1..];
        assert_eq!(display_string, expected_base64);

        // Test that to_string() works (provided by Display)
        assert_eq!(sha256_chunked.to_string(), expected_base64);
    }

    #[test]
    fn test_sha256_chunked_hash_serde_errors() {
        // Test invalid type
        let invalid_type = r#"{"type":"INVALID","value":"dGVzdA=="}"#;
        let result: Result<Sha256ChunkedHash, _> = serde_json::from_str(invalid_type);
        assert!(result.is_err());

        // Test invalid base64
        let invalid_base64 = r#"{"type":"sha2-256-chunked","value":"invalid_base64!"}"#;
        let result: Result<Sha256ChunkedHash, _> = serde_json::from_str(invalid_base64);
        assert!(result.is_err());

        // Test missing fields
        let missing_type = r#"{"value":"dGVzdA=="}"#;
        let result: Result<Sha256ChunkedHash, _> = serde_json::from_str(missing_type);
        assert!(result.is_err());

        let missing_value = r#"{"type":"sha2-256-chunked"}"#;
        let result: Result<Sha256ChunkedHash, _> = serde_json::from_str(missing_value);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_files_less_8mb() -> crate::Res {
        use base64::{prelude::BASE64_STANDARD, Engine};

        let bytes = crate::fixtures::objects::less_than_8mb();
        let hash = Sha256ChunkedHash::from_file(bytes, bytes.len() as u64).await?;
        assert_eq!(hash.multihash().code(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            crate::fixtures::objects::LESS_THAN_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_files_equal_to_8mb() -> crate::Res {
        use base64::{prelude::BASE64_STANDARD, Engine};

        let bytes = crate::fixtures::objects::equal_to_8mb();
        let hash = Sha256ChunkedHash::from_file(bytes.as_ref(), bytes.len() as u64).await?;
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            crate::fixtures::objects::EQUAL_TO_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_chunked_empty() -> crate::Res {
        use base64::{prelude::BASE64_STANDARD, Engine};

        let bytes = crate::fixtures::objects::zero_bytes();
        let hash = Sha256ChunkedHash::from_file(bytes, bytes.len() as u64).await?;
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            crate::fixtures::objects::ZERO_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_files_bigger_than_8mb() -> crate::Res {
        use base64::{prelude::BASE64_STANDARD, Engine};

        let bytes = crate::fixtures::objects::more_than_8mb();
        let hash = Sha256ChunkedHash::from_file(bytes.as_ref(), bytes.len() as u64).await?;
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            crate::fixtures::objects::MORE_THAN_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_chunked_from_bytes() -> crate::Res {
        use base64::{prelude::BASE64_STANDARD, Engine};

        let bytes = crate::fixtures::objects::less_than_8mb();
        let hash = Sha256ChunkedHash::from_file(bytes, bytes.len() as u64).await?;

        assert_eq!(hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            crate::fixtures::objects::LESS_THAN_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_chunked_hash_conversions_from_file() -> crate::Res {
        let bytes = crate::fixtures::objects::less_than_8mb();

        // Test Sha256ChunkedHash conversions
        let sha256_chunked = Sha256ChunkedHash::from_file(bytes, bytes.len() as u64).await?;
        let multihash: Multihash<256> = sha256_chunked.clone().into();
        let back_to_sha256_chunked = Sha256ChunkedHash::try_from(multihash)?;
        assert_eq!(sha256_chunked, back_to_sha256_chunked);

        Ok(())
    }
}
