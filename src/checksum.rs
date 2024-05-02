use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;
use serde::Deserialize;
use serde::Serialize;
use sha2::digest::Output;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;
use tokio::io::{self};

use crate::Error;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum ContentHash {
    SHA256(String),
    #[serde(rename = "sha2-256-chunked")]
    SHA256Chunked(String),
}

pub const MULTIHASH_SHA256: u64 = 0x16;
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

pub const MPU_MAX_PARTS: u64 = 10_000;
pub const MULTIPART_THRESHOLD: u64 = 8 * 1024 * 1024;

pub fn get_checksum_chunksize_and_parts(file_size: u64) -> (u64, u64) {
    let mut chunksize = MULTIPART_THRESHOLD;
    let mut num_parts = file_size.div_ceil(chunksize);

    while num_parts > MPU_MAX_PARTS {
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
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;

    #[tokio::test]
    async fn test_sha256() {
        let bytes = "0123456789abcdef".as_bytes();
        let hash = calculate_sha256_chunked_checksum(bytes, bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash),
            "Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA="
        );
    }

    #[tokio::test]
    async fn test_edge_case() {
        let bytes = "12345678".as_bytes().repeat(1024 * 1024);
        let hash = calculate_sha256_chunked_checksum(bytes.as_ref(), bytes.len() as u64)
            .await
            .unwrap();
        assert_eq!(
            BASE64_STANDARD.encode(hash),
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
