use sha2::{digest::Output, Digest, Sha256};
use tokio::io::{self, AsyncReadExt, BufReader};

use crate::quilt::s3;

pub fn get_checksum_chunksize_and_parts(file_size: u64) -> (u64, u64) {
    let mut chunksize = s3::MULTIPART_THRESHOLD;
    let mut num_parts = file_size.div_ceil(chunksize);

    while num_parts > s3::MPU_MAX_PARTS {
        chunksize *= 2;
        num_parts = file_size.div_ceil(chunksize);
    }

    (chunksize, num_parts)
}

pub async fn calculate_sha256_checksum<F: io::AsyncRead + Unpin>(
    file: F,
) -> io::Result<Output<Sha256>> {
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
    Ok(sha256.finalize())
}

pub async fn calculate_sha256_chunked_checksum<F: io::AsyncRead + Unpin>(
    file: F,
    length: u64,
) -> io::Result<Output<Sha256>> {
    let (chunksize, num_parts) = get_checksum_chunksize_and_parts(length);

    let mut sha256 = Sha256::new();

    let mut chunk = file.take(0);
    for _ in 0..num_parts {
        chunk.set_limit(chunksize);
        let chunk_hash = calculate_sha256_checksum(&mut chunk).await?;
        sha256.update(chunk_hash);
    }

    Ok(sha256.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{prelude::BASE64_STANDARD, Engine};

    #[tokio::test]
    async fn test_sha256() {
        let bytes = "Hello, World!".as_bytes();
        let hash = calculate_sha256_checksum(bytes).await.unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash),
            "3/1gIbsr1bCvZ2KQgJ7DpTGR3YHH9wpLKGiKNiGCmG8="
        );
    }

    #[tokio::test]
    async fn test_sha256_chunked_empty() {
        let bytes: &[u8] = &[];
        let hash = calculate_sha256_chunked_checksum(bytes, bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash),
            "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[tokio::test]
    async fn test_sha256_chunked_long() {
        let bytes = "1234567890abcdefgh".as_bytes().repeat(1024 * 1024);
        let hash = calculate_sha256_chunked_checksum(bytes.as_ref(), bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash),
            "T+rt/HKRJOiAkEGXKvc+DhCwRcrZiDrFkjKonDT1zgs="
        );
    }
}
