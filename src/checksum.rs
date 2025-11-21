//! This module contains helpers and structs for creating and managing checkums.

use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_smithy_checksums::ChecksumAlgorithm;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;
use serde::Deserialize;
use serde::Serialize;
use tokio::fs::File;

use crate::Error;
use crate::Res;

mod crc64nvme;
mod sha256;
mod sha256_chunked;

// Re-export CRC64-NVMe related items
pub use crc64nvme::{Crc64Hash, MULTIHASH_CRC64_NVME};
// Re-export SHA256 related items
pub use sha256::{Sha256Hash, MULTIHASH_SHA256};
// Re-export SHA256 chunked related items
pub use sha256_chunked::{Sha256ChunkedHash, MULTIHASH_SHA256_CHUNKED};

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

pub async fn verify_hash(file: File, hash: Multihash<256>) -> Res<Option<(u64, Multihash<256>)>> {
    let file_metadata = file.metadata().await?;
    let size = file_metadata.len();

    let calculated_hash: Res<Multihash<256>> = match hash.code() {
        MULTIHASH_CRC64_NVME => Ok(Crc64Hash::from_async_read(file).await?.into()),
        MULTIHASH_SHA256 => Ok(Sha256Hash::from_async_read(file).await?.into()),
        MULTIHASH_SHA256_CHUNKED => {
            Ok(Sha256ChunkedHash::from_async_read(file, size).await?.into())
        }
        code => Err(Error::InvalidMultihash(format!(
            "Wrong multihash type {}",
            code
        ))),
    };

    let calculated_hash = calculated_hash?;
    Ok((hash != calculated_hash).then_some((size, calculated_hash)))
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

/// Maximum number of parts for splitting the file to create chunksum
/// This is a "hard requirement" for chunksums. We don't outstrip that number of chunks.
pub const MPU_MAX_PARTS: u64 = 10_000;
/// Size threshold when the next chunk cut.
/// This is a "soft requirement" for chunksum size. We can increase threshold if we can't fit into
/// `MPU_MAX_PARTS`.
/// Since it's a minimum size for chunksumed chunk, file less than this threshold is treated like
/// single chunk.
pub const MULTIPART_THRESHOLD: u64 = 8 * 1024 * 1024;

/// Examines if chunksum size is suitable to split file and get less chunks then supported.
/// If not, we tries to increas chunksum until it find chunk size that can split into reasonable
/// number of chunks (`MPU_MAX_PARTS`).
pub fn get_checksum_chunksize_and_parts(file_size: u64) -> (u64, u64) {
    let mut chunksize = MULTIPART_THRESHOLD;
    let mut num_parts = file_size.div_ceil(chunksize);

    while num_parts > MPU_MAX_PARTS {
        chunksize *= 2;
        num_parts = file_size.div_ceil(chunksize);
    }

    (chunksize, num_parts)
}

/// Takes checksum got from S3 and convert it to Chunksum.
pub fn get_compliant_chunked_checksum(attrs: &GetObjectAttributesOutput) -> Option<Vec<u8>> {
    let checksum = attrs.checksum.as_ref()?;
    let checksum_sha256 = checksum.checksum_sha256.as_ref()?;
    // XXX: defer decoding until we know it's compatible?
    let checksum_sha256_decoded = BASE64_STANDARD
        .decode(checksum_sha256.as_bytes())
        .expect("AWS checksum must be valid base64");
    let object_size = attrs.object_size.expect("ObjectSize must be requested");
    if (object_size as u64) < MULTIPART_THRESHOLD {
        if let Some(object_parts) = &attrs.object_parts {
            if object_parts
                .total_parts_count
                .expect("ObjectParts is expected to have TotalParts")
                == 1
            {
                return Some(checksum_sha256_decoded);
            }
        }
        let mut hasher = ChecksumAlgorithm::Sha256.into_impl();
        hasher.update(&checksum_sha256_decoded);
        return Some(hasher.finalize().into());
    } else if let Some(object_parts) = &attrs.object_parts {
        let parts = object_parts.parts();
        // Make sure we requested all parts.
        assert_eq!(
            parts.len(),
            object_parts
                .total_parts_count
                .expect("ObjectParts is expected to have TotalParts") as usize
        );
        let expected_chunk_size = get_checksum_chunksize_and_parts(object_size as u64).0;
        if parts[..parts.len() - 1]
            .iter()
            .all(|p| p.size.expect("Part is expected to have size") as u64 == expected_chunk_size)
        {
            return Some(checksum_sha256_decoded);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Error;
    use crate::Res;

    use aws_sdk_s3::types::Checksum;
    use aws_sdk_s3::types::GetObjectAttributesParts;
    use aws_sdk_s3::types::ObjectPart;

    #[test]
    fn test_get_checksum_chunksize_and_parts() {
        // Test file smaller than threshold
        let (chunksize, parts) = get_checksum_chunksize_and_parts(MULTIPART_THRESHOLD - 1);
        assert_eq!(chunksize, MULTIPART_THRESHOLD);
        assert_eq!(parts, 1);

        // Test file equal to threshold
        let (chunksize, parts) = get_checksum_chunksize_and_parts(MULTIPART_THRESHOLD);
        assert_eq!(chunksize, MULTIPART_THRESHOLD);
        assert_eq!(parts, 1);

        // Test file requiring exactly MPU_MAX_PARTS
        let file_size = MULTIPART_THRESHOLD * MPU_MAX_PARTS;
        let (chunksize, parts) = get_checksum_chunksize_and_parts(file_size);
        assert_eq!(chunksize, MULTIPART_THRESHOLD);
        assert_eq!(parts, MPU_MAX_PARTS);

        // Test file requiring more than MPU_MAX_PARTS at base chunk size
        let file_size = MULTIPART_THRESHOLD * (MPU_MAX_PARTS + 1);
        let (chunksize, parts) = get_checksum_chunksize_and_parts(file_size);
        assert_eq!(chunksize, MULTIPART_THRESHOLD * 2);
        assert_eq!(parts, (MPU_MAX_PARTS / 2) + 1);

        // Test very large file requiring multiple chunk size doublings
        let file_size = MULTIPART_THRESHOLD * MPU_MAX_PARTS * 8;
        let (chunksize, parts) = get_checksum_chunksize_and_parts(file_size);
        assert_eq!(chunksize, MULTIPART_THRESHOLD * 8);
        assert_eq!(parts, MPU_MAX_PARTS);
    }

    #[test]
    fn test_get_compliant_chunked_checksum() -> Res {
        fn b64decode(data: &str) -> Result<Vec<u8>, Error> {
            let prefixed_value = format!("{}{}", multibase::Base::Base64Pad.code(), data);
            let (_, decoded) = multibase::decode(&prefixed_value)?;
            Ok(decoded)
        }

        fn sha256(data: Vec<u8>) -> Vec<u8> {
            let mut hasher = ChecksumAlgorithm::Sha256.into_impl();
            hasher.update(&data);
            hasher.finalize().to_vec()
        }

        let builder = GetObjectAttributesOutput::builder;
        let test_data = [
            (builder(), None),
            (
                builder().checksum(
                    Checksum::builder()
                        .checksum_sha1("X94czmA+ZWbSDagRyci8zLBE1K4=")
                        .build(),
                ),
                None,
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=")
                            .build(),
                    )
                    .object_size(1048576), // below the threshold
                Some(sha256(b64decode(
                    "MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=",
                )?)),
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(1)
                            .parts(
                                ObjectPart::builder()
                                    .size(5242880)
                                    .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(5242880), // below the threshold
                Some(b64decode("vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=")?),
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                            .build(),
                    )
                    .object_size(8388608), // above the threshold
                None,
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(1)
                            .parts(
                                ObjectPart::builder()
                                    .size(8388608)
                                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(8388608), // above the threshold
                Some(b64decode("MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=")?),
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("nlR6I2vcFqpTXrJSmMglmCYoByajfADbDQ6kESbPIlE=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(2)
                            .parts(
                                ObjectPart::builder()
                                    .size(5242880)
                                    .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                                    .build(),
                            )
                            .parts(
                                ObjectPart::builder()
                                    .size(8388608)
                                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(13631488), // above the threshold
                None,
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(2)
                            .parts(
                                ObjectPart::builder()
                                    .size(8388608)
                                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                                    .build(),
                            )
                            .parts(
                                ObjectPart::builder()
                                    .size(5242880)
                                    .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(13631488), // above the threshold
                Some(b64decode("bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=")?),
            ),
        ];

        for (attrs, expected) in test_data {
            assert_eq!(get_compliant_chunked_checksum(&attrs.build()), expected);
        }
        Ok(())
    }

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
}
