//! SHA256 checksum implementation

use multibase;
use multihash::Multihash;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

impl Serialize for Sha256Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "SHA256")?;
        // Use multibase encoding but strip the prefix
        let base = multibase::Base::Base16Lower;
        let multibase_encoded = multibase::encode(base, self.0.digest());
        let hex_value = &multibase_encoded[1..]; // Remove the multibase prefix
        map.serialize_entry("value", hex_value)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Sha256Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct Sha256HashVisitor;

        impl<'de> Visitor<'de> for Sha256HashVisitor {
            type Value = Sha256Hash;

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
                            if type_value != "SHA256" {
                                return Err(de::Error::custom(format!(
                                    "Expected type 'SHA256', got '{}'",
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

                // Add multibase prefix to plain hex and decode with multibase
                let prefixed_value =
                    format!("{}{}", multibase::Base::Base16Lower.code(), value_field);
                let (_, hash_bytes) = multibase::decode(&prefixed_value)
                    .map_err(|e| de::Error::custom(format!("Invalid hex: {}", e)))?;
                let multihash = Multihash::wrap(MULTIHASH_SHA256, &hash_bytes)
                    .map_err(|e| de::Error::custom(format!("Invalid multihash: {}", e)))?;

                Ok(Sha256Hash(multihash))
            }
        }

        deserializer.deserialize_map(Sha256HashVisitor)
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
}
