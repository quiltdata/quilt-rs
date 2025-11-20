//! CRC64-NVMe checksum implementation

use multihash::Multihash;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
        // Add multibase prefix to plain base64 and decode with multibase
        let prefixed_value = format!("{}{}", multibase::Base::Base64Pad.code(), base64_str);
        let (_, hash_bytes) = multibase::decode(&prefixed_value)?;
        let multihash = Multihash::wrap(MULTIHASH_CRC64_NVME, &hash_bytes)?;
        Ok(Self(multihash))
    }
}

impl Serialize for Crc64Hash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "CRC64NVME")?;
        // Use multibase encoding but strip the prefix to maintain backward compatibility
        let multibase_encoded = multibase::encode(multibase::Base::Base64Pad, self.digest());
        let base64_value = &multibase_encoded[1..]; // Remove the multibase prefix
        map.serialize_entry("value", base64_value)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Crc64Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct Crc64HashVisitor;

        impl<'de> Visitor<'de> for Crc64HashVisitor {
            type Value = Crc64Hash;

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
                            if type_value != "CRC64NVME" {
                                return Err(de::Error::custom(format!(
                                    "Expected type 'CRC64NVME', got '{}'",
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

                // Add multibase prefix to plain base64 and decode with multibase
                let prefixed_value =
                    format!("{}{}", multibase::Base::Base64Pad.code(), value_field);
                let (_, hash_bytes) = multibase::decode(&prefixed_value)
                    .map_err(|e| de::Error::custom(format!("Invalid base64: {}", e)))?;
                let multihash = Multihash::wrap(MULTIHASH_CRC64_NVME, &hash_bytes)
                    .map_err(|e| de::Error::custom(format!("Invalid multihash: {}", e)))?;

                Ok(Crc64Hash(multihash))
            }
        }

        deserializer.deserialize_map(Crc64HashVisitor)
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
        let encoded_back = &multibase::encode(multibase::Base::Base64Pad, hash.digest())[1..];
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

    #[test]
    fn test_crc64_hash_serde() {
        let original_hash = multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test_data").unwrap();
        let crc64 = Crc64Hash::try_from(original_hash).unwrap();

        // Test serialization
        let serialized = serde_json::to_string(&crc64).unwrap();

        // Test deserialization
        let deserialized: Crc64Hash = serde_json::from_str(&serialized).unwrap();
        assert_eq!(crc64, deserialized);

        // Test specific format
        let test_json = r#"{"type":"CRC64NVME","value":"dGVzdCBkYXRh"}"#;
        let parsed: Crc64Hash = serde_json::from_str(test_json).unwrap();
        assert_eq!(
            &multibase::encode(multibase::Base::Base64Pad, parsed.digest())[1..],
            "dGVzdCBkYXRh"
        );

        // Test serialized format
        let expected_base64 = &multibase::encode(multibase::Base::Base64Pad, crc64.digest())[1..];
        assert!(serialized.contains("\"type\":\"CRC64NVME\""));
        assert!(serialized.contains(&format!("\"value\":\"{}\"", expected_base64)));
    }

    #[test]
    fn test_crc64_hash_serde_errors() {
        // Test invalid type
        let invalid_type = r#"{"type":"INVALID","value":"dGVzdA=="}"#;
        let result: Result<Crc64Hash, _> = serde_json::from_str(invalid_type);
        assert!(result.is_err());

        // Test invalid base64
        let invalid_base64 = r#"{"type":"CRC64NVME","value":"invalid_base64!"}"#;
        let result: Result<Crc64Hash, _> = serde_json::from_str(invalid_base64);
        assert!(result.is_err());

        // Test missing fields
        let missing_type = r#"{"value":"dGVzdA=="}"#;
        let result: Result<Crc64Hash, _> = serde_json::from_str(missing_type);
        assert!(result.is_err());

        let missing_value = r#"{"type":"CRC64NVME"}"#;
        let result: Result<Crc64Hash, _> = serde_json::from_str(missing_value);
        assert!(result.is_err());
    }
}
