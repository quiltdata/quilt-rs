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

    use std::path::PathBuf;
    use tokio::io::AsyncWriteExt;

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

    #[tokio::test]
    async fn test_concurrent_writes() {
        let test_file = PathBuf::from("target/test_concurrent_writes.txt");
        
        // Create two write file descriptors
        let mut fd1 = File::create(&test_file).await.unwrap();
        let mut fd2 = File::create(&test_file).await.unwrap();

        // Write different content through each descriptor
        fd1.write_all(b"first write").await.unwrap();
        fd2.write_all(b"second write").await.unwrap();
        
        // Ensure writes are flushed
        fd1.flush().await.unwrap();
        fd2.flush().await.unwrap();

        // Read the final content
        let mut final_content = Vec::new();
        let mut read_fd = File::open(&test_file).await.unwrap();
        tokio::io::AsyncReadExt::read_to_end(&mut read_fd, &mut final_content).await.unwrap();

        // Verify the content matches the second write
        assert_eq!(final_content, b"second write");
    }
}
