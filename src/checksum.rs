//! This module contains helpers and structs for creating and managing checkums.

mod crc64nvme;
mod sha256;
mod sha256_chunked;

use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;

use crate::Error;
use crate::Res;

// Re-export CRC64-NVMe related items
pub use crc64nvme::{Crc64Hash, MULTIHASH_CRC64_NVME};
// Re-export SHA256 related items
pub use sha256::{Sha256Hash, MULTIHASH_SHA256};
// Re-export SHA256 chunked related items
pub use sha256_chunked::{Sha256ChunkedHash, MULTIHASH_SHA256_CHUNKED};

/// Container for object's checksum
/// You can convert it to or from `Multihash<256>`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum ContentHash {
    /// Legacy checksum
    SHA256(String),
    /// Chunked checksum
    #[serde(rename = "sha2-256-chunked")]
    SHA256Chunked(String),
    /// CRC64-NVMe checksum
    CRC64NVME(String),
}

impl TryFrom<Multihash<256>> for ContentHash {
    type Error = Error;

    fn try_from(value: Multihash<256>) -> Result<Self, Self::Error> {
        match value.code() {
            MULTIHASH_SHA256 => Ok(ContentHash::SHA256(hex::encode(value.digest()))),
            MULTIHASH_SHA256_CHUNKED => Ok(ContentHash::SHA256Chunked(
                BASE64_STANDARD.encode(value.digest()),
            )),
            MULTIHASH_CRC64_NVME => Ok(ContentHash::CRC64NVME(
                BASE64_STANDARD.encode(value.digest()),
            )),
            code => Err(Error::InvalidMultihash(format!(
                "Unexpected code: {code:#06x}"
            ))),
        }
    }
}

impl TryFrom<ContentHash> for Multihash<256> {
    type Error = Error;

    fn try_from(content_hash: ContentHash) -> Result<Self, Self::Error> {
        match content_hash {
            ContentHash::SHA256(hash) => {
                let hash_bytes =
                    hex::decode(hash).map_err(|err| Error::InvalidMultihash(err.to_string()))?;
                Multihash::wrap(MULTIHASH_SHA256, &hash_bytes)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))
            }
            ContentHash::SHA256Chunked(hash) => {
                let hash_bytes = BASE64_STANDARD
                    .decode(hash)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
                Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &hash_bytes)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))
            }
            ContentHash::CRC64NVME(hash) => {
                let hash_bytes = BASE64_STANDARD
                    .decode(hash)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
                Multihash::wrap(MULTIHASH_CRC64_NVME, &hash_bytes)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))
            }
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

/// Caclulates legacy or single-chunk checksum from file or from single chunk
pub async fn sha256<F: AsyncRead + Unpin>(file: F) -> Res<Sha256Hash> {
    let mut sha256 = Sha256::new();
    let mut reader = BufReader::new(file);
    let mut buf = [0; 4096];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        sha256.update(&buf[0..n]);
    }
    Ok(Sha256Hash::try_from(Multihash::wrap(
        MULTIHASH_SHA256,
        &sha256.finalize(),
    )?)?)
}

/// Calculates chunksum from a file
pub async fn sha256_chunked<F: AsyncRead + Unpin + Send>(
    file: F,
    length: u64,
) -> Res<Sha256ChunkedHash> {
    let (chunksize, num_parts) = get_checksum_chunksize_and_parts(length);

    let mut sha256_hasher = Sha256::new();

    let mut chunk = file.take(0);
    for _ in 0..num_parts {
        chunk.set_limit(chunksize);
        sha256_hasher.update(sha256(&mut chunk).await?.digest());
    }

    Ok(Sha256ChunkedHash::try_from(Multihash::wrap(
        MULTIHASH_SHA256_CHUNKED,
        &sha256_hasher.finalize(),
    )?)?)
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
        #[allow(deprecated)]
        return Some(Sha256::digest(checksum_sha256_decoded).as_slice().into());
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

    use crate::fixtures;

    use aws_sdk_s3::types::Checksum;
    use aws_sdk_s3::types::GetObjectAttributesParts;
    use aws_sdk_s3::types::ObjectPart;
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;

    #[tokio::test]
    async fn test_calculate_sha256_checksum() -> Res {
        let bytes = fixtures::objects::less_than_8mb();
        let hash = sha256(bytes).await?;

        assert_eq!(hash.multihash().code(), MULTIHASH_SHA256);

        let double_hash = Sha256::digest(hash.digest());
        assert_eq!(
            hex::encode(double_hash),
            fixtures::objects::LESS_THAN_8MB_HASH_HEX
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_files_less_8mb() -> Res {
        let bytes = fixtures::objects::less_than_8mb();
        let hash = sha256_chunked(bytes, bytes.len() as u64).await?;
        assert_eq!(hash.multihash().code(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            fixtures::objects::LESS_THAN_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_files_equal_to_8mb() -> Res {
        let bytes = fixtures::objects::equal_to_8mb();
        let hash = sha256_chunked(bytes.as_ref(), bytes.len() as u64).await?;
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            fixtures::objects::EQUAL_TO_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_chunked_empty() -> Res {
        let bytes = fixtures::objects::zero_bytes();
        let hash = sha256_chunked(bytes, bytes.len() as u64).await?;
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            fixtures::objects::ZERO_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_files_bigger_than_8mb() -> Res {
        let bytes = fixtures::objects::more_than_8mb();
        let hash = sha256_chunked(bytes.as_ref(), bytes.len() as u64).await?;
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            fixtures::objects::MORE_THAN_8MB_HASH_B64
        );
        Ok(())
    }

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
    fn test_content_hash_try_into_hex_decode_error() {
        let result: Result<Multihash<256>, Error> = ContentHash::SHA256("a".repeat(45)).try_into();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid multihash: Odd number of digits"
        );
    }

    #[test]
    fn test_content_hash_chunked_try_into_hex_decode_error() {
        let result: Result<Multihash<256>, Error> =
            ContentHash::SHA256Chunked("a".repeat(45)).try_into();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid multihash: Invalid input length: 45"
        );
    }

    #[test]
    fn test_content_hash_try_into_multihash_oversized() {
        // Create a hash that's too large (>32 bytes)
        let oversized_hash = "a".repeat(600); // 65 hex chars = 32.5 bytes
        let content_hash = ContentHash::SHA256(oversized_hash);

        let result: Result<Multihash<256>, Error> = content_hash.try_into();
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid multihash: Invalid multihash size 300."
        );
    }

    #[test]
    fn test_content_hash_chunked_try_into_multihash_oversized() -> Res {
        let oversized_hash = "a".repeat(600);
        let content_hash = ContentHash::SHA256Chunked(oversized_hash);
        let result = Multihash::<256>::try_from(content_hash);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid multihash: Invalid multihash size 450."
        );
        Ok(())
    }

    #[test]
    fn test_content_hash_try_from_multihash_invalid_code() -> Res {
        // Create a multihash with an unsupported code
        let digest = [0u8; 32];
        let invalid_code = 0x42; // Some random code that's not SHA256 or SHA256_CHUNKED
        let multihash = Multihash::wrap(invalid_code, &digest)?;

        let result = ContentHash::try_from(multihash);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid multihash: Unexpected code: 0x0042"
        );

        Ok(())
    }

    #[test]
    fn test_get_compliant_chunked_checksum() -> Res {
        fn b64decode(data: &str) -> Result<Vec<u8>, Error> {
            Ok(BASE64_STANDARD.decode(data.as_bytes())?)
        }

        fn sha256(data: Vec<u8>) -> Vec<u8> {
            Sha256::digest(data).to_vec()
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

    #[tokio::test]
    async fn test_sha256_chunked_from_bytes() -> Res {
        let bytes = fixtures::objects::less_than_8mb();
        let hash = sha256_chunked(bytes, bytes.len() as u64).await?;

        assert_eq!(hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            fixtures::objects::LESS_THAN_8MB_HASH_B64
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_sha256_from_bytes() -> Res {
        let bytes = fixtures::objects::less_than_8mb();
        let hash = sha256(&bytes[..]).await?;

        assert_eq!(hash.algorithm(), MULTIHASH_SHA256);
        Ok(())
    }

    #[tokio::test]
    async fn test_conversions() -> Res {
        let bytes = fixtures::objects::less_than_8mb();

        // Test Sha256ChunkedHash conversions
        let sha256_chunked = sha256_chunked(bytes, bytes.len() as u64).await?;
        let multihash: Multihash<256> = sha256_chunked.clone().into();
        let back_to_sha256_chunked = Sha256ChunkedHash::try_from(multihash)?;
        assert_eq!(sha256_chunked, back_to_sha256_chunked);

        // Test Sha256Hash conversions
        let sha256 = sha256(&bytes[..]).await?;
        let multihash: Multihash<256> = sha256.clone().into();
        let back_to_sha256 = Sha256Hash::try_from(multihash)?;
        assert_eq!(sha256, back_to_sha256);

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
}
