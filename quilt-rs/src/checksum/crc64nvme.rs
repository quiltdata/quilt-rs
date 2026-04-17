//! CRC64-NVMe checksum implementation

use aws_smithy_checksums::ChecksumAlgorithm;
use multihash::Multihash;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use std::fmt;
use tokio::fs::File;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;

use crate::checksum::hash::Hash;
use crate::Error;
use crate::error::ChecksumError;
use crate::Res;

/// Multihash code for CRC64-NVMe
pub const MULTIHASH_CRC64_NVME: u64 = 0x0165;

/// CRC64-NVMe checksum wrapper
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crc64Hash(Multihash<256>);

impl Default for Crc64Hash {
    fn default() -> Self {
        Self(Multihash::wrap(MULTIHASH_CRC64_NVME, &[]).unwrap())
    }
}

impl Crc64Hash {
    /// Calculates CRC64-NVMe checksum from any async reader
    pub async fn from_async_read<F: AsyncRead + Unpin>(file: F) -> Res<Self> {
        let mut hasher = ChecksumAlgorithm::Crc64Nvme.into_impl();
        let mut reader = BufReader::new(file);
        let mut buf = [0; 4096];

        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[0..n]);
        }

        Ok(Self(Multihash::wrap(
            MULTIHASH_CRC64_NVME,
            &hasher.finalize(),
        )?))
    }
}

impl crate::checksum::Hash for Crc64Hash {
    /// Get the inner multihash
    fn multihash(&self) -> &Multihash<256> {
        &self.0
    }

    /// Calculates CRC64-NVMe checksum from a file
    async fn from_file(file: File) -> Res<Self> {
        Self::from_async_read(file).await
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
            Err(Error::Checksum(ChecksumError::InvalidMultihash(format!(
                "Expected CRC64-NVMe hash (code {:#06x}), got code {:#06x}",
                MULTIHASH_CRC64_NVME,
                hash.code()
            ))))
        }
    }
}

impl TryFrom<&str> for Crc64Hash {
    type Error = Error;

    fn try_from(base64_str: &str) -> Result<Self, Self::Error> {
        // Add multibase prefix to plain base64 and decode with multibase
        let prefixed_value = format!("{}{}", multibase::Base::Base64Pad.code(), base64_str);
        let (_, hash_bytes) = multibase::decode(&prefixed_value)?;
        Multihash::wrap(MULTIHASH_CRC64_NVME, &hash_bytes)?.try_into()
    }
}

impl TryFrom<&String> for Crc64Hash {
    type Error = Error;

    fn try_from(base64_str: &String) -> Result<Self, Self::Error> {
        base64_str.as_str().try_into()
    }
}

impl fmt::Display for Crc64Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use multibase encoding but strip the prefix to get plain base64
        let multibase_encoded = multibase::encode(multibase::Base::Base64Pad, self.digest());
        let base64_value = &multibase_encoded[1..]; // Remove the multibase prefix
        write!(f, "{}", base64_value)
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
        map.serialize_entry("value", &self.to_string())?;
        map.end()
    }
}

#[derive(Deserialize)]
struct Crc64HashJson {
    #[serde(rename = "type")]
    hash_type: String,
    value: String,
}

impl<'de> Deserialize<'de> for Crc64Hash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        use serde::de::Unexpected;

        let json = Crc64HashJson::deserialize(deserializer)?;

        if json.hash_type != "CRC64NVME" {
            return Err(Error::invalid_value(
                Unexpected::Str(&json.hash_type),
                &"CRC64NVME",
            ));
        }

        Crc64Hash::try_from(json.value.as_str())
            .map_err(|_| Error::invalid_value(Unexpected::Str(&json.value), &"valid base64 string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::path::Path;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::Storage;

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
        let crc64 = Crc64Hash::try_from(original_hash).unwrap();
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
    fn test_crc64_hash_display() {
        let original_hash = multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test_data").unwrap();
        let crc64 = Crc64Hash::try_from(original_hash).unwrap();

        // Test Display implementation
        let display_string = format!("{}", crc64);

        // Should be base64 without multibase prefix
        let expected_base64 = &multibase::encode(multibase::Base::Base64Pad, b"test_data")[1..];
        assert_eq!(display_string, expected_base64);

        // Test that to_string() works (provided by Display)
        assert_eq!(crc64.to_string(), expected_base64);
    }

    #[test(tokio::test)]
    async fn test_crc64_hash_from_file() -> crate::Res {
        let storage = MockStorage::default();
        let test_data = crate::fixtures::objects::less_than_8mb();
        let test_path = Path::new("test_file.txt");

        // Write test data to mock storage
        storage
            .write_byte_stream(test_path, ByteStream::from_static(test_data))
            .await?;

        // Test from_file method
        let file = storage.open_file(test_path).await?;
        let hash_from_file = Crc64Hash::from_async_read(file).await?;
        assert_eq!(hash_from_file.algorithm(), MULTIHASH_CRC64_NVME);

        // Test that digest is 8 bytes (CRC64 size)
        assert_eq!(hash_from_file.digest().len(), 8);

        // Test with known fixture data - the hash should be consistent
        let expected_hash = "CRSFynAYcw4="; // CRC64 hash of less_than_8mb fixture
        assert_eq!(hash_from_file.to_string(), expected_hash);

        // Test that different data produces different hashes
        let different_data = crate::fixtures::objects::zero_bytes();
        let different_path = Path::new("different_file.txt");
        storage
            .write_byte_stream(different_path, ByteStream::from_static(different_data))
            .await?;

        let different_file = storage.open_file(different_path).await?;
        let different_hash = Crc64Hash::from_async_read(different_file).await?;
        assert_ne!(hash_from_file, different_hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_crc64_nvme_algorithm() -> crate::Res {
        let storage = MockStorage::default();

        // Test with known data to verify CRC64-NVMe implementation
        let test_data = b"hello world";
        let test_path = Path::new("hello_world.txt");
        storage
            .write_byte_stream(test_path, ByteStream::from_static(test_data))
            .await?;

        let file1 = storage.open_file(test_path).await?;
        let hash = Crc64Hash::from_async_read(file1).await?;

        // Verify it's exactly 8 bytes
        assert_eq!(hash.digest().len(), 8);

        // Test consistency - same input should give same output
        let file2 = storage.open_file(test_path).await?;
        let hash2 = Crc64Hash::from_async_read(file2).await?;
        assert_eq!(hash, hash2);

        // Different input should give different output
        let different_data = b"hello world!";
        let different_path = Path::new("hello_world_exclamation.txt");
        storage
            .write_byte_stream(different_path, ByteStream::from_static(different_data))
            .await?;

        let file3 = storage.open_file(different_path).await?;
        let hash3 = Crc64Hash::from_async_read(file3).await?;
        assert_ne!(hash, hash3);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_crc64_hash_user_settings_fixture() -> crate::Res {
        let storage = MockStorage::default();

        // Write fixture content to mock storage
        let test_path = Path::new("user-settings.mkfg");
        let fixture_path = Path::new("fixtures/user-settings.mkfg");
        storage
            .write_byte_stream(test_path, ByteStream::from_path(fixture_path).await?)
            .await?;

        // Calculate hash from file
        let file = storage.open_file(test_path).await?;
        let hash = Crc64Hash::from_async_read(file).await?;

        // Verify the expected base64 hash
        let expected_base64 = "LZmmpqbBItw=";
        assert_eq!(hash.to_string(), expected_base64);

        Ok(())
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
