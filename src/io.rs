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
        let fd1 = File::open("fixtures/manifest.jsonl").await.unwrap();
        let fd2 = File::open("fixtures/manifest.jsonl").await.unwrap();

        // If we got here without errors, the test passes
        // The descriptors will be automatically closed when dropped
    }
}
