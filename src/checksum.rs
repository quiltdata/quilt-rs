//! This module contains helpers and structs for creating and managing checkums.

use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;
use tokio::io::{self};

use crate::Error;
use crate::Res;

// TODO: Introduce struct Chunksum {}, that
//       * wraps `Multihash`
//       * can be converted `Chunksum::from(GetObjectAttributesOutput)`

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
}

/// Multihash code for legacy or single-chunked checksums
pub const MULTIHASH_SHA256: u64 = 0x16;
/// Multihash code for chunksums
pub const MULTIHASH_SHA256_CHUNKED: u64 = 0xb510;

impl TryFrom<Multihash<256>> for ContentHash {
    type Error = Error;

    fn try_from(value: Multihash<256>) -> Result<Self, Self::Error> {
        match value.code() {
            MULTIHASH_SHA256 => Ok(ContentHash::SHA256(hex::encode(value.digest()))),
            MULTIHASH_SHA256_CHUNKED => Ok(ContentHash::SHA256Chunked(
                BASE64_STANDARD.encode(value.digest()),
            )),
            code => Err(Error::InvalidMultihash(format!(
                "Unexpected code: {:#06x}",
                code
            ))),
        }
    }
}

impl TryInto<Multihash<256>> for ContentHash {
    type Error = Error;

    fn try_into(self) -> Result<Multihash<256>, Self::Error> {
        match self {
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
pub async fn calculate_sha256_checksum<F: io::AsyncRead + Unpin>(file: F) -> Res<Multihash<256>> {
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
    Ok(Multihash::wrap(MULTIHASH_SHA256, &sha256.finalize())?)
}

/// Calculates chunksum from a file
pub async fn calculate_sha256_chunked_checksum<F: io::AsyncRead + Unpin>(
    file: F,
    length: u64,
) -> Res<Multihash<256>> {
    let (chunksize, num_parts) = get_checksum_chunksize_and_parts(length);

    let mut sha256 = Sha256::new();

    let mut chunk = file.take(0);
    for _ in 0..num_parts {
        chunk.set_limit(chunksize);
        let chunk_hash = calculate_sha256_checksum(&mut chunk).await?;
        sha256.update(chunk_hash.digest());
    }

    Ok(Multihash::wrap(
        MULTIHASH_SHA256_CHUNKED,
        &sha256.finalize(),
    )?)
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

    use aws_sdk_s3::types::Checksum;
    use aws_sdk_s3::types::GetObjectAttributesParts;
    use aws_sdk_s3::types::ObjectPart;
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;

    #[tokio::test]
    async fn test_files_less_8mb() {
        let bytes = "0123456789abcdef".as_bytes();
        let hash = calculate_sha256_chunked_checksum(bytes, bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(hash.code(), MULTIHASH_SHA256_CHUNKED);
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            "Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA="
        );
    }

    #[tokio::test]
    async fn test_files_equal_to_8mb() {
        let bytes = "12345678".as_bytes().repeat(1024 * 1024);
        let hash = calculate_sha256_chunked_checksum(bytes.as_ref(), bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            "7V3rZ3Q/AmAYax2wsQBZbc7N1EMIxlxRyMiMthGRdwg="
        );
    }

    #[tokio::test]
    async fn test_sha256_chunked_empty() {
        let bytes: &[u8] = &[];
        let hash = calculate_sha256_chunked_checksum(bytes, bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[tokio::test]
    async fn test_files_bigger_then_8mb() {
        let bytes = "1234567890abcdefgh".as_bytes().repeat(1024 * 1024);
        let hash = calculate_sha256_chunked_checksum(bytes.as_ref(), bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash.digest()),
            "T+rt/HKRJOiAkEGXKvc+DhCwRcrZiDrFkjKonDT1zgs="
        );
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
        assert_eq!(parts, (MPU_MAX_PARTS + 1) / 2);

        // Test very large file requiring multiple chunk size doublings
        let file_size = MULTIPART_THRESHOLD * MPU_MAX_PARTS * 8;
        let (chunksize, parts) = get_checksum_chunksize_and_parts(file_size);
        assert_eq!(chunksize, MULTIPART_THRESHOLD * 8);
        assert_eq!(parts, MPU_MAX_PARTS);
    }

    #[test]
    fn test_get_compliant_chunked_checksum() {
        fn b64decode(data: &str) -> Vec<u8> {
            BASE64_STANDARD.decode(data.as_bytes()).unwrap()
        }

        fn sha256(data: Vec<u8>) -> Vec<u8> {
            Sha256::digest(data).as_slice().into()
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
                ))),
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
                Some(b64decode("vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=")),
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
                Some(b64decode("MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=")),
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
                Some(b64decode("bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=")),
            ),
        ];

        for (attrs, expected) in test_data {
            assert_eq!(get_compliant_chunked_checksum(&attrs.build()), expected);
        }
    }
}
