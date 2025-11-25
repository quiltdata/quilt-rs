use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_smithy_checksums::ChecksumAlgorithm;
use aws_smithy_types::base64;

use crate::checksum::sha256_chunked::get_checksum_chunksize_and_parts;
use crate::checksum::sha256_chunked::MULTIPART_THRESHOLD;
use crate::checksum::{ObjectHash, Sha256ChunkedHash};

/// Compute SHA256 hash of the given bytes
fn sha256_hash_bytes(data: &[u8]) -> Vec<u8> {
    let mut hasher = ChecksumAlgorithm::Sha256.into_impl();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Hash a base64-encoded checksum with SHA256 and return as Sha256ChunkedHash
pub fn hash_sha256_checksum(checksum_b64: &str) -> Option<String> {
    let checksum_decoded = base64::decode(checksum_b64).ok()?;
    let hashed_bytes = sha256_hash_bytes(&checksum_decoded);
    Some(base64::encode(&hashed_bytes))
}

fn get_compliant_chunked_checksum(attrs: &GetObjectAttributesOutput) -> Option<String> {
    let checksum = attrs.checksum.as_ref()?;
    let checksum_str_opt = checksum.checksum_sha256.clone();
    let object_size = attrs.object_size.expect("ObjectSize must be requested");
    if (object_size as u64) < MULTIPART_THRESHOLD {
        if let Some(object_parts) = &attrs.object_parts {
            if object_parts
                .total_parts_count
                .expect("ObjectParts is expected to have TotalParts")
                == 1
            {
                return checksum_str_opt;
            }
        }

        return hash_sha256_checksum(&checksum_str_opt?);
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
            return checksum_str_opt;
        }
    }
    None
}

/// Takes checksum got from S3 and convert it to Chunksum.
pub fn get_compliant_checksum(attrs: &GetObjectAttributesOutput) -> Option<ObjectHash> {
    Sha256ChunkedHash::try_from(get_compliant_chunked_checksum(attrs)?.as_str())
        .ok()
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_s3::types::Checksum;
    use aws_sdk_s3::types::GetObjectAttributesParts;
    use aws_sdk_s3::types::ObjectPart;

    use crate::Error;
    use crate::Res;

    #[test]
    fn test_sha256_hash_bytes() {
        let input = b"test data";
        let result = sha256_hash_bytes(input);

        // Should return 32-byte SHA256 hash
        assert_eq!(result.len(), 32);

        // Should be deterministic
        let result2 = sha256_hash_bytes(input);
        assert_eq!(result, result2);
    }

    #[test]
    fn test_hash_sha256_checksum() -> Res {
        let input_checksum = "MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=";
        let result = hash_sha256_checksum(input_checksum);

        // Should return Some(ObjectHash) for valid base64 input
        assert!(result.is_some());

        // Should return None for invalid base64 input
        let invalid_result = hash_sha256_checksum("invalid-base64!");
        assert!(invalid_result.is_none());

        Ok(())
    }

    // Helper functions shared across tests
    fn b64decode(data: &str) -> Result<Vec<u8>, Error> {
        let prefixed_value = format!("{}{}", multibase::Base::Base64Pad.code(), data);
        let (_, decoded) = multibase::decode(&prefixed_value)?;
        Ok(decoded)
    }

    fn sha256(data: Vec<u8>) -> Vec<u8> {
        sha256_hash_bytes(&data)
    }

    fn to_object_hash(data: Vec<u8>) -> Result<ObjectHash, Error> {
        let multibase_encoded = multibase::encode(multibase::Base::Base64Pad, &data);
        let b64_str = &multibase_encoded[1..]; // Remove multibase prefix
        Ok(ObjectHash::Sha256Chunked(Sha256ChunkedHash::try_from(
            b64_str,
        )?))
    }

    #[test]
    fn test_get_compliant_chunked_checksum_no_checksum() {
        let attrs = GetObjectAttributesOutput::builder().build();
        let result = get_compliant_checksum(&attrs);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_compliant_chunked_checksum_sha1_only() {
        let attrs = GetObjectAttributesOutput::builder()
            .checksum(
                Checksum::builder()
                    .checksum_sha1("X94czmA+ZWbSDagRyci8zLBE1K4=")
                    .build(),
            )
            .object_size(1048576) // below the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_compliant_chunked_checksum_small_file_no_parts() -> Res {
        let attrs = GetObjectAttributesOutput::builder()
            .checksum(
                Checksum::builder()
                    .checksum_sha256("MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=")
                    .build(),
            )
            .object_size(1048576) // below the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        let expected = Some(to_object_hash(sha256(b64decode(
            "MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=",
        )?))?);
        assert_eq!(result, expected);
        Ok(())
    }

    #[test]
    fn test_get_compliant_chunked_checksum_small_file_single_part() -> Res {
        let attrs = GetObjectAttributesOutput::builder()
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
            .object_size(5242880) // below the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        let expected = Some(to_object_hash(b64decode(
            "vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=",
        )?)?);
        assert_eq!(result, expected);
        Ok(())
    }

    #[test]
    fn test_get_compliant_chunked_checksum_large_file_no_parts() {
        let attrs = GetObjectAttributesOutput::builder()
            .checksum(
                Checksum::builder()
                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                    .build(),
            )
            .object_size(8388608) // above the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_compliant_chunked_checksum_large_file_single_compliant_part() -> Res {
        let attrs = GetObjectAttributesOutput::builder()
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
            .object_size(8388608) // above the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        let expected = Some(to_object_hash(b64decode(
            "MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=",
        )?)?);
        assert_eq!(result, expected);
        Ok(())
    }

    #[test]
    fn test_get_compliant_chunked_checksum_large_file_non_compliant_parts() {
        let attrs = GetObjectAttributesOutput::builder()
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
                            .size(5242880) // Different sizes - not compliant
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
            .object_size(13631488) // above the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_compliant_chunked_checksum_large_file_compliant_parts() -> Res {
        let attrs = GetObjectAttributesOutput::builder()
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
                            .size(8388608) // Same size for all parts except last - compliant
                            .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                            .build(),
                    )
                    .parts(
                        ObjectPart::builder()
                            .size(5242880) // Last part can be different size
                            .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                            .build(),
                    )
                    .build(),
            )
            .object_size(13631488) // above the threshold
            .build();
        let result = get_compliant_checksum(&attrs);
        let expected = Some(to_object_hash(b64decode(
            "bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=",
        )?)?);
        assert_eq!(result, expected);
        Ok(())
    }
}
