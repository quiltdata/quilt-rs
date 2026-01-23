//! This module contains helpers and structs for creating and managing checkums.

use multihash::Multihash;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::io::storage::Storage;
use crate::manifest::Row;
use crate::Error;
use crate::Res;

mod crc64nvme;
mod hash;
mod remote;
mod sha256;
mod sha256_chunked;

pub use crc64nvme::Crc64Hash;
pub use crc64nvme::MULTIHASH_CRC64_NVME;
pub use hash::Hash;
pub use remote::hash_sha256_checksum;
pub use sha256::Sha256Hash;
pub use sha256::MULTIHASH_SHA256;
pub use sha256_chunked::get_checksum_chunksize_and_parts;
pub use sha256_chunked::Sha256ChunkedHash;
pub use sha256_chunked::MULTIHASH_SHA256_CHUNKED;

/// Type-safe container for object's checksum using struct types
/// You can convert it to or from `Multihash<256>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObjectHash {
    /// Legacy SHA256 checksum
    Sha256(Sha256Hash),
    /// Chunked SHA256 checksum
    Sha256Chunked(Sha256ChunkedHash),
    /// CRC64-NVMe checksum
    Crc64(Crc64Hash),
}

/// Refresh hash for a file using the same algorithm as the reference row
/// Returns None if hash hasn't changed, Some(Row) if it has changed
pub async fn refresh_hash(
    storage: &impl Storage,
    path: &PathBuf,
    row: Row,
) -> Res<Option<Row>> {
    let file = storage.open_file(path).await?;
    let file_metadata = file.metadata().await?;
    let size = file_metadata.len();

    match row.hash.code() {
        MULTIHASH_CRC64_NVME => Ok(Crc64Hash::from_file(file).await?.into()),
        MULTIHASH_SHA256 => Ok(Sha256Hash::from_file(file).await?.into()),
        MULTIHASH_SHA256_CHUNKED => Ok(Sha256ChunkedHash::from_file(file).await?.into()),
        code => Err(Error::InvalidMultihash(format!(
            "Wrong multihash type {}",
            code
        ))),
    }
    .map(|hash| {
        if row.hash == hash {
            None
        } else {
            Some(Row {
                hash,
                size,
                ..row.clone()
            })
        }
    })
}

/// Calculate hash for a file using the algorithm specified by host config
pub async fn calculate_hash(
    storage: &impl Storage,
    path: &PathBuf,
    logical_key: PathBuf,
    host_config: &HostConfig,
) -> Res<Row> {
    let file = storage.open_file(path).await?;
    let file_metadata = file.metadata().await?;
    let size = file_metadata.len();

    let hash = match host_config.checksums {
        HostChecksums::Crc64 => Crc64Hash::from_file(file).await?.into(),
        HostChecksums::Sha256Chunked => Sha256ChunkedHash::from_file(file).await?.into(),
    };

    Ok(Row {
        name: logical_key,
        size,
        hash,
        ..Row::default()
    })
}

/// Verify hash for a file and optionally recalculate with host's preferred algorithm
/// Returns None if hash hasn't changed, Some(Row) if it has changed
pub async fn verify_hash(
    storage: &impl Storage,
    path: &PathBuf,
    logical_key: &PathBuf,
    row: Row,
    host_config: &HostConfig,
) -> Res<Option<Row>> {
    if let Some(modified) = refresh_hash(storage, path, row).await? {
        // File has changed, check if we need to recalculate with host's preferred algorithm
        if modified.hash.code() == host_config.checksums.algorithm_code() {
            // Already using the correct algorithm, no need to recalculate
            Ok(Some(modified))
        } else {
            // Need to recalculate with host's preferred algorithm
            Ok(Some(
                calculate_hash(storage, path, logical_key.clone(), host_config).await?,
            ))
        }
    } else {
        Ok(None)
    }
}

impl TryFrom<Multihash<256>> for ObjectHash {
    type Error = crate::Error;

    fn try_from(multihash: Multihash<256>) -> Result<Self, Self::Error> {
        match multihash.code() {
            MULTIHASH_SHA256 => Ok(ObjectHash::Sha256(Sha256Hash::try_from(multihash)?)),
            MULTIHASH_SHA256_CHUNKED => Ok(ObjectHash::Sha256Chunked(Sha256ChunkedHash::try_from(
                multihash,
            )?)),
            MULTIHASH_CRC64_NVME => Ok(ObjectHash::Crc64(Crc64Hash::try_from(multihash)?)),
            _ => Err(crate::Error::InvalidMultihash(format!(
                "Unsupported multihash code: {:#06x}",
                multihash.code()
            ))),
        }
    }
}

impl From<ObjectHash> for Multihash<256> {
    fn from(object_hash: ObjectHash) -> Self {
        match object_hash {
            ObjectHash::Sha256(hash) => hash.into(),
            ObjectHash::Sha256Chunked(hash) => hash.into(),
            ObjectHash::Crc64(hash) => hash.into(),
        }
    }
}

impl From<Sha256Hash> for ObjectHash {
    fn from(hash: Sha256Hash) -> Self {
        ObjectHash::Sha256(hash)
    }
}

impl From<Sha256ChunkedHash> for ObjectHash {
    fn from(hash: Sha256ChunkedHash) -> Self {
        ObjectHash::Sha256Chunked(hash)
    }
}

impl From<Crc64Hash> for ObjectHash {
    fn from(hash: Crc64Hash) -> Self {
        ObjectHash::Crc64(hash)
    }
}

impl ObjectHash {
    /// Get the inner multihash
    pub fn multihash(&self) -> &Multihash<256> {
        match self {
            ObjectHash::Sha256(hash) => hash.multihash(),
            ObjectHash::Sha256Chunked(hash) => hash.multihash(),
            ObjectHash::Crc64(hash) => hash.multihash(),
        }
    }

    /// Get the algorithm code
    pub fn algorithm(&self) -> u64 {
        match self {
            ObjectHash::Sha256(hash) => hash.algorithm(),
            ObjectHash::Sha256Chunked(hash) => hash.algorithm(),
            ObjectHash::Crc64(hash) => hash.algorithm(),
        }
    }

    /// Get the digest bytes
    pub fn digest(&self) -> &[u8] {
        match self {
            ObjectHash::Sha256(hash) => hash.digest(),
            ObjectHash::Sha256Chunked(hash) => hash.digest(),
            ObjectHash::Crc64(hash) => hash.digest(),
        }
    }
}

impl fmt::Display for ObjectHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectHash::Sha256(hash) => hash.fmt(f),
            ObjectHash::Sha256Chunked(hash) => hash.fmt(f),
            ObjectHash::Crc64(hash) => hash.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use std::path::Path;

    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::Storage;
    use crate::Error;
    use crate::Res;

    #[test]
    fn test_conversion_errors() {
        // Create a SHA256 hash and try to convert it to SHA256Chunked (should fail)
        let sha256_hash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"test").unwrap();
        let result = Sha256ChunkedHash::try_from(sha256_hash);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected SHA256 chunked hash"));

        // Create a SHA256Chunked hash and try to convert it to SHA256 (should fail)
        let sha256_chunked_hash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test").unwrap();
        let result = Sha256Hash::try_from(sha256_chunked_hash);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Expected SHA256 hash"));
    }

    #[test]
    fn test_object_hash_display() -> Res {
        // Test SHA256 display (hex format)
        let object_hash = ObjectHash::Sha256(Sha256Hash::try_from("deadbeef")?);
        assert_eq!(object_hash.to_string(), "deadbeef");

        // Test SHA256Chunked display (base64 format)
        let object_hash = ObjectHash::Sha256Chunked(Sha256ChunkedHash::try_from("Zm9vYmFy")?);
        assert_eq!(object_hash.to_string(), "Zm9vYmFy");

        // Test CRC64 display (base64 format)
        let object_hash = ObjectHash::Crc64(Crc64Hash::try_from("aGVsbG8gd29ybGQ=")?);
        assert_eq!(object_hash.to_string(), "aGVsbG8gd29ybGQ=");

        Ok(())
    }

    #[test]
    fn test_object_hash_conversions() -> Res {
        // Test SHA256 conversion
        let sha256_multihash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"test_data").unwrap();
        let object_hash = ObjectHash::try_from(sha256_multihash.clone())?;
        let back_to_multihash: Multihash<256> = object_hash.clone().into();
        assert_eq!(sha256_multihash, back_to_multihash);
        assert_eq!(object_hash.algorithm(), MULTIHASH_SHA256);

        // Test SHA256Chunked conversion
        let sha256_chunked_multihash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test_data").unwrap();
        let object_hash = ObjectHash::try_from(sha256_chunked_multihash.clone())?;
        let back_to_multihash: Multihash<256> = object_hash.clone().into();
        assert_eq!(sha256_chunked_multihash, back_to_multihash);
        assert_eq!(object_hash.algorithm(), MULTIHASH_SHA256_CHUNKED);

        // Test CRC64 conversion
        let crc64_multihash =
            multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test_data").unwrap();
        let object_hash = ObjectHash::try_from(crc64_multihash.clone())?;
        let back_to_multihash: Multihash<256> = object_hash.clone().into();
        assert_eq!(crc64_multihash, back_to_multihash);
        assert_eq!(object_hash.algorithm(), MULTIHASH_CRC64_NVME);

        let invalid_multihash = multihash::Multihash::wrap(0x9999, b"invalid_data").unwrap();
        let result = ObjectHash::try_from(invalid_multihash);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unsupported multihash code: 0x9999"));

        Ok(())
    }

    #[test]
    fn test_object_hash_from_individual_types() -> Res {
        // Test from individual hash types
        let sha256_hash =
            Sha256Hash::try_from(multihash::Multihash::wrap(MULTIHASH_SHA256, b"test").unwrap())?;
        let object_hash: ObjectHash = sha256_hash.clone().into();
        assert_eq!(object_hash.algorithm(), MULTIHASH_SHA256);

        let sha256_chunked_hash = Sha256ChunkedHash::try_from(
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test").unwrap(),
        )?;
        let object_hash: ObjectHash = sha256_chunked_hash.clone().into();
        assert_eq!(object_hash.algorithm(), MULTIHASH_SHA256_CHUNKED);

        let crc64_hash = Crc64Hash::try_from(
            multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test").unwrap(),
        )?;
        let object_hash: ObjectHash = crc64_hash.clone().into();
        assert_eq!(object_hash.algorithm(), MULTIHASH_CRC64_NVME);

        Ok(())
    }

    #[test]
    fn test_object_hash_serde() -> Res {
        // Test SHA256 serde
        let sha256_hash = Sha256Hash::try_from(
            multihash::Multihash::wrap(MULTIHASH_SHA256, b"test_data").unwrap(),
        )?;
        let object_hash = ObjectHash::Sha256(sha256_hash);
        let serialized = serde_json::to_string(&object_hash)?;
        let deserialized: ObjectHash = serde_json::from_str(&serialized)?;
        assert_eq!(object_hash, deserialized);

        // Test SHA256Chunked serde
        let sha256_chunked_hash = Sha256ChunkedHash::try_from(
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"test_data").unwrap(),
        )?;
        let object_hash = ObjectHash::Sha256Chunked(sha256_chunked_hash);
        let serialized = serde_json::to_string(&object_hash)?;
        let deserialized: ObjectHash = serde_json::from_str(&serialized)?;
        assert_eq!(object_hash, deserialized);

        // Test CRC64 serde
        let crc64_hash = Crc64Hash::try_from(
            multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"test_data").unwrap(),
        )?;
        let object_hash = ObjectHash::Crc64(crc64_hash);
        let serialized = serde_json::to_string(&object_hash)?;
        let deserialized: ObjectHash = serde_json::from_str(&serialized)?;
        assert_eq!(object_hash, deserialized);

        Ok(())
    }

    #[test]
    fn test_object_hash_json_format_translation() -> Res {
        // Test SHA256 JSON format translation
        let sha256_json = r#"{"type":"SHA256","value":"7465737464617461000000000000000000000000000000000000000000000000"}"#;
        let object_hash: ObjectHash = serde_json::from_str(sha256_json)?;
        match object_hash {
            ObjectHash::Sha256(hash) => {
                assert_eq!(hash.algorithm(), MULTIHASH_SHA256);
                assert_eq!(
                    hex::encode(hash.digest()),
                    "7465737464617461000000000000000000000000000000000000000000000000"
                );
            }
            _ => {
                return Err(crate::Error::InvalidMultihash(
                    "Expected ObjectHash::Sha256 variant".to_string(),
                ))
            }
        }

        // Test SHA256Chunked JSON format translation
        let sha256_chunked_json =
            r#"{"type":"sha2-256-chunked","value":"dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA"}"#;
        let object_hash: ObjectHash = serde_json::from_str(sha256_chunked_json)?;
        match object_hash {
            ObjectHash::Sha256Chunked(hash) => {
                assert_eq!(hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
                assert_eq!(
                    &multibase::encode(multibase::Base::Base64Pad, hash.digest())[1..],
                    "dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA"
                );
            }
            _ => {
                return Err(crate::Error::InvalidMultihash(
                    "Expected ObjectHash::Sha256Chunked variant".to_string(),
                ))
            }
        }

        // Test CRC64 JSON format translation
        let crc64_json = r#"{"type":"CRC64NVME","value":"dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA"}"#;
        let object_hash: ObjectHash = serde_json::from_str(crc64_json)?;
        match object_hash {
            ObjectHash::Crc64(hash) => {
                assert_eq!(hash.algorithm(), MULTIHASH_CRC64_NVME);
                assert_eq!(
                    &multibase::encode(multibase::Base::Base64Pad, hash.digest())[1..],
                    "dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA"
                );
            }
            _ => {
                return Err(crate::Error::InvalidMultihash(
                    "Expected ObjectHash::Crc64 variant".to_string(),
                ))
            }
        }

        // Test that serialization produces the correct format
        let hex_bytes =
            hex::decode("7465737464617461000000000000000000000000000000000000000000000000")
                .map_err(|e| Error::InvalidMultihash(e.to_string()))?;
        let sha256_hash =
            Sha256Hash::try_from(multihash::Multihash::wrap(MULTIHASH_SHA256, &hex_bytes)?)?;
        let object_hash = ObjectHash::Sha256(sha256_hash);
        let serialized = serde_json::to_string(&object_hash)?;
        assert!(serialized.contains("\"type\":\"SHA256\""));
        assert!(serialized.contains(
            "\"value\":\"7465737464617461000000000000000000000000000000000000000000000000\""
        ));

        Ok(())
    }

    #[test]
    fn test_object_hash_json_invalid_type() {
        // Test invalid type field
        let invalid_json = r#"{"type":"UNKNOWN","value":"deadbeef"}"#;
        let result: Result<ObjectHash, _> = serde_json::from_str(invalid_json);
        assert!(result.is_err());

        // Test mismatched type/encoding
        let mismatched_json = r#"{"type":"SHA256","value":"dGVzdA=="}"#; // base64 in SHA256 field
        let result: Result<ObjectHash, _> = serde_json::from_str(mismatched_json);
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn test_hash_trait_and_verify_functionality() -> Res {
        let storage = MockStorage::default();
        let test_data = b"test data for Hash trait and verify functionality";
        let test_path = Path::new("hash_trait_test.txt");

        // Write test data to mock storage
        storage.write_file(test_path, test_data).await?;

        // Test Hash trait implementation and consistent from_file signatures
        let file = storage.open_file(test_path).await?;
        let sha256_hash = <Sha256Hash as Hash>::from_file(file).await?;

        let file = storage.open_file(test_path).await?;
        let sha256_chunked_hash = <Sha256ChunkedHash as Hash>::from_file(file).await?;

        let file = storage.open_file(test_path).await?;
        let crc64_hash = <Crc64Hash as Hash>::from_file(file).await?;

        // Test Hash trait methods
        assert_eq!(sha256_hash.algorithm(), MULTIHASH_SHA256);
        assert_eq!(sha256_chunked_hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(crc64_hash.algorithm(), MULTIHASH_CRC64_NVME);

        assert!(!sha256_hash.digest().is_empty());
        assert!(!sha256_chunked_hash.digest().is_empty());
        assert!(!crc64_hash.digest().is_empty());

        // Test trait object polymorphism
        fn check_hash_trait<T: Hash>(hash: &T) -> u64 {
            hash.algorithm()
        }

        assert_eq!(check_hash_trait(&sha256_hash), MULTIHASH_SHA256);
        assert_eq!(
            check_hash_trait(&sha256_chunked_hash),
            MULTIHASH_SHA256_CHUNKED
        );
        assert_eq!(check_hash_trait(&crc64_hash), MULTIHASH_CRC64_NVME);

        // Test that refresh_hash works with all hash types
        let sha256_multihash: Multihash<256> = sha256_hash.into();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: sha256_multihash,
            size: test_data.len() as u64, // Correct size
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await?;
        // Since hash and content match, should return None
        assert!(result.is_none(), "Unchanged file should return None");

        // Test with wrong hash to trigger refresh
        let wrong_hash = multihash::Multihash::wrap(MULTIHASH_SHA256, b"wrong_hash_data").unwrap();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: wrong_hash,
            size: 999, // Wrong size to test that refresh_hash updates it
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await?;
        let refreshed_row = result.expect("Changed hash should return Some");
        assert_eq!(refreshed_row.hash, sha256_multihash); // Should match actual file
        assert_eq!(refreshed_row.size, test_data.len() as u64); // Should be updated

        let sha256_chunked_multihash: Multihash<256> = sha256_chunked_hash.into();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: sha256_chunked_multihash,
            size: test_data.len() as u64, // Correct size
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await?;
        // Since hash and content match, should return None
        assert!(result.is_none(), "Unchanged file should return None");

        // Test with wrong hash to trigger refresh
        let wrong_chunked_hash =
            multihash::Multihash::wrap(MULTIHASH_SHA256_CHUNKED, b"wrong_chunked_data").unwrap();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: wrong_chunked_hash,
            size: 999, // Wrong size to test that refresh_hash updates it
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await?;
        let refreshed_row = result.expect("Changed hash should return Some");
        assert_eq!(refreshed_row.hash, sha256_chunked_multihash); // Should match actual file
        assert_eq!(refreshed_row.size, test_data.len() as u64); // Should be updated

        let crc64_multihash: Multihash<256> = crc64_hash.into();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: crc64_multihash,
            size: test_data.len() as u64, // Correct size
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await?;
        // Since hash and content match, should return None
        assert!(result.is_none(), "Unchanged file should return None");

        // Test with wrong hash to trigger refresh
        let wrong_crc64_hash =
            multihash::Multihash::wrap(MULTIHASH_CRC64_NVME, b"wrong_crc64_data").unwrap();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: wrong_crc64_hash,
            size: 999, // Wrong size to test that refresh_hash updates it
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await?;
        let refreshed_row = result.expect("Changed hash should return Some");
        assert_eq!(refreshed_row.hash, crc64_multihash); // Should match actual file
        assert_eq!(refreshed_row.size, test_data.len() as u64); // Should be updated

        let unknown_hash = multihash::Multihash::wrap(0x9999, b"test_hash_data").unwrap();
        let test_row = Row {
            name: PathBuf::from("test.txt"),
            hash: unknown_hash,
            size: test_data.len() as u64,
            ..Row::default()
        };
        let result = refresh_hash(&storage, &test_path.to_path_buf(), test_row).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Wrong multihash type"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_calculate_hash() -> Res {
        let storage = MockStorage::default();
        let test_data = b"test data for calculate_hash function";
        let test_path = Path::new("calculate_hash_test.txt");

        // Write test data to mock storage
        storage.write_file(test_path, test_data).await?;

        // Test with CRC64 host config
        let crc64_host_config = HostConfig {
            checksums: HostChecksums::Crc64,
            host: None,
        };

        let logical_key = PathBuf::from("test_file.txt");
        let result = calculate_hash(
            &storage,
            &test_path.to_path_buf(),
            logical_key.clone(),
            &crc64_host_config,
        )
        .await?;
        assert_eq!(result.hash.code(), MULTIHASH_CRC64_NVME);
        assert_eq!(result.name, logical_key);
        assert_eq!(result.size, test_data.len() as u64);

        // Test with SHA256-chunked host config
        let sha256_chunked_host_config = HostConfig {
            checksums: HostChecksums::Sha256Chunked,
            host: None,
        };

        let logical_key = PathBuf::from("test_file2.txt");
        let result = calculate_hash(
            &storage,
            &test_path.to_path_buf(),
            logical_key.clone(),
            &sha256_chunked_host_config,
        )
        .await?;
        assert_eq!(result.hash.code(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(result.name, logical_key);
        assert_eq!(result.size, test_data.len() as u64);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_verify_hash() -> Res {
        let storage = MockStorage::default();
        let test_data = b"test data for verify_hash function";
        let test_path = Path::new("verify_hash_test.txt");

        // Write test data to mock storage
        storage.write_file(test_path, test_data).await?;

        // Test with CRC64 host config - file unchanged
        let crc64_host_config = HostConfig {
            checksums: HostChecksums::Crc64,
            host: None,
        };

        let logical_key = PathBuf::from("test_file.txt");

        // Create initial row with correct CRC64 hash
        let initial_row = calculate_hash(
            &storage,
            &test_path.to_path_buf(),
            logical_key.clone(),
            &crc64_host_config,
        )
        .await?;

        // Verify unchanged file returns None
        let result = verify_hash(
            &storage,
            &test_path.to_path_buf(),
            &logical_key,
            initial_row.clone(),
            &crc64_host_config,
        )
        .await?;
        assert!(result.is_none(), "Unchanged file should return None");

        // Test with different host config (SHA256-chunked) - won't trigger recalculation
        // because verify_hash only recalculates if the file has actually changed
        let sha256_host_config = HostConfig {
            checksums: HostChecksums::Sha256Chunked,
            host: None,
        };

        // Use the CRC64 row but with SHA256-chunked host config - file hasn't changed
        let result = verify_hash(
            &storage,
            &test_path.to_path_buf(),
            &logical_key,
            initial_row.clone(),
            &sha256_host_config,
        )
        .await?;
        // Since the file hasn't changed, verify_hash returns None regardless of algorithm mismatch
        assert!(
            result.is_none(),
            "Unchanged file should return None even with different algorithm"
        );

        // Test with modified file content - should return CRC64 hash (matching host config)
        let modified_data = b"modified test data for verify_hash function";
        storage.write_file(test_path, modified_data).await?;

        let result = verify_hash(
            &storage,
            &test_path.to_path_buf(),
            &logical_key,
            initial_row.clone(),
            &crc64_host_config,
        )
        .await?;
        assert!(result.is_some(), "Modified file should return Some");

        let modified_row = result.unwrap();
        assert_eq!(modified_row.hash.code(), MULTIHASH_CRC64_NVME);
        assert_eq!(modified_row.size, modified_data.len() as u64);

        // Test algorithm preference optimization: if refreshed hash already matches host preference
        // Create a row with SHA256-chunked hash, then modify file and verify with CRC64 host config
        let sha256_row = calculate_hash(
            &storage,
            &test_path.to_path_buf(),
            logical_key.clone(),
            &sha256_host_config,
        )
        .await?;

        // Now write different content
        let new_data = b"completely different content for algorithm test";
        storage.write_file(test_path, new_data).await?;

        // Verify with CRC64 host config - should recalculate because algorithms don't match
        let result = verify_hash(
            &storage,
            &test_path.to_path_buf(),
            &logical_key,
            sha256_row,
            &crc64_host_config,
        )
        .await?;
        assert!(
            result.is_some(),
            "Modified file with algorithm mismatch should return Some"
        );

        let final_row = result.unwrap();
        assert_eq!(
            final_row.hash.code(),
            MULTIHASH_CRC64_NVME,
            "Should use host's preferred algorithm"
        );
        assert_eq!(final_row.size, new_data.len() as u64);

        Ok(())
    }
}
