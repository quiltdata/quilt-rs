//! Contains helpers and wrappers to work with IO.

pub mod manifest;
mod parquet;
/// It is public only for documentation and testing
pub mod remote;
/// It is public only for documentation and testing
pub mod storage;

pub use parquet::ParquetWriter;

#[cfg(test)]
mod tests {
    use tokio::fs::File;

    #[tokio::test]
    async fn test_multiple_read_descriptors() {
        // Open two separate file descriptors for reading the same file
        let mut fd1 = File::open("fixtures/manifest.jsonl").await.unwrap();
        let mut fd2 = File::open("fixtures/manifest.jsonl").await.unwrap();

        // Read contents from both descriptors
        let mut contents1 = Vec::new();
        let mut contents2 = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut fd1, &mut contents1).await.unwrap();
        tokio::io::AsyncReadExt::read_to_end(&mut fd2, &mut contents2).await.unwrap();

        // Assert contents match
        assert_eq!(contents1, contents2);
    }
}
