use aws_smithy_checksums::ChecksumAlgorithm;
use aws_smithy_types::base64;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
