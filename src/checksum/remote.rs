use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_smithy_checksums::ChecksumAlgorithm;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;

use crate::checksum::sha256_chunked::get_checksum_chunksize_and_parts;
use crate::checksum::sha256_chunked::MULTIPART_THRESHOLD;
use crate::checksum::{ObjectHash, Sha256ChunkedHash, MULTIHASH_SHA256_CHUNKED};
use multihash::Multihash;

// TODO: rename to get_compliant_checksum
/// Takes checksum got from S3 and convert it to Chunksum.
pub fn get_compliant_chunked_checksum(attrs: &GetObjectAttributesOutput) -> Option<ObjectHash> {
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
                return Some(ObjectHash::Sha256Chunked(
                    Sha256ChunkedHash::try_from(
                        Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &checksum_sha256_decoded).ok()?,
                    )
                    .ok()?,
                ));
            }
        }
        let mut hasher = ChecksumAlgorithm::Sha256.into_impl();
        hasher.update(&checksum_sha256_decoded);
        return Some(ObjectHash::Sha256Chunked(
            Sha256ChunkedHash::try_from(
                Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &hasher.finalize()).ok()?,
            )
            .ok()?,
        ));
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
            return Some(ObjectHash::Sha256Chunked(
                Sha256ChunkedHash::try_from(
                    Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &checksum_sha256_decoded).ok()?,
                )
                .ok()?,
            ));
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

    use crate::Error;
    use crate::Res;

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

        fn to_object_hash(data: Vec<u8>) -> Result<ObjectHash, Error> {
            let multihash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &data)?;
            Ok(ObjectHash::Sha256Chunked(Sha256ChunkedHash::try_from(
                multihash,
            )?))
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
                Some(to_object_hash(sha256(b64decode(
                    "MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=",
                )?))?),
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
                Some(to_object_hash(b64decode(
                    "vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=",
                )?)?),
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
                Some(to_object_hash(b64decode(
                    "MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=",
                )?)?),
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
                Some(to_object_hash(b64decode(
                    "bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=",
                )?)?),
            ),
        ];

        for (attrs, expected) in test_data {
            assert_eq!(get_compliant_chunked_checksum(&attrs.build()), expected);
        }
        Ok(())
    }
}
